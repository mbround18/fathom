//! Spec 068 US5 (FR-011, T049, T050): check mode. A check run or check converge is sent to the
//! runner as `Assign.check`, records no host revision (no base, drift or rollback point moves),
//! shows `check` on the run, and is matched by approval policies on their `check` field.

mod common;
mod converge_support;
mod repos;

use converge_support::Fixture;
use keel_exec::fake::{FakeHandle, FakeRunner, Script};
use keel_exec::frames::Frame;
use keel_protocol::{Cmd, ConvergeTarget, Event, Pattern, PolicySpec, RunInfo, TargetSelection};
use uuid::Uuid;

async fn job(f: &Fixture, name: &str) -> Uuid {
    sqlx::query_scalar("select id from jobs where binding_id = $1 and name = $2")
        .bind(f.binding)
        .bind(name)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("job")
}

/// Previews and confirms the `debian` job on `node-1`; returns the summary and the run.
async fn start_debian(f: &Fixture, check: bool) -> (String, RunInfo) {
    let (p, out) =
        f.h.confirm(Cmd::PreviewRunStart {
            job: job(f, "debian").await,
            limit: Some(Pattern::new("node-1").expect("limit")),
            params: Default::default(),
            check,
        })
        .await;
    match out.as_slice() {
        [_, Event::Run { run, .. }] => (p.summary, run.clone()),
        other => panic!("run start: {other:?}"),
    }
}

async fn revisions(f: &Fixture) -> i64 {
    sqlx::query_scalar("select count(*) from host_revisions")
        .fetch_one(&f.h.db.owner)
        .await
        .expect("host revisions")
}

/// Every `Assign` a runner has received so far, as its `check` flag.
fn assigns(runner: &mut FakeHandle) -> Vec<bool> {
    let mut out = Vec::new();
    while let Ok(frame) = runner.received.try_recv() {
        if let Frame::Assign { check, .. } = frame {
            out.push(check);
        }
    }
    out
}

#[tokio::test]
async fn a_check_run_is_sent_as_check_and_records_no_revision() {
    let Some(f) = Fixture::new("check_run", false).await else {
        return;
    };
    let mut runners = f
        .attach(|_, id| FakeRunner::new(id.to_string()).default_script(Script::finish(0)))
        .await;
    let (summary, run) = start_debian(&f, true).await;
    assert!(summary.starts_with("Check "), "{summary}");
    assert!(run.check, "the run shows check mode");
    assert_eq!(f.ended(run.id).await.0, "succeeded");
    assert_eq!(assigns(&mut runners[0]), vec![true]);
    assert_eq!(revisions(&f).await, 0, "a check run applies nothing");
    match f.h.cmd(Cmd::GetRun { run: run.id }).await.as_slice() {
        [Event::Run { run, .. }] => assert!(run.check),
        other => panic!("get run: {other:?}"),
    }

    // The same job for real: no check flag, and the host revision is recorded.
    let (summary, run) = start_debian(&f, false).await;
    assert!(summary.starts_with("Run "), "{summary}");
    assert!(!run.check);
    assert_eq!(f.ended(run.id).await.0, "succeeded");
    assert_eq!(assigns(&mut runners[0]), vec![false]);
    assert_eq!(revisions(&f).await, 1);
    f.h.finish().await;
}

#[tokio::test]
async fn a_check_converge_checks_every_step_and_records_no_revision() {
    let Some(f) = Fixture::new("check_converge", false).await else {
        return;
    };
    let mut runners = f
        .attach(|_, id| FakeRunner::new(id.to_string()).default_script(Script::finish(0)))
        .await;
    let (token, _) = f
        .plan(Cmd::PreviewConverge(ConvergeTarget {
            binding_id: f.binding,
            hosts: TargetSelection::All,
            check: true,
        }))
        .await;
    let converge = f.confirm(token).await;
    assert_eq!(f.ended(converge).await.0, "succeeded");
    let steps = f.steps(converge).await;
    assert!(!steps.is_empty());
    for runner in &mut runners {
        let sent = assigns(runner);
        assert!(!sent.is_empty());
        assert!(sent.iter().all(|c| *c), "every step runs --check: {sent:?}");
    }
    let (check,): (bool,) = sqlx::query_as("select check_mode from runs where id = $1")
        .bind(converge)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("converge");
    assert!(check);
    let children: i64 =
        sqlx::query_scalar("select count(*) from runs where parent_run_id = $1 and not check_mode")
            .bind(converge)
            .fetch_one(&f.h.db.owner)
            .await
            .expect("children");
    assert_eq!(children, 0, "every step is a check run");
    assert_eq!(revisions(&f).await, 0);
    f.h.finish().await;
}

/// Upserts the priority-1 policy (no approval) matching on `check`; returns its id.
async fn upsert_policy(f: &Fixture, id: Option<Uuid>, check: bool) -> Uuid {
    let (_, out) =
        f.h.confirm(Cmd::PreviewPolicyUpsert(PolicySpec {
            workspace: f.h.workspace,
            id,
            priority: 1,
            job: None,
            inventory: None,
            limit: None,
            requires_approval: false,
            approvals_needed: 1,
            four_eyes: false,
            approval_ttl_secs: 3600,
            check: Some(check),
        }))
        .await;
    let id = match out.as_slice() {
        [_, Event::Policies { items, .. }] => {
            let p = items.iter().find(|p| p.priority == 1).expect("policy");
            assert_eq!(p.check, Some(check), "listed with its check field");
            p.id
        }
        other => panic!("policy upsert: {other:?}"),
    };
    let stored: serde_json::Value =
        sqlx::query_scalar("select match from approval_policies where id = $1")
            .bind(id)
            .fetch_one(&f.h.db.owner)
            .await
            .expect("policy");
    assert_eq!(stored["check"], serde_json::json!(check), "{stored}");
    id
}

/// Whether a `debian` run with `check` waits for approval.
async fn needs_approval(f: &Fixture, check: bool) -> bool {
    let (_, run) = start_debian(f, check).await;
    serde_json::to_value(run.state).expect("state") == "pending_approval"
}

#[tokio::test]
async fn a_policy_matches_check_runs_by_their_check_field() {
    // Only the catch-all, which requires approval.
    let Some(f) = Fixture::new("check_policy", true).await else {
        return;
    };
    // `check: false` matches only real runs: a check run falls to the catch-all.
    let id = upsert_policy(&f, None, false).await;
    assert!(needs_approval(&f, true).await);
    assert!(!needs_approval(&f, false).await);
    // `check: true` matches only check runs.
    upsert_policy(&f, Some(id), true).await;
    assert!(!needs_approval(&f, true).await);
    assert!(needs_approval(&f, false).await);
    f.h.finish().await;
}
