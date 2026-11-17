//! Spec 067 T014, US1 end to end: the real `keel_relay::Relay`, the real `keel-runner` and Keel's
//! relay supervisor, all in-process on loopback.
//!
//! - A runner that dials only the relay enrolls through it, connects, and `hosts.last_path`
//!   names the relay (acceptance 1, 5).
//! - A run dispatched to it streams its log and its result back through the relay (2).
//! - A relay answering the runner port with its own SSH key is refused before any
//!   authentication or token leaves the runner (3).
//! - With Keel unable to reach the relay the runner keeps retrying and the host is disconnected,
//!   then it reconnects once Keel can reach it again (4).
//! - The enrollment token never appears in the relay's tracing output, captured at TRACE (5).
//! - The real relay's `Hello` names its key's `SHA256:` fingerprint, so it is not quarantined.
//! - A carried stream that the runner half-closes, or drops, is not leaked: the relay's own
//!   stream count (`Pong.streams`) and Keel's both go back to 0.
//!
//! The relay runs on its own thread with a current-thread runtime and a thread-scoped tracing
//! subscriber, so every event it and its tasks emit (including russh's `log` records) lands in
//! one buffer, and nothing from Keel or the runner does. Executor: a fake on the Unix socket
//! (no Ansible).
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`) except the rogue-relay test; the others
//! skip without it.

mod common;
mod repos;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant, SystemTime};

use keel_exec::ca::{CaKind, RunnerCertVerifier, RunnerIdentity, SshCa, TrustedCa};
use keel_exec::codec::{read_frame, write_frame};
use keel_exec::events::HostStats;
use keel_exec::frames::{Frame, LogStream};
use keel_exec::krl::Krl;
use keel_exec::server_config::{
    Access, BareKey, KeelSshHandler, RunnerAuthority, SubsystemService, server_config,
};
use keel_protocol::{Cmd, Event, HostName, HostProfiles, ProfileName};
use keel_relay::Relay;
use keel_relay::config::Config as RelayConfig;
use keel_runner::runner::{CERT_FILE, HOST_ID_FILE, Runner, RunnerConfig, RunnerError, TOKEN_FILE};
use keel_server::audit::Actor;
use keel_server::hosts;
use keel_server::relays::{
    self, RelayLive, RelayRegistration, RelayState, RelaySupervisor, SupervisorOptions,
};
use keel_server::runner_listener::RunnerHost;
use keel_server::runs::{RunSettings, Runs};
use repos::{Harness, Source, bind};
use russh::ChannelStream;
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, PrivateKey};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream, UnixListener};
use tokio::sync::oneshot;
use tracing_subscriber::util::SubscriberInitExt as _;
use uuid::Uuid;

const WAIT: Duration = Duration::from_secs(30);
/// Keel's runner host name: `KEEL_PUBLIC_URL`'s host in the harness.
const SERVER_NAME: &str = "keel.example.com";
const INVENTORY_NAME: &str = "web-1";
const LOG_LINE: &[u8] = b"TASK [install] via the relay\n";

/// The `web` job runs on pushes under `playbooks/`, on inventory `prod`.
const MANIFEST: &str = "version: 1\nprofiles: { debian: {} }\njobs:\n  web:\n    \
                        playbook: playbooks/web.yaml\n    profiles: [debian]\n    \
                        inventory: prod\n    paths: ['playbooks/**']\n";

fn key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("key")
}

// ---- the relay, on its own thread, with its tracing captured ---------------------------------

/// A shared buffer the relay's tracing subscriber writes to.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

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
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("lock")).into_owned()
    }

    fn count(&self, needle: &str) -> usize {
        self.text().matches(needle).count()
    }
}

/// Routes `log` records (russh's) into tracing, so they reach whichever subscriber is current
/// on the emitting thread. The global default is a registry with no layers: Keel's and the
/// runner's events go nowhere, the relay thread's go to its buffer.
fn bridge_log() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = tracing_subscriber::registry().try_init();
    });
}

/// The real relay, serving on ephemeral loopback ports.
struct RunningRelay {
    link: SocketAddr,
    runner: SocketAddr,
    host_key: PrivateKey,
    logs: Captured,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl RunningRelay {
    fn start(hub_key: KeyData) -> Self {
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

    fn host_key_data(&self) -> KeyData {
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
async fn harness(test: &str) -> Option<(Harness, Uuid, Uuid, Source)> {
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

/// Registers the relay at `addr`, pinning `host_key`.
async fn register(h: &Harness, zone: Uuid, addr: &str, host_key: &KeyData) -> (Uuid, String) {
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
            name: "edge-1",
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

fn supervisor(h: &Harness) -> RelaySupervisor {
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
async fn mint(h: &Harness) -> String {
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
async fn until<T, F, Fut>(what: &str, mut check: F) -> T
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

async fn wait_live(h: &Harness, id: Uuid, what: &str, ok: impl Fn(&RelayLive) -> bool) {
    until(what, || {
        let matched = h.app.relay_live.get(id).is_some_and(|l| ok(&l));
        async move { matched.then_some(()) }
    })
    .await;
}

// ---- the runner host -------------------------------------------------------------------------

/// A runner host's directories: trust files for Keel's host CA, a state dir, and a fake
/// executor on the Unix socket.
struct RunnerHostDir {
    _dir: tempfile::TempDir,
    trust: PathBuf,
    state: PathBuf,
    venv: PathBuf,
    socket: PathBuf,
}

impl RunnerHostDir {
    fn new(host_ca_line: &str) -> Self {
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

    fn write_token(&self, token: &str) {
        std::fs::write(self.state.join(TOKEN_FILE), format!("{token}\n")).expect("token");
    }

    /// A runner whose `--server` is Keel's name on a port nothing listens on, dialling `dial`.
    fn runner(&self, dial: Vec<String>) -> Runner {
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

    fn host_id(&self) -> Option<Uuid> {
        std::fs::read_to_string(self.state.join(HOST_ID_FILE))
            .ok()
            .and_then(|s| s.trim().parse().ok())
    }
}

/// A fake executor: an empty ledger, and for each run, once the whole pack is in, `Started`,
/// one log line and `Finished` (exit 0, `ok=2 changed=1` for `web-1`).
fn fake_executor(socket: &Path) {
    let listener = UnixListener::bind(socket).expect("executor socket");
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            tokio::spawn(async move {
                while let Ok(Some(frame)) = read_frame(&mut s).await {
                    let answer = match frame {
                        Frame::ReadLedger => vec![Frame::Ledger {
                            ledger_head: None,
                            bases: Vec::new(),
                        }],
                        Frame::Pack {
                            run_id, last: true, ..
                        } => vec![
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
                        ],
                        _ => vec![],
                    };
                    for f in answer {
                        if write_frame(&mut s, &f).await.is_err() {
                            return;
                        }
                    }
                }
            });
        }
    });
}

/// Keel, the real relay linked to it, and a real runner dialling only the relay, enrolled with
/// a fresh token and connected.
struct Linked {
    h: Harness,
    binding: Uuid,
    src: Source,
    relay: RunningRelay,
    relay_id: Uuid,
    sup: RelaySupervisor,
    dir: RunnerHostDir,
    token: String,
    runner: tokio::task::JoinHandle<()>,
    host: Uuid,
}

impl Linked {
    async fn new(test: &str) -> Option<Self> {
        let (h, zone, binding, src) = harness(test).await?;
        let hub = h.app.relay_link.get().expect("relay-link key");
        let relay = RunningRelay::start(hub.key().public_key().key_data().clone());
        let (relay_id, pinned) =
            register(&h, zone, &relay.link.to_string(), &relay.host_key_data()).await;
        let sup = supervisor(&h);
        sup.sync().await.expect("sync");
        wait_live(&h, relay_id, "the relay to connect", |l| {
            l.state == RelayState::Connected
        })
        .await;
        // The real relay's Hello names the pinned key's SHA256 fingerprint: not quarantined.
        assert!(pinned.starts_with("SHA256:"), "{pinned}");
        assert_eq!(
            pinned,
            keel_exec::ca::fingerprint(&relay.host_key_data()),
            "registration pins the relay's key"
        );
        let (quarantined,): (Option<String>,) =
            sqlx::query_as("select quarantine_reason from relays where id = $1")
                .bind(relay_id)
                .fetch_one(&h.db.owner)
                .await
                .expect("relay");
        assert_eq!(quarantined, None);

        let token = mint(&h).await;
        let cas = h.app.ssh.get().expect("cas");
        let dir = RunnerHostDir::new(&cas.host.cert_authority_line(SERVER_NAME));
        dir.write_token(&token);
        fake_executor(&dir.socket);
        let runner = dir.runner(vec![relay.runner.to_string()]);
        let runner = tokio::spawn(async move { runner.run_forever().await });

        let host = until("the runner to enroll", || {
            let id = dir.host_id();
            async move { id }
        })
        .await;
        assert!(dir.state.join(CERT_FILE).exists());
        assert!(!dir.state.join(TOKEN_FILE).exists(), "token consumed");
        let app = h.app.clone();
        until("the runner to connect", || {
            let path = app.runs.path(host);
            async move { path }
        })
        .await;
        Some(Self {
            h,
            binding,
            src,
            relay,
            relay_id,
            sup,
            dir,
            token,
            runner,
            host,
        })
    }

    async fn last_path(&self) -> serde_json::Value {
        let (path,): (Option<serde_json::Value>,) =
            sqlx::query_as("select last_path from hosts where id = $1")
                .bind(self.host)
                .fetch_one(&self.h.db.owner)
                .await
                .expect("host");
        path.expect("last_path")
    }

    async fn set_dial_address(&self, addr: &str) {
        sqlx::query("update relays set dial_address = $2 where id = $1")
            .bind(self.relay_id)
            .bind(addr)
            .execute(&self.h.db.owner)
            .await
            .expect("dial address");
        self.sup.sync().await.expect("sync");
    }

    async fn finish(self) {
        self.runner.abort();
        let _ = self.runner.await;
        self.sup.shutdown();
        drop(self.relay);
        drop(self.dir);
        self.h.finish().await;
    }
}

#[tokio::test]
async fn a_runner_enrolls_connects_and_runs_through_the_real_relay() {
    let Some(l) = Linked::new("relay_link_run").await else {
        return;
    };

    // 1: the path is the relay, in the database and in the live session.
    let path = l.last_path().await;
    assert_eq!(path["kind"], "relay", "{path}");
    assert_eq!(path["relay"], l.relay_id.to_string(), "{path}");
    assert!(
        path["peer"]
            .as_str()
            .is_some_and(|p| p.starts_with("127.0.0.1:")),
        "{path}"
    );
    assert_eq!(
        l.h.app.runs.path(l.host).and_then(|p| p.relay()),
        Some(l.relay_id)
    );

    // 2: a push makes a run on web-1; its log and result come back through the relay.
    let head = l
        .src
        .commit("playbooks/web.yaml", "- hosts: web\n", "change");
    l.src.push();
    let out = l.h.cmd(Cmd::SyncBinding { binding: l.binding }).await;
    assert!(
        matches!(out.as_slice(), [Event::BindingSynced { .. }]),
        "{out:?}"
    );
    let db = l.h.db.owner.clone();
    let run: Uuid = until("a run", || {
        let db = db.clone();
        let head = head.clone();
        async move {
            sqlx::query_as::<_, (Uuid,)>("select id from runs where commit = $1")
                .bind(head)
                .fetch_optional(&db)
                .await
                .expect("runs")
                .map(|r| r.0)
        }
    })
    .await;
    let state = until("the run to end", || {
        let db = db.clone();
        async move {
            let (state, ended): (String, bool) =
                sqlx::query_as("select state, ended_at is not null from runs where id = $1")
                    .bind(run)
                    .fetch_one(&db)
                    .await
                    .expect("run");
            ended.then_some(state)
        }
    })
    .await;
    assert_eq!(state, "succeeded");
    let hosts: Vec<(String, String)> =
        sqlx::query_as("select host_name, state from run_hosts where run_id = $1")
            .bind(run)
            .fetch_all(&db)
            .await
            .expect("run hosts");
    assert_eq!(hosts, [(INVENTORY_NAME.to_owned(), "succeeded".to_owned())]);
    let results: Vec<(i32, i32)> =
        sqlx::query_as("select ok, changed from run_host_results where run_id = $1")
            .bind(run)
            .fetch_all(&db)
            .await
            .expect("results");
    assert_eq!(results, [(2, 1)]);
    let log: Vec<(Vec<u8>,)> =
        sqlx::query_as("select bytes from run_log_chunks where run_id = $1 order by seq")
            .bind(run)
            .fetch_all(&db)
            .await
            .expect("log");
    let log: Vec<u8> = log.into_iter().flat_map(|c| c.0).collect();
    assert_eq!(log, LOG_LINE);

    // 5: the relay carried the enrollment and the session, and never logged the token, at
    // TRACE, russh included.
    let logs = l.relay.logs.text();
    assert!(logs.contains("TRACE"), "captured at TRACE:\n{logs}");
    assert!(logs.contains("link up"), "{logs}");
    assert!(
        l.relay.logs.count("stream open") >= 2,
        "enrollment and session streams:\n{logs}"
    );
    let secret = l.token.split('.').nth(1).expect("token secret");
    assert!(secret.len() >= 16, "{}", l.token);
    assert!(
        !logs.contains(l.token.trim()),
        "the token is in the relay's logs"
    );
    assert!(
        !logs.contains(secret),
        "the token secret is in the relay's logs"
    );

    // The runner going away ends its stream on both sides: none leaked.
    l.runner.abort();
    wait_live(
        &l.h,
        l.relay_id,
        "streams to drain after the runner left",
        |s| s.streams == 0 && s.reported_streams == Some(0),
    )
    .await;
    let app = l.h.app.clone();
    let host = l.host;
    until("the host to disconnect", || {
        let gone = app.runs.path(host).is_none();
        async move { gone.then_some(()) }
    })
    .await;
    l.finish().await;
}

#[tokio::test]
async fn when_keel_cannot_reach_the_relay_the_runner_retries_and_reconnects() {
    let Some(l) = Linked::new("relay_link_unreachable").await else {
        return;
    };
    let good = l.relay.link.to_string();

    // Keel now dials a dead address: the relay is unreachable, its link and streams are gone.
    let dead = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let dead_addr = dead.local_addr().expect("addr").to_string();
    drop(dead);
    let refused_before = l.relay.logs.count("no link to Keel");
    l.set_dial_address(&dead_addr).await;
    wait_live(&l.h, l.relay_id, "the relay to be unreachable", |s| {
        s.state == RelayState::Unreachable
    })
    .await;
    let app = l.h.app.clone();
    let host = l.host;
    until("the host to disconnect", || {
        let gone = app.runs.path(host).is_none();
        async move { gone.then_some(()) }
    })
    .await;
    // Disconnected, not failed: the host row is untouched.
    let (revoked,): (bool,) =
        sqlx::query_as("select revoked_at is not null from hosts where id = $1")
            .bind(l.host)
            .fetch_one(&l.h.db.owner)
            .await
            .expect("host");
    assert!(!revoked);
    // The runner keeps dialling the relay, which has no link and turns it away at once.
    let logs = l.relay.logs.clone();
    until("the runner to retry twice", || {
        let n = logs.count("no link to Keel") - refused_before;
        async move { (n >= 2).then_some(()) }
    })
    .await;

    // Keel can reach the relay again: it relinks and the runner comes back through it.
    l.set_dial_address(&good).await;
    wait_live(&l.h, l.relay_id, "the relay to reconnect", |s| {
        s.state == RelayState::Connected
    })
    .await;
    let path = until("the runner to reconnect", || {
        let path = app.runs.path(host);
        async move { path }
    })
    .await;
    assert_eq!(path.relay(), Some(l.relay_id));
    l.finish().await;
}

#[tokio::test]
async fn a_half_closed_carried_stream_is_not_leaked() {
    let Some((h, zone, _binding, _src)) = harness("relay_link_half_close").await else {
        return;
    };
    let hub = h.app.relay_link.get().expect("relay-link key");
    let relay = RunningRelay::start(hub.key().public_key().key_data().clone());
    let (id, _) = register(&h, zone, &relay.link.to_string(), &relay.host_key_data()).await;
    let sup = supervisor(&h);
    sup.sync().await.expect("sync");
    wait_live(&h, id, "the relay to connect", |l| {
        l.state == RelayState::Connected
    })
    .await;

    // A "runner" connects and half-closes at once: it sends nothing more but keeps reading.
    let mut tcp = TcpStream::connect(relay.runner).await.expect("connect");
    wait_live(&h, id, "the stream to be carried", |l| {
        l.streams == 1 && l.reported_streams == Some(1)
    })
    .await;
    tcp.shutdown().await.expect("half-close");
    // Keel's SSH server sent its banner; then the stream is closed towards the runner.
    let mut got = Vec::new();
    tokio::time::timeout(WAIT, tcp.read_to_end(&mut got))
        .await
        .expect("the relay kept the runner's read side open")
        .expect("read");
    assert!(got.starts_with(b"SSH-2.0-"), "{got:?}");
    wait_live(&h, id, "streams to drain after a half-close", |l| {
        l.streams == 0 && l.reported_streams == Some(0)
    })
    .await;
    drop(tcp);
    sup.shutdown();
    drop(relay);
    h.finish().await;
}

// ---- acceptance 3: a relay answering with its own key -----------------------------------------

/// Counts every authentication decision the rogue is asked for.
struct CountingAuthority {
    verifier: Arc<RunnerCertVerifier>,
    calls: Arc<AtomicUsize>,
}

impl RunnerAuthority for CountingAuthority {
    fn verifier(&self) -> Arc<RunnerCertVerifier> {
        Arc::clone(&self.verifier)
    }

    async fn is_enrolled(&self, _identity: &RunnerIdentity, _key: &KeyData) -> bool {
        self.calls.fetch_add(1, Ordering::SeqCst);
        false
    }

    async fn bare_key(&self, _key: &KeyData, _now: SystemTime) -> BareKey {
        self.calls.fetch_add(1, Ordering::SeqCst);
        BareKey::Unknown
    }
}

/// Records every frame it is sent (an `Enroll` would carry the token).
struct CountingService {
    frames: Arc<Mutex<Vec<Frame>>>,
}

impl SubsystemService for CountingService {
    async fn serve(&self, _access: Access, mut stream: ChannelStream<russh::server::Msg>) {
        while let Ok(Some(frame)) = read_frame(&mut stream).await {
            self.frames.lock().expect("lock").push(frame);
        }
    }
}

#[tokio::test]
async fn a_relay_answering_with_its_own_key_is_refused_and_gets_nothing() {
    let host_ca = SshCa::generate(CaKind::Host).expect("ca");
    let runner_ca = SshCa::generate(CaKind::Runner).expect("ca");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let rogue = listener.local_addr().expect("addr");
    // The relay's own host key, no certificate.
    let mut config = server_config(key(), None);
    config.auth_rejection_time = Duration::from_millis(1);
    let config = Arc::new(config);
    let accepts = Arc::new(AtomicUsize::new(0));
    let auth_calls = Arc::new(AtomicUsize::new(0));
    let frames = Arc::new(Mutex::new(Vec::new()));
    let authority = Arc::new(CountingAuthority {
        verifier: Arc::new(RunnerCertVerifier::new(
            vec![TrustedCa::active(runner_ca.key_data().clone())],
            Krl::default(),
        )),
        calls: Arc::clone(&auth_calls),
    });
    let service = Arc::new(CountingService {
        frames: Arc::clone(&frames),
    });
    let counted = Arc::clone(&accepts);
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            counted.fetch_add(1, Ordering::SeqCst);
            let handler = KeelSshHandler::new(Arc::clone(&authority), Arc::clone(&service));
            let config = Arc::clone(&config);
            tokio::spawn(async move {
                if let Ok(session) = russh::server::run_stream(config, socket, handler).await {
                    let _ = session.await;
                }
            });
        }
    });

    let dir = RunnerHostDir::new(&host_ca.cert_authority_line(SERVER_NAME));
    let token = format!("keel-enroll-v1.0123456789abcdef.{}", host_ca.fingerprint());
    dir.write_token(&token);
    let runner = dir.runner(vec![rogue.to_string()]);
    let err = tokio::time::timeout(WAIT, runner.run_once())
        .await
        .expect("hung")
        .unwrap_err();
    assert!(matches!(err, RunnerError::ServerRefused(_)), "{err}");
    assert_eq!(accepts.load(Ordering::SeqCst), 1, "the runner dialled it");
    assert_eq!(auth_calls.load(Ordering::SeqCst), 0, "no authentication");
    assert!(frames.lock().expect("lock").is_empty(), "no frame");
    assert_eq!(
        std::fs::read_to_string(dir.state.join(TOKEN_FILE))
            .expect("token kept")
            .trim(),
        token
    );
    assert!(!dir.state.join(CERT_FILE).exists());
}
