//! Spec 067 T024 and T063 (SC-002, contracts/relay.md "Hostile-relay test matrix"): the runner
//! fails closed through a hostile relay.
//!
//! A relay only carries the runner's SSH bytes. Here a TCP hop stands where the relay would, in
//! front of a Keel server that has a run ready for this signed-mode host. Through an honest hop
//! the run executes (the control); through a tampering hop it never does:
//!
//! - flip one byte mid-session, reorder chunks, drop chunks (all after the session is
//!   authenticated and has said `Hello`): the inner SSH integrity check fails;
//! - replay a recorded session's bytes to the runner (or the runner's bytes to Keel): the fresh
//!   key exchange does not verify;
//! - inject bytes before either side's version banner: the handshake fails.
//!
//! Each ends with the session closed, no run directory, the ledger unchanged byte for byte, no
//! playbook run, nothing acted on at Keel, and the runner's reconnect loop still dialling.
//!
//! T063: a relay that answers with its own SSH server, or replays Keel's host certificate behind
//! a key of its own, gets nothing: no authentication, no enrollment token, no ledger bytes. An
//! algorithm offer outside keel-exec's lists is refused before the key exchange. The relay that
//! replays Keel's certificate is a hand-written SSH server (russh will not present a certificate
//! without its private key); a control run with Keel's real key proves that server correct.

use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime};

use keel_exec::ca::{
    CaKind, HostCertRequest, RunnerCertRequest, RunnerCertVerifier, RunnerIdentity, SshCa,
    TrustedCa,
};
use keel_exec::client_config::client_preferred;
use keel_exec::codec::{read_frame, write_frame};
use keel_exec::frames::Frame;
use keel_exec::krl::Krl;
use keel_exec::pack_builder;
use keel_exec::server_config::{
    Access, BareKey, KeelSshHandler, RunnerAuthority, SubsystemService, server_config,
    server_preferred,
};
use keel_runner::executor::{ExecutorConfig, LEDGER_FILE, serve as serve_executor};
use keel_runner::runner::{CERT_FILE, HOST_ID_FILE, Runner, RunnerConfig, TOKEN_FILE};
use keel_trust::CommitId;
use keel_trust::ledger::{Auth, Entry, Ledger, Outcome};
use russh::ChannelStream;
use russh::keys::signature::Signer as _;
use russh::keys::ssh_encoding::Encode as _;
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, Certificate, PrivateKey};
use russh::server::Msg;
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream, UnixListener};
use tokio::sync::mpsc;

/// The name on Keel's host certificate and the host part of `--server`.
const SERVER_NAME: &str = "localhost";
const INVENTORY_NAME: &str = "web-1";
const JOB: &str = "web-deploy";
const SEED_RUN: &str = "seed-run";
const RUN: &str = "run-through-relay";
const DAY: Duration = Duration::from_secs(24 * 3600);
const WAIT: Duration = Duration::from_secs(30);
/// The enrollment token's secret part; it must never reach a relay.
const TOKEN_SECRET: &str = "relay-must-never-see-this";

const MANIFEST: &str = r#"
version: 1
profiles:
  base: { description: "Every host" }
inventories:
  prod:
    path: inventories/prod/hosts.yaml
jobs:
  web-deploy:
    playbook: playbooks/site.yaml
    profiles: [base]
    inventory: prod
"#;

fn key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
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

    fn host_cert(&self, host_key: &PrivateKey, name: &str) -> Certificate {
        self.host_ca
            .issue_host_cert(&HostCertRequest {
                host_names: &[name.to_owned()],
                public_key: host_key.public_key().key_data(),
                ttl: DAY,
                now: SystemTime::now(),
            })
            .unwrap()
    }
}

// --- The host ------------------------------------------------------------------------------

/// A signed-mode host: the runner's trust and state, the executor's state and a fake venv whose
/// `ansible-playbook` leaves a marker file, and a commit-signing key it trusts.
struct Host {
    dir: tempfile::TempDir,
    trust: PathBuf,
    state: PathBuf,
    socket: PathBuf,
    exec: ExecutorConfig,
    marker: PathBuf,
    signing_key: PathBuf,
}

impl Host {
    fn new(pki: &Pki) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let trust = root.join("trust");
        let state = root.join("state");
        let exec_state = root.join("exec-state");
        let venv = root.join("venv");
        for d in [&trust, &state, &exec_state] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(
            trust.join("policy.toml"),
            format!(
                "require_signed = true\norigin = \"https://keel.test\"\nrp_id = \"keel.test\"\n\
                 inventory_name = \"{INVENTORY_NAME}\"\nallowed_profiles = [\"base\"]\n"
            ),
        )
        .unwrap();
        std::fs::write(
            trust.join("host_ca.pub"),
            format!("{}\n", pki.host_ca.cert_authority_line(SERVER_NAME)),
        )
        .unwrap();
        let signing_key = root.join("signing_key");
        let out = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", "keel-test", "-f"])
            .arg(&signing_key)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let public = std::fs::read_to_string(signing_key.with_extension("pub")).unwrap();
        std::fs::write(trust.join("signers"), format!("keel {}\n", public.trim())).unwrap();
        std::fs::write(trust.join("approvers.json"), "[]").unwrap();
        std::fs::create_dir_all(
            venv.join("lib/python3.12/site-packages/ansible_core-2.21.5.dist-info"),
        )
        .unwrap();
        std::fs::create_dir_all(venv.join("bin")).unwrap();
        let marker = root.join("playbook-ran");
        let playbook = venv.join("bin/ansible-playbook");
        std::fs::write(
            &playbook,
            format!("#!/bin/sh\ntouch '{}'\nexit 0\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&playbook, std::fs::Permissions::from_mode(0o755)).unwrap();
        let exec = ExecutorConfig::with_defaults(exec_state, trust.clone(), venv);
        let socket = root.join("executor.sock");
        Host {
            dir,
            trust,
            state,
            socket,
            exec,
            marker,
            signing_key,
        }
    }

    /// A runner that reaches Keel only through `dial`; `--server` is never dialled.
    fn runner(&self, dial: Vec<String>) -> Runner {
        Runner::load(RunnerConfig {
            server: format!("{SERVER_NAME}:1"),
            dial,
            trust_dir: self.trust.clone(),
            state_dir: self.state.clone(),
            executor_socket: self.socket.clone(),
            venv: self.state.join("no-venv"),
        })
        .unwrap()
        .with_age_recipient_file(self.state.join("age.pub"))
    }

    fn enroll(&self, pki: &Pki) -> Certificate {
        let key = self.runner(vec![]).public_key().clone();
        let profiles = vec!["base".to_owned()];
        let cert = pki
            .runner_ca
            .issue_runner_cert(&RunnerCertRequest {
                runner_id: "r-1",
                inventory_name: INVENTORY_NAME,
                assigned_profiles: &profiles,
                allowed_profiles: &profiles,
                public_key: &key,
                ttl: DAY,
                now: SystemTime::now(),
            })
            .unwrap();
        std::fs::write(
            self.state.join(CERT_FILE),
            format!("{}\n", cert.to_openssh().unwrap()),
        )
        .unwrap();
        std::fs::write(self.state.join(HOST_ID_FILE), "h-1\n").unwrap();
        cert
    }

    fn write_token(&self, pki: &Pki) {
        std::fs::write(
            self.state.join(TOKEN_FILE),
            format!(
                "keel-enroll-v1.{TOKEN_SECRET}.{}\n",
                pki.host_ca.fingerprint()
            ),
        )
        .unwrap();
    }

    fn token_kept(&self) -> bool {
        self.state.join(TOKEN_FILE).exists() && !self.state.join(CERT_FILE).exists()
    }

    fn ledger(&self) -> String {
        std::fs::read_to_string(self.exec.state_dir.join(LEDGER_FILE)).unwrap()
    }

    fn run_dirs(&self) -> usize {
        std::fs::read_dir(self.exec.state_dir.join("runs"))
            .map(|d| d.count())
            .unwrap_or(0)
    }

    /// The real executor on the host's socket, one `serve` per connection.
    fn start_executor(&self) {
        let listener = UnixListener::bind(&self.socket).unwrap();
        let cfg = self.exec.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let cfg = cfg.clone();
                tokio::spawn(async move { serve_executor(stream, &cfg).await });
            }
        });
    }

    /// Two commits signed by the host's trusted key, `base` then `target`, and a pack of both.
    fn repo(&self) -> (String, String, Vec<u8>) {
        let repo = self.dir.path().join("repo");
        for d in ["playbooks", "inventories/prod", ".keel"] {
            std::fs::create_dir_all(repo.join(d)).unwrap();
        }
        std::fs::write(repo.join(".keel/jobs.yaml"), MANIFEST).unwrap();
        std::fs::write(
            repo.join("inventories/prod/hosts.yaml"),
            format!("all:\n  hosts:\n    {INVENTORY_NAME}: {{}}\n"),
        )
        .unwrap();
        let git = |args: &[&str], stdin: Option<&[u8]>| -> Vec<u8> {
            let mut child = Command::new("git")
                .current_dir(&repo)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .args([
                    "-c",
                    "user.name=Keel Test",
                    "-c",
                    "user.email=keel@example.invalid",
                    "-c",
                    "init.defaultBranch=main",
                    "-c",
                    "gpg.format=ssh",
                    "-c",
                ])
                .arg(format!("user.signingkey={}", self.signing_key.display()))
                .args(args)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            {
                use std::io::Write as _;
                let mut input = child.stdin.take().unwrap();
                if let Some(bytes) = stdin {
                    input.write_all(bytes).unwrap();
                }
            }
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success(), "git {args:?} failed");
            out.stdout
        };
        let head = || {
            String::from_utf8(git(&["rev-parse", "HEAD"], None))
                .unwrap()
                .trim()
                .to_owned()
        };
        git(&["init", "-q"], None);
        let mut commits = vec![];
        for (i, name) in ["base", "target"].iter().enumerate() {
            std::fs::write(
                repo.join("playbooks/site.yaml"),
                format!("# {i}\n- hosts: all\n  tasks: []\n"),
            )
            .unwrap();
            git(&["add", "-A"], None);
            git(&["commit", "-q", "-S", "-m", name], None);
            commits.push(head());
        }
        let pack = git(
            &["pack-objects", "--stdout", "--revs", "-q"],
            Some(b"HEAD\n"),
        );
        (commits[0].clone(), commits[1].clone(), pack)
    }

    /// The host already ran `base` for the job, as `SEED_RUN`.
    fn seed_ledger(&self, base: &str) {
        let mut ledger = Ledger::empty();
        let mut text = ledger
            .append(Entry::Start {
                at: "2026-01-01T00:00:00Z".to_owned(),
                run: SEED_RUN.to_owned(),
                job: JOB.to_owned(),
                base: None,
                target: CommitId::parse(base).unwrap(),
                auth: Auth::Signed {
                    signers: vec!["SHA256:seed".to_owned()],
                },
            })
            .unwrap();
        text.push_str(
            &ledger
                .append(Entry::End {
                    at: "2026-01-01T00:01:00Z".to_owned(),
                    run: SEED_RUN.to_owned(),
                    job: JOB.to_owned(),
                    outcome: Outcome::Succeeded,
                    exit: Some(0),
                })
                .unwrap(),
        );
        std::fs::write(self.exec.state_dir.join(LEDGER_FILE), text).unwrap();
    }
}

/// A run the host accepts: a signed target on top of its base, empty secrets and approvals.
fn run_frames(base: &str, target: &str, pack: &[u8]) -> Vec<Frame> {
    let mut frames = vec![Frame::Assign {
        run_id: RUN.to_owned(),
        workspace: "ws-1".to_owned(),
        binding: "main".to_owned(),
        job: JOB.to_owned(),
        base: Some(base.to_owned()),
        target: target.to_owned(),
        deadline: now() + 600,
        pack_size: pack.len() as u64,
        quorum: None,
        params: Default::default(),
    }];
    frames.extend(pack_builder::frames(RUN, pack));
    frames.push(Frame::Secrets {
        run_id: RUN.to_owned(),
        items: vec![],
    });
    frames.push(Frame::Approvals {
        run_id: RUN.to_owned(),
        items: vec![],
    });
    frames
}

/// Frames that mean the host acted on a run.
fn acted(frames: &[Frame]) -> Vec<&'static str> {
    frames
        .iter()
        .filter(|f| {
            matches!(
                f,
                Frame::Started { .. }
                    | Frame::Refused { .. }
                    | Frame::Event { .. }
                    | Frame::Log { .. }
                    | Frame::Finished { .. }
            )
        })
        .map(Frame::name)
        .collect()
}

// --- Keel (and SSH servers that pretend to be it) -------------------------------------------

struct Authority {
    verifier: Arc<RunnerCertVerifier>,
    enrolled: Vec<KeyData>,
    auth_calls: Arc<AtomicUsize>,
}

impl RunnerAuthority for Authority {
    fn verifier(&self) -> Arc<RunnerCertVerifier> {
        Arc::clone(&self.verifier)
    }

    async fn is_enrolled(&self, _identity: &RunnerIdentity, key: &KeyData) -> bool {
        self.auth_calls.fetch_add(1, Ordering::SeqCst);
        self.enrolled.contains(key)
    }

    async fn bare_key(&self, _key: &KeyData, _now: SystemTime) -> BareKey {
        self.auth_calls.fetch_add(1, Ordering::SeqCst);
        BareKey::Unknown
    }
}

/// Keel's side of a session: on `Hello` it counts the hello, then sends `script`; it records
/// every frame and ends the session once a run ends.
struct Script {
    seen: mpsc::UnboundedSender<Frame>,
    hellos: Arc<AtomicUsize>,
    script: Vec<Frame>,
}

impl SubsystemService for Script {
    async fn serve(&self, access: Access, mut stream: ChannelStream<Msg>) {
        while let Ok(Some(frame)) = read_frame(&mut stream).await {
            if !access.permits(&frame) {
                return;
            }
            let _ = self.seen.send(frame.clone());
            match frame {
                Frame::Hello { .. } => {
                    // The hop strikes from here on: everything Keel sends next is the run.
                    self.hellos.fetch_add(1, Ordering::SeqCst);
                    for f in &self.script {
                        if write_frame(&mut stream, f).await.is_err() {
                            return;
                        }
                    }
                }
                Frame::Refused { .. } | Frame::Finished { .. } => return,
                _ => {}
            }
        }
    }
}

struct Server {
    port: u16,
    hellos: Arc<AtomicUsize>,
    auth_calls: Arc<AtomicUsize>,
    seen: mpsc::UnboundedReceiver<Frame>,
    frames: Vec<Frame>,
}

impl Server {
    fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    /// Every frame received so far.
    fn frames(&mut self) -> &[Frame] {
        while let Ok(f) = self.seen.try_recv() {
            self.frames.push(f);
        }
        &self.frames
    }

    /// Nothing that matters reached this server: no authentication decision, no frame.
    fn got_nothing(&mut self, what: &str) {
        assert_eq!(self.auth_calls.load(Ordering::SeqCst), 0, "{what}: auth");
        assert!(self.frames().is_empty(), "{what}: {:?}", self.frames());
    }
}

async fn ssh_server(
    pki: &Pki,
    host_key: PrivateKey,
    host_cert: Option<Certificate>,
    enrolled: Vec<KeyData>,
    script: Vec<Frame>,
) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut config = server_config(host_key, host_cert);
    config.auth_rejection_time = Duration::from_millis(1);
    let config = Arc::new(config);
    let auth_calls = Arc::new(AtomicUsize::new(0));
    let authority = Arc::new(Authority {
        verifier: Arc::new(RunnerCertVerifier::new(
            vec![TrustedCa::active(pki.runner_ca.key_data().clone())],
            Krl::default(),
        )),
        enrolled,
        auth_calls: Arc::clone(&auth_calls),
    });
    let (tx, seen) = mpsc::unbounded_channel();
    let hellos = Arc::new(AtomicUsize::new(0));
    let service = Arc::new(Script {
        seen: tx,
        hellos: Arc::clone(&hellos),
        script,
    });
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
    Server {
        port,
        hellos,
        auth_calls,
        seen,
        frames: vec![],
    }
}

/// An enrolled signed-mode host with a seeded ledger and its executor, and Keel with a run for
/// it. `keel_key` and `keel_cert` are Keel's real host identity.
struct Scene {
    pki: Pki,
    host: Host,
    base: String,
    keel: Server,
    keel_key: PrivateKey,
    keel_cert: Certificate,
    runner_cert: Certificate,
    ledger_before: String,
}

async fn scene() -> Scene {
    let pki = Pki::new();
    let host = Host::new(&pki);
    let (base, target, pack) = host.repo();
    host.seed_ledger(&base);
    let runner_cert = host.enroll(&pki);
    host.start_executor();
    let keel_key = key();
    let keel_cert = pki.host_cert(&keel_key, SERVER_NAME);
    let enrolled = vec![host.runner(vec![]).public_key().clone()];
    let keel = ssh_server(
        &pki,
        keel_key.clone(),
        Some(keel_cert.clone()),
        enrolled,
        run_frames(&base, &target, &pack),
    )
    .await;
    let ledger_before = host.ledger();
    Scene {
        pki,
        host,
        base,
        keel,
        keel_key,
        keel_cert,
        runner_cert,
        ledger_before,
    }
}

impl Scene {
    /// Nothing ran and nothing changed on the host, and Keel saw no run frame.
    fn nothing_ran(&mut self, what: &str) {
        assert!(!self.host.marker.exists(), "{what}: a playbook ran");
        assert_eq!(self.host.run_dirs(), 0, "{what}: a run directory");
        assert_eq!(
            self.host.ledger(),
            self.ledger_before,
            "{what}: the ledger changed"
        );
        assert!(
            acted(self.keel.frames()).is_empty(),
            "{what}: {:?}",
            acted(self.keel.frames())
        );
    }
}

// --- The hop: where the relay would be -----------------------------------------------------

/// What the hop does to the bytes it carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tamper {
    Honest,
    /// Flips one bit inside the first SSH packet Keel sends after the runner's `Hello`.
    Flip,
    /// Swaps two 32-byte chunks of Keel's stream right after `Hello`.
    Reorder,
    /// Drops 64 bytes of Keel's stream right after `Hello`.
    Drop,
    /// Writes a line before the runner's version banner, towards Keel.
    InjectUpstream,
    /// Writes 512 bytes before Keel's version banner, towards the runner.
    InjectDownstream,
}

/// How many bytes of Keel's post-`Hello` stream the hop holds back to tamper with.
const TAMPER_WINDOW: usize = 72;

/// One carried connection's bytes, each way.
#[derive(Debug, Default, Clone)]
struct Recording {
    up: Vec<u8>,
    down: Vec<u8>,
}

struct Hop {
    addr: String,
    accepts: Arc<AtomicUsize>,
    recordings: Arc<StdMutex<Vec<Arc<StdMutex<Recording>>>>>,
}

impl Hop {
    fn recording(&self, i: usize) -> Recording {
        let all = self.recordings.lock().unwrap();
        all[i].lock().unwrap().clone()
    }

    fn connections(&self) -> usize {
        self.recordings.lock().unwrap().len()
    }
}

/// A TCP hop to `upstream`. Keel's `hellos` count tells the hop when connection `k` is past
/// `Hello`: Keel bumps it before it writes the run, and the runner has read everything Keel sent
/// before `Hello` (it waited for the subsystem to open), so the next bytes read from Keel start an
/// SSH packet.
async fn hop(upstream: String, tamper: Tamper, hellos: Arc<AtomicUsize>) -> Hop {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let accepts = Arc::new(AtomicUsize::new(0));
    let recordings = Arc::new(StdMutex::new(vec![]));
    let (counted, recs) = (Arc::clone(&accepts), Arc::clone(&recordings));
    tokio::spawn(async move {
        while let Ok((runner, _)) = listener.accept().await {
            let k = counted.fetch_add(1, Ordering::SeqCst) + 1;
            let rec = Arc::new(StdMutex::new(Recording::default()));
            recs.lock().unwrap().push(Arc::clone(&rec));
            let Ok(keel) = TcpStream::connect(&upstream).await else {
                continue;
            };
            let (mut rr, mut rw) = runner.into_split();
            let (mut kr, mut kw) = keel.into_split();
            let up_rec = Arc::clone(&rec);
            tokio::spawn(async move {
                if tamper == Tamper::InjectUpstream
                    && kw.write_all(b"relay was here\r\n").await.is_err()
                {
                    return;
                }
                let mut buf = vec![0u8; 16 * 1024];
                while let Ok(n) = rr.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    up_rec.lock().unwrap().up.extend_from_slice(&buf[..n]);
                    if kw.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
                let _ = kw.shutdown().await;
            });
            let hellos = Arc::clone(&hellos);
            tokio::spawn(async move {
                if tamper == Tamper::InjectDownstream && rw.write_all(&[0u8; 512]).await.is_err() {
                    return;
                }
                let mut buf = vec![0u8; 16 * 1024];
                let mut held: Vec<u8> = vec![];
                let mut struck = !matches!(tamper, Tamper::Flip | Tamper::Reorder | Tamper::Drop);
                while let Ok(n) = kr.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    rec.lock().unwrap().down.extend_from_slice(&buf[..n]);
                    // Checked before these bytes reach the runner, so a chunk read before
                    // `Hello` is never mistaken for one after it.
                    if !struck && hellos.load(Ordering::SeqCst) >= k {
                        held.extend_from_slice(&buf[..n]);
                        if held.len() < TAMPER_WINDOW {
                            continue;
                        }
                        // Bytes 0..4 are the packet's (encrypted) length, left alone so the
                        // runner reads exactly one packet and checks its tag.
                        match tamper {
                            Tamper::Flip => held[6] ^= 0x01,
                            Tamper::Reorder => held[8..TAMPER_WINDOW].rotate_left(32),
                            Tamper::Drop => {
                                held.drain(8..TAMPER_WINDOW);
                            }
                            _ => unreachable!(),
                        }
                        struck = true;
                        if rw.write_all(&held).await.is_err() {
                            break;
                        }
                        continue;
                    }
                    if rw.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
                let _ = rw.shutdown().await;
            });
        }
    });
    Hop {
        addr,
        accepts,
        recordings,
    }
}

/// A relay that answers every connection with `bytes` (a recorded stream from Keel), then
/// swallows whatever the runner sends until it hangs up.
async fn replayer(bytes: Vec<u8>) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let accepts = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&accepts);
    let bytes = Arc::new(bytes);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            counted.fetch_add(1, Ordering::SeqCst);
            let bytes = Arc::clone(&bytes);
            tokio::spawn(async move {
                if socket.write_all(&bytes).await.is_err() {
                    return;
                }
                let mut sink = [0u8; 4096];
                while matches!(socket.read(&mut sink).await, Ok(n) if n > 0) {}
            });
        }
    });
    (addr, accepts)
}

async fn wait_for(what: &str, cond: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !cond() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Runs `runner.run_forever()` until `count` reaches `at_least`, and checks it is still running:
/// a hostile relay ends sessions, never the reconnect loop.
async fn the_loop_keeps_dialling(runner: Runner, count: impl Fn() -> usize, at_least: usize) {
    let task = tokio::spawn(async move {
        runner.run_forever().await;
    });
    wait_for("the runner to reconnect", || count() >= at_least).await;
    assert!(!task.is_finished(), "the reconnect loop stopped");
    task.abort();
}

// --- T024 ----------------------------------------------------------------------------------

#[tokio::test]
async fn an_honest_hop_carries_the_run_end_to_end() {
    let mut s = scene().await;
    let hop = hop(s.keel.addr(), Tamper::Honest, Arc::clone(&s.keel.hellos)).await;
    tokio::time::timeout(WAIT, s.host.runner(vec![hop.addr.clone()]).run_once())
        .await
        .expect("hung")
        .unwrap();
    assert!(s.host.marker.exists(), "the playbook did not run");
    assert_eq!(acted(s.keel.frames()), ["Started", "Finished"]);
    let ledger = s.host.ledger();
    assert!(ledger.starts_with(&s.ledger_before));
    assert_eq!(ledger.lines().count(), s.ledger_before.lines().count() + 2);
    // The hop saw only ciphertext: not even the run id crossed it in the clear.
    let rec = hop.recording(0);
    assert!(!contains(&rec.down, RUN.as_bytes()));
    assert!(!contains(&rec.up, s.base.as_bytes()));
}

/// One tampering hop: the session closes, nothing runs or changes, Keel acts on nothing, and the
/// reconnect loop goes on dialling.
async fn a_tampering_hop_gets_nothing_run(tamper: Tamper) {
    let mut s = scene().await;
    let hop = hop(s.keel.addr(), tamper, Arc::clone(&s.keel.hellos)).await;
    let result = tokio::time::timeout(WAIT, s.host.runner(vec![hop.addr.clone()]).run_once())
        .await
        .unwrap_or_else(|_| panic!("{tamper:?}: the session did not close"));
    eprintln!("{tamper:?}: run_once ended with {result:?}");
    match tamper {
        // The tamper landed mid-session, after Keel accepted the runner and got Hello.
        Tamper::Flip | Tamper::Reorder | Tamper::Drop => {
            assert_eq!(s.keel.hellos.load(Ordering::SeqCst), 1, "{tamper:?}");
        }
        _ => {
            assert_eq!(s.keel.hellos.load(Ordering::SeqCst), 0, "{tamper:?}");
            assert_eq!(s.keel.auth_calls.load(Ordering::SeqCst), 0, "{tamper:?}");
        }
    }
    s.nothing_ran(&format!("{tamper:?}"));

    let accepts = Arc::clone(&hop.accepts);
    the_loop_keeps_dialling(
        s.host.runner(vec![hop.addr.clone()]),
        move || accepts.load(Ordering::SeqCst),
        3,
    )
    .await;
    s.nothing_ran(&format!("{tamper:?}, reconnecting"));
    assert!(hop.connections() >= 3);
}

#[tokio::test]
async fn a_flipped_byte_mid_session_closes_it_and_nothing_runs() {
    a_tampering_hop_gets_nothing_run(Tamper::Flip).await;
}

#[tokio::test]
async fn reordered_chunks_close_the_session_and_nothing_runs() {
    a_tampering_hop_gets_nothing_run(Tamper::Reorder).await;
}

#[tokio::test]
async fn dropped_chunks_close_the_session_and_nothing_runs() {
    a_tampering_hop_gets_nothing_run(Tamper::Drop).await;
}

#[tokio::test]
async fn bytes_before_the_runners_banner_fail_the_handshake() {
    a_tampering_hop_gets_nothing_run(Tamper::InjectUpstream).await;
}

#[tokio::test]
async fn bytes_before_keels_banner_fail_the_handshake() {
    a_tampering_hop_gets_nothing_run(Tamper::InjectDownstream).await;
}

#[tokio::test]
async fn a_replayed_session_runs_nothing_either_way() {
    let mut s = scene().await;
    // Record a real session that ran the job.
    let hop = hop(s.keel.addr(), Tamper::Honest, Arc::clone(&s.keel.hellos)).await;
    tokio::time::timeout(WAIT, s.host.runner(vec![hop.addr.clone()]).run_once())
        .await
        .expect("hung")
        .unwrap();
    assert!(s.host.marker.exists(), "the recorded session did not run");
    let recorded = hop.recording(0);
    std::fs::remove_file(&s.host.marker).unwrap();
    s.ledger_before = s.host.ledger();
    let dirs_before = s.host.run_dirs();
    let frames_before = s.keel.frames().len();

    // Keel's half, replayed to the runner: its fresh key exchange does not verify.
    let (addr, accepts) = replayer(recorded.down.clone()).await;
    let result = tokio::time::timeout(WAIT, s.host.runner(vec![addr.clone()]).run_once())
        .await
        .expect("the replayed session did not close");
    assert!(result.is_err(), "{result:?}");
    let counted = Arc::clone(&accepts);
    the_loop_keeps_dialling(
        s.host.runner(vec![addr]),
        move || counted.load(Ordering::SeqCst),
        3,
    )
    .await;
    assert!(!s.host.marker.exists(), "a replay ran the playbook");
    assert_eq!(s.host.run_dirs(), dirs_before);
    assert_eq!(s.host.ledger(), s.ledger_before, "the ledger changed");

    // The runner's half, replayed to Keel: no authentication, no Hello, no frame.
    let hellos = s.keel.hellos.load(Ordering::SeqCst);
    let auth = s.keel.auth_calls.load(Ordering::SeqCst);
    let mut keel = TcpStream::connect(s.keel.addr()).await.unwrap();
    keel.write_all(&recorded.up).await.unwrap();
    let mut sink = vec![];
    tokio::time::timeout(WAIT, keel.read_to_end(&mut sink))
        .await
        .expect("Keel kept a replayed session open")
        .ok();
    assert_eq!(s.keel.hellos.load(Ordering::SeqCst), hellos);
    assert_eq!(s.keel.auth_calls.load(Ordering::SeqCst), auth);
    assert_eq!(s.keel.frames().len(), frames_before);
}

// --- T063 ----------------------------------------------------------------------------------

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

/// The SSH message numbers in a client's cleartext stream (its banner line, then unencrypted
/// binary packets) and how many bytes follow the last packet that parsed: an encrypted packet,
/// or nothing.
fn client_messages(raw: &[u8]) -> (Vec<u8>, usize) {
    let Some(eol) = raw.iter().position(|&b| b == b'\n') else {
        return (vec![], raw.len());
    };
    let mut rest = &raw[eol + 1..];
    let mut out = vec![];
    while rest.len() >= 6 {
        let len = u32::from_be_bytes(rest[..4].try_into().unwrap()) as usize;
        if !(5..=35_000).contains(&len) || rest.len() < 4 + len {
            break;
        }
        out.push(rest[5]);
        rest = &rest[4 + len..];
    }
    (out, rest.len())
}

const MSG_DISCONNECT: u8 = 1;
const MSG_KEXINIT: u8 = 20;
const MSG_NEWKEYS: u8 = 21;
const MSG_KEX_ECDH_INIT: u8 = 30;
const MSG_KEX_ECDH_REPLY: u8 = 31;

/// What a client may send a server whose host key it has not yet accepted. russh sends NEWKEYS
/// once the exchange signature verifies and checks the host key after it, so NEWKEYS is allowed
/// here; anything encrypted after it is not.
fn only_key_exchange(msgs: &[u8]) -> bool {
    msgs.iter().all(|m| {
        matches!(
            *m,
            MSG_KEXINIT | MSG_KEX_ECDH_INIT | MSG_NEWKEYS | MSG_DISCONNECT
        )
    })
}

#[tokio::test]
async fn a_relay_answering_with_its_own_ssh_server_gets_no_auth_and_no_ledger() {
    let mut s = scene().await;
    // Its own bare key, and a certificate from Keel's CA for the relay's own name. (Another
    // CA's certificate and enrollment are covered in dial.rs.)
    let mut bare = ssh_server(&s.pki, key(), None, vec![], vec![]).await;
    let k = key();
    let cert = s.pki.host_cert(&k, "relay-1.zone");
    let mut named = ssh_server(&s.pki, k, Some(cert), vec![], vec![]).await;
    let no_hellos = Arc::new(AtomicUsize::new(0));
    let bare_hop = hop(bare.addr(), Tamper::Honest, Arc::clone(&no_hellos)).await;
    let named_hop = hop(named.addr(), Tamper::Honest, no_hellos).await;

    let (a, b) = (
        Arc::clone(&bare_hop.accepts),
        Arc::clone(&named_hop.accepts),
    );
    the_loop_keeps_dialling(
        s.host
            .runner(vec![bare_hop.addr.clone(), named_hop.addr.clone()]),
        move || a.load(Ordering::SeqCst).min(b.load(Ordering::SeqCst)),
        2,
    )
    .await;

    bare.got_nothing("a bare key");
    named.got_nothing("the relay's own name");
    let ledger = s.host.ledger();
    let cert_blob = s.runner_cert.encode_vec().unwrap();
    for (what, hop) in [("bare", &bare_hop), ("named", &named_hop)] {
        for i in 0..hop.connections() {
            let up = hop.recording(i).up;
            let (msgs, encrypted) = client_messages(&up);
            assert!(only_key_exchange(&msgs), "{what}: {msgs:?}");
            assert_eq!(
                encrypted, 0,
                "{what}: the runner sent encrypted bytes ({msgs:?})"
            );
            assert!(!contains(&up, s.base.as_bytes()), "{what}: a base left");
            assert!(
                !contains(&up, SEED_RUN.as_bytes()),
                "{what}: ledger bytes left"
            );
            assert!(!contains(&up, ledger.as_bytes()), "{what}: the ledger left");
            assert!(!contains(&up, &cert_blob), "{what}: the certificate left");
        }
    }
    s.nothing_ran("own server");
}

/// What a hand-written relay SSH server saw on one connection.
#[derive(Debug, Clone, Default)]
struct Seen {
    /// Every byte the runner sent.
    raw: Vec<u8>,
    /// The message numbers of the runner's cleartext packets.
    msgs: Vec<u8>,
}

/// How the hand-written relay behaves after the banners.
#[derive(Clone)]
enum Rogue {
    /// Offers keel-exec's algorithms and Keel's host certificate, signs the exchange with
    /// `signer`, and sends `NEWKEYS`.
    Kex {
        cert: Box<Certificate>,
        signer: PrivateKey,
    },
    /// Offers `lists` (the ten KEXINIT name-lists) and records what the runner does next.
    Offer { lists: [String; 10] },
}

/// keel-exec's algorithms as a server's KEXINIT name-lists. Both are AEAD, so no MAC.
fn good_lists() -> [String; 10] {
    let cipher = "chacha20-poly1305@openssh.com".to_owned();
    [
        "curve25519-sha256,kex-strict-s-v00@openssh.com".to_owned(),
        "ssh-ed25519-cert-v01@openssh.com".to_owned(),
        cipher.clone(),
        cipher,
        String::new(),
        String::new(),
        "none".to_owned(),
        "none".to_owned(),
        String::new(),
        String::new(),
    ]
}

/// A raw SSH connection that records every byte the peer sends.
struct Wire {
    s: TcpStream,
    seen: Seen,
}

impl Wire {
    async fn exact(&mut self, n: usize) -> std::io::Result<Vec<u8>> {
        let mut buf = vec![0u8; n];
        self.s.read_exact(&mut buf).await?;
        self.seen.raw.extend_from_slice(&buf);
        Ok(buf)
    }

    /// The peer's version line, without its line ending.
    async fn banner(&mut self) -> std::io::Result<Vec<u8>> {
        let mut line = vec![];
        while line.last() != Some(&b'\n') {
            line.extend(self.exact(1).await?);
            if line.len() > 255 {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
        }
        while matches!(line.last(), Some(b'\n' | b'\r')) {
            line.pop();
        }
        Ok(line)
    }

    /// One unencrypted binary packet's payload.
    async fn packet(&mut self) -> std::io::Result<Vec<u8>> {
        let len = u32::from_be_bytes(self.exact(4).await?.try_into().unwrap()) as usize;
        if !(5..=35_000).contains(&len) {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        let body = self.exact(len).await?;
        let pad = body[0] as usize;
        if pad + 1 >= len {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        let payload = body[1..len - pad].to_vec();
        self.seen.msgs.push(payload[0]);
        Ok(payload)
    }

    async fn send(&mut self, payload: &[u8]) -> std::io::Result<()> {
        let mut pad = 8 - (5 + payload.len()) % 8;
        if pad < 4 {
            pad += 8;
        }
        let mut out = ((1 + payload.len() + pad) as u32).to_be_bytes().to_vec();
        out.push(pad as u8);
        out.extend_from_slice(payload);
        out.extend(std::iter::repeat_n(0u8, pad));
        self.s.write_all(&out).await
    }
}

fn put_string(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

fn kexinit(lists: &[String; 10]) -> Vec<u8> {
    let mut out = vec![MSG_KEXINIT];
    out.extend_from_slice(&[0x5a; 16]);
    for list in lists {
        put_string(&mut out, list.as_bytes());
    }
    out.push(0);
    out.extend_from_slice(&[0; 4]);
    out
}

/// The string fields of a packet after its message number.
fn strings(payload: &[u8]) -> Vec<Vec<u8>> {
    let mut rest = &payload[1..];
    let mut out = vec![];
    while rest.len() >= 4 {
        let len = u32::from_be_bytes(rest[..4].try_into().unwrap()) as usize;
        if rest.len() < 4 + len {
            break;
        }
        out.push(rest[4..4 + len].to_vec());
        rest = &rest[4 + len..];
    }
    out
}

/// `K` as an SSH mpint: the X25519 output read as a big-endian unsigned integer (RFC 8731).
fn mpint(k: &[u8]) -> Vec<u8> {
    let k = &k[k.iter().position(|&b| b != 0).unwrap_or(k.len())..];
    let mut out = vec![];
    if k.first().is_some_and(|b| b & 0x80 != 0) {
        out.push(0);
    }
    out.extend_from_slice(k);
    out
}

/// One connection to the hand-written relay.
async fn rogue_session(s: TcpStream, mode: &Rogue) -> Seen {
    let mut w = Wire {
        s,
        seen: Seen::default(),
    };
    let v_s = b"SSH-2.0-OpenSSH_9.9".to_vec();
    let _ = async {
        w.s.write_all(&[v_s.as_slice(), b"\r\n"].concat()).await?;
        let v_c = w.banner().await?;
        let lists = match mode {
            Rogue::Kex { .. } => good_lists(),
            Rogue::Offer { lists } => lists.clone(),
        };
        let i_s = kexinit(&lists);
        w.send(&i_s).await?;
        let i_c = w.packet().await?;
        let Rogue::Kex { cert, signer } = mode else {
            // Record what follows the offer until the runner hangs up or starts the exchange.
            while w.packet().await?[0] != MSG_KEX_ECDH_INIT {}
            return Ok(());
        };
        let init = w.packet().await?;
        if init[0] != MSG_KEX_ECDH_INIT {
            return Ok(());
        }
        let q_c: [u8; 32] = strings(&init)[0].as_slice().try_into().unwrap();
        // A fixed ephemeral is fine for a test: the runner's side is fresh.
        let secret = [0x42u8; 32];
        let q_s = x25519::scalarmult(&secret, &x25519::BASE);
        let k = x25519::scalarmult(&secret, &q_c);
        let k_s = cert.encode_vec().unwrap();
        let mut hashed = vec![];
        for field in [&v_c, &v_s, &i_c, &i_s, &k_s, &q_c.to_vec(), &q_s.to_vec()] {
            put_string(&mut hashed, field);
        }
        put_string(&mut hashed, &mpint(&k));
        let h = Sha256::digest(&hashed).to_vec();
        let sig: russh::keys::ssh_key::Signature = signer.try_sign(&h).unwrap();
        let mut sig_blob = vec![];
        put_string(&mut sig_blob, b"ssh-ed25519");
        put_string(&mut sig_blob, sig.as_bytes());
        let mut reply = vec![MSG_KEX_ECDH_REPLY];
        put_string(&mut reply, &k_s);
        put_string(&mut reply, &q_s);
        put_string(&mut reply, &sig_blob);
        w.send(&reply).await?;
        w.send(&[MSG_NEWKEYS]).await?;
        // The runner's answer: NEWKEYS if it accepted us, nothing if it did not.
        w.packet().await?;
        std::io::Result::Ok(())
    }
    .await;
    w.seen
}

/// The hand-written relay: one [`rogue_session`] per connection, recorded in order.
async fn rogue(mode: Rogue) -> (String, Arc<StdMutex<Vec<Seen>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let seen = Arc::new(StdMutex::new(vec![]));
    let record = Arc::clone(&seen);
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let got = tokio::time::timeout(WAIT, rogue_session(socket, &mode))
                .await
                .unwrap_or_default();
            record.lock().unwrap().push(got);
        }
    });
    (addr, seen)
}

/// A host with an enrollment token and no certificate, dialling only `addr`.
fn token_host(pki: &Pki) -> Host {
    let host = Host::new(pki);
    host.write_token(pki);
    host
}

#[tokio::test]
async fn the_hand_written_relay_is_a_correct_ssh_server_with_keels_own_key() {
    // The control for the next test: with Keel's real key the runner accepts the exchange and
    // sends NEWKEYS, so a refusal there is the key, not a broken server.
    let s = scene().await;
    let (addr, seen) = rogue(Rogue::Kex {
        cert: Box::new(s.keel_cert.clone()),
        signer: s.keel_key.clone(),
    })
    .await;
    let host = token_host(&s.pki);
    let result = tokio::time::timeout(WAIT, host.runner(vec![addr]).run_once())
        .await
        .expect("hung");
    assert!(result.is_err(), "{result:?}");
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].msgs,
        [MSG_KEXINIT, MSG_KEX_ECDH_INIT, MSG_NEWKEYS],
        "the runner did not accept a correct exchange"
    );
    assert!(host.token_kept());
}

#[tokio::test]
async fn a_relay_replaying_keels_certificate_with_another_key_gets_nothing() {
    let mut s = scene().await;
    let (addr, seen) = rogue(Rogue::Kex {
        cert: Box::new(s.keel_cert.clone()),
        signer: key(),
    })
    .await;

    // Enrollment: the token never leaves.
    let host = token_host(&s.pki);
    let result = tokio::time::timeout(WAIT, host.runner(vec![addr.clone()]).run_once())
        .await
        .expect("hung");
    assert!(result.is_err(), "{result:?}");
    assert!(host.token_kept());

    // An enrolled host's session: no certificate, no Hello, no ledger; the loop goes on.
    let count = Arc::clone(&seen);
    the_loop_keeps_dialling(
        s.host.runner(vec![addr]),
        move || count.lock().unwrap().len(),
        3,
    )
    .await;

    let ledger = s.host.ledger();
    let cert_blob = s.runner_cert.encode_vec().unwrap();
    let seen = seen.lock().unwrap().clone();
    assert!(seen.len() >= 3);
    for (i, got) in seen.iter().enumerate() {
        assert!(
            !got.msgs.contains(&MSG_NEWKEYS) && only_key_exchange(&got.msgs),
            "connection {i}: {:?}",
            got.msgs
        );
        assert_eq!(got.msgs[..2], [MSG_KEXINIT, MSG_KEX_ECDH_INIT], "{i}");
        assert!(!contains(&got.raw, TOKEN_SECRET.as_bytes()), "{i}: token");
        assert!(!contains(&got.raw, s.base.as_bytes()), "{i}: a base");
        assert!(!contains(&got.raw, ledger.as_bytes()), "{i}: the ledger");
        assert!(!contains(&got.raw, &cert_blob), "{i}: the certificate");
    }
    s.nothing_ran("Keel's certificate, another key");
}

#[tokio::test]
async fn an_algorithm_downgrade_is_refused_before_the_key_exchange() {
    let preferred = server_preferred();
    let client = client_preferred();
    // Whether keel-exec accepts `name` in KEXINIT name-list `field`.
    let ours = |field: usize, name: &str| match field {
        0 => {
            preferred.kex.iter().any(|n| n.as_ref() == name)
                || client.kex.iter().any(|n| n.as_ref() == name)
        }
        1 => {
            preferred.key.iter().any(|a| a.as_str() == name)
                || client
                    .host_key_certificates
                    .iter()
                    .any(|a| a.to_certificate_type() == name)
        }
        2 | 3 => preferred.cipher.iter().any(|n| n.as_ref() == name),
        6 | 7 => preferred.compression.iter().any(|n| n.as_ref() == name),
        _ => unreachable!(),
    };
    // (field index, a list of names keel-exec does not accept)
    let cases: [(usize, &str); 5] = [
        (
            0,
            "diffie-hellman-group14-sha1,diffie-hellman-group1-sha1,ecdh-sha2-nistp256,\
             diffie-hellman-group-exchange-sha256,diffie-hellman-group16-sha512",
        ),
        (
            1,
            "ssh-rsa,rsa-sha2-512,ecdsa-sha2-nistp256,ssh-dss,ssh-rsa-cert-v01@openssh.com",
        ),
        (
            2,
            "aes128-ctr,aes256-ctr,aes128-cbc,3des-cbc,none,aes128-gcm@openssh.com",
        ),
        (6, "zlib@openssh.com,zlib"),
        (7, "zlib@openssh.com"),
    ];
    let pki = Pki::new();
    for (field, names) in cases {
        for name in names.split(',') {
            assert!(!ours(field, name), "{name} is in keel-exec's lists");
        }
        let mut lists = good_lists();
        lists[field] = names.to_owned();
        let (addr, seen) = rogue(Rogue::Offer { lists }).await;
        let host = token_host(&pki);
        let result = tokio::time::timeout(WAIT, host.runner(vec![addr]).run_once())
            .await
            .expect("hung");
        assert!(result.is_err(), "{names}: {result:?}");
        wait_for("the relay to record", || seen.lock().unwrap().len() == 1).await;
        let got = seen.lock().unwrap()[0].clone();
        assert_eq!(got.msgs.first(), Some(&MSG_KEXINIT), "{names}");
        assert!(
            !got.msgs.contains(&MSG_KEX_ECDH_INIT)
                && got
                    .msgs
                    .iter()
                    .all(|m| matches!(*m, MSG_KEXINIT | MSG_DISCONNECT)),
            "{names}: {:?}",
            got.msgs
        );
        assert!(!contains(&got.raw, TOKEN_SECRET.as_bytes()), "{names}");
        assert!(host.token_kept(), "{names}");
    }

    // The control: the same offer path with keel-exec's own lists gets a key exchange.
    // A weak server-to-client cipher list behaves the same: russh 0.65 picks one cipher from
    // the client-to-server list and uses it both ways, never reading the other list, so a relay
    // cannot steer either direction onto a cipher outside keel-exec's (it is not refused
    // either; see the 067 T063 report).
    let mut weak_s2c = good_lists();
    weak_s2c[3] = "aes256-cbc,none".to_owned();
    for name in weak_s2c[3].split(',') {
        assert!(!ours(3, name), "{name} is in keel-exec's lists");
    }
    for lists in [good_lists(), weak_s2c] {
        let (addr, seen) = rogue(Rogue::Offer { lists }).await;
        let host = token_host(&pki);
        let _ = tokio::time::timeout(WAIT, host.runner(vec![addr]).run_once())
            .await
            .expect("hung");
        wait_for("the relay to record", || seen.lock().unwrap().len() == 1).await;
        assert_eq!(
            seen.lock().unwrap()[0].msgs,
            [MSG_KEXINIT, MSG_KEX_ECDH_INIT]
        );
        assert!(host.token_kept());
    }
}

/// X25519 (RFC 7748) for the hand-written relay's key exchange, after TweetNaCl. Test code only:
/// it lets the relay compute the exchange hash without another dependency.
mod x25519 {
    type Gf = [i64; 16];

    pub const BASE: [u8; 32] = {
        let mut b = [0u8; 32];
        b[0] = 9;
        b
    };

    const A24: Gf = {
        let mut g = [0i64; 16];
        g[0] = 0xdb41;
        g[1] = 1;
        g
    };

    fn carry(o: &mut Gf) {
        for i in 0..16 {
            o[i] += 1 << 16;
            let c = o[i] >> 16;
            if i < 15 {
                o[i + 1] += c - 1;
            } else {
                o[0] += 38 * (c - 1);
            }
            o[i] -= c << 16;
        }
    }

    fn swap(p: &mut Gf, q: &mut Gf, bit: i64) {
        let mask = !(bit - 1);
        for i in 0..16 {
            let t = mask & (p[i] ^ q[i]);
            p[i] ^= t;
            q[i] ^= t;
        }
    }

    fn add(a: &Gf, b: &Gf) -> Gf {
        std::array::from_fn(|i| a[i] + b[i])
    }

    fn sub(a: &Gf, b: &Gf) -> Gf {
        std::array::from_fn(|i| a[i] - b[i])
    }

    fn mul(a: &Gf, b: &Gf) -> Gf {
        let mut t = [0i64; 31];
        for i in 0..16 {
            for j in 0..16 {
                t[i + j] += a[i] * b[j];
            }
        }
        for i in 0..15 {
            t[i] += 38 * t[i + 16];
        }
        let mut o: Gf = std::array::from_fn(|i| t[i]);
        carry(&mut o);
        carry(&mut o);
        o
    }

    fn invert(i: &Gf) -> Gf {
        let mut c = *i;
        for a in (0..=253).rev() {
            c = mul(&c, &c);
            if a != 2 && a != 4 {
                c = mul(&c, i);
            }
        }
        c
    }

    fn unpack(n: &[u8; 32]) -> Gf {
        let mut o: Gf = std::array::from_fn(|i| n[2 * i] as i64 + ((n[2 * i + 1] as i64) << 8));
        o[15] &= 0x7fff;
        o
    }

    fn pack(n: &Gf) -> [u8; 32] {
        let mut t = *n;
        carry(&mut t);
        carry(&mut t);
        carry(&mut t);
        let mut m = [0i64; 16];
        for _ in 0..2 {
            m[0] = t[0] - 0xffed;
            for i in 1..15 {
                m[i] = t[i] - 0xffff - ((m[i - 1] >> 16) & 1);
                m[i - 1] &= 0xffff;
            }
            m[15] = t[15] - 0x7fff - ((m[14] >> 16) & 1);
            let b = (m[15] >> 16) & 1;
            m[14] &= 0xffff;
            swap(&mut t, &mut m, 1 - b);
        }
        let mut o = [0u8; 32];
        for i in 0..16 {
            o[2 * i] = (t[i] & 0xff) as u8;
            o[2 * i + 1] = ((t[i] >> 8) & 0xff) as u8;
        }
        o
    }

    pub fn scalarmult(n: &[u8; 32], p: &[u8; 32]) -> [u8; 32] {
        let mut z = *n;
        z[31] = (n[31] & 127) | 64;
        z[0] &= 248;
        let x = unpack(p);
        let (mut a, mut b, mut c, mut d) = ([0i64; 16], x, [0i64; 16], [0i64; 16]);
        a[0] = 1;
        d[0] = 1;
        for i in (0..=254).rev() {
            let r = ((z[i >> 3] >> (i & 7)) & 1) as i64;
            swap(&mut a, &mut b, r);
            swap(&mut c, &mut d, r);
            let mut e = add(&a, &c);
            a = sub(&a, &c);
            c = add(&b, &d);
            b = sub(&b, &d);
            d = mul(&e, &e);
            let f = mul(&a, &a);
            a = mul(&c, &a);
            c = mul(&b, &e);
            e = add(&a, &c);
            a = sub(&a, &c);
            b = mul(&a, &a);
            c = sub(&d, &f);
            a = mul(&c, &A24);
            a = add(&a, &d);
            c = mul(&c, &a);
            a = mul(&d, &f);
            d = mul(&b, &x);
            b = mul(&e, &e);
            swap(&mut a, &mut b, r);
            swap(&mut c, &mut d, r);
        }
        pack(&mul(&a, &invert(&c)))
    }

    #[test]
    fn rfc_7748_vector() {
        let hex = |s: &str| -> [u8; 32] {
            std::array::from_fn(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        };
        assert_eq!(
            scalarmult(
                &hex("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4"),
                &hex("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c"),
            ),
            hex("c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552")
        );
    }
}
