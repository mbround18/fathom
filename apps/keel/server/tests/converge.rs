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

// ---- spec 068 US2: converge on enrollment (T032) ----------------------------------------------

/// Adds `name` to the pinned `prod` revision (unless `listed` is false), loads the SSH CAs and
/// enrolls a fresh runner key as `name` with profile `keel-node` through a token minted with
/// `auto_converge`. Returns the new host's id.
async fn enroll(f: &Fixture, name: &str, auto_converge: Option<bool>, listed: bool) -> Uuid {
    use keel_exec::ca::{CaKind, SshCa};
    use keel_protocol::{HostName, HostProfiles, ProfileName};
    use keel_server::runner_listener::{EnrollRequest, enroll_key};

    keel_server::hosts::bootstrap(&f.h.app)
        .await
        .expect("bootstrap");
    if listed {
        sqlx::query(
            "insert into inventory_hosts (revision_id, host, groups) \
             select r.id, $2, '{}' from inventory_revisions r \
             join inventories i on i.id = r.inventory_id \
             where i.workspace_id = $1 and i.name = 'prod'",
        )
        .bind(f.h.workspace)
        .bind(name)
        .execute(&f.h.db.owner)
        .await
        .expect("inventory host");
    }
    let (_, out) =
        f.h.confirm(Cmd::PreviewHostEnroll {
            workspace: f.h.workspace,
            inventory_name: HostName::new(name).expect("name"),
            profiles: HostProfiles(vec![ProfileName::new("keel-node").expect("profile")]),
            auto_converge,
        })
        .await;
    let token = match out.as_slice() {
        [Event::Confirmed { .. }, Event::HostEnrollment { token, .. }] => token.0.clone(),
        other => panic!("enroll: {other:?}"),
    };
    let key = SshCa::generate(CaKind::Runner)
        .expect("key")
        .key_data()
        .clone();
    let allowed = vec!["debian".to_owned(), "keel-node".to_owned()];
    let (host, _) = enroll_key(
        &f.h.app,
        &key,
        &EnrollRequest {
            token: &token,
            inventory_name: name,
            allowed_profiles: &allowed,
            require_signed: false,
        },
    )
    .await
    .unwrap_or_else(|e| panic!("enroll: {e:?}"));
    host
}

/// The converges that include `host`: (run, state, requested_by).
async fn converges_of(f: &Fixture, host: Uuid) -> Vec<(Uuid, String, Option<Uuid>)> {
    sqlx::query_as(
        "select r.id, r.state, r.requested_by from runs r \
         join converge_hosts c on c.run_id = r.id where c.host_id = $1 order by r.id",
    )
    .bind(host)
    .fetch_all(&f.h.db.owner)
    .await
    .expect("converges")
}

#[tokio::test]
async fn enroll_starts_a_converge_as_system_enroll() {
    let Some(f) = Fixture::new("converge_enroll", false).await else {
        return;
    };
    let host = enroll(&f, "node-2", None, true).await;
    let runs = converges_of(&f, host).await;
    assert_eq!(runs.len(), 1, "{runs:?}");
    let (run, state, by) = &runs[0];
    assert_eq!(state, "queued", "no approval needed: it runs");
    assert_eq!(*by, None, "no user started it");
    let (hosts,): (Vec<String>,) = sqlx::query_as("select resolved_hosts from runs where id = $1")
        .bind(run)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("run");
    assert_eq!(hosts, vec!["node-2".to_owned()], "that host alone");
    let (actor,): (String,) = sqlx::query_as(
        "select actor from audit_event where action = 'converge.create' \
         and target->>'run' = $1",
    )
    .bind(run.to_string())
    .fetch_one(&f.h.db.owner)
    .await
    .expect("audit");
    assert_eq!(actor, "system:enroll");
    let jobs: Vec<String> = f.steps(*run).await.into_iter().map(|s| s.2).collect();
    assert_eq!(jobs, vec!["debian".to_owned(), "keel-node".to_owned()]);
    f.h.finish().await;
}

#[tokio::test]
async fn enroll_converge_goes_through_the_approval_policy() {
    let Some(f) = Fixture::new("converge_enroll_approval", true).await else {
        return;
    };
    let host = enroll(&f, "node-2", Some(true), true).await;
    let runs = converges_of(&f, host).await;
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0].1, "pending_approval");
    f.h.finish().await;
}

#[tokio::test]
async fn enroll_without_auto_converge_or_the_flag_makes_none() {
    let Some(mut f) = Fixture::new("converge_enroll_off", false).await else {
        return;
    };
    let unticked = enroll(&f, "node-2", Some(false), true).await;
    assert!(converges_of(&f, unticked).await.is_empty());
    let (kept,): (bool,) =
        sqlx::query_as("select auto_converge from host_enrollments where host_id = $1")
            .bind(unticked)
            .fetch_one(&f.h.db.owner)
            .await
            .expect("enrollment");
    assert!(!kept);
    let listed =
        f.h.cmd(Cmd::ListEnrollments {
            state: None,
            workspace: Some(f.h.workspace),
        })
        .await;
    match listed.as_slice() {
        [Event::Enrollments { items }] => {
            assert!(items.iter().all(|e| !e.auto_converge), "{items:?}")
        }
        other => panic!("list: {other:?}"),
    }
    f.h.app.cfg = Arc::new(Config {
        converge: false,
        ..(*f.h.app.cfg).clone()
    });
    let flag_off = enroll(&f, "node-3", None, true).await;
    assert!(converges_of(&f, flag_off).await.is_empty());
    f.h.finish().await;
}

#[tokio::test]
async fn enroll_plan_failure_keeps_the_enrollment() {
    let Some(f) = Fixture::new("converge_enroll_fail", false).await else {
        return;
    };
    // Not in the pinned inventory: no binding can plan a step for it.
    let host = enroll(&f, "node-9", None, false).await;
    assert!(converges_of(&f, host).await.is_empty());
    let (state,): (String,) = sqlx::query_as("select state from hosts where id = $1")
        .bind(host)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("still enrolled");
    assert_eq!(state, "offline");
    let (actor, reason, context): (String, Option<String>, serde_json::Value) = sqlx::query_as(
        "select actor, reason, context from audit_event \
         where action = 'converge.enroll_skipped' and target->>'host' = $1",
    )
    .bind(host.to_string())
    .fetch_one(&f.h.db.owner)
    .await
    .expect("skip audited");
    assert_eq!(actor, "system:enroll");
    assert_eq!(reason.as_deref(), Some("no_plan"));
    assert_eq!(context["bindings"][0]["code"], "no_steps");
    f.h.finish().await;
}

// ---- spec 068 US2 scenario 3: converge on join acceptance (T034) -------------------------------

/// Lists `name` in the pinned `prod` revision, has a fresh runner key ask to join as `name`
/// (ceiling debian, keel-node), and accepts it with profile `keel-node` and `auto_converge`.
/// Returns the new host's id.
async fn join(f: &Fixture, name: &str, auto_converge: Option<bool>) -> Uuid {
    use keel_exec::ca::{CaKind, SshCa};
    use keel_protocol::{HostName, HostProfiles, ProfileName};
    use keel_server::join::{JoinAttempt, JoinOutcome, join_key};
    use keel_server::runner_listener::RunnerPath;

    keel_server::hosts::bootstrap(&f.h.app)
        .await
        .expect("ssh bootstrap");
    keel_server::join::bootstrap(&f.h.app)
        .await
        .expect("join bootstrap");
    sqlx::query(
        "insert into inventory_hosts (revision_id, host, groups) \
         select r.id, $2, '{}' from inventory_revisions r \
         join inventories i on i.id = r.inventory_id \
         where i.workspace_id = $1 and i.name = 'prod'",
    )
    .bind(f.h.workspace)
    .bind(name)
    .execute(&f.h.db.owner)
    .await
    .expect("inventory host");
    let (_, out) =
        f.h.confirm(Cmd::PreviewJoinTokenIssue {
            workspace: f.h.workspace,
            expires_in_days: 30,
            inventory_name: None,
            profiles: None,
        })
        .await;
    let token = match out.as_slice() {
        [
            Event::Confirmed { .. },
            Event::JoinTokenSecret { token, .. },
        ] => token.0.clone(),
        other => panic!("issue: {other:?}"),
    };
    let key = SshCa::generate(CaKind::Runner)
        .expect("key")
        .key_data()
        .clone();
    let allowed = vec!["debian".to_owned(), "keel-node".to_owned()];
    let attempt = JoinAttempt {
        token: &token,
        inventory_name: name,
        allowed_profiles: &allowed,
        require_signed: false,
        hostname: name,
        os: "Debian GNU/Linux 13 (trixie)",
        arch: "x86_64",
        runner_version: "0.1.0",
    };
    let path = RunnerPath::Direct {
        peer: Some("203.0.113.7:40000".parse().expect("peer")),
    };
    let JoinOutcome::Pending(request) = join_key(&f.h.app, &key, &attempt, &path)
        .await
        .expect("join")
    else {
        panic!("expected a pending join request");
    };
    let (_, out) =
        f.h.confirm(Cmd::PreviewJoinAccept {
            request,
            inventory_name: HostName::new(name).expect("name"),
            profiles: HostProfiles(vec![ProfileName::new("keel-node").expect("profile")]),
            auto_converge,
        })
        .await;
    match out.as_slice() {
        [
            Event::Confirmed { .. },
            Event::JoinRequestDecided { request },
        ] => request.host.expect("accepted into a host"),
        other => panic!("accept: {other:?}"),
    }
}

#[tokio::test]
async fn join_acceptance_starts_a_converge_as_system_enroll() {
    let Some(f) = Fixture::new("converge_join", false).await else {
        return;
    };
    let host = join(&f, "node-2", None).await;
    let runs = converges_of(&f, host).await;
    assert_eq!(runs.len(), 1, "{runs:?}");
    let (run, state, by) = &runs[0];
    assert_eq!(state, "queued");
    assert_eq!(*by, None, "no user started it");
    let (actor,): (String,) = sqlx::query_as(
        "select actor from audit_event where action = 'converge.create' \
         and target->>'run' = $1",
    )
    .bind(run.to_string())
    .fetch_one(&f.h.db.owner)
    .await
    .expect("audit");
    assert_eq!(actor, "system:enroll");
    let jobs: Vec<String> = f.steps(*run).await.into_iter().map(|s| s.2).collect();
    assert_eq!(jobs, vec!["debian".to_owned(), "keel-node".to_owned()]);
    let (kept,): (serde_json::Value,) = sqlx::query_as(
        "select context->'auto_converge' from audit_event where action = 'join_accepted' \
         and target->>'host' = $1",
    )
    .bind(host.to_string())
    .fetch_one(&f.h.db.owner)
    .await
    .expect("acceptance audited");
    assert_eq!(kept, serde_json::json!(true));
    f.h.finish().await;
}

#[tokio::test]
async fn join_acceptance_without_auto_converge_makes_none() {
    let Some(f) = Fixture::new("converge_join_off", false).await else {
        return;
    };
    let host = join(&f, "node-2", Some(false)).await;
    assert!(converges_of(&f, host).await.is_empty());
    let (state,): (String,) = sqlx::query_as("select state from hosts where id = $1")
        .bind(host)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("accepted");
    assert_eq!(state, "offline");
    f.h.finish().await;
}

// ---- signed mode (FR-005, research R3) ------------------------------------------------------

mod signed {
    //! A signed-mode converge: every step pinned to the plan's one commit, each step (child run)
    //! early-checked over its own hosts, its review groups bound to its own run id and job. The
    //! converge waits in review until every step's groups are approved, and no step goes out
    //! before.

    use std::time::Duration;

    use keel_exec::fake::FakeRunner;
    use keel_exec::frames::Frame;
    use keel_protocol::{
        AssertionCredential, AttestationCredential, Cmd, Event, PasskeyAssertion,
        PasskeyCredential, PasskeyName, PasskeyPurpose, PasskeyResponse, ReviewComment,
    };
    use serde_json::{Value, json};
    use uuid::Uuid;
    use webauthn_authenticator_rs::WebauthnAuthenticator;
    use webauthn_authenticator_rs::softpasskey::SoftPasskey;
    use webauthn_rs::prelude::{CreationChallengeResponse, RequestChallengeResponse, Url};

    use crate::converge_support::Fixture;
    use crate::repos::{self, Harness, Source, op_error};

    type Authenticator = WebauthnAuthenticator<SoftPasskey>;

    const JOBS: &str = include_str!("fixtures/converge-repo/.keel/jobs.yaml");

    /// The converge fixture with `prod` as a repository inventory, in signed mode, under a policy
    /// that requires `approvals` approvals (no four-eyes, so the owner may review).
    async fn fixture(test: &str, approvals: i32) -> Option<Fixture> {
        let mut h = Harness::new(test).await?;
        h.app.runs = keel_server::runs::Runs::new(keel_server::runs::RunSettings {
            ping_interval: Duration::from_millis(50),
            flush_interval: Duration::from_millis(20),
            ..keel_server::runs::RunSettings::default()
        });
        h.app.cfg = std::sync::Arc::new(keel_server::config::Config {
            converge: true,
            ..(*h.app.cfg).clone()
        });
        let src = Source::fixture("converge-repo");
        src.commit(
            ".keel/jobs.yaml",
            &format!("{JOBS}inventories:\n  prod:\n    path: inventories/prod/hosts.yaml\n"),
            "repository inventory",
        );
        src.commit(
            "inventories/prod/hosts.yaml",
            "prod:\n  hosts:\n    node-1:\n    master-1:\n",
            "inventory",
        );
        src.push();
        let binding = repos::bind(&h, &src).await;
        let owner = &h.db.owner;
        let mut ids = Vec::new();
        for (name, profile) in [("node-1", "keel-node"), ("master-1", "k3s-master")] {
            let id = Uuid::now_v7();
            sqlx::query(
                "insert into hosts (id, workspace_id, inventory_name, allowed_profiles) \
                 values ($1, $2, $3, array['debian', $4])",
            )
            .bind(id)
            .bind(h.workspace)
            .bind(name)
            .bind(profile)
            .execute(owner)
            .await
            .expect("host");
            sqlx::query(
                "insert into host_profiles (host_id, profile_id) \
                 select $1, id from profiles where workspace_id = $2 and name = $3",
            )
            .bind(id)
            .bind(h.workspace)
            .bind(profile)
            .execute(owner)
            .await
            .expect("host profile");
            ids.push(id);
        }
        sqlx::query(
            "insert into approval_policies (id, workspace_id, priority, requires_approval, \
             approvals_needed, four_eyes) values ($1, $2, 1, true, $3, false)",
        )
        .bind(Uuid::now_v7())
        .bind(h.workspace)
        .bind(approvals)
        .execute(owner)
        .await
        .expect("policy");
        h.confirm(Cmd::PreviewSignedMode { binding, on: true })
            .await;
        Some(Fixture {
            h,
            src,
            binding,
            node: ids[0],
            master: ids[1],
        })
    }

    /// Each step's review groups: (group, step run, job, hosts, target commit), by job.
    async fn groups(f: &Fixture, parent: Uuid) -> Vec<(Uuid, Uuid, String, Vec<String>, String)> {
        sqlx::query_as(
            "select g.id, r.id, r.job_name, g.hosts, g.target_commit from review_groups g \
             join runs r on r.id = g.run_id where r.parent_run_id = $1 order by r.job_name",
        )
        .bind(parent)
        .fetch_all(&f.h.db.owner)
        .await
        .expect("groups")
    }

    async fn state(f: &Fixture, run: Uuid) -> String {
        let (s,): (String,) = sqlx::query_as("select state from runs where id = $1")
            .bind(run)
            .fetch_one(&f.h.db.owner)
            .await
            .expect("run");
        s
    }

    /// Previews and confirms an override of `group` with a passkey assertion over its own
    /// challenge; returns the challenge's canonical JSON.
    async fn review(f: &Fixture, auth: &mut Authenticator, group: Uuid) -> String {
        let out =
            f.h.cmd(Cmd::PreviewOverride {
                group,
                comment: ReviewComment::new("Reviewed the diff.").expect("comment"),
            })
            .await;
        let [Event::Preview(p), Event::OverrideChallenge(c)] = out.as_slice() else {
            panic!("override preview: {out:?}");
        };
        let out =
            f.h.cmd(Cmd::Confirm {
                preview_id: p.preview_id,
                step_up: Some(PasskeyAssertion {
                    ceremony: c.request.ceremony,
                    credential: assert_with(auth, &c.request.options),
                }),
            })
            .await;
        assert!(
            matches!(out.first(), Some(Event::Confirmed { .. })),
            "{out:?}"
        );
        c.canonical.clone()
    }

    /// Registers the owner's first passkey.
    async fn passkey(h: &Harness) -> Authenticator {
        let mut auth = WebauthnAuthenticator::new(SoftPasskey::new(true));
        let out = h
            .cmd(Cmd::PasskeyBegin {
                purpose: PasskeyPurpose::Register {
                    name: PasskeyName::new("laptop").expect("name"),
                },
            })
            .await;
        let [Event::PasskeyChallenge(r)] = out.as_slice() else {
            panic!("challenge: {out:?}");
        };
        let credential = attest(&mut auth, &r.options);
        let out = h
            .cmd(Cmd::PasskeyFinish(PasskeyResponse {
                ceremony: r.ceremony,
                credential: PasskeyCredential::Attestation(credential),
            }))
            .await;
        assert!(
            !out.iter().any(|e| matches!(e, Event::OpError { .. })),
            "{out:?}"
        );
        auth
    }

    fn browser_json(mut v: Value) -> Value {
        let obj = v.as_object_mut().expect("credential object");
        obj.remove("extensions");
        let response = obj["response"].as_object_mut().expect("response");
        response.retain(|_, f| !f.is_null());
        v
    }

    fn attest(auth: &mut Authenticator, options: &Value) -> AttestationCredential {
        let ccr: CreationChallengeResponse =
            serde_json::from_value(json!({ "publicKey": options })).expect("creation options");
        let reg = auth
            .do_registration(Url::parse(repos::ORIGIN).unwrap(), ccr)
            .expect("softpasskey registers");
        serde_json::from_value(browser_json(serde_json::to_value(reg).unwrap()))
            .expect("protocol attestation")
    }

    fn assert_with(auth: &mut Authenticator, options: &Value) -> AssertionCredential {
        let rcr: RequestChallengeResponse =
            serde_json::from_value(json!({ "publicKey": options })).expect("request options");
        let cred = auth
            .do_authentication(Url::parse(repos::ORIGIN).unwrap(), rcr)
            .expect("softpasskey asserts");
        serde_json::from_value(browser_json(serde_json::to_value(cred).unwrap()))
            .expect("protocol assertion")
    }

    #[tokio::test]
    async fn each_step_is_reviewed_at_one_commit_and_none_goes_out_before() {
        let Some(f) = fixture("converge_signed", 1).await else {
            return;
        };
        let mut auth = passkey(&f.h).await;
        let mut runners = f.attach(|_, id| FakeRunner::new(id.to_string())).await;
        let (token, plan) = f.plan(Cmd::PreviewConverge(f.target())).await;
        // The preview lists each step's review at the plan's one commit.
        let mut review_rows: Vec<(String, String, bool)> = plan
            .review
            .iter()
            .map(|r| (r.job.clone(), r.commit.clone(), r.needs_review))
            .collect();
        review_rows.sort();
        let jobs: Vec<&str> = review_rows.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(jobs, ["debian", "k3s-master", "keel-node"]);
        assert!(
            review_rows.iter().all(|r| r.1 == plan.commit && r.2),
            "{review_rows:?}"
        );
        let parent = f.confirm(token).await;
        assert_eq!(state(&f, parent).await, "needs_review");
        // One child per job, each pinned to the plan's commit, each with its own group.
        let children: Vec<(Uuid, String, String, String)> = sqlx::query_as(
            "select id, job_name, commit, state from runs where parent_run_id = $1 \
             order by job_name",
        )
        .bind(parent)
        .fetch_all(&f.h.db.owner)
        .await
        .expect("children");
        assert_eq!(children.len(), 3, "{children:?}");
        assert!(
            children
                .iter()
                .all(|c| c.2 == plan.commit && c.3 == "needs_review"),
            "{children:?}"
        );
        let groups = groups(&f, parent).await;
        let shape: Vec<(&str, Vec<String>)> = groups
            .iter()
            .map(|g| {
                let mut hosts = g.3.clone();
                hosts.sort();
                (g.2.as_str(), hosts)
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                ("debian", vec!["master-1".to_owned(), "node-1".to_owned()]),
                ("k3s-master", vec!["master-1".to_owned()]),
                ("keel-node", vec!["node-1".to_owned()]),
            ]
        );
        assert!(groups.iter().all(|g| g.4 == plan.commit), "{groups:?}");
        for (g, c) in groups.iter().zip(&children) {
            assert_eq!(g.1, c.0, "each group belongs to its own step");
        }
        // The plan's approval waits for review.
        let out = f.h.cmd(Cmd::PreviewRunApprove { run: parent }).await;
        assert!(!op_error(&out).is_empty());

        // One step reviewed: its override is bound to its own run id and job, and still nothing
        // goes out while the other steps wait.
        let debian = &groups[0];
        let canonical = review(&f, &mut auth, debian.0).await;
        assert!(
            canonical.contains(&debian.1.to_string()) && canonical.contains("debian"),
            "{canonical}"
        );
        assert_eq!(state(&f, debian.1).await, "queued");
        assert_eq!(state(&f, parent).await, "needs_review");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            f.steps(parent).await.iter().all(|s| s.3 == "blocked"),
            "nothing runs before every step is reviewed"
        );

        // The last steps reviewed: one reviewer meets the policy, the converge goes out.
        for g in &groups[1..] {
            review(&f, &mut auth, g.0).await;
        }
        assert_eq!(f.ended(parent).await, ("succeeded".into(), None));
        assert!(f.steps(parent).await.iter().all(|s| s.3 == "succeeded"));
        let actions = f.h.audit_actions().await;
        assert!(
            actions.iter().any(|(a, _)| a == "run_overridden"),
            "{actions:?}"
        );
        // Each step went out with its own step's override, nothing else.
        let mut sent = 0;
        for r in &mut runners {
            while let Ok(frame) = r.received.try_recv() {
                let Frame::Approvals { run_id, items } = frame else {
                    continue;
                };
                let run: Uuid = run_id.parse().expect("run id");
                let own: Vec<(String,)> = sqlx::query_as(
                    "select assertion ->> 'signature' from approvals \
                     where run_id = $1 and kind = 'override' and decision = 'approve'",
                )
                .bind(run)
                .fetch_all(&f.h.db.owner)
                .await
                .expect("approvals");
                assert!(!items.is_empty(), "{run}");
                for item in &items {
                    assert!(own.iter().any(|o| o.0 == item.signature), "{run}");
                }
                sent += 1;
            }
        }
        // node-1: debian, keel-node; master-1: debian, k3s-master.
        assert_eq!(sent, 4);
        f.h.finish().await;
    }

    #[tokio::test]
    async fn a_reviewed_plan_still_waits_for_the_approvals_it_lacks() {
        let Some(f) = fixture("converge_signed_approval", 2).await else {
            return;
        };
        let mut auth = passkey(&f.h).await;
        let _runners = f.attach(|_, id| FakeRunner::new(id.to_string())).await;
        let parent = f.start().await;
        for g in groups(&f, parent).await {
            review(&f, &mut auth, g.0).await;
        }
        // One reviewer of two: the converge is held for approval, its window open.
        let (s, open): (String, bool) =
            sqlx::query_as("select state, approval_expires_at > now() from runs where id = $1")
                .bind(parent)
                .fetch_one(&f.h.db.owner)
                .await
                .expect("parent");
        assert_eq!((s.as_str(), open), ("pending_approval", true));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(f.steps(parent).await.iter().all(|s| s.3 == "blocked"));
        f.h.finish().await;
    }

    #[tokio::test]
    async fn denying_one_steps_review_denies_the_converge() {
        let Some(f) = fixture("converge_signed_deny", 1).await else {
            return;
        };
        let parent = f.start().await;
        let groups = groups(&f, parent).await;
        let (_, out) =
            f.h.confirm(Cmd::PreviewOverrideDeny {
                group: groups[1].0,
                comment: ReviewComment::new("Not this one.").expect("comment"),
            })
            .await;
        assert!(
            matches!(out.first(), Some(Event::Confirmed { .. })),
            "{out:?}"
        );
        assert_eq!(f.ended(parent).await.0, "denied");
        assert_eq!(state(&f, groups[1].1).await, "denied");
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
        // The other steps' reviews close with it.
        let out =
            f.h.cmd(Cmd::PreviewOverride {
                group: groups[0].0,
                comment: ReviewComment::new("Too late.").expect("comment"),
            })
            .await;
        assert_eq!(op_error(&out), "review_closed");
        f.h.finish().await;
    }
}
