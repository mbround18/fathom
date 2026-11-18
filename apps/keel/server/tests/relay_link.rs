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
//! The harness (relay thread with captured tracing, runner host, fake executor) is
//! `real_relay/`, shared with `relay_failover.rs`.
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`) except the rogue-relay test; the others
//! skip without it.

mod common;
mod real_relay;
mod repos;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use keel_exec::ca::{CaKind, RunnerCertVerifier, RunnerIdentity, SshCa, TrustedCa};
use keel_exec::codec::read_frame;
use keel_exec::frames::Frame;
use keel_exec::krl::Krl;
use keel_exec::server_config::{
    Access, BareKey, KeelSshHandler, RunnerAuthority, SubsystemService, server_config,
};
use keel_protocol::{Cmd, Event};
use keel_runner::runner::{CERT_FILE, RunnerError, TOKEN_FILE};
use keel_server::relays::{RelayState, RelaySupervisor};
use real_relay::{
    INVENTORY_NAME, LOG_LINE, RunnerHostDir, RunningRelay, SERVER_NAME, WAIT, fake_executor,
    harness, key, mint, register, supervisor, until, wait_live,
};
use repos::{Harness, Source};
use russh::ChannelStream;
use russh::keys::ssh_key::public::KeyData;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;

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
