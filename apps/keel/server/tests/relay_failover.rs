//! Spec 067 T037 and T038's Keel half, US4: relay failover, with the real `keel_relay::Relay`
//! (two of them, in one zone), the real `keel-runner` and Keel's relay supervisor, all
//! in-process on loopback.
//!
//! - A runner dialling both relays connects through the first. Stopping it, the runner
//!   reconnects through the other within 60 s and `hosts.last_path` names the new relay.
//! - A run in progress on the stopped relay is handled as a lost connection: its host is `lost`
//!   and it is never started again, on the runner or by Keel.
//! - A run queued meanwhile dispatches after the reconnect and succeeds.
//! - With the relays first in `--dial` and down (one gone, one up but with no link to Keel), the
//!   runner connects to the direct address last in the list, and Keel shows `direct`.
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`); skips without it.

mod common;
mod real_relay;
mod repos;

use std::sync::atomic::Ordering;
use std::time::Duration;

use keel_protocol::{Cmd, Event};
use keel_server::relays::{RelayState, RelaySupervisor};
use keel_server::runner_listener::{self, RunnerPath};
use real_relay::{
    FakeExecutor, RunnerHostDir, RunningRelay, SERVER_NAME, harness, mint, register_named,
    supervisor, until, wait_live,
};
use repos::{Harness, Source};
use sqlx::PgPool;
use tokio::net::TcpListener;
use uuid::Uuid;

/// US4's bound on the reconnect through the other relay.
const FAILOVER: Duration = Duration::from_secs(60);

async fn last_path(h: &Harness, host: Uuid) -> serde_json::Value {
    let (path,): (Option<serde_json::Value>,) =
        sqlx::query_as("select last_path from hosts where id = $1")
            .bind(host)
            .fetch_one(&h.db.owner)
            .await
            .expect("host");
    path.expect("last_path")
}

/// Pushes a change under `playbooks/` and syncs: one new run on `web-1`. Returns its id.
async fn push_run(h: &Harness, src: &Source, binding: Uuid, content: &str) -> Uuid {
    let head = src.commit("playbooks/web.yaml", content, "change");
    src.push();
    let out = h.cmd(Cmd::SyncBinding { binding }).await;
    assert!(
        matches!(out.as_slice(), [Event::BindingSynced { .. }]),
        "{out:?}"
    );
    let db = h.db.owner.clone();
    until("a run", || {
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
    .await
}

async fn host_state(db: &PgPool, run: Uuid) -> String {
    let (state,): (String,) = sqlx::query_as("select state from run_hosts where run_id = $1")
        .bind(run)
        .fetch_one(db)
        .await
        .expect("run host");
    state
}

/// Waits until `run`'s host row is in `state`.
async fn host_reaches(h: &Harness, run: Uuid, state: &str) {
    let db = h.db.owner.clone();
    until(&format!("run {run} to be {state}"), || {
        let db = db.clone();
        async move { (host_state(&db, run).await == state).then_some(()) }
    })
    .await;
}

/// Waits until `run` ended; returns its state and reason.
async fn run_ends(h: &Harness, run: Uuid) -> (String, Option<String>) {
    let db = h.db.owner.clone();
    until(&format!("run {run} to end"), || {
        let db = db.clone();
        async move {
            let (state, reason, ended): (String, Option<String>, bool) = sqlx::query_as(
                "select state, state_reason, ended_at is not null from runs where id = $1",
            )
            .bind(run)
            .fetch_one(&db)
            .await
            .expect("run");
            ended.then_some((state, reason))
        }
    })
    .await
}

/// Waits until the runner of `host` is connected by a path satisfying `ok`.
async fn connected_by(
    h: &Harness,
    host: Uuid,
    what: &str,
    ok: impl Fn(&RunnerPath) -> bool,
) -> RunnerPath {
    let app = h.app.clone();
    until(what, || {
        let path = app.runs.path(host).filter(|p| ok(p));
        async move { path }
    })
    .await
}

/// The runner enrolls and connects; returns its host id.
async fn enrolled(dir: &RunnerHostDir) -> Uuid {
    until("the runner to enroll", || {
        let id = dir.host_id();
        async move { id }
    })
    .await
}

struct Zone {
    h: Harness,
    binding: Uuid,
    src: Source,
    sup: RelaySupervisor,
}

#[tokio::test]
async fn when_the_relay_in_use_stops_the_runner_fails_over_and_nothing_runs_twice() {
    let Some((h, zone, binding, src)) = harness("relay_failover").await else {
        return;
    };
    let hub = h.app.relay_link.get().expect("relay-link key");
    let hub_key = hub.key().public_key().key_data().clone();
    let relay_a = RunningRelay::start(hub_key.clone());
    let relay_b = RunningRelay::start(hub_key);
    let (a, _) = register_named(
        &h,
        zone,
        "edge-1",
        &relay_a.link.to_string(),
        &relay_a.host_key_data(),
    )
    .await;
    let (b, _) = register_named(
        &h,
        zone,
        "edge-2",
        &relay_b.link.to_string(),
        &relay_b.host_key_data(),
    )
    .await;
    let z = Zone {
        sup: supervisor(&h),
        h,
        binding,
        src,
    };
    z.sup.sync().await.expect("sync");
    for (id, name) in [(a, "edge-1"), (b, "edge-2")] {
        wait_live(&z.h, id, &format!("{name} to connect"), |l| {
            l.state == RelayState::Connected
        })
        .await;
    }

    // A runner dialling both relays, the one in use first.
    let token = mint(&z.h).await;
    let cas = z.h.app.ssh.get().expect("cas");
    let dir = RunnerHostDir::new(&cas.host.cert_authority_line(SERVER_NAME));
    dir.write_token(&token);
    let exec = FakeExecutor::start(&dir.socket);
    let runner = dir.runner(vec![relay_a.runner.to_string(), relay_b.runner.to_string()]);
    let runner = tokio::spawn(async move { runner.run_forever().await });
    let host = enrolled(&dir).await;
    connected_by(&z.h, host, "the runner to connect through edge-1", |p| {
        p.relay() == Some(a)
    })
    .await;
    assert_eq!(last_path(&z.h, host).await["relay"], a.to_string());

    // A run in progress on edge-1: the executor holds it.
    exec.hold.store(true, Ordering::SeqCst);
    let held = push_run(&z.h, &z.src, z.binding, "- hosts: web\n").await;
    host_reaches(&z.h, held, "running").await;
    assert_eq!(exec.started(), [held.to_string()]);

    // edge-1 goes away, and a run is queued while the runner has no session.
    let stopped_at = tokio::time::Instant::now();
    drop(relay_a);
    exec.hold.store(false, Ordering::SeqCst);
    let queued = push_run(
        &z.h,
        &z.src,
        z.binding,
        "- hosts: web\n  gather_facts: false\n",
    )
    .await;

    // The runner is back through edge-2 within 60 s, and the path says so.
    let path = tokio::time::timeout(
        FAILOVER,
        connected_by(&z.h, host, "the runner to reconnect through edge-2", |p| {
            p.relay() == Some(b)
        }),
    )
    .await
    .expect("no failover within 60 s");
    assert!(stopped_at.elapsed() < FAILOVER);
    assert_eq!(path.relay(), Some(b));
    let db = z.h.db.owner.clone();
    let path = until("last_path to name edge-2", || {
        let db = db.clone();
        async move {
            let (p,): (Option<serde_json::Value>,) =
                sqlx::query_as("select last_path from hosts where id = $1")
                    .bind(host)
                    .fetch_one(&db)
                    .await
                    .expect("host");
            p.filter(|p| p["relay"] == b.to_string())
        }
    })
    .await;
    assert_eq!(path["kind"], "relay", "{path}");
    wait_live(&z.h, a, "edge-1 to be unreachable", |l| {
        l.state == RelayState::Unreachable
    })
    .await;

    // The held run was a lost connection: lost, ended, and its socket closed on the runner.
    host_reaches(&z.h, held, "lost").await;
    let (state, reason) = run_ends(&z.h, held).await;
    assert_eq!(reason.as_deref(), Some("runner_lost"), "{state}");
    let exec2 = exec.clone();
    until("the runner to close the held run", || {
        let closed = exec2.closed();
        async move { (closed == [held.to_string()]).then_some(()) }
    })
    .await;

    // The queued run dispatches after the reconnect, through edge-2, and succeeds.
    let (state, _) = run_ends(&z.h, queued).await;
    assert_eq!(state, "succeeded");
    assert_eq!(host_state(&db, queued).await, "succeeded");

    // Nothing ran twice: each run reached the executor once, and the lost one stayed lost.
    assert_eq!(exec.started(), [held.to_string(), queued.to_string()]);
    assert_eq!(host_state(&db, held).await, "lost");
    let (events,): (i64,) = sqlx::query_as(
        "select count(*) from run_events where run_id = $1 and kind = 'runner_lost'",
    )
    .bind(held)
    .fetch_one(&db)
    .await
    .expect("events");
    assert_eq!(events, 1);

    runner.abort();
    let _ = runner.await;
    z.sup.shutdown();
    drop(relay_b);
    drop(dir);
    z.h.finish().await;
}

/// T038, Keel's half: relays first in `--dial`, the direct address last.
#[tokio::test]
async fn with_the_relays_down_the_runner_connects_direct_and_keel_shows_direct() {
    let Some((h, zone, _binding, _src)) = harness("relay_failover_direct").await else {
        return;
    };
    let hub = h.app.relay_link.get().expect("relay-link key");
    let hub_key = hub.key().public_key().key_data().clone();
    // edge-1 is gone; edge-2 is up but Keel cannot reach it, so it turns runners away.
    let gone = RunningRelay::start(hub_key.clone());
    let gone_runner = gone.runner.to_string();
    drop(gone);
    let unlinked = RunningRelay::start(hub_key);
    let dead = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let dead_addr = dead.local_addr().expect("addr").to_string();
    drop(dead);
    let (edge2, _) =
        register_named(&h, zone, "edge-2", &dead_addr, &unlinked.host_key_data()).await;
    let sup = supervisor(&h);
    sup.sync().await.expect("sync");
    wait_live(&h, edge2, "edge-2 to be unreachable", |l| {
        l.state == RelayState::Unreachable
    })
    .await;

    // Keel's own runner listener.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let direct = listener.local_addr().expect("addr");
    let serving = runner_listener::spawn(h.app.clone(), listener).expect("listener");

    let token = mint(&h).await;
    let cas = h.app.ssh.get().expect("cas");
    let dir = RunnerHostDir::new(&cas.host.cert_authority_line(SERVER_NAME));
    dir.write_token(&token);
    let _exec = FakeExecutor::start(&dir.socket);
    let runner = dir.runner(vec![
        gone_runner,
        unlinked.runner.to_string(),
        direct.to_string(),
    ]);
    let runner = tokio::spawn(async move { runner.run_forever().await });
    let host = enrolled(&dir).await;
    let path = connected_by(&h, host, "the runner to connect direct", |p| {
        p.relay().is_none()
    })
    .await;
    assert!(matches!(path, RunnerPath::Direct { .. }), "{path}");
    let db = h.db.owner.clone();
    let shown = until("last_path to say direct", || {
        let db = db.clone();
        async move {
            let (p,): (Option<serde_json::Value>,) =
                sqlx::query_as("select last_path from hosts where id = $1")
                    .bind(host)
                    .fetch_one(&db)
                    .await
                    .expect("host");
            p.filter(|p| p["kind"] == "direct")
        }
    })
    .await;
    assert!(
        shown["peer"]
            .as_str()
            .is_some_and(|p| p.starts_with("127.0.0.1:")),
        "{shown}"
    );
    // The up relay was tried first, and turned the runner away.
    assert!(
        unlinked.logs.count("no link to Keel") >= 1,
        "{}",
        unlinked.logs.text()
    );

    runner.abort();
    let _ = runner.await;
    serving.abort();
    sup.shutdown();
    drop(unlinked);
    drop(dir);
    h.finish().await;
}
