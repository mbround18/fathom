//! T076: the runner's client side against an in-process Keel listener over loopback TCP.
//!
//! - A server host certificate from another CA, a bare host key or an expired host certificate
//!   is refused before anything is sent (no authentication attempt, no frame).
//! - An enrollment token whose fingerprint names no CA in `host_ca.pub` is refused before the
//!   runner even connects.
//! - With an expired runner certificate (within the server's offline window) the runner sends
//!   only `RenewCert`, and saves the certificate it gets back.
//! - Enrollment saves the certificate and host id and deletes the token; a session sends
//!   `Hello` with the executor's ledger and relays a run both ways unchanged.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use keel_exec::ca::{
    CaKind, CertRefusal, HostCertRequest, RunnerCertRequest, RunnerCertVerifier, RunnerIdentity,
    SshCa, TrustedCa,
};
use keel_exec::client_config::ServerKeyRefusal;
use keel_exec::codec::{read_frame, write_frame};
use keel_exec::frames::{Base, Frame};
use keel_exec::krl::Krl;
use keel_exec::server_config::{
    Access, BareKey, KeelSshHandler, RunnerAuthority, SubsystemService, server_config,
};
use keel_runner::runner::{CERT_FILE, HOST_ID_FILE, Runner, RunnerConfig, RunnerError, TOKEN_FILE};
use russh::ChannelStream;
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, Certificate, PrivateKey};
use russh::server::Msg;
use tokio::net::{TcpListener, UnixListener};
use tokio::sync::{Mutex, mpsc};

const SERVER_NAME: &str = "localhost";
const INVENTORY_NAME: &str = "web-1";
const DAY: Duration = Duration::from_secs(24 * 3600);
const WAIT: Duration = Duration::from_secs(20);

fn key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()
}

fn host_cert(ca: &SshCa, host_key: &PrivateKey, now: SystemTime) -> Certificate {
    ca.issue_host_cert(&HostCertRequest {
        host_names: &[SERVER_NAME.to_owned()],
        public_key: host_key.public_key().key_data(),
        ttl: DAY,
        now,
    })
    .unwrap()
}

fn runner_cert(ca: &SshCa, public: &KeyData, now: SystemTime) -> Certificate {
    let profiles = vec!["base".to_owned()];
    ca.issue_runner_cert(&RunnerCertRequest {
        runner_id: "r-1",
        inventory_name: INVENTORY_NAME,
        assigned_profiles: &profiles,
        allowed_profiles: &profiles,
        public_key: public,
        ttl: DAY,
        now,
    })
    .unwrap()
}

struct Pki {
    host_ca: SshCa,
    runner_ca: SshCa,
}

impl Pki {
    fn new() -> Self {
        Pki {
            host_ca: SshCa::generate(CaKind::Host).unwrap(),
            runner_ca: SshCa::generate(CaKind::Runner).unwrap(),
        }
    }
}

/// A host's trust and state directories.
struct Host {
    _dir: tempfile::TempDir,
    trust: PathBuf,
    state: PathBuf,
    socket: PathBuf,
}

impl Host {
    fn new(pki: &Pki) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let trust = dir.path().join("trust");
        let state = dir.path().join("state");
        std::fs::create_dir_all(&trust).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(
            trust.join("policy.toml"),
            format!(
                "origin = \"https://keel.test\"\nrp_id = \"keel.test\"\n\
                 inventory_name = \"{INVENTORY_NAME}\"\nallowed_profiles = [\"base\"]\n"
            ),
        )
        .unwrap();
        std::fs::write(
            trust.join("host_ca.pub"),
            format!("{}\n", pki.host_ca.cert_authority_line(SERVER_NAME)),
        )
        .unwrap();
        let socket = dir.path().join("executor.sock");
        Host {
            _dir: dir,
            trust,
            state,
            socket,
        }
    }

    fn runner(&self, port: u16) -> Runner {
        Runner::load(RunnerConfig {
            server: format!("{SERVER_NAME}:{port}"),
            trust_dir: self.trust.clone(),
            state_dir: self.state.clone(),
            executor_socket: self.socket.clone(),
            venv: self.state.join("no-venv"),
        })
        .unwrap()
    }

    fn write_token(&self, fingerprint: &str) {
        std::fs::write(
            self.state.join(TOKEN_FILE),
            format!("keel-enroll-v1.0123456789abcdef.{fingerprint}\n"),
        )
        .unwrap();
    }

    fn write_cert(&self, cert: &Certificate) {
        std::fs::write(
            self.state.join(CERT_FILE),
            format!("{}\n", cert.to_openssh().unwrap()),
        )
        .unwrap();
        std::fs::write(self.state.join(HOST_ID_FILE), "h-1\n").unwrap();
    }
}

/// Counts every authentication decision the server makes.
struct TestAuthority {
    verifier: Arc<RunnerCertVerifier>,
    enrolled: Vec<KeyData>,
    auth_calls: Arc<AtomicUsize>,
}

impl RunnerAuthority for TestAuthority {
    fn verifier(&self) -> Arc<RunnerCertVerifier> {
        Arc::clone(&self.verifier)
    }

    async fn is_enrolled(&self, _identity: &RunnerIdentity, key: &KeyData) -> bool {
        self.auth_calls.fetch_add(1, Ordering::SeqCst);
        self.enrolled.contains(key)
    }

    async fn bare_key(&self, key: &KeyData, _now: SystemTime) -> BareKey {
        self.auth_calls.fetch_add(1, Ordering::SeqCst);
        if self.enrolled.contains(key) {
            BareKey::Renewable {
                host_id: "h-1".to_owned(),
            }
        } else {
            BareKey::Unknown
        }
    }
}

/// Plays Keel's side: answers `Enroll` with `Enrolled`, `RenewCert` with `Cert`, and on a full
/// session sends `assign` after `Hello` and stops after `Finished`. Records every frame with
/// its access and whether that access permits it; stops at the first refused frame.
struct TestService {
    seen: mpsc::UnboundedSender<(Access, Frame, bool)>,
    cert: Mutex<Option<String>>,
    assign: Vec<Frame>,
}

impl SubsystemService for TestService {
    async fn serve(&self, access: Access, mut stream: ChannelStream<Msg>) {
        while let Ok(Some(frame)) = read_frame(&mut stream).await {
            let permitted = access.permits(&frame);
            let _ = self.seen.send((access.clone(), frame.clone(), permitted));
            if !permitted {
                return;
            }
            let cert = self.cert.lock().await.clone().unwrap_or_default();
            let answer = match frame {
                Frame::Enroll { .. } => vec![Frame::Enrolled {
                    host_id: "h-1".to_owned(),
                    cert,
                }],
                Frame::RenewCert {} => vec![Frame::Cert { cert }],
                Frame::Hello { .. } => self.assign.clone(),
                Frame::Finished { .. } => return,
                _ => vec![],
            };
            for f in answer {
                if write_frame(&mut stream, &f).await.is_err() {
                    return;
                }
            }
        }
    }
}

struct Server {
    port: u16,
    seen: mpsc::UnboundedReceiver<(Access, Frame, bool)>,
    auth_calls: Arc<AtomicUsize>,
    service: Arc<TestService>,
}

impl Server {
    /// Every frame seen so far.
    fn frames(&mut self) -> Vec<(Access, Frame, bool)> {
        let mut out = vec![];
        while let Ok(f) = self.seen.try_recv() {
            out.push(f);
        }
        out
    }
}

async fn serve(
    host_key: PrivateKey,
    host_cert: Option<Certificate>,
    pki: &Pki,
    enrolled: Vec<KeyData>,
    assign: Vec<Frame>,
) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut config = server_config(host_key, host_cert);
    config.auth_rejection_time = Duration::from_millis(1);
    let config = Arc::new(config);
    let verifier = Arc::new(RunnerCertVerifier::new(
        vec![TrustedCa::active(pki.runner_ca.key_data().clone())],
        Krl::default(),
    ));
    let auth_calls = Arc::new(AtomicUsize::new(0));
    let authority = Arc::new(TestAuthority {
        verifier,
        enrolled,
        auth_calls: Arc::clone(&auth_calls),
    });
    let (tx, seen) = mpsc::unbounded_channel();
    let service = Arc::new(TestService {
        seen: tx,
        cert: Mutex::new(None),
        assign,
    });
    let svc = Arc::clone(&service);
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let handler = KeelSshHandler::new(Arc::clone(&authority), Arc::clone(&svc));
            let config = Arc::clone(&config);
            tokio::spawn(async move {
                if let Ok(session) = russh::server::run_stream(config, socket, handler).await {
                    let _ = session.await;
                }
            });
        }
    });
    Server {
        port,
        seen,
        auth_calls,
        service,
    }
}

async fn good_server(pki: &Pki, enrolled: Vec<KeyData>, assign: Vec<Frame>) -> Server {
    let host_key = key();
    let cert = host_cert(&pki.host_ca, &host_key, SystemTime::now());
    serve(host_key, Some(cert), pki, enrolled, assign).await
}

fn cert_on_disk(state: &Path) -> String {
    std::fs::read_to_string(state.join(CERT_FILE))
        .unwrap()
        .trim()
        .to_owned()
}

#[tokio::test]
async fn the_runner_refuses_an_untrusted_server_before_sending_anything() {
    let pki = Pki::new();
    let rogue = SshCa::generate(CaKind::Host).unwrap();
    let past = SystemTime::now() - 3 * DAY;

    let host_key_a = key();
    let host_key_c = key();
    let cases: Vec<(&str, PrivateKey, Option<Certificate>, ServerKeyRefusal)> = vec![
        (
            "another CA",
            host_key_a.clone(),
            Some(host_cert(&rogue, &host_key_a, SystemTime::now())),
            ServerKeyRefusal::Certificate(CertRefusal::UntrustedCa),
        ),
        ("a bare host key", key(), None, ServerKeyRefusal::BareKey),
        (
            "an expired host certificate",
            host_key_c.clone(),
            Some(host_cert(&pki.host_ca, &host_key_c, past)),
            ServerKeyRefusal::Certificate(CertRefusal::Expired),
        ),
    ];

    for (what, host_key, cert, want) in cases {
        let mut server = serve(host_key, cert, &pki, vec![], vec![]).await;
        let host = Host::new(&pki);
        let ca = pki.host_ca.fingerprint();
        host.write_token(&ca);
        let runner = host.runner(server.port);
        let err = tokio::time::timeout(WAIT, runner.enroll_if_needed())
            .await
            .expect("hung")
            .expect_err(what);
        match err {
            RunnerError::ServerRefused(got) => assert_eq!(got, want, "{what}"),
            other => panic!("{what}: {other}"),
        }
        assert_eq!(server.auth_calls.load(Ordering::SeqCst), 0, "{what}: auth");
        assert!(server.frames().is_empty(), "{what}: frames");
        assert!(host.state.join(TOKEN_FILE).exists(), "{what}: token kept");

        // The same server refused on the renewal and session paths too.
        let past_cert = runner_cert(&pki.runner_ca, runner.public_key(), past);
        host.write_cert(&past_cert);
        let err = tokio::time::timeout(WAIT, runner.renew_expired())
            .await
            .expect("hung")
            .expect_err(what);
        assert!(
            matches!(err, RunnerError::ServerRefused(_)),
            "{what}: {err}"
        );
        assert_eq!(server.auth_calls.load(Ordering::SeqCst), 0, "{what}: auth");
        assert!(server.frames().is_empty(), "{what}: frames");
    }
}

#[tokio::test]
async fn a_token_for_another_ca_is_refused_before_connecting() {
    let pki = Pki::new();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let host = Host::new(&pki);
    let other = SshCa::generate(CaKind::Host).unwrap();
    host.write_token(&other.fingerprint());
    let runner = host.runner(port);

    let err = runner.enroll_if_needed().await.unwrap_err();
    assert!(matches!(err, RunnerError::TokenCaMismatch), "{err}");
    assert!(
        tokio::time::timeout(Duration::from_millis(300), listener.accept())
            .await
            .is_err(),
        "the runner connected"
    );
    assert!(host.state.join(TOKEN_FILE).exists());
    assert!(!host.state.join(CERT_FILE).exists());

    // A malformed token is refused the same way.
    std::fs::write(host.state.join(TOKEN_FILE), "not-a-token\n").unwrap();
    assert!(matches!(
        runner.enroll_if_needed().await.unwrap_err(),
        RunnerError::Trust(_)
    ));
    // No token and no certificate: not enrolled.
    std::fs::remove_file(host.state.join(TOKEN_FILE)).unwrap();
    assert!(matches!(
        runner.enroll_if_needed().await.unwrap_err(),
        RunnerError::NotEnrolled
    ));
}

#[tokio::test]
async fn enrollment_saves_the_certificate_and_deletes_the_token() {
    let pki = Pki::new();
    let mut server = good_server(&pki, vec![], vec![]).await;
    let host = Host::new(&pki);
    host.write_token(&pki.host_ca.fingerprint());
    let runner = host.runner(server.port);
    let issued = runner_cert(&pki.runner_ca, runner.public_key(), SystemTime::now());
    *server.service.cert.lock().await = Some(issued.to_openssh().unwrap());

    let enrolled = tokio::time::timeout(WAIT, runner.enroll_if_needed())
        .await
        .expect("hung")
        .unwrap();
    assert!(enrolled);
    assert!(!host.state.join(TOKEN_FILE).exists(), "token deleted");
    assert_eq!(cert_on_disk(&host.state), issued.to_openssh().unwrap());
    assert_eq!(
        std::fs::read_to_string(host.state.join(HOST_ID_FILE)).unwrap(),
        "h-1\n"
    );
    let frames = server.frames();
    assert_eq!(frames.len(), 1);
    let (access, frame, permitted) = &frames[0];
    assert_eq!(*access, Access::EnrollOnly);
    assert!(*permitted);
    match frame {
        Frame::Enroll {
            token,
            inventory_name,
            allowed_profiles,
            require_signed,
        } => {
            assert!(token.expose().starts_with("keel-enroll-v1."));
            assert_eq!(inventory_name, INVENTORY_NAME);
            assert_eq!(allowed_profiles, &["base".to_owned()]);
            assert!(!require_signed);
        }
        other => panic!("{other:?}"),
    }
    // Enrolled once: no second enrollment.
    assert!(!runner.enroll_if_needed().await.unwrap());
}

#[tokio::test]
async fn an_enrollment_answer_for_another_key_is_refused() {
    let pki = Pki::new();
    let server = good_server(&pki, vec![], vec![]).await;
    let host = Host::new(&pki);
    host.write_token(&pki.host_ca.fingerprint());
    let runner = host.runner(server.port);
    let stranger = key();
    let issued = runner_cert(
        &pki.runner_ca,
        stranger.public_key().key_data(),
        SystemTime::now(),
    );
    *server.service.cert.lock().await = Some(issued.to_openssh().unwrap());

    let err = tokio::time::timeout(WAIT, runner.enroll_if_needed())
        .await
        .expect("hung")
        .unwrap_err();
    assert!(matches!(err, RunnerError::BadCert(_)), "{err}");
    assert!(host.state.join(TOKEN_FILE).exists(), "token kept");
    assert!(!host.state.join(CERT_FILE).exists());
}

#[tokio::test]
async fn an_expired_runner_certificate_sends_only_renew_cert() {
    let pki = Pki::new();
    let host = Host::new(&pki);
    // Load once to create the key, then start a server that knows it.
    let key_data = host.runner(1).public_key().clone();
    let mut server = good_server(&pki, vec![key_data.clone()], vec![]).await;
    let runner = host.runner(server.port);
    let expired = runner_cert(&pki.runner_ca, &key_data, SystemTime::now() - 3 * DAY);
    host.write_cert(&expired);
    let fresh = runner_cert(&pki.runner_ca, &key_data, SystemTime::now());
    *server.service.cert.lock().await = Some(fresh.to_openssh().unwrap());

    // run_once renews, then fails to reach the (absent) executor before any session.
    let err = tokio::time::timeout(WAIT, runner.run_once())
        .await
        .expect("hung")
        .unwrap_err();
    assert!(matches!(err, RunnerError::Executor(_)), "{err}");

    let frames = server.frames();
    assert_eq!(frames.len(), 1, "{frames:?}");
    let (access, frame, permitted) = &frames[0];
    assert_eq!(
        *access,
        Access::RenewOnly {
            host_id: "h-1".to_owned()
        }
    );
    assert_eq!(*frame, Frame::RenewCert {});
    assert!(*permitted);
    assert_eq!(cert_on_disk(&host.state), fresh.to_openssh().unwrap());
}

/// A fake executor: answers `ReadLedger`, and for each `Assign` sends `Started` and `Finished`
/// after it has seen the run's `Secrets`. Reports what it received.
async fn fake_executor(socket: &Path) -> mpsc::UnboundedReceiver<Frame> {
    let listener = UnixListener::bind(socket).unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let tx = tx.clone();
            tokio::spawn(async move {
                while let Ok(Some(frame)) = read_frame(&mut s).await {
                    let _ = tx.send(frame.clone());
                    let answer = match frame {
                        Frame::ReadLedger => vec![Frame::Ledger {
                            ledger_head: Some("ab".repeat(32)),
                            bases: vec![Base {
                                job: "site".to_owned(),
                                commit: "c".repeat(40),
                            }],
                        }],
                        Frame::Secrets { run_id, .. } => vec![
                            Frame::Started {
                                run_id: run_id.clone(),
                            },
                            Frame::Finished {
                                run_id,
                                exit_code: 0,
                                stats: Default::default(),
                                ledger_line: String::new(),
                            },
                        ],
                        _ => vec![],
                    };
                    for f in answer {
                        let _ = write_frame(&mut s, &f).await;
                    }
                }
            });
        }
    });
    rx
}

#[tokio::test]
async fn a_session_says_hello_and_relays_a_run_unchanged() {
    let pki = Pki::new();
    let host = Host::new(&pki);
    let key_data = host.runner(1).public_key().clone();
    let assign = Frame::Assign {
        run_id: "run-1".to_owned(),
        job: "site".to_owned(),
        base: None,
        target: INVENTORY_NAME.to_owned(),
        deadline: 4_000_000_000,
        pack_size: 0,
    };
    let secrets = Frame::Secrets {
        run_id: "run-1".to_owned(),
        items: vec![],
    };
    let stray = Frame::Cancel {
        run_id: "no-such-run".to_owned(),
    };
    let mut server = good_server(
        &pki,
        vec![key_data.clone()],
        vec![assign.clone(), stray, secrets.clone()],
    )
    .await;
    let mut exec = fake_executor(&host.socket).await;
    host.write_cert(&runner_cert(&pki.runner_ca, &key_data, SystemTime::now()));
    let runner = host.runner(server.port);

    tokio::time::timeout(WAIT, runner.run_once())
        .await
        .expect("hung")
        .unwrap();

    let frames: Vec<Frame> = server
        .frames()
        .into_iter()
        .map(|(access, f, permitted)| {
            assert!(matches!(access, Access::Runner(_)));
            assert!(permitted, "{f:?}");
            f
        })
        .collect();
    let names: Vec<&str> = frames.iter().map(Frame::name).collect();
    assert_eq!(names, ["Hello", "Started", "Finished"], "{frames:?}");
    match &frames[0] {
        Frame::Hello {
            host_id,
            ledger_head,
            bases,
            allowed_profiles,
            ansible_core,
            ..
        } => {
            assert_eq!(host_id, "h-1");
            assert_eq!(ledger_head.as_deref(), Some("ab".repeat(32).as_str()));
            assert_eq!(bases.len(), 1);
            assert_eq!(allowed_profiles, &["base".to_owned()]);
            assert_eq!(ansible_core, "unknown");
        }
        other => panic!("{other:?}"),
    }

    let mut got = vec![];
    while let Ok(f) = exec.try_recv() {
        got.push(f);
    }
    assert_eq!(got, vec![Frame::ReadLedger, assign, secrets]);
}

#[test]
fn the_host_key_is_created_private_and_a_readable_one_is_refused() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let key = keel_runner::runner::load_or_create_key(dir.path()).unwrap();
    let path = dir.path().join("id_ed25519");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let again = keel_runner::runner::load_or_create_key(dir.path()).unwrap();
    assert_eq!(key.public_key(), again.public_key());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    assert!(keel_runner::runner::load_or_create_key(dir.path()).is_err());
}
