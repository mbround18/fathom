//! T073: the runner SSH channel policy, exercised with an in-process russh server and client over
//! loopback TCP.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use keel_exec::ca::{
    CaKind, CertRefusal, HostCertRequest, HostCertVerifier, RunnerCertRequest, RunnerCertVerifier,
    RunnerIdentity, SshCa, TrustedCa,
};
use keel_exec::client_config::{
    KeelSshClient, ServerKeyRefusal, SubsystemError, client_config, open_subsystem,
};
use keel_exec::codec::{read_frame, write_frame};
use keel_exec::frames::{Frame, SecretString, Subsystem};
use keel_exec::krl::Krl;
use keel_exec::server_config::{
    Access, BareKey, KeelSshHandler, RunnerAuthority, SubsystemService, server_config,
};
use russh::client::{self, AuthResult, Handle};
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, Certificate, PrivateKey, PrivateKeyWithHashAlg};
use russh::server::Msg;
use russh::{ChannelMsg, ChannelStream, Preferred, cipher, kex, mac};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

const HOST_NAME: &str = "keel.test";
const DAY: Duration = Duration::from_secs(24 * 3600);
const WAIT: Duration = Duration::from_secs(10);

fn key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()
}

fn issued_at() -> SystemTime {
    SystemTime::now() - Duration::from_secs(60)
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

    fn host_cert(&self, ca: &SshCa, host_key: &PrivateKey, name: &str) -> Certificate {
        ca.issue_host_cert(&HostCertRequest {
            host_names: &[name.to_owned()],
            public_key: host_key.public_key().key_data(),
            ttl: DAY,
            now: SystemTime::now(),
        })
        .unwrap()
    }

    fn runner_cert(&self, ca: &SshCa, runner: &PrivateKey) -> Certificate {
        let profiles = vec!["base".to_owned()];
        ca.issue_runner_cert(&RunnerCertRequest {
            runner_id: "r-1",
            inventory_name: "web-1",
            assigned_profiles: &profiles,
            allowed_profiles: &profiles,
            public_key: runner.public_key().key_data(),
            ttl: DAY,
            now: issued_at(),
        })
        .unwrap()
    }

    fn client(&self) -> KeelSshClient {
        KeelSshClient::new(HostCertVerifier::new(
            vec![TrustedCa::active(self.host_ca.key_data().clone())],
            HOST_NAME,
        ))
    }
}

/// Enrolled keys: (key, host id, still within the offline window).
struct TestAuthority {
    verifier: Arc<RunnerCertVerifier>,
    enrolled: Vec<(KeyData, String, bool)>,
}

impl RunnerAuthority for TestAuthority {
    fn verifier(&self) -> Arc<RunnerCertVerifier> {
        Arc::clone(&self.verifier)
    }

    async fn is_enrolled(&self, _identity: &RunnerIdentity, key: &KeyData) -> bool {
        self.enrolled.iter().any(|(k, _, _)| k == key)
    }

    async fn bare_key(&self, key: &KeyData, _now: SystemTime) -> BareKey {
        match self.enrolled.iter().find(|(k, _, _)| k == key) {
            None => BareKey::Unknown,
            Some((_, host_id, true)) => BareKey::Renewable {
                host_id: host_id.clone(),
            },
            Some((_, _, false)) => BareKey::Refused,
        }
    }
}

/// Reports each frame it reads and whether the access permits it; answers a permitted `Ping`
/// with `Pong`; stops at the first refused frame.
struct TestService {
    seen: mpsc::UnboundedSender<(Access, Frame, bool)>,
}

impl SubsystemService for TestService {
    async fn serve(&self, access: Access, mut stream: ChannelStream<Msg>) {
        while let Ok(Some(frame)) = read_frame(&mut stream).await {
            let permitted = access.permits(&frame);
            let ping = matches!(frame, Frame::Ping);
            let _ = self.seen.send((access.clone(), frame, permitted));
            if !permitted {
                return;
            }
            if ping && write_frame(&mut stream, &Frame::Pong).await.is_err() {
                return;
            }
        }
    }
}

struct Server {
    addr: std::net::SocketAddr,
    seen: mpsc::UnboundedReceiver<(Access, Frame, bool)>,
}

/// Serves every connection on a loopback port with the given host key/cert, runner CA and KRL.
async fn serve(
    host_key: PrivateKey,
    host_cert: Option<Certificate>,
    runner_ca: &SshCa,
    krl: Krl,
    enrolled: Vec<(KeyData, String, bool)>,
) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mut config = server_config(host_key, host_cert);
    config.auth_rejection_time = Duration::from_millis(1);
    let config = Arc::new(config);
    let verifier = Arc::new(RunnerCertVerifier::new(
        vec![TrustedCa::active(runner_ca.key_data().clone())],
        krl,
    ));
    let authority = Arc::new(TestAuthority { verifier, enrolled });
    let (tx, seen) = mpsc::unbounded_channel();
    let service = Arc::new(TestService { seen: tx });
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let handler = KeelSshHandler::new(Arc::clone(&authority), Arc::clone(&service));
            let config = Arc::clone(&config);
            tokio::spawn(async move {
                if let Ok(session) = russh::server::run_stream(config, socket, handler).await {
                    let _ = session.await;
                }
            });
        }
    });
    Server { addr, seen }
}

/// A server with a valid host certificate.
async fn standard_server(pki: &Pki, krl: Krl, enrolled: Vec<(KeyData, String, bool)>) -> Server {
    let host_key = key();
    let cert = pki.host_cert(&pki.host_ca, &host_key, HOST_NAME);
    serve(host_key, Some(cert), &pki.runner_ca, krl, enrolled).await
}

async fn connect_with(
    addr: std::net::SocketAddr,
    config: client::Config,
    handler: KeelSshClient,
) -> Result<Handle<KeelSshClient>, russh::Error> {
    let stream = TcpStream::connect(addr).await.unwrap();
    tokio::time::timeout(
        WAIT,
        client::connect_stream(Arc::new(config), stream, handler),
    )
    .await
    .expect("handshake hung")
}

async fn connect(pki: &Pki, addr: std::net::SocketAddr) -> Handle<KeelSshClient> {
    connect_with(addr, client_config(), pki.client())
        .await
        .expect("handshake")
}

async fn auth_cert(
    handle: &mut Handle<KeelSshClient>,
    user: &str,
    runner: &PrivateKey,
    cert: Certificate,
) -> bool {
    matches!(
        handle
            .authenticate_openssh_cert(user, Arc::new(runner.clone()), cert)
            .await
            .unwrap(),
        AuthResult::Success
    )
}

async fn auth_key(handle: &mut Handle<KeelSshClient>, runner: &PrivateKey) -> bool {
    let key = PrivateKeyWithHashAlg::new(Arc::new(runner.clone()), None);
    matches!(
        handle.authenticate_publickey("runner", key).await.unwrap(),
        AuthResult::Success
    )
}

async fn next_seen(server: &mut Server) -> (Access, Frame, bool) {
    tokio::time::timeout(WAIT, server.seen.recv())
        .await
        .expect("service saw nothing")
        .unwrap()
}

/// A certified, enrolled runner on its own connection.
async fn certified(pki: &Pki) -> (Server, Handle<KeelSshClient>) {
    let runner = key();
    let cert = pki.runner_cert(&pki.runner_ca, &runner);
    let server = standard_server(
        pki,
        Krl::default(),
        vec![(runner.public_key().key_data().clone(), "h-1".into(), true)],
    )
    .await;
    let mut handle = connect(pki, server.addr).await;
    assert!(auth_cert(&mut handle, "runner", &runner, cert).await);
    (server, handle)
}

async fn reply(channel: &mut russh::Channel<client::Msg>) -> ChannelMsg {
    loop {
        match tokio::time::timeout(WAIT, channel.wait())
            .await
            .expect("no reply")
        {
            Some(ChannelMsg::Success) => return ChannelMsg::Success,
            Some(ChannelMsg::Failure) => return ChannelMsg::Failure,
            Some(ChannelMsg::WindowAdjusted { .. }) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_certified_runner_opens_keel_runner_v1_and_exchanges_frames() {
    let pki = Pki::new();
    let (mut server, handle) = certified(&pki).await;
    let mut stream = open_subsystem(&handle, Subsystem::Runner).await.unwrap();
    write_frame(&mut stream, &Frame::Ping).await.unwrap();
    let (access, frame, permitted) = next_seen(&mut server).await;
    assert!(matches!(frame, Frame::Ping));
    assert!(permitted);
    let Access::Runner(identity) = access else {
        panic!("expected runner access, got {access:?}")
    };
    assert_eq!(identity.runner_id, "r-1");
    assert_eq!(identity.inventory_name, "web-1");
    assert_eq!(identity.profiles, ["base"]);
    assert!(matches!(
        read_frame(&mut stream).await.unwrap(),
        Some(Frame::Pong)
    ));
}

#[tokio::test]
async fn a_certified_runner_cannot_enroll_or_open_a_second_session() {
    let pki = Pki::new();
    let (_server, handle) = certified(&pki).await;
    let err = open_subsystem(&handle, Subsystem::Enroll)
        .await
        .err()
        .unwrap();
    assert!(
        matches!(err, SubsystemError::Refused("keel-enroll-v1")),
        "{err:?}"
    );
    assert!(
        handle.channel_open_session().await.is_err(),
        "one session channel per connection"
    );
}

#[tokio::test]
async fn shell_exec_pty_env_agent_x11_and_unknown_subsystems_are_refused() {
    let pki = Pki::new();
    let (_server, handle) = certified(&pki).await;
    let mut channel = handle.channel_open_session().await.unwrap();

    channel
        .request_pty(true, "xterm", 80, 24, 0, 0, &[])
        .await
        .unwrap();
    assert!(
        matches!(reply(&mut channel).await, ChannelMsg::Failure),
        "pty"
    );
    channel.request_shell(true).await.unwrap();
    assert!(
        matches!(reply(&mut channel).await, ChannelMsg::Failure),
        "shell"
    );
    channel.exec(true, "id").await.unwrap();
    assert!(
        matches!(reply(&mut channel).await, ChannelMsg::Failure),
        "exec"
    );
    channel
        .set_env(true, "LD_PRELOAD", "/tmp/x.so")
        .await
        .unwrap();
    assert!(
        matches!(reply(&mut channel).await, ChannelMsg::Failure),
        "env"
    );
    channel.agent_forward(true).await.unwrap();
    assert!(
        matches!(reply(&mut channel).await, ChannelMsg::Failure),
        "agent"
    );
    channel
        .request_x11(true, false, "MIT-MAGIC-COOKIE-1", "00", 0)
        .await
        .unwrap();
    assert!(
        matches!(reply(&mut channel).await, ChannelMsg::Failure),
        "x11"
    );
    channel.request_subsystem(true, "sftp").await.unwrap();
    assert!(
        matches!(reply(&mut channel).await, ChannelMsg::Failure),
        "sftp"
    );

    // The same channel still carries the allowed subsystem afterwards.
    channel
        .request_subsystem(true, Subsystem::Runner.name())
        .await
        .unwrap();
    assert!(
        matches!(reply(&mut channel).await, ChannelMsg::Success),
        "keel-runner-v1"
    );
}

#[tokio::test]
async fn forwarding_in_both_directions_and_other_channel_types_are_refused() {
    let pki = Pki::new();
    let (_server, handle) = certified(&pki).await;
    assert!(
        handle
            .channel_open_direct_tcpip("127.0.0.1", 22, "127.0.0.1", 40000)
            .await
            .is_err(),
        "local forward"
    );
    assert!(
        handle
            .channel_open_direct_streamlocal("/run/docker.sock")
            .await
            .is_err(),
        "streamlocal"
    );
    assert!(
        handle.channel_open_x11("127.0.0.1", 6000).await.is_err(),
        "x11 channel"
    );
    assert!(
        handle.tcpip_forward("127.0.0.1", 0).await.is_err(),
        "remote forward"
    );
    assert!(
        handle.streamlocal_forward("/tmp/keel.sock").await.is_err(),
        "remote streamlocal"
    );
}

#[tokio::test]
async fn an_unknown_key_may_only_enroll() {
    let pki = Pki::new();
    let mut server = standard_server(&pki, Krl::default(), vec![]).await;

    let mut handle = connect(&pki, server.addr).await;
    assert!(auth_key(&mut handle, &key()).await);
    let err = open_subsystem(&handle, Subsystem::Runner)
        .await
        .err()
        .unwrap();
    assert!(
        matches!(err, SubsystemError::Refused("keel-runner-v1")),
        "{err:?}"
    );

    let mut handle = connect(&pki, server.addr).await;
    assert!(auth_key(&mut handle, &key()).await);
    let mut stream = open_subsystem(&handle, Subsystem::Enroll).await.unwrap();
    let enroll = Frame::Enroll {
        token: SecretString::from("keel-enroll-v1.secret.SHA256:x"),
        inventory_name: "web-9".into(),
        allowed_profiles: vec!["base".into()],
        require_signed: false,
    };
    write_frame(&mut stream, &enroll).await.unwrap();
    let (access, frame, permitted) = next_seen(&mut server).await;
    assert_eq!(access, Access::EnrollOnly);
    assert!(matches!(frame, Frame::Enroll { .. }));
    assert!(permitted);
    write_frame(&mut stream, &Frame::Ping).await.unwrap();
    let (_, frame, permitted) = next_seen(&mut server).await;
    assert!(matches!(frame, Frame::Ping));
    assert!(
        !permitted,
        "an enroll-only connection sends nothing but Enroll"
    );
}

#[tokio::test]
async fn a_bare_enrolled_key_may_only_renew_within_the_offline_window() {
    let pki = Pki::new();
    let renewable = key();
    let stale = key();
    let mut server = standard_server(
        &pki,
        Krl::default(),
        vec![
            (
                renewable.public_key().key_data().clone(),
                "h-1".into(),
                true,
            ),
            (stale.public_key().key_data().clone(), "h-2".into(), false),
        ],
    )
    .await;

    let mut handle = connect(&pki, server.addr).await;
    assert!(auth_key(&mut handle, &renewable).await);
    let mut stream = open_subsystem(&handle, Subsystem::Runner).await.unwrap();
    write_frame(&mut stream, &Frame::RenewCert {})
        .await
        .unwrap();
    let (access, _, permitted) = next_seen(&mut server).await;
    assert_eq!(
        access,
        Access::RenewOnly {
            host_id: "h-1".into()
        }
    );
    assert!(permitted);
    write_frame(&mut stream, &Frame::Ping).await.unwrap();
    assert!(
        !next_seen(&mut server).await.2,
        "renew-only sends nothing but RenewCert"
    );

    let mut handle = connect(&pki, server.addr).await;
    assert!(
        !auth_key(&mut handle, &stale).await,
        "offline too long, or revoked host"
    );
}

#[tokio::test]
async fn revoked_untrusted_unenrolled_and_wrong_user_certificates_are_refused() {
    let pki = Pki::new();
    let runner = key();
    let good = pki.runner_cert(&pki.runner_ca, &runner);
    let mut krl = Krl::new(1, 0);
    let revoked_key = key();
    krl.revoke_key(revoked_key.public_key().key_data());
    let enrolled = vec![
        (runner.public_key().key_data().clone(), "h-1".into(), true),
        (
            revoked_key.public_key().key_data().clone(),
            "h-3".into(),
            true,
        ),
    ];
    let server = standard_server(&pki, krl, enrolled).await;

    let mut h = connect(&pki, server.addr).await;
    assert!(
        !auth_cert(&mut h, "root", &runner, good.clone()).await,
        "user must be runner"
    );
    let mut h = connect(&pki, server.addr).await;
    let rogue = SshCa::generate(CaKind::Runner).unwrap();
    assert!(
        !auth_cert(&mut h, "runner", &runner, pki.runner_cert(&rogue, &runner)).await,
        "untrusted CA"
    );
    let mut h = connect(&pki, server.addr).await;
    let stranger = key();
    assert!(
        !auth_cert(
            &mut h,
            "runner",
            &stranger,
            pki.runner_cert(&pki.runner_ca, &stranger)
        )
        .await,
        "not enrolled"
    );
    let mut h = connect(&pki, server.addr).await;
    let revoked_cert = pki.runner_cert(&pki.runner_ca, &revoked_key);
    assert!(
        !auth_cert(&mut h, "runner", &revoked_key, revoked_cert).await,
        "revoked in the KRL"
    );
    let mut h = connect(&pki, server.addr).await;
    assert!(
        !auth_key(&mut h, &revoked_key).await,
        "a revoked key may not even enroll"
    );
    let mut h = connect(&pki, server.addr).await;
    assert!(auth_cert(&mut h, "runner", &runner, good).await, "control");
}

#[tokio::test]
async fn the_runner_refuses_a_server_without_a_trusted_host_certificate() {
    let pki = Pki::new();
    let runner_ca = SshCa::generate(CaKind::Runner).unwrap();

    // A bare host key.
    let server = serve(key(), None, &runner_ca, Krl::default(), vec![]).await;
    let client = pki.client();
    let slot = client.refusal_slot();
    assert!(
        connect_with(server.addr, client_config(), client)
            .await
            .is_err()
    );
    assert_eq!(*slot.lock().unwrap(), Some(ServerKeyRefusal::BareKey));

    // A certificate from another host CA.
    let host_key = key();
    let rogue = SshCa::generate(CaKind::Host).unwrap();
    let cert = pki.host_cert(&rogue, &host_key, HOST_NAME);
    let server = serve(host_key, Some(cert), &runner_ca, Krl::default(), vec![]).await;
    let client = pki.client();
    let slot = client.refusal_slot();
    assert!(
        connect_with(server.addr, client_config(), client)
            .await
            .is_err()
    );
    assert_eq!(
        *slot.lock().unwrap(),
        Some(ServerKeyRefusal::Certificate(CertRefusal::UntrustedCa))
    );

    // The right CA, but for another name.
    let host_key = key();
    let cert = pki.host_cert(&pki.host_ca, &host_key, "other.test");
    let server = serve(host_key, Some(cert), &runner_ca, Krl::default(), vec![]).await;
    let client = pki.client();
    let slot = client.refusal_slot();
    assert!(
        connect_with(server.addr, client_config(), client)
            .await
            .is_err()
    );
    assert_eq!(
        *slot.lock().unwrap(),
        Some(ServerKeyRefusal::Certificate(CertRefusal::BadPrincipals))
    );
}

fn weak_client(edit: impl FnOnce(&mut Preferred)) -> client::Config {
    let mut config = client_config();
    edit(&mut config.preferred);
    config
}

#[tokio::test]
async fn the_server_refuses_other_algorithms() {
    let pki = Pki::new();
    let server = standard_server(&pki, Krl::default(), vec![]).await;

    let ctr = weak_client(|p| {
        p.cipher = vec![cipher::AES_128_CTR, cipher::AES_256_CTR].into();
        p.mac = vec![mac::HMAC_SHA256_ETM, mac::HMAC_SHA256].into();
    });
    assert!(
        connect_with(server.addr, ctr, pki.client()).await.is_err(),
        "aes-ctr + hmac"
    );

    let gcm128 = weak_client(|p| p.cipher = vec![cipher::AES_128_GCM].into());
    assert!(
        connect_with(server.addr, gcm128, pki.client())
            .await
            .is_err(),
        "aes128-gcm"
    );

    let dh =
        weak_client(|p| p.kex = vec![kex::DH_G14_SHA256, kex::EXTENSION_SUPPORT_AS_CLIENT].into());
    assert!(
        connect_with(server.addr, dh, pki.client()).await.is_err(),
        "diffie-hellman-group14"
    );

    let mlkem = weak_client(|p| p.kex = vec![kex::MLKEM768X25519_SHA256].into());
    assert!(
        connect_with(server.addr, mlkem, pki.client())
            .await
            .is_err(),
        "mlkem768x25519 is not on the list"
    );

    let ecdsa = weak_client(|p| {
        let alg = Algorithm::Ecdsa {
            curve: russh::keys::EcdsaCurve::NistP256,
        };
        p.key = vec![alg.clone()].into();
        p.host_key_certificates = vec![alg].into();
    });
    assert!(
        connect_with(server.addr, ecdsa, pki.client())
            .await
            .is_err(),
        "ecdsa host keys"
    );

    // Each allowed cipher works on its own.
    for c in [cipher::CHACHA20_POLY1305, cipher::AES_256_GCM] {
        let only = weak_client(|p| p.cipher = vec![c].into());
        assert!(
            connect_with(server.addr, only, pki.client()).await.is_ok(),
            "{c:?}"
        );
    }
}

#[test]
fn access_permits_only_its_own_frames() {
    let identity = RunnerIdentity {
        runner_id: "r-1".into(),
        inventory_name: "web-1".into(),
        profiles: vec![],
        serial: 1,
        valid_before: 0,
        ca_fingerprint: String::new(),
    };
    let runner = Access::Runner(identity);
    let renew = Access::RenewOnly {
        host_id: "h".into(),
    };
    let enroll = Access::EnrollOnly;
    let enroll_frame = Frame::Enroll {
        token: SecretString::from("t"),
        inventory_name: "web-1".into(),
        allowed_profiles: vec![],
        require_signed: false,
    };
    let started = Frame::Started {
        run_id: "run-1".into(),
    };
    let cancel = Frame::Cancel {
        run_id: "run-1".into(),
    };

    assert!(
        runner.permits(&Frame::Ping)
            && runner.permits(&started)
            && runner.permits(&Frame::RenewCert {})
    );
    assert!(
        !runner.permits(&cancel),
        "server-to-runner frames are never accepted from a runner"
    );
    assert!(!runner.permits(&enroll_frame) && !runner.permits(&Frame::ReadLedger));
    assert!(renew.permits(&Frame::RenewCert {}));
    assert!(!renew.permits(&Frame::Ping) && !renew.permits(&started));
    assert!(enroll.permits(&enroll_frame));
    assert!(!enroll.permits(&Frame::Ping) && !enroll.permits(&Frame::RenewCert {}));
    assert_eq!(enroll.subsystem(), Subsystem::Enroll);
    assert_eq!(renew.subsystem(), Subsystem::Runner);
}
