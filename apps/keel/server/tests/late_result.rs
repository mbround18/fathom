//! Late run results after a Keel restart (spec 060 US7, T045, contracts/runner-replay.md).
//!
//! A runner that lost its connection before Keel recorded `Finished` replays the frame after its
//! next `Hello`:
//! - a `lost` row takes the real outcome, the `runner_lost` run is recomputed, and
//!   `run_result_late` is audited;
//! - the same frame again is a duplicate and changes nothing;
//! - a frame for a run on another host is ignored.
//!
//! The server answers a runner's `Ping` with `Pong` in order, which is the runner's
//! acknowledgement that every earlier frame was handled.
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`); skips without it.

mod common;
mod repos;

use std::collections::BTreeMap;
use std::time::Duration;

use keel_exec::codec;
use keel_exec::events::HostStats;
use keel_exec::frames::Frame;
use repos::Harness;
use serde_json::Value;
use tokio::io::DuplexStream;
use uuid::Uuid;

fn hello(host: Uuid) -> Frame {
    Frame::Hello {
        host_id: host.to_string(),
        version: env!("CARGO_PKG_VERSION").into(),
        ansible_core: "2.21.0".into(),
        python: "3.12.3".into(),
        allowed_profiles: vec!["debian".into()],
        require_signed: false,
        override_quorum: 1,
        ledger_head: None,
        bases: Vec::new(),
        age_recipient: None,
    }
}

fn finished(run: Uuid, name: &str) -> Frame {
    Frame::Finished {
        run_id: run.to_string(),
        exit_code: 0,
        stats: BTreeMap::from([(
            name.to_owned(),
            HostStats {
                ok: 3,
                changed: 1,
                ..HostStats::default()
            },
        )]),
        ledger_line: String::new(),
    }
}

/// Connects a runner for `host` and returns its end of the channel.
async fn connect(h: &Harness, host: Uuid) -> DuplexStream {
    let (mut ours, theirs) = tokio::io::duplex(1 << 20);
    codec::write_frame(&mut ours, &hello(host))
        .await
        .expect("hello");
    keel_server::queue::attach(h.app.clone(), host, theirs)
        .await
        .expect("attach");
    ours
}

/// Sends `frames` then a `Ping`, and waits for the `Pong` that acknowledges them.
async fn send_acked(s: &mut DuplexStream, frames: &[Frame]) {
    for f in frames {
        codec::write_frame(s, f).await.expect("frame");
    }
    codec::write_frame(s, &Frame::Ping).await.expect("ping");
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match codec::read_frame(s).await.expect("read") {
                Some(Frame::Pong) => return,
                Some(_) => {}
                None => panic!("closed before Pong"),
            }
        }
    })
    .await
    .expect("Pong within 10 s");
}

async fn host(h: &Harness, name: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into hosts (id, workspace_id, inventory_name, allowed_profiles) \
         values ($1, $2, $3, '{debian}')",
    )
    .bind(id)
    .bind(h.workspace)
    .bind(name)
    .execute(&h.db.owner)
    .await
    .expect("host");
    id
}

/// A run that ended `failed`/`runner_lost` because its only host was lost.
async fn lost_run(h: &Harness, host: Uuid, name: &str) -> Uuid {
    let run = Uuid::now_v7();
    sqlx::query(
        "insert into runs (id, workspace_id, job_name, trigger, commit, inventory_name, state, \
         state_reason, started_at, ended_at) \
         values ($1, $2, 'web', 'manual', 'abc', 'prod', 'failed', 'runner_lost', now(), now())",
    )
    .bind(run)
    .bind(h.workspace)
    .execute(&h.db.owner)
    .await
    .expect("run");
    sqlx::query(
        "insert into run_hosts (run_id, host_id, host_name, state, started_at, ended_at) \
         values ($1, $2, $3, 'lost', now(), now())",
    )
    .bind(run)
    .bind(host)
    .bind(name)
    .execute(&h.db.owner)
    .await
    .expect("run host");
    run
}

async fn run_state(h: &Harness, run: Uuid) -> (String, Option<String>, String) {
    let (state, reason): (String, Option<String>) =
        sqlx::query_as("select state, state_reason from runs where id = $1")
            .bind(run)
            .fetch_one(&h.db.owner)
            .await
            .expect("run");
    let (host,): (String,) = sqlx::query_as("select state from run_hosts where run_id = $1")
        .bind(run)
        .fetch_one(&h.db.owner)
        .await
        .expect("run host");
    (state, reason, host)
}

async fn late_audits(h: &Harness) -> Vec<Value> {
    sqlx::query_as::<_, (Value,)>(
        "select to_jsonb(a) from audit_event a where action = 'run_result_late' order by seq",
    )
    .fetch_all(&h.db.owner)
    .await
    .expect("audit")
    .into_iter()
    .map(|r| r.0)
    .collect()
}

#[tokio::test]
async fn a_lost_host_takes_a_late_finished_once() {
    let Some(h) = Harness::new("late_result").await else {
        return;
    };
    let web = host(&h, "web-1").await;
    let run = lost_run(&h, web, "web-1").await;
    let mut runner = connect(&h, web).await;

    send_acked(&mut runner, &[finished(run, "web-1")]).await;
    assert_eq!(
        run_state(&h, run).await,
        ("succeeded".into(), None, "succeeded".into())
    );
    let (ok,): (i32,) = sqlx::query_as("select ok from run_host_results where run_id = $1")
        .bind(run)
        .fetch_one(&h.db.owner)
        .await
        .expect("results");
    assert_eq!(ok, 3);
    let audits = late_audits(&h).await;
    assert_eq!(audits.len(), 1, "{audits:?}");
    assert_eq!(audits[0]["target"]["run"], run.to_string());
    assert_eq!(audits[0]["target"]["host"], web.to_string());
    assert_eq!(audits[0]["context"]["outcome"], "succeeded");

    // A duplicate (a replay whose acknowledgement was lost) changes nothing.
    send_acked(&mut runner, &[finished(run, "web-1")]).await;
    assert_eq!(late_audits(&h).await.len(), 1);
    assert_eq!(run_state(&h, run).await.0, "succeeded");
    h.finish().await;
}

#[tokio::test]
async fn a_finished_from_another_host_is_ignored() {
    let Some(h) = Harness::new("late_result_foreign").await else {
        return;
    };
    let (web, other) = (host(&h, "web-1").await, host(&h, "web-2").await);
    let run = lost_run(&h, web, "web-1").await;
    let mut runner = connect(&h, other).await;

    send_acked(
        &mut runner,
        &[
            finished(run, "web-1"),
            // Not a run id at all, and a run that doesn't exist.
            Frame::Finished {
                run_id: "../../etc/passwd".into(),
                exit_code: 0,
                stats: BTreeMap::new(),
                ledger_line: String::new(),
            },
            finished(Uuid::now_v7(), "web-2"),
        ],
    )
    .await;
    assert_eq!(
        run_state(&h, run).await,
        ("failed".into(), Some("runner_lost".into()), "lost".into())
    );
    assert!(late_audits(&h).await.is_empty());
    h.finish().await;
}
