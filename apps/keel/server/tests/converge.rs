//! Converge (spec 068 US1, T016) against the fake runner: a preview lists each host's steps in
//! order; confirming makes a parent run with one child run per job; each host walks its steps in
//! order; a failed step stops that host only; approval holds every step until the parent is
//! approved, and a denial cancels them; a held converge is replaced by a newer one on the same
//! hosts; a retry re-plans only the hosts that did not succeed; and the flag gates it all.

mod common;
mod repos;

use std::sync::Arc;
use std::time::{Duration, Instant};

use keel_exec::fake::{FakeHandle, FakeRunner, Script};
use keel_exec::frames::CAP_MANIFEST_V068;
use keel_protocol::{
    Cmd, ConvergeHostState, ConvergeTarget, Event, PreviewId, StepState, TargetSelection,
};
use keel_server::config::Config;
use keel_server::runs::{RunSettings, Runs};
use repos::{Harness, op_error};
use uuid::Uuid;

const WITHIN: Duration = Duration::from_secs(20);

struct Fixture {
    h: Harness,
    binding: Uuid,
    /// `node-1` (keel-node) and `master-1` (k3s-master).
    node: Uuid,
    master: Uuid,
}

impl Fixture {
    async fn new(test: &str, approval: bool) -> Option<Self> {
        let mut h = Harness::new(test).await?;
        h.app.runs = Runs::new(RunSettings {
            ping_interval: Duration::from_millis(50),
            flush_interval: Duration::from_millis(20),
            ..RunSettings::default()
        });
        h.app.cfg = Arc::new(Config {
            converge: true,
            ..(*h.app.cfg).clone()
        });
        let (_src, binding) = repos::bind_converge_repo(&h).await;
        let owner = &h.db.owner;
        let inventory = Uuid::now_v7();
        sqlx::query(
            "insert into inventories (id, workspace_id, name, source) \
             values ($1, $2, 'prod', 'managed')",
        )
        .bind(inventory)
        .bind(h.workspace)
        .execute(owner)
        .await
        .expect("inventory");
        let revision = Uuid::now_v7();
        sqlx::query(
            "insert into inventory_revisions (id, inventory_id, seq, content, content_hash, \
             status) values ($1, $2, 1, '{}', 'h1', 'valid')",
        )
        .bind(revision)
        .bind(inventory)
        .execute(owner)
        .await
        .expect("revision");
        let mut ids = Vec::new();
        for (name, profile) in [("node-1", "keel-node"), ("master-1", "k3s-master")] {
            sqlx::query(
                "insert into inventory_hosts (revision_id, host, groups) values ($1, $2, '{}')",
            )
            .bind(revision)
            .bind(name)
            .execute(owner)
            .await
            .expect("inventory host");
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
        if !approval {
            sqlx::query(
                "insert into approval_policies (id, workspace_id, priority, requires_approval) \
                 values ($1, $2, 1, false)",
            )
            .bind(Uuid::now_v7())
            .bind(h.workspace)
            .execute(owner)
            .await
            .expect("policy");
        }
        Some(Self {
            h,
            binding,
            node: ids[0],
            master: ids[1],
        })
    }

    /// Connects a 068 runner for each host, built by `runner`.
    async fn attach(&self, runner: impl Fn(&str, Uuid) -> FakeRunner) -> Vec<FakeHandle> {
        let mut out = Vec::new();
        for (name, id) in [("node-1", self.node), ("master-1", self.master)] {
            let (stream, handle) = runner(name, id)
                .with_capabilities([CAP_MANIFEST_V068])
                .spawn();
            keel_server::queue::attach(self.h.app.clone(), id, stream)
                .await
                .expect("attach");
            out.push(handle);
        }
        out
    }

    fn target(&self) -> ConvergeTarget {
        ConvergeTarget {
            binding_id: self.binding,
            hosts: TargetSelection::All,
            check: false,
        }
    }

    /// Previews `cmd`, returns the plan's token after checking it answered a plan.
    async fn plan(&self, cmd: Cmd) -> (PreviewId, keel_protocol::ConvergePlan) {
        let out = self.h.cmd(cmd).await;
        match out.as_slice() {
            [
                Event::ConvergePlan {
                    preview_token,
                    plan,
                },
                Event::Preview(p),
            ] => {
                assert_eq!(*preview_token, p.preview_id);
                (*preview_token, plan.clone())
            }
            other => panic!("expected a plan, got {other:?}"),
        }
    }

    /// Previews and confirms a converge of every host; returns the parent run.
    async fn start(&self) -> Uuid {
        let (token, _) = self.plan(Cmd::PreviewConverge(self.target())).await;
        self.confirm(token).await
    }

    async fn confirm(&self, token: PreviewId) -> Uuid {
        let out = self
            .h
            .cmd(Cmd::ConfirmConverge {
                preview_token: token,
                step_up: None,
            })
            .await;
        match out.as_slice() {
            [Event::Confirmed { .. }, Event::ConvergeStarted { run_id }] => *run_id,
            other => panic!("confirm: {other:?}"),
        }
    }

    async fn ended(&self, run: Uuid) -> (String, Option<String>) {
        let start = Instant::now();
        loop {
            let (state, reason, ended): (String, Option<String>, bool) = sqlx::query_as(
                "select state, state_reason, ended_at is not null from runs where id = $1",
            )
            .bind(run)
            .fetch_one(&self.h.db.owner)
            .await
            .expect("run");
            if ended {
                return (state, reason);
            }
            assert!(start.elapsed() < WITHIN, "run {run} still {state}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Each child step as (host, position, job, run_hosts state, skip reason), in order.
    async fn steps(&self, parent: Uuid) -> Vec<(String, i16, String, String, Option<String>)> {
        sqlx::query_as(
            "select h.inventory_name, s.position, s.job_name, rh.state, rh.skip_reason \
             from converge_steps s join hosts h on h.id = s.host_id \
             join run_hosts rh on rh.run_id = s.step_run_id and rh.host_id = s.host_id \
             where s.run_id = $1 order by h.inventory_name, s.position",
        )
        .bind(parent)
        .fetch_all(&self.h.db.owner)
        .await
        .expect("steps")
    }

    async fn converge(&self, run: Uuid) -> keel_protocol::ConvergeView {
        match self
            .h
            .cmd(Cmd::GetConverge { run_id: run })
            .await
            .as_slice()
        {
            [Event::Converge(view)] => view.clone(),
            other => panic!("get converge: {other:?}"),
        }
    }
}

fn row(
    host: &str,
    pos: i16,
    job: &str,
    state: &str,
    skip: Option<&str>,
) -> (String, i16, String, String, Option<String>) {
    (
        host.into(),
        pos,
        job.into(),
        state.into(),
        skip.map(Into::into),
    )
}

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
