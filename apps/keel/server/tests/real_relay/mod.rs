//! The real relay, Keel and a real runner in one process (spec 067): shared by `relay_link.rs`
//! (US1) and `relay_failover.rs` (US4).
//!
//! The relay runs on its own thread with a current-thread runtime and a thread-scoped tracing
//! subscriber, so every event it and its tasks emit (including russh's `log` records) lands in
//! one buffer, and nothing from Keel or the runner does. Executor: a fake on the Unix socket
//! (no Ansible).

#![allow(dead_code)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use keel_exec::codec::{read_frame, write_frame};
use keel_exec::events::HostStats;
use keel_exec::frames::{Frame, LogStream};
use keel_protocol::{Cmd, Event, HostName, HostProfiles, ProfileName};
use keel_relay::Relay;
use keel_relay::config::Config as RelayConfig;
use keel_runner::runner::{HOST_ID_FILE, Runner, RunnerConfig, TOKEN_FILE};
use keel_server::audit::Actor;
use keel_server::hosts;
use keel_server::relays::{self, RelayLive, RelayRegistration, RelaySupervisor, SupervisorOptions};
use keel_server::runner_listener::RunnerHost;
use keel_server::runs::{RunSettings, Runs};
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, PrivateKey};
use tokio::net::UnixListener;
use tokio::sync::oneshot;
use tracing_subscriber::util::SubscriberInitExt as _;
use uuid::Uuid;

use crate::repos::{Harness, Source, bind};

pub const WAIT: Duration = Duration::from_secs(30);
/// Keel's runner host name: `KEEL_PUBLIC_URL`'s host in the harness.
pub const SERVER_NAME: &str = "keel.example.com";
pub const INVENTORY_NAME: &str = "web-1";
pub const LOG_LINE: &[u8] = b"TASK [install] via the relay\n";

/// The `web` job runs on pushes under `playbooks/`, on inventory `prod`.
pub const MANIFEST: &str = "version: 1\nprofiles: { debian: {} }\njobs:\n  web:\n    \
                        playbook: playbooks/web.yaml\n    profiles: [debian]\n    \
                        inventory: prod\n    paths: ['playbooks/**']\n";

pub fn key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("key")
}

// ---- the relay, on its own thread, with its tracing captured ---------------------------------

/// A shared buffer the relay's tracing subscriber writes to.
#[derive(Clone, Default)]
pub struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("lock").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Captured {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("lock")).into_owned()
    }

    pub fn count(&self, needle: &str) -> usize {
        self.text().matches(needle).count()
    }
}

/// Routes `log` records (russh's) into tracing, so they reach whichever subscriber is current
/// on the emitting thread. The global default is a registry with no layers: Keel's and the
/// runner's events go nowhere, the relay thread's go to its buffer.
pub fn bridge_log() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = tracing_subscriber::registry().try_init();
    });
}

/// The real relay, serving on ephemeral loopback ports.
pub struct RunningRelay {
    pub link: SocketAddr,
    pub runner: SocketAddr,
    pub host_key: PrivateKey,
    pub logs: Captured,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl RunningRelay {
    pub fn start(hub_key: KeyData) -> Self {
        bridge_log();
        let logs = Captured::default();
        let host_key = key();
        let (addr_tx, addr_rx) = std::sync::mpsc::channel();
        let (stop, stop_rx) = oneshot::channel::<()>();
        let writer = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        let relay_key = host_key.clone();
        let thread = std::thread::spawn(move || {
            tracing::subscriber::with_default(subscriber, || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("relay runtime");
                rt.block_on(async move {
                    let config = RelayConfig {
                        link_listen: "127.0.0.1:0".parse().expect("addr"),
                        runner_listen: "127.0.0.1:0".parse().expect("addr"),
                        hub_keys: "/unused/hub_keys".into(),
                        http_listen: None,
                        max_streams: 256,
                    };
                    let relay = Relay::bind(&config, relay_key, vec![hub_key])
                        .await
                        .expect("bind the relay");
                    addr_tx
                        .send((relay.link_addr(), relay.runner_addr()))
                        .expect("addresses");
                    let _ = relay
                        .run_until(async {
                            let _ = stop_rx.await;
                        })
                        .await;
                });
            });
        });
        let (link, runner) = addr_rx.recv().expect("the relay bound");
        Self {
            link,
            runner,
            host_key,
            logs,
            stop: Some(stop),
            thread: Some(thread),
        }
    }

    pub fn host_key_data(&self) -> KeyData {
        self.host_key.public_key().key_data().clone()
    }
}

impl Drop for RunningRelay {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

// ---- Keel ----------------------------------------------------------------------------------

/// A harness with the SSH CAs, the relay-link key, fast run settings, a zone `edge`, a binding
/// running `web` on inventory `prod` (which lists `web-1`), no approvals, and the `debian`
/// profile.
pub async fn harness(test: &str) -> Option<(Harness, Uuid, Uuid, Source)> {
    let mut h = Harness::new(test).await?;
    h.app.runs = Runs::new(RunSettings {
        ping_interval: Duration::from_millis(200),
        flush_interval: Duration::from_millis(20),
        ..RunSettings::default()
    });
    hosts::bootstrap(&h.app).await.expect("ssh bootstrap");
    relays::bootstrap(&h.app).await.expect("relay bootstrap");
    let owner = &h.db.owner;
    let zone = Uuid::now_v7();
    sqlx::query("insert into zones (id, workspace_id, name) values ($1, $2, 'edge')")
        .bind(zone)
        .bind(h.workspace)
        .execute(owner)
        .await
        .expect("zone");
    let src = Source::new(MANIFEST);
    let binding = bind(&h, &src).await;
    let inventory = Uuid::now_v7();
    sqlx::query(
        "insert into inventories (id, workspace_id, name, source) values ($1, $2, 'prod', 'managed')",
    )
    .bind(inventory)
    .bind(h.workspace)
    .execute(owner)
    .await
    .expect("inventory");
    let revision = Uuid::now_v7();
    sqlx::query(
        "insert into inventory_revisions (id, inventory_id, seq, content, content_hash, status) \
         values ($1, $2, 1, '{}', 'h1', 'valid')",
    )
    .bind(revision)
    .bind(inventory)
    .execute(owner)
    .await
    .expect("revision");
    sqlx::query("insert into inventory_hosts (revision_id, host, groups) values ($1, $2, '{web}')")
        .bind(revision)
        .bind(INVENTORY_NAME)
        .execute(owner)
        .await
        .expect("inventory host");
    sqlx::query(
        "insert into approval_policies (id, workspace_id, priority, requires_approval) \
         values ($1, $2, 1, false)",
    )
    .bind(Uuid::now_v7())
    .bind(h.workspace)
    .execute(owner)
    .await
    .expect("policy");
    sqlx::query(
        "insert into profiles (id, workspace_id, name) select $1, $2, 'debian' \
         where not exists (select 1 from profiles where workspace_id = $2 and name = 'debian')",
    )
    .bind(Uuid::now_v7())
    .bind(h.workspace)
    .execute(owner)
    .await
    .expect("profile");
    Some((h, zone, binding, src))
}

/// Registers the relay `edge-1` at `addr`, pinning `host_key`.
pub async fn register(h: &Harness, zone: Uuid, addr: &str, host_key: &KeyData) -> (Uuid, String) {
    register_named(h, zone, "edge-1", addr, host_key).await
}

/// Registers the relay `name` at `addr`, pinning `host_key`.
pub async fn register_named(
    h: &Harness,
    zone: Uuid,
    name: &str,
    addr: &str,
    host_key: &KeyData,
) -> (Uuid, String) {
    let line = russh::keys::PublicKey::from(host_key.clone())
        .to_openssh()
        .expect("line");
    let mut tx = h.app.db.begin().await.expect("tx");
    let r = relays::register_relay(
        &mut tx,
        &Actor::System,
        &RelayRegistration {
            workspace: h.workspace,
            zone,
            name,
            dial_address: addr,
            host_key: &line,
            host: None,
        },
    )
    .await
    .expect("register");
    tx.commit().await.expect("commit");
    (r.id, r.fingerprint)
}

pub fn supervisor(h: &Harness) -> RelaySupervisor {
    RelaySupervisor::new(
        h.app.clone(),
        RunnerHost::new().expect("host"),
        SupervisorOptions {
            health_interval: Duration::from_millis(200),
            hello_timeout: Duration::from_secs(5),
            ..SupervisorOptions::default()
        },
    )
}

/// Mints an enrollment token for `web-1` (profile `debian`).
pub async fn mint(h: &Harness) -> String {
    let cmd = Cmd::PreviewHostEnroll {
        workspace: h.workspace,
        inventory_name: HostName::new(INVENTORY_NAME).expect("name"),
        profiles: HostProfiles(vec![
            ProfileName::new("debian".to_owned()).expect("profile"),
        ]),
    };
    let (_, out) = h.confirm(cmd).await;
    match out.as_slice() {
        [Event::Confirmed { .. }, Event::HostEnrollment { token, .. }] => token.0.clone(),
        other => panic!("enrol: {other:?}"),
    }
}

/// Polls `check` every 20 ms until it returns `Some`, failing after [`WAIT`] with `what`.
pub async fn until<T, F, Fut>(what: &str, mut check: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(v) = check().await {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

pub async fn wait_live(h: &Harness, id: Uuid, what: &str, ok: impl Fn(&RelayLive) -> bool) {
    until(what, || {
        let matched = h.app.relay_live.get(id).is_some_and(|l| ok(&l));
        async move { matched.then_some(()) }
    })
    .await;
}

// ---- the runner host -------------------------------------------------------------------------

/// A runner host's directories: trust files for Keel's host CA, a state dir, and a fake
/// executor on the Unix socket.
pub struct RunnerHostDir {
    _dir: tempfile::TempDir,
    pub trust: PathBuf,
    pub state: PathBuf,
    pub venv: PathBuf,
    pub socket: PathBuf,
}

impl RunnerHostDir {
    pub fn new(host_ca_line: &str) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let trust = dir.path().join("trust");
        let state = dir.path().join("state");
        std::fs::create_dir_all(&trust).expect("trust");
        std::fs::create_dir_all(&state).expect("state");
        std::fs::write(
            trust.join("policy.toml"),
            format!(
                "origin = \"https://{SERVER_NAME}\"\nrp_id = \"{SERVER_NAME}\"\n\
                 inventory_name = \"{INVENTORY_NAME}\"\nallowed_profiles = [\"debian\"]\n"
            ),
        )
        .expect("policy");
        std::fs::write(trust.join("host_ca.pub"), format!("{host_ca_line}\n")).expect("ca");
        // A venv as `uv` leaves it: the versions `Hello` reports (Keel refuses an unknown
        // ansible-core).
        let venv = dir.path().join("venv");
        std::fs::create_dir_all(
            venv.join("lib/python3.12/site-packages/ansible_core-2.21.0.dist-info"),
        )
        .expect("venv");
        std::fs::write(venv.join("pyvenv.cfg"), "version_info = 3.12.3\n").expect("pyvenv");
        let socket = dir.path().join("executor.sock");
        Self {
            _dir: dir,
            venv,
            trust,
            state,
            socket,
        }
    }

    pub fn write_token(&self, token: &str) {
        std::fs::write(self.state.join(TOKEN_FILE), format!("{token}\n")).expect("token");
    }

    /// A runner whose `--server` is Keel's name on a port nothing listens on, dialling `dial`.
    pub fn runner(&self, dial: Vec<String>) -> Runner {
        let dead = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = dead.local_addr().expect("addr").port();
        drop(dead);
        Runner::load(RunnerConfig {
            server: format!("{SERVER_NAME}:{port}"),
            dial,
            trust_dir: self.trust.clone(),
            state_dir: self.state.clone(),
            executor_socket: self.socket.clone(),
            venv: self.venv.clone(),
        })
        .expect("runner")
        .with_age_recipient_file(self.state.join("age.pub"))
    }

    pub fn host_id(&self) -> Option<Uuid> {
        std::fs::read_to_string(self.state.join(HOST_ID_FILE))
            .ok()
            .and_then(|s| s.trim().parse().ok())
    }
}

/// A fake executor: an empty ledger, and for each run, once the whole pack is in, `Started`,
/// one log line and `Finished` (exit 0, `ok=2 changed=1` for `web-1`).
pub fn fake_executor(socket: &Path) {
    let _ = FakeExecutor::start(socket);
}

/// [`fake_executor`] that records every run it was told to start and can hold runs: while
/// `hold` is set, a run gets `Started` and then nothing until the runner closes its socket.
#[derive(Clone, Default)]
pub struct FakeExecutor {
    /// The run ids whose last `Pack` arrived, in order: a run started twice shows up twice.
    pub started: Arc<Mutex<Vec<String>>>,
    pub hold: Arc<AtomicBool>,
    /// Held runs whose socket the runner closed.
    pub closed: Arc<Mutex<Vec<String>>>,
}

impl FakeExecutor {
    pub fn start(socket: &Path) -> Self {
        let exec = Self::default();
        let listener = UnixListener::bind(socket).expect("executor socket");
        let shared = exec.clone();
        tokio::spawn(async move {
            while let Ok((s, _)) = listener.accept().await {
                tokio::spawn(shared.clone().serve(s));
            }
        });
        exec
    }

    pub fn started(&self) -> Vec<String> {
        self.started.lock().expect("lock").clone()
    }

    pub fn closed(&self) -> Vec<String> {
        self.closed.lock().expect("lock").clone()
    }

    async fn serve(self, mut s: tokio::net::UnixStream) {
        while let Ok(Some(frame)) = read_frame(&mut s).await {
            let answer = match frame {
                Frame::ReadLedger => vec![Frame::Ledger {
                    ledger_head: None,
                    bases: Vec::new(),
                }],
                Frame::Pack {
                    run_id, last: true, ..
                } => {
                    self.started.lock().expect("lock").push(run_id.clone());
                    if self.hold.load(Ordering::SeqCst) {
                        let started = Frame::Started {
                            run_id: run_id.clone(),
                        };
                        if write_frame(&mut s, &started).await.is_err() {
                            return;
                        }
                        // Held: nothing more until the runner closes the run's socket.
                        while let Ok(Some(_)) = read_frame(&mut s).await {}
                        self.closed.lock().expect("lock").push(run_id);
                        return;
                    }
                    vec![
                        Frame::Started {
                            run_id: run_id.clone(),
                        },
                        Frame::Log {
                            run_id: run_id.clone(),
                            seq: 1,
                            stream: LogStream::Stdout,
                            bytes: LOG_LINE.to_vec(),
                        },
                        Frame::Finished {
                            run_id,
                            exit_code: 0,
                            stats: [(
                                INVENTORY_NAME.to_owned(),
                                HostStats {
                                    ok: 2,
                                    changed: 1,
                                    ..HostStats::default()
                                },
                            )]
                            .into(),
                            ledger_line: String::new(),
                        },
                    ]
                }
                _ => vec![],
            };
            for f in answer {
                if write_frame(&mut s, &f).await.is_err() {
                    return;
                }
            }
        }
    }
}
