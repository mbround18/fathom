//! T146: the runner's join flow against an in-process Keel listener over loopback TCP
//! (contracts/join.md, "Runner side").
//!
//! - `JoinPending` backs off (30 s doubling, on a fake clock) and asks again with the same frame;
//!   `Enrolled` saves the certificate and host id and deletes `join-token`.
//! - `JoinRejected` deletes `join-token` and returns a configuration error (exit 78).
//! - An expired token, or one pinning a CA not in `host_ca.pub`, is refused before connecting.
//! - An `enroll-token` beside the join token wins, and enrolling deletes the join token.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use keel_exec::ca::{
    CaKind, HostCertRequest, RunnerCertRequest, RunnerCertVerifier, RunnerIdentity, SshCa,
    TrustedCa,
};
use keel_exec::codec::{read_frame, write_frame};
use keel_exec::frames::Frame;
use keel_exec::join_token::{AUDIENCE, JoinClaims, JoinSigningKey, new_jti};
use keel_exec::krl::Krl;
use keel_exec::server_config::{
    Access, BareKey, KeelSshHandler, RunnerAuthority, SubsystemService, server_config,
};
use keel_runner::runner::join::{JOIN_TOKEN_FILE, JoinBackoff, Sleeper};
use keel_runner::runner::{CERT_FILE, HOST_ID_FILE, Runner, RunnerConfig, RunnerError, TOKEN_FILE};
use russh::ChannelStream;
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, Certificate, PrivateKey};
use russh::server::Msg;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

const SERVER_NAME: &str = "localhost";
const INVENTORY_NAME: &str = "web-1";
const DAY: Duration = Duration::from_secs(24 * 3600);
const WAIT: Duration = Duration::from_secs(20);

fn key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()
}

fn now_secs() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap()
}

fn runner_cert(ca: &SshCa, public: &KeyData) -> Certificate {
    let profiles = vec!["base".to_owned()];
    ca.issue_runner_cert(&RunnerCertRequest {
        runner_id: "r-1",
        inventory_name: INVENTORY_NAME,
        assigned_profiles: &profiles,
        allowed_profiles: &profiles,
        public_key: public,
        ttl: DAY,
        now: SystemTime::now(),
    })
    .unwrap()
}

fn join_token(host_ca: &SshCa, exp_in: i64) -> String {
    let now = now_secs();
    JoinSigningKey::generate().sign(&JoinClaims {
        iss: "https://keel.test".to_owned(),
        aud: AUDIENCE.to_owned(),
        sub: "00000000-0000-0000-0000-000000000001".to_owned(),
        jti: new_jti(),
        iat: now - 10,
        exp: now + exp_in,
        server: format!("{SERVER_NAME}:7831"),
        host_ca: host_ca.cert_authority_line("*"),
        origin: "https://keel.test".to_owned(),
        rp_id: "keel.test".to_owned(),
        inventory_name: None,
        profiles: None,
    })
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
        Host {
            _dir: dir,
            trust,
            state,
        }
    }

    fn runner(&self, port: u16) -> Runner {
        Runner::load(RunnerConfig {
            dial: vec![],
            server: format!("{SERVER_NAME}:{port}"),
            trust_dir: self.trust.clone(),
            state_dir: self.state.clone(),
            executor_socket: self.state.join("executor.sock"),
            venv: self.state.join("no-venv"),
        })
        .unwrap()
        .with_age_recipient_file(self.state.join("age.pub"))
    }

    fn write_join_token(&self, token: &str) {
        std::fs::write(self.state.join(JOIN_TOKEN_FILE), format!("{token}\n")).unwrap();
    }
}

/// Every key is unknown: the runner may only enroll or join.
struct UnknownKeys {
    verifier: Arc<RunnerCertVerifier>,
    auth_calls: Arc<AtomicUsize>,
}

impl RunnerAuthority for UnknownKeys {
    fn verifier(&self) -> Arc<RunnerCertVerifier> {
        Arc::clone(&self.verifier)
    }

    async fn is_enrolled(&self, _identity: &RunnerIdentity, _key: &KeyData) -> bool {
        self.auth_calls.fetch_add(1, Ordering::SeqCst);
        false
    }

    async fn bare_key(&self, _key: &KeyData, _now: SystemTime) -> BareKey {
        self.auth_calls.fetch_add(1, Ordering::SeqCst);
        BareKey::Unknown
    }
}

/// Answers each `JoinRequest` with the next scripted frame, and `Enroll` with `Enrolled`.
struct Scripted {
    seen: mpsc::UnboundedSender<(Access, Frame)>,
    /// `None`: close without answering.
    answers: StdMutex<VecDeque<Option<Frame>>>,
    enrolled: StdMutex<Option<Frame>>,
}

impl SubsystemService for Scripted {
    async fn serve(&self, access: Access, mut stream: ChannelStream<Msg>) {
        let Ok(Some(frame)) = read_frame(&mut stream).await else {
            return;
        };
        if !access.permits(&frame) {
            return;
        }
        let _ = self.seen.send((access, frame.clone()));
        let answer = match frame {
            Frame::JoinRequest { .. } => self.answers.lock().unwrap().pop_front().flatten(),
            Frame::Enroll { .. } => self.enrolled.lock().unwrap().clone(),
            _ => None,
        };
        if let Some(f) = answer {
            let _ = write_frame(&mut stream, &f).await;
        }
    }
}

struct Server {
    port: u16,
    seen: mpsc::UnboundedReceiver<(Access, Frame)>,
    service: Arc<Scripted>,
}

impl Server {
    fn frames(&mut self) -> Vec<(Access, Frame)> {
        let mut out = vec![];
        while let Ok(f) = self.seen.try_recv() {
            out.push(f);
        }
        out
    }
}

async fn serve(pki: &Pki, answers: Vec<Option<Frame>>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let host_key = key();
    let cert = pki
        .host_ca
        .issue_host_cert(&HostCertRequest {
            host_names: &[SERVER_NAME.to_owned()],
            public_key: host_key.public_key().key_data(),
            ttl: DAY,
            now: SystemTime::now(),
        })
        .unwrap();
    let mut config = server_config(host_key, Some(cert));
    config.auth_rejection_time = Duration::from_millis(1);
    let config = Arc::new(config);
    let authority = Arc::new(UnknownKeys {
        verifier: Arc::new(RunnerCertVerifier::new(
            vec![TrustedCa::active(pki.runner_ca.key_data().clone())],
            Krl::default(),
        )),
        auth_calls: Arc::new(AtomicUsize::new(0)),
    });
    let (tx, seen) = mpsc::unbounded_channel();
    let service = Arc::new(Scripted {
        seen: tx,
        answers: StdMutex::new(answers.into()),
        enrolled: StdMutex::new(None),
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
        service,
    }
}

/// Records each wait instead of sleeping.
#[derive(Default)]
struct FakeClock(StdMutex<Vec<Duration>>);

impl Sleeper for FakeClock {
    async fn sleep(&self, d: Duration) {
        self.0.lock().unwrap().push(d);
    }
}

#[test]
fn the_backoff_doubles_to_ten_minutes_and_jitter_only_shortens() {
    let mut b = JoinBackoff::new();
    let waits: Vec<u64> = (0..7).map(|_| b.next_delay(0.0).as_secs()).collect();
    assert_eq!(waits, [30, 60, 120, 240, 480, 600, 600]);
    let mut b = JoinBackoff::new();
    let d = b.next_delay(0.999);
    assert!(
        d > Duration::from_secs(22) && d < Duration::from_secs(30),
        "{d:?}"
    );
    // Out-of-range jitter is clamped.
    assert_eq!(JoinBackoff::new().next_delay(-3.0), Duration::from_secs(30));
}

#[tokio::test]
async fn pending_backs_off_and_enrolled_saves_the_cert_and_deletes_the_token() {
    let pki = Pki::new();
    let host = Host::new(&pki);
    let runner_key_host = host.runner(1);
    let issued = runner_cert(&pki.runner_ca, runner_key_host.public_key());
    let pending = Frame::JoinPending {
        request_id: "11111111-1111-1111-1111-111111111111".to_owned(),
    };
    let mut server = serve(
        &pki,
        vec![
            Some(pending.clone()),
            Some(pending),
            Some(Frame::Enrolled {
                host_id: "h-9".to_owned(),
                cert: issued.to_openssh().unwrap(),
            }),
        ],
    )
    .await;
    let token = join_token(&pki.host_ca, 3600);
    host.write_join_token(&token);
    let runner = host.runner(server.port);
    let clock = FakeClock::default();

    let joined = tokio::time::timeout(WAIT, runner.join_if_needed_with(&clock, || 0.0))
        .await
        .expect("hung")
        .unwrap();
    assert!(joined);
    assert_eq!(
        *clock.0.lock().unwrap(),
        [Duration::from_secs(30), Duration::from_secs(60)]
    );
    assert!(!host.state.join(JOIN_TOKEN_FILE).exists(), "token deleted");
    assert_eq!(
        std::fs::read_to_string(host.state.join(CERT_FILE))
            .unwrap()
            .trim(),
        issued.to_openssh().unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(host.state.join(HOST_ID_FILE)).unwrap(),
        "h-9\n"
    );

    let frames = server.frames();
    assert_eq!(frames.len(), 3, "the same request three times");
    for (access, frame) in &frames {
        assert_eq!(*access, Access::JoinOnly);
        match frame {
            Frame::JoinRequest {
                token: sent,
                inventory_name,
                allowed_profiles,
                require_signed,
                hostname,
                os,
                arch,
                runner_version,
            } => {
                assert_eq!(sent.expose(), token);
                assert_eq!(inventory_name, INVENTORY_NAME);
                assert_eq!(allowed_profiles, &["base".to_owned()]);
                assert!(!require_signed);
                assert!(hostname.len() <= 253);
                assert!(os.len() <= 128);
                assert_eq!(arch, std::env::consts::ARCH);
                assert_eq!(runner_version, env!("CARGO_PKG_VERSION"));
            }
            other => panic!("{other:?}"),
        }
    }
    // Enrolled: nothing more to do.
    assert!(
        !runner
            .join_if_needed_with(&FakeClock::default(), || 0.0)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn a_rejection_deletes_the_token_and_is_a_config_error() {
    let pki = Pki::new();
    let server = serve(
        &pki,
        vec![Some(Frame::JoinRejected {
            reason: "token_rotated".to_owned(),
        })],
    )
    .await;
    let host = Host::new(&pki);
    host.write_join_token(&join_token(&pki.host_ca, 3600));
    let runner = host.runner(server.port);

    let err = tokio::time::timeout(
        WAIT,
        runner.join_if_needed_with(&FakeClock::default(), || 0.0),
    )
    .await
    .expect("hung")
    .unwrap_err();
    assert!(
        matches!(&err, RunnerError::JoinRejected(r) if r == "token_rotated"),
        "{err:?}"
    );
    assert!(err.is_config());
    assert!(!host.state.join(JOIN_TOKEN_FILE).exists(), "token deleted");
    assert!(!host.state.join(CERT_FILE).exists());
}

#[tokio::test]
async fn a_closed_connection_is_retried() {
    let pki = Pki::new();
    let host = Host::new(&pki);
    let issued = runner_cert(&pki.runner_ca, host.runner(1).public_key());
    // The first request is closed unanswered (a token that failed verification, or the rate
    // limit); the second is accepted.
    let server = serve(
        &pki,
        vec![
            None,
            Some(Frame::Enrolled {
                host_id: "h-9".to_owned(),
                cert: issued.to_openssh().unwrap(),
            }),
        ],
    )
    .await;
    host.write_join_token(&join_token(&pki.host_ca, 3600));
    let runner = host.runner(server.port);
    let clock = FakeClock::default();
    let joined = tokio::time::timeout(WAIT, runner.join_if_needed_with(&clock, || 0.0))
        .await
        .expect("hung")
        .unwrap();
    assert!(joined);
    assert_eq!(
        clock.0.lock().unwrap().len(),
        1,
        "one wait before the retry"
    );
}

#[tokio::test]
async fn an_expired_token_or_a_foreign_ca_is_refused_before_connecting() {
    let pki = Pki::new();
    let server = serve(&pki, vec![]).await;
    let host = Host::new(&pki);
    let runner = host.runner(server.port);

    host.write_join_token(&join_token(&pki.host_ca, -60));
    let err = runner
        .join_if_needed_with(&FakeClock::default(), || 0.0)
        .await
        .unwrap_err();
    assert!(matches!(err, RunnerError::JoinTokenExpired), "{err:?}");
    assert!(err.is_config());
    assert!(
        host.state.join(JOIN_TOKEN_FILE).exists(),
        "kept for the operator to see"
    );

    let rogue = SshCa::generate(CaKind::Host).unwrap();
    host.write_join_token(&join_token(&rogue, 3600));
    let err = runner
        .join_if_needed_with(&FakeClock::default(), || 0.0)
        .await
        .unwrap_err();
    assert!(matches!(err, RunnerError::JoinCaMismatch), "{err:?}");

    host.write_join_token("keel-enroll-v1.0123456789abcdef.nope");
    let err = runner
        .join_if_needed_with(&FakeClock::default(), || 0.0)
        .await
        .unwrap_err();
    assert!(matches!(err, RunnerError::JoinTokenInvalid), "{err:?}");

    let mut server = server;
    assert!(server.frames().is_empty(), "never connected");
}

#[tokio::test]
async fn an_enroll_token_wins_and_enrolling_deletes_the_join_token() {
    let pki = Pki::new();
    let mut server = serve(&pki, vec![]).await;
    let host = Host::new(&pki);
    host.write_join_token(&join_token(&pki.host_ca, 3600));
    std::fs::write(
        host.state.join(TOKEN_FILE),
        format!(
            "keel-enroll-v1.0123456789abcdef.{}\n",
            pki.host_ca.fingerprint()
        ),
    )
    .unwrap();
    let runner = host.runner(server.port);
    let issued = runner_cert(&pki.runner_ca, runner.public_key());
    *server.service.enrolled.lock().unwrap() = Some(Frame::Enrolled {
        host_id: "h-1".to_owned(),
        cert: issued.to_openssh().unwrap(),
    });

    assert!(
        !runner
            .join_if_needed_with(&FakeClock::default(), || 0.0)
            .await
            .unwrap(),
        "the enroll token wins"
    );
    assert!(server.frames().is_empty());
    assert!(
        tokio::time::timeout(WAIT, runner.enroll_if_needed())
            .await
            .expect("hung")
            .unwrap()
    );
    assert!(!host.state.join(TOKEN_FILE).exists());
    assert!(
        !host.state.join(JOIN_TOKEN_FILE).exists(),
        "join token deleted too"
    );
    let frames = server.frames();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].0, Access::EnrollOnly);
}
