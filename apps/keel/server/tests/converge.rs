//! Converge (spec 068 US1, T016) against the fake runner: a preview lists each host's steps in
//! order; confirming makes a parent run with one child run per job; each host walks its steps in
//! order; a failed step stops that host only; approval holds every step until the parent is
//! approved, and a denial cancels them; a held converge is replaced by a newer one on the same
//! hosts; a retry re-plans only the hosts that did not succeed; and the flag gates it all.

mod common;
mod converge_support;
mod repos;

use std::sync::Arc;
use std::time::{Duration, Instant};

use converge_support::{Fixture, WITHIN, row};
use keel_exec::fake::{FakeRunner, Script};
use keel_protocol::{Cmd, ConvergeHostState, Event, StepState};
use keel_server::config::Config;
use repos::op_error;
use uuid::Uuid;

#[tokio::test]
async fn a_preview_lists_each_hosts_steps_in_order() {
    let Some(f) = Fixture::new("converge_preview", false).await else {
        return;
    };
    let (_, plan) = f.plan(Cmd::PreviewConverge(f.target())).await;
    assert_eq!(plan.binding_id, f.binding);
    assert!(!plan.policy.requires_approval);
    assert!(plan.replaces.is_empty() && plan.blocked_by.is_empty());
    let mut hosts: Vec<(String, Vec<(u16, String)>)> = plan
        .hosts
        .iter()
        .map(|h| {
            assert!(h.refusals.is_empty(), "{h:?}");
            (
                h.host.clone(),
                h.steps
                    .iter()
                    .map(|s| (s.position, s.job.clone()))
                    .collect(),
            )
        })
        .collect();
    hosts.sort();
    assert_eq!(
        hosts,
        vec![
            (
                "master-1".into(),
                vec![(1, "debian".into()), (2, "k3s-master".into())]
            ),
            (
                "node-1".into(),
                vec![(1, "debian".into()), (2, "keel-node".into())]
            ),
        ]
    );
    // Nothing is made by a preview.
    let (n,): (i64,) = sqlx::query_as("select count(*) from runs")
        .fetch_one(&f.h.db.owner)
        .await
        .expect("count");
    assert_eq!(n, 0);
    f.h.finish().await;
}

#[tokio::test]
async fn converge_is_off_unless_the_flag_is_on() {
    let Some(mut f) = Fixture::new("converge_flag_off", false).await else {
        return;
    };
    f.h.app.cfg = Arc::new(Config {
        converge: false,
        ..(*f.h.app.cfg).clone()
    });
    let code = op_error(&f.h.cmd(Cmd::PreviewConverge(f.target())).await);
    assert_eq!(code, "feature_disabled");
    let code = op_error(
        &f.h.cmd(Cmd::GetConverge {
            run_id: Uuid::now_v7(),
        })
        .await,
    );
    assert_eq!(code, "feature_disabled");
    f.h.finish().await;
}

#[tokio::test]
async fn each_host_walks_its_steps_in_order() {
    let Some(f) = Fixture::new("converge_order", false).await else {
        return;
    };
    let _runners = f.attach(|_, id| FakeRunner::new(id.to_string())).await;
    let parent = f.start().await;
    assert_eq!(f.ended(parent).await, ("succeeded".into(), None));
    assert_eq!(
        f.steps(parent).await,
        vec![
            row("master-1", 0, "debian", "succeeded", None),
            row("master-1", 1, "k3s-master", "succeeded", None),
            row("node-1", 0, "debian", "succeeded", None),
            row("node-1", 1, "keel-node", "succeeded", None),
        ]
    );
    // One child per job, each pointing at the parent; a step starts after the previous ended.
    let children: Vec<(String, String, String)> = sqlx::query_as(
        "select job_name, kind, state from runs where parent_run_id = $1 order by job_name",
    )
    .bind(parent)
    .fetch_all(&f.h.db.owner)
    .await
    .expect("children");
    assert_eq!(
        children,
        vec![
            ("debian".into(), "normal".into(), "succeeded".into()),
            ("k3s-master".into(), "normal".into(), "succeeded".into()),
            ("keel-node".into(), "normal".into(), "succeeded".into()),
        ]
    );
    let (ordered,): (bool,) = sqlx::query_as(
        "select bool_and(a.ended_at <= b.started_at) from converge_steps sa \
         join converge_steps sb on sb.run_id = sa.run_id and sb.host_id = sa.host_id \
              and sb.position = sa.position + 1 \
         join run_hosts a on a.run_id = sa.step_run_id and a.host_id = sa.host_id \
         join run_hosts b on b.run_id = sb.step_run_id and b.host_id = sb.host_id \
         where sa.run_id = $1",
    )
    .bind(parent)
    .fetch_one(&f.h.db.owner)
    .await
    .expect("order");
    assert!(ordered);
    let view = f.converge(parent).await;
    assert_eq!(view.run.kind, keel_protocol::RunKind::Converge);
    assert!(
        view.hosts
            .iter()
            .all(|h| h.state == ConvergeHostState::Succeeded && h.steps.len() == 2),
        "{view:?}"
    );
    assert!(
        view.hosts
            .iter()
            .flat_map(|h| &h.steps)
            .all(|s| s.state == StepState::Succeeded)
    );
    let actions = f.h.audit_actions().await;
    assert!(
        actions.iter().any(|(a, _)| a == "converge.create"),
        "{actions:?}"
    );
    f.h.finish().await;
}

#[tokio::test]
async fn a_failed_step_stops_that_host_only() {
    let Some(f) = Fixture::new("converge_failure", false).await else {
        return;
    };
    let _runners = f
        .attach(|name, id| {
            let r = FakeRunner::new(id.to_string());
            if name == "master-1" {
                r.script("debian", Script::finish(2))
            } else {
                r
            }
        })
        .await;
    let parent = f.start().await;
    assert_eq!(f.ended(parent).await.0, "failed");
    assert_eq!(
        f.steps(parent).await,
        vec![
            row("master-1", 0, "debian", "failed", None),
            row(
                "master-1",
                1,
                "k3s-master",
                "skipped",
                Some("dependency_failed")
            ),
            row("node-1", 0, "debian", "succeeded", None),
            row("node-1", 1, "keel-node", "succeeded", None),
        ]
    );
    let view = f.converge(parent).await;
    let state = |id| view.hosts.iter().find(|h| h.host_id == id).map(|h| h.state);
    assert_eq!(state(f.master), Some(ConvergeHostState::Failed));
    assert_eq!(state(f.node), Some(ConvergeHostState::Succeeded));
    f.h.finish().await;
}

#[tokio::test]
async fn one_approval_releases_every_step() {
    let Some(f) = Fixture::new("converge_approve", true).await else {
        return;
    };
    let _runners = f.attach(|_, id| FakeRunner::new(id.to_string())).await;
    let (token, plan) = f.plan(Cmd::PreviewConverge(f.target())).await;
    assert!(plan.policy.requires_approval);
    let parent = f.confirm(token).await;
    let (state,): (String,) = sqlx::query_as("select state from runs where id = $1")
        .bind(parent)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("parent");
    assert_eq!(state, "pending_approval");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        f.steps(parent).await.iter().all(|s| s.3 == "blocked"),
        "nothing runs before approval"
    );
    f.h.confirm(Cmd::PreviewRunApprove { run: parent }).await;
    assert_eq!(f.ended(parent).await, ("succeeded".into(), None));
    assert!(f.steps(parent).await.iter().all(|s| s.3 == "succeeded"));
    f.h.finish().await;
}

#[tokio::test]
async fn a_denial_cancels_every_step() {
    let Some(f) = Fixture::new("converge_deny", true).await else {
        return;
    };
    let parent = f.start().await;
    f.h.confirm(Cmd::PreviewRunDeny {
        run: parent,
        reason: None,
    })
    .await;
    assert_eq!(f.ended(parent).await.0, "denied");
    let steps = f.steps(parent).await;
    assert!(
        steps
            .iter()
            .all(|s| (s.1 == 0 && s.3 == "cancelled") || (s.1 > 0 && s.3 == "skipped")),
        "{steps:?}"
    );
    let open: Vec<(String,)> =
        sqlx::query_as("select state from runs where parent_run_id = $1 and ended_at is null")
            .bind(parent)
            .fetch_all(&f.h.db.owner)
            .await
            .expect("children");
    assert!(open.is_empty(), "{open:?}");
    let (hosts,): (Vec<String>,) = sqlx::query_as(
        "select array_agg(state order by state) from converge_hosts where run_id = $1",
    )
    .bind(parent)
    .fetch_one(&f.h.db.owner)
    .await
    .expect("hosts");
    assert_eq!(hosts, vec!["cancelled", "cancelled"]);
    f.h.finish().await;
}

#[tokio::test]
async fn a_newer_converge_replaces_a_held_one() {
    let Some(f) = Fixture::new("converge_replace", true).await else {
        return;
    };
    let first = f.start().await;
    let (token, plan) = f.plan(Cmd::PreviewConverge(f.target())).await;
    assert_eq!(plan.replaces, vec![first]);
    let second = f.confirm(token).await;
    assert_eq!(
        f.ended(first).await,
        ("cancelled".into(), Some("replaced".into()))
    );
    let (state,): (String,) = sqlx::query_as("select state from runs where id = $1")
        .bind(second)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("second");
    assert_eq!(state, "pending_approval");
    let actions = f.h.audit_actions().await;
    assert!(
        actions.iter().any(|(a, _)| a == "converge.replace"),
        "{actions:?}"
    );
    f.h.finish().await;
}

#[tokio::test]
async fn cancelling_a_converge_cancels_what_has_not_run() {
    let Some(f) = Fixture::new("converge_cancel", true).await else {
        return;
    };
    let parent = f.start().await;
    match f
        .h
        .cmd(Cmd::CancelConverge { run_id: parent })
        .await
        .as_slice()
    {
        [Event::Converge(view)] => {
            assert!(
                view.hosts
                    .iter()
                    .all(|h| h.state == ConvergeHostState::Cancelled),
                "{view:?}"
            );
        }
        other => panic!("cancel: {other:?}"),
    }
    assert_eq!(
        f.ended(parent).await,
        ("cancelled".into(), Some("cancelled_by_user".into()))
    );
    let actions = f.h.audit_actions().await;
    assert!(
        actions.iter().any(|(a, _)| a == "converge.cancel"),
        "{actions:?}"
    );
    f.h.finish().await;
}

#[tokio::test]
async fn a_retry_replans_only_the_hosts_that_did_not_succeed() {
    let Some(f) = Fixture::new("converge_retry", false).await else {
        return;
    };
    let runners = f
        .attach(|name, id| {
            let r = FakeRunner::new(id.to_string());
            if name == "master-1" {
                r.script("k3s-master", Script::finish(2))
            } else {
                r
            }
        })
        .await;
    let parent = f.start().await;
    assert_eq!(f.ended(parent).await.0, "failed");
    drop(runners);
    // The master's runner comes back fixed.
    let _runners = f.attach(|_, id| FakeRunner::new(id.to_string())).await;
    let (token, plan) = f
        .plan(Cmd::PreviewConvergeRetry {
            run_id: parent,
            replan: false,
        })
        .await;
    let hosts: Vec<Uuid> = plan.hosts.iter().map(|h| h.host_id).collect();
    assert_eq!(hosts, vec![f.master]);
    let out =
        f.h.cmd(Cmd::ConfirmConvergeRetry {
            preview_token: token,
            step_up: None,
        })
        .await;
    let retry = match out.as_slice() {
        [Event::Confirmed { .. }, Event::ConvergeStarted { run_id }] => *run_id,
        other => panic!("retry: {other:?}"),
    };
    assert_eq!(f.ended(retry).await, ("succeeded".into(), None));
    let view = f.converge(retry).await;
    assert_eq!(view.retry_of, Some(parent));
    assert_eq!(view.hosts.len(), 1);
    let actions = f.h.audit_actions().await;
    assert!(
        actions.iter().any(|(a, _)| a == "converge.retry"),
        "{actions:?}"
    );
    f.h.finish().await;
}

#[tokio::test]
async fn a_converge_holds_the_host_against_other_runs() {
    let Some(f) = Fixture::new("converge_hold", false).await else {
        return;
    };
    let _runners = f
        .attach(|_, id| FakeRunner::new(id.to_string()).script("keel-node", Script::hang()))
        .await;
    let parent = f.start().await;
    // node-1 is holding once its debian step has gone out.
    let start = Instant::now();
    loop {
        let (state,): (String,) = sqlx::query_as(
            "select state from run_hosts rh join converge_steps s \
             on s.step_run_id = rh.run_id and s.host_id = rh.host_id \
             where s.run_id = $1 and s.host_id = $2 and s.position = 1",
        )
        .bind(parent)
        .bind(f.node)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("step");
        if state == "running" {
            break;
        }
        assert!(start.elapsed() < WITHIN, "keel-node still {state}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // A plain run of `debian` queued on node-1 waits while the converge holds it.
    let other = Uuid::now_v7();
    sqlx::query(
        "insert into runs (id, workspace_id, binding_id, job_id, job_name, trigger, commit, \
         inventory_name, state) \
         select $1, $2, b.id, j.id, j.name, 'manual', b.last_commit, 'prod', 'queued' \
           from repo_bindings b join jobs j on j.binding_id = b.id and j.name = 'debian' \
          where b.id = $3",
    )
    .bind(other)
    .bind(f.h.workspace)
    .bind(f.binding)
    .execute(&f.h.db.owner)
    .await
    .expect("run");
    sqlx::query("insert into run_hosts (run_id, host_id, host_name) values ($1, $2, 'node-1')")
        .bind(other)
        .bind(f.node)
        .execute(&f.h.db.owner)
        .await
        .expect("run host");
    keel_server::queue::kick(&f.h.app);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (state,): (String,) = sqlx::query_as("select state from run_hosts where run_id = $1")
        .bind(other)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("other");
    assert_eq!(state, "queued");
    // Cancelling the converge releases the hold, and the plain run goes out.
    f.h.cmd(Cmd::CancelConverge { run_id: parent }).await;
    let (state, _) = f.ended(other).await;
    assert_eq!(state, "succeeded");
    f.h.finish().await;
}

/// A converge cancelled between two of a host's steps frees the host at once: no step is in
/// flight, so no `Finished` will come to wake the dispatcher, and the cancel itself must.
#[tokio::test]
async fn cancelling_between_steps_releases_the_host_at_once() {
    let Some(f) = Fixture::new("converge_cancel_between", false).await else {
        return;
    };
    // Only node-1 hangs, so master-1's steps finish and nothing of the cancel is in flight.
    let _runners = f
        .attach(|name, id| {
            let r = FakeRunner::new(id.to_string());
            if name == "node-1" {
                r.script("debian", Script::hang())
            } else {
                r
            }
        })
        .await;
    let parent = f.start().await;
    let step = |pos: i16| {
        let f = &f;
        async move {
            sqlx::query_as::<_, (Uuid, String)>(
                "select rh.run_id, rh.state from run_hosts rh join converge_steps s \
                 on s.step_run_id = rh.run_id and s.host_id = rh.host_id \
                 where s.run_id = $1 and s.host_id = $2 and s.position = $3",
            )
            .bind(parent)
            .bind(f.node)
            .bind(pos)
            .fetch_one(&f.h.db.owner)
            .await
            .expect("step")
        }
    };
    let start = Instant::now();
    let debian = loop {
        let (run, state) = step(0).await;
        if state == "running" {
            break run;
        }
        assert!(start.elapsed() < WITHIN, "debian still {state}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    // Let master-1 finish and its last dispatch settle, so no frame wakes the dispatcher
    // from here on.
    let start = Instant::now();
    loop {
        let (state,): (String,) =
            sqlx::query_as("select state from converge_hosts where run_id = $1 and host_id = $2")
                .bind(parent)
                .bind(f.master)
                .fetch_one(&f.h.db.owner)
                .await
                .expect("master-1");
        if state == "succeeded" {
            break;
        }
        assert!(start.elapsed() < WITHIN, "master-1 still {state}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    // A plain run of `debian` waits behind the hold.
    let other = Uuid::now_v7();
    sqlx::query(
        "insert into runs (id, workspace_id, binding_id, job_id, job_name, trigger, commit, \
         inventory_name, state) \
         select $1, $2, b.id, j.id, j.name, 'manual', b.last_commit, 'prod', 'queued' \
           from repo_bindings b join jobs j on j.binding_id = b.id and j.name = 'debian' \
          where b.id = $3",
    )
    .bind(other)
    .bind(f.h.workspace)
    .bind(f.binding)
    .execute(&f.h.db.owner)
    .await
    .expect("run");
    sqlx::query("insert into run_hosts (run_id, host_id, host_name) values ($1, $2, 'node-1')")
        .bind(other)
        .bind(f.node)
        .execute(&f.h.db.owner)
        .await
        .expect("run host");
    // The first step's result lands with no dispatch after it: the trigger queues the second
    // step and the host stays held.
    sqlx::query(
        "update run_hosts set state = 'succeeded', ended_at = now() \
         where run_id = $1 and host_id = $2",
    )
    .bind(debian)
    .bind(f.node)
    .execute(&f.h.db.owner)
    .await
    .expect("debian done");
    assert_eq!(step(1).await.1, "queued");
    // Cancelling frees node-1 and the plain run goes out without anything else waking it (it
    // hangs there, as `debian` does on this runner).
    f.h.cmd(Cmd::CancelConverge { run_id: parent }).await;
    let start = Instant::now();
    loop {
        let (state,): (String,) = sqlx::query_as("select state from run_hosts where run_id = $1")
            .bind(other)
            .fetch_one(&f.h.db.owner)
            .await
            .expect("other");
        if state == "running" {
            break;
        }
        assert!(start.elapsed() < WITHIN, "the plain run is still {state}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    f.h.finish().await;
}
