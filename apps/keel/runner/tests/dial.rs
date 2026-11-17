//! Spec 067 T015 (and T062's runner half): the runner's dial list.
//!
//! - With `dial`, the runner tries each address in order and uses the first whose host
//!   certificate verifies for `--server`'s host name, never the dialled address.
//! - An address that answers with anything else (another CA, a certificate for the dialled name
//!   or the relay's own name, a bare key) is skipped, and gets nothing: no authentication, no
//!   frame, no enrollment token.
//! - One failing pass tries every address once; the reconnect backoff is applied per pass,
//!   doubles from 1 s and is capped at 60 s.
//! - With no `dial`, the runner dials `--server` as before.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use keel_exec::ca::{
    CaKind, HostCertRequest, RunnerCertRequest, RunnerCertVerifier, RunnerIdentity, SshCa,
    TrustedCa,
};
use keel_exec::codec::{read_frame, write_frame};
use keel_exec::frames::Frame;
use keel_exec::krl::Krl;
use keel_exec::server_config::{
    Access, BareKey, KeelSshHandler, RunnerAuthority, SubsystemService, server_config,
};
use keel_runner::runner::{
    Backoff, CERT_FILE, HOST_ID_FILE, Runner, RunnerConfig, RunnerError, TOKEN_FILE,
};
use russh::ChannelStream;
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, Certificate, PrivateKey};
use russh::server::Msg;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, mpsc};

/// The name Keel's host certificate carries, and the host part of `--server`.
const SERVER_NAME: &str = "localhost";
const INVENTORY_NAME: &str = "web-1";
const DAY: Duration = Duration::from_secs(24 * 3600);
const WAIT: Duration = Duration::from_secs(30);

fn key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()
}

fn host_cert_for(ca: &SshCa, host_key: &PrivateKey, name: &str) -> Certificate {
    ca.issue_host_cert(&HostCertRequest {
        host_names: &[name.to_owned()],
        public_key: host_key.public_key().key_data(),
        ttl: DAY,
        now: SystemTime::now(),
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

    fn config(&self, server: String, dial: Vec<String>) -> RunnerConfig {
        RunnerConfig {
            server,
            dial,
            trust_dir: self.trust.clone(),
            state_dir: self.state.clone(),
            executor_socket: self.socket.clone(),
            venv: self.state.join("no-venv"),
        }
    }

    fn runner(&self, server: String, dial: Vec<String>) -> Runner {
        Runner::load(self.config(server, dial))
            .unwrap()
            .with_age_recipient_file(self.state.join("age.pub"))
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

/// Answers `Enroll` with `Enrolled` and `RenewCert` with `Cert`; records every frame.
struct TestService {
    seen: mpsc::UnboundedSender<Frame>,
    cert: Mutex<Option<String>>,
}

impl SubsystemService for TestService {
    async fn serve(&self, _access: Access, mut stream: ChannelStream<Msg>) {
        while let Ok(Some(frame)) = read_frame(&mut stream).await {
            let _ = self.seen.send(frame.clone());
            let cert = self.cert.lock().await.clone().unwrap_or_default();
            let answer = match frame {
                Frame::Enroll { .. } => Frame::Enrolled {
                    host_id: "h-1".to_owned(),
                    cert,
                },
                Frame::RenewCert {} => Frame::Cert { cert },
                _ => return,
            };
            if write_frame(&mut stream, &answer).await.is_err() {
                return;
            }
        }
    }
}

/// An SSH server presenting `host_key` (and `host_cert`), counting TCP accepts, authentication
/// decisions and frames.
struct Server {
    port: u16,
    accepts: Arc<AtomicUsize>,
    auth_calls: Arc<AtomicUsize>,
    seen: mpsc::UnboundedReceiver<Frame>,
    service: Arc<TestService>,
}

impl Server {
    fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    fn frames(&mut self) -> Vec<Frame> {
        let mut out = vec![];
        while let Ok(f) = self.seen.try_recv() {
            out.push(f);
        }
        out
    }

    /// Nothing that matters reached this server: no authentication, no frame.
    fn got_nothing(&mut self, what: &str) {
        assert_eq!(self.auth_calls.load(Ordering::SeqCst), 0, "{what}: auth");
        assert!(self.frames().is_empty(), "{what}: frames");
    }
}

async fn serve(
    host_key: PrivateKey,
    host_cert: Option<Certificate>,
    pki: &Pki,
    enrolled: Vec<KeyData>,
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
    let accepts = Arc::new(AtomicUsize::new(0));
    let authority = Arc::new(TestAuthority {
        verifier,
        enrolled,
        auth_calls: Arc::clone(&auth_calls),
    });
    let (tx, seen) = mpsc::unbounded_channel();
    let service = Arc::new(TestService {
        seen: tx,
        cert: Mutex::new(None),
    });
    let svc = Arc::clone(&service);
    let counted = Arc::clone(&accepts);
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            counted.fetch_add(1, Ordering::SeqCst);
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
        accepts,
        auth_calls,
        seen,
        service,
    }
}

/// Keel: a host certificate from the trusted CA for `SERVER_NAME`.
async fn keel_server(pki: &Pki, enrolled: Vec<KeyData>) -> Server {
    let host_key = key();
    let cert = host_cert_for(&pki.host_ca, &host_key, SERVER_NAME);
    serve(host_key, Some(cert), pki, enrolled).await
}

/// A TCP listener that accepts and closes at once, counting connections.
async fn closer() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let accepts = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&accepts);
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            counted.fetch_add(1, Ordering::SeqCst);
            drop(socket);
        }
    });
    (addr, accepts)
}

/// A port nothing listens on.
async fn dead_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

async fn issue(server: &Server, pki: &Pki, runner: &Runner) -> Certificate {
    let issued = runner_cert(&pki.runner_ca, runner.public_key(), SystemTime::now());
    *server.service.cert.lock().await = Some(issued.to_openssh().unwrap());
    issued
}

#[tokio::test]
async fn the_runner_dials_in_order_and_uses_the_first_that_verifies() {
    let pki = Pki::new();
    let (first, first_accepts) = closer().await;
    let mut keel = keel_server(&pki, vec![]).await;
    let (last, last_accepts) = closer().await;

    let host = Host::new(&pki);
    host.write_token(&pki.host_ca.fingerprint());
    // `--server` names a dead port: the runner must not fall back to it.
    let server = format!("{SERVER_NAME}:{}", dead_port().await);
    let runner = host.runner(server, vec![first, keel.addr(), last]);
    let issued = issue(&keel, &pki, &runner).await;

    let enrolled = tokio::time::timeout(WAIT, runner.enroll_if_needed())
        .await
        .expect("hung")
        .unwrap();
    assert!(enrolled);
    assert_eq!(first_accepts.load(Ordering::SeqCst), 1, "first tried once");
    assert_eq!(keel.accepts.load(Ordering::SeqCst), 1, "Keel second");
    assert_eq!(last_accepts.load(Ordering::SeqCst), 0, "never reached");
    // Keel's certificate is for `localhost`; the runner dialled `127.0.0.1`.
    let frames = keel.frames();
    assert_eq!(frames.len(), 1, "{frames:?}");
    assert!(matches!(frames[0], Frame::Enroll { .. }));
    assert!(!host.state.join(TOKEN_FILE).exists());
    assert_eq!(
        std::fs::read_to_string(host.state.join(CERT_FILE))
            .unwrap()
            .trim(),
        issued.to_openssh().unwrap()
    );
}

#[tokio::test]
async fn a_dialled_address_that_does_not_verify_for_server_is_skipped_and_gets_nothing() {
    let pki = Pki::new();
    let rogue_ca = SshCa::generate(CaKind::Host).unwrap();

    // Each one answers on the dial list before Keel.
    let k1 = key();
    let mut other_ca = serve(
        k1.clone(),
        Some(host_cert_for(&rogue_ca, &k1, SERVER_NAME)),
        &pki,
        vec![],
    )
    .await;
    // The right CA, but for the address the runner dialled.
    let k2 = key();
    let mut dialled_name = serve(
        k2.clone(),
        Some(host_cert_for(&pki.host_ca, &k2, "127.0.0.1")),
        &pki,
        vec![],
    )
    .await;
    // The right CA, but for the relay's own name.
    let k3 = key();
    let mut relay_name = serve(
        k3.clone(),
        Some(host_cert_for(&pki.host_ca, &k3, "relay-1.zone")),
        &pki,
        vec![],
    )
    .await;
    // A relay answering with its own bare key.
    let mut bare = serve(key(), None, &pki, vec![]).await;
    let mut keel = keel_server(&pki, vec![]).await;

    let host = Host::new(&pki);
    host.write_token(&pki.host_ca.fingerprint());
    let server = format!("{SERVER_NAME}:{}", dead_port().await);
    let dial = vec![
        other_ca.addr(),
        dialled_name.addr(),
        relay_name.addr(),
        bare.addr(),
        keel.addr(),
    ];
    let runner = host.runner(server.clone(), dial);
    issue(&keel, &pki, &runner).await;

    assert!(
        tokio::time::timeout(WAIT, runner.enroll_if_needed())
            .await
            .expect("hung")
            .unwrap()
    );
    for (what, s) in [
        ("another CA", &mut other_ca),
        ("the dialled name", &mut dialled_name),
        ("the relay's name", &mut relay_name),
        ("a bare key", &mut bare),
    ] {
        assert_eq!(s.accepts.load(Ordering::SeqCst), 1, "{what}: dialled");
        s.got_nothing(what);
    }
    assert!(matches!(keel.frames()[..], [Frame::Enroll { .. }]));

    // The renewal path skips them the same way, and sends them nothing.
    let enrolled_key = runner.public_key().clone();
    let mut keel = keel_server(&pki, vec![enrolled_key.clone()]).await;
    host.write_cert(&runner_cert(
        &pki.runner_ca,
        &enrolled_key,
        SystemTime::now() - 3 * DAY,
    ));
    let dial = vec![
        other_ca.addr(),
        dialled_name.addr(),
        relay_name.addr(),
        bare.addr(),
        keel.addr(),
    ];
    let runner = host.runner(server, dial);
    issue(&keel, &pki, &runner).await;
    tokio::time::timeout(WAIT, runner.renew_expired())
        .await
        .expect("hung")
        .unwrap();
    for (what, s) in [
        ("another CA", &mut other_ca),
        ("the dialled name", &mut dialled_name),
        ("the relay's name", &mut relay_name),
        ("a bare key", &mut bare),
    ] {
        assert_eq!(s.accepts.load(Ordering::SeqCst), 2, "{what}: dialled");
        s.got_nothing(what);
    }
    assert_eq!(keel.frames(), vec![Frame::RenewCert {}]);
}

#[tokio::test]
async fn when_nothing_verifies_the_token_never_leaves_the_runner() {
    let pki = Pki::new();
    let rogue_ca = SshCa::generate(CaKind::Host).unwrap();
    let k = key();
    let mut rogue = serve(
        k.clone(),
        Some(host_cert_for(&rogue_ca, &k, SERVER_NAME)),
        &pki,
        vec![],
    )
    .await;
    let host = Host::new(&pki);
    host.write_token(&pki.host_ca.fingerprint());
    let server = format!("{SERVER_NAME}:{}", dead_port().await);
    let runner = host.runner(server, vec![rogue.addr()]);

    let err = tokio::time::timeout(WAIT, runner.run_once())
        .await
        .expect("hung")
        .unwrap_err();
    assert!(matches!(err, RunnerError::ServerRefused(_)), "{err}");
    rogue.got_nothing("rogue");
    assert!(host.state.join(TOKEN_FILE).exists(), "token kept");
    assert!(!host.state.join(CERT_FILE).exists());
}

#[tokio::test]
async fn with_no_dial_the_runner_dials_server_as_before() {
    let pki = Pki::new();
    let mut keel = keel_server(&pki, vec![]).await;
    let host = Host::new(&pki);
    host.write_token(&pki.host_ca.fingerprint());
    let runner = host.runner(format!("{SERVER_NAME}:{}", keel.port), vec![]);
    issue(&keel, &pki, &runner).await;

    assert!(
        tokio::time::timeout(WAIT, runner.enroll_if_needed())
            .await
            .expect("hung")
            .unwrap()
    );
    assert_eq!(keel.accepts.load(Ordering::SeqCst), 1);
    assert!(matches!(keel.frames()[..], [Frame::Enroll { .. }]));
}

#[tokio::test]
async fn one_failing_pass_tries_every_address_once() {
    let pki = Pki::new();
    let (a, a_accepts) = closer().await;
    let (b, b_accepts) = closer().await;
    let c = format!("127.0.0.1:{}", dead_port().await);
    let host = Host::new(&pki);
    host.write_token(&pki.host_ca.fingerprint());
    let server = format!("{SERVER_NAME}:{}", dead_port().await);
    let runner = host.runner(server, vec![a, b, c]);

    let err = tokio::time::timeout(WAIT, runner.run_once())
        .await
        .expect("hung")
        .unwrap_err();
    assert!(matches!(err, RunnerError::Connect(_)), "{err}");
    assert_eq!(a_accepts.load(Ordering::SeqCst), 1);
    assert_eq!(b_accepts.load(Ordering::SeqCst), 1);
}

#[test]
fn backoff_doubles_per_pass_and_is_capped_at_60s() {
    let mut backoff = Backoff::default();
    let short = Duration::from_secs(1);
    let got: Vec<u64> = (0..9).map(|_| backoff.next(short).as_secs()).collect();
    assert_eq!(got, [1, 2, 4, 8, 16, 32, 60, 60, 60]);
    // A session that lasted over a minute resets it.
    assert_eq!(
        backoff.next(Duration::from_secs(61)),
        Duration::from_secs(1)
    );
    assert_eq!(backoff.next(short), Duration::from_secs(2));
}

#[test]
fn dial_entries_must_be_host_and_port() {
    let pki = Pki::new();
    let host = Host::new(&pki);
    let server = format!("{SERVER_NAME}:7831");
    for bad in [
        "relay-1",
        "relay-1:",
        "relay-1:0",
        ":7831",
        "relay 1:7831",
        "a:b",
    ] {
        let err = Runner::load(host.config(server.clone(), vec![bad.to_owned()])).unwrap_err();
        assert!(matches!(err, RunnerError::Connect(_)), "{bad}: {err}");
    }
    for good in ["relay-1:7831", "10.0.0.5:7831", "relay-1.zone.example:1"] {
        Runner::load(host.config(server.clone(), vec![good.to_owned()])).expect(good);
    }
}
