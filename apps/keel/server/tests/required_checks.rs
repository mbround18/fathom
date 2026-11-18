//! Spec 068 US7 (FR-013, T058, T059): required forge checks. A binding names commit-status
//! contexts; a run or converge from it waits in `waiting_checks` until every one is `success`,
//! then moves on to approval or the queue. A failure ends it `checks_failed`, the binding's
//! timeout `checks_timeout`. Setting the contexts is refused on a forge without statuses.

mod common;
mod converge_support;
mod repos;

use std::sync::{Arc, Mutex};

use converge_support::Fixture;
use keel_core::ports::{
    ChangeRequest, CommitStatusState, Forge, ForgeAccess, ForgeError, ForgeFuture, ForgeStatus,
    OpenedChange,
};
use keel_exec::fake::{FakeHandle, FakeRunner, Script};
use keel_exec::frames::Frame;
use keel_protocol::{
    CheckState, Cmd, CredentialKind, Event, ObjectName, Pattern, RequiredCheck, RunCheckView,
    RunInfo, RunPhase, SecretValue,
};
use keel_server::proposals::{Connector, Forges};
use uuid::Uuid;

/// A forge whose commit statuses the test sets; `None` is a forge that cannot report them.
#[derive(Clone, Default)]
struct FakeForge {
    statuses: Arc<Mutex<Option<Vec<ForgeStatus>>>>,
}

impl FakeForge {
    fn set(&self, statuses: Option<&[(&str, CommitStatusState)]>) {
        *self.statuses.lock().unwrap() = statuses.map(|s| {
            s.iter()
                .map(|(context, state)| ForgeStatus {
                    context: (*context).to_owned(),
                    state: *state,
                    target_url: Some(format!("https://ci.example/{context}")),
                })
                .collect()
        });
    }
}

impl Forge for FakeForge {
    fn probe(&self) -> ForgeFuture<'_, ForgeAccess> {
        Box::pin(async {
            Ok(ForgeAccess {
                can_push: true,
                default_branch: "main".to_owned(),
            })
        })
    }

    fn open_change<'a>(&'a self, _: &'a ChangeRequest) -> ForgeFuture<'a, OpenedChange> {
        Box::pin(async { Err(ForgeError::Unsupported) })
    }

    fn commit_statuses<'a>(&'a self, _: &'a str) -> ForgeFuture<'a, Vec<ForgeStatus>> {
        let s = self.statuses.lock().unwrap().clone();
        Box::pin(async move { s.ok_or(ForgeError::Unsupported) })
    }
}

/// The fixture with `forge` as its workspace's forge; `connected` adds the forge connection.
async fn fixture(
    test: &str,
    approval: bool,
    forge: &FakeForge,
    connected: bool,
) -> Option<Fixture> {
    let mut f = Fixture::new(test, approval).await?;
    let fake = forge.clone();
    let connect: Connector = Arc::new(move |_| Ok(Arc::new(fake.clone()) as Arc<dyn Forge>));
    f.h.app =
        f.h.app
            .clone()
            .with_forges(Forges::default().with_connector(connect));
    if connected {
        let (_, out) =
            f.h.confirm(Cmd::PreviewCredentialCreate {
                workspace: f.h.workspace,
                name: ObjectName::new("forge").expect("name"),
                kind: CredentialKind::ForgeToken,
                value: SecretValue::new("forge-token-value").expect("value"),
            })
            .await;
        let [_, Event::Credentials { items }] = out.as_slice() else {
            panic!("{out:?}")
        };
        sqlx::query(
            "insert into forge_connections (workspace_id, provider, api_base, credential_id) \
             values ($1, 'forgejo', 'https://forge.example/api/v1', $2)",
        )
        .bind(f.h.workspace)
        .bind(items[0].id)
        .execute(&f.h.db.owner)
        .await
        .expect("forge connection");
    }
    Some(f)
}

fn require(f: &Fixture, contexts: &[&str]) -> Cmd {
    Cmd::PreviewRequiredChecks {
        binding_id: f.binding,
        contexts: contexts.iter().map(|c| (*c).to_owned()).collect(),
        timeout_secs: None,
    }
}

/// Requires `contexts` through the command (step-up preview, then `Confirm`).
async fn set_required(f: &Fixture, contexts: &[&str]) {
    let (p, out) = f.h.confirm(require(f, contexts)).await;
    assert!(p.step_up, "required checks need step-up");
    match out.as_slice() {
        [_, Event::Binding { binding, .. }] => assert_eq!(
            binding.required_checks,
            contexts
                .iter()
                .map(|c| RequiredCheck {
                    context: (*c).to_owned()
                })
                .collect::<Vec<_>>()
        ),
        other => panic!("confirm: {other:?}"),
    }
}

async fn poll(f: &Fixture) -> usize {
    keel_server::checks::poll_once(&f.h.app)
        .await
        .expect("poll")
}

async fn job(f: &Fixture, name: &str) -> Uuid {
    sqlx::query_scalar("select id from jobs where binding_id = $1 and name = $2")
        .bind(f.binding)
        .bind(name)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("job")
}

async fn start_debian(f: &Fixture) -> RunInfo {
    let (_, out) =
        f.h.confirm(Cmd::PreviewRunStart {
            job: job(f, "debian").await,
            limit: Some(Pattern::new("node-1").expect("limit")),
            params: Default::default(),
            check: false,
        })
        .await;
    match out.as_slice() {
        [_, Event::Run { run, .. }] => run.clone(),
        other => panic!("run start: {other:?}"),
    }
}

async fn run(f: &Fixture, id: Uuid) -> RunInfo {
    match f.h.cmd(Cmd::GetRun { run: id }).await.as_slice() {
        [Event::Run { run, .. }] => run.clone(),
        other => panic!("get run: {other:?}"),
    }
}

fn assigned(runner: &mut FakeHandle) -> usize {
    let mut n = 0;
    while let Ok(frame) = runner.received.try_recv() {
        if matches!(frame, Frame::Assign { .. }) {
            n += 1;
        }
    }
    n
}

async fn audits(f: &Fixture, action: &str) -> i64 {
    sqlx::query_scalar("select count(*) from audit_event where action = $1")
        .bind(action)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("audit")
}

fn check(context: &str, state: CheckState, url: bool) -> RunCheckView {
    RunCheckView {
        context: context.to_owned(),
        state,
        target_url: url.then(|| format!("https://ci.example/{context}")),
    }
}

#[tokio::test]
async fn the_preview_is_refused_on_a_forge_without_commit_statuses() {
    let forge = FakeForge::default();
    let Some(f) = fixture("checks_refused", false, &forge, false).await else {
        return;
    };
    // No forge connection at all.
    let out = f.h.cmd(require(&f, &["ci/lint"])).await;
    assert!(
        matches!(out.as_slice(), [Event::OpError { code, .. }] if code == "checks_unsupported"),
        "{out:?}"
    );
    let f = {
        f.h.finish().await;
        fixture("checks_refused2", false, &forge, true)
            .await
            .expect("fixture")
    };
    // A forge that cannot report statuses.
    forge.set(None);
    let out = f.h.cmd(require(&f, &["ci/lint"])).await;
    assert!(
        matches!(out.as_slice(), [Event::OpError { code, .. }] if code == "checks_unsupported"),
        "{out:?}"
    );
    // Bad input is refused before the forge is asked.
    let out = f.h.cmd(require(&f, &[""])).await;
    assert!(
        matches!(out.as_slice(), [Event::OpError { code, .. }] if code == "invalid_context"),
        "{out:?}"
    );
    let many: Vec<String> = (0..17).map(|i| format!("ci/{i}")).collect();
    let out =
        f.h.cmd(Cmd::PreviewRequiredChecks {
            binding_id: f.binding,
            contexts: many,
            timeout_secs: None,
        })
        .await;
    assert!(
        matches!(out.as_slice(), [Event::OpError { code, .. }] if code == "too_many_contexts"),
        "{out:?}"
    );
    let out =
        f.h.cmd(Cmd::PreviewRequiredChecks {
            binding_id: f.binding,
            contexts: vec!["ci/lint".into()],
            timeout_secs: Some(10),
        })
        .await;
    assert!(
        matches!(out.as_slice(), [Event::OpError { code, .. }] if code == "invalid_timeout"),
        "{out:?}"
    );
    // A forge with statuses: the contexts are set (step-up) and audited.
    forge.set(Some(&[]));
    set_required(&f, &["ci/lint", "ci/test"]).await;
    assert_eq!(audits(&f, "binding.required_checks_set").await, 1);
    let out = f.h.cmd(require(&f, &["ci/test", "ci/lint"])).await;
    assert!(
        matches!(out.as_slice(), [Event::OpError { code, .. }] if code == "no_change"),
        "{out:?}"
    );
    // Clearing them does not ask the forge.
    forge.set(None);
    set_required(&f, &[]).await;
    f.h.finish().await;
}

#[tokio::test]
async fn a_run_waits_for_its_checks_then_queues() {
    let forge = FakeForge::default();
    forge.set(Some(&[]));
    let Some(f) = fixture("checks_pass", false, &forge, true).await else {
        return;
    };
    set_required(&f, &["ci/lint", "ci/test"]).await;
    let mut runners = f
        .attach(|_, id| FakeRunner::new(id.to_string()).default_script(Script::finish(0)))
        .await;
    let started = start_debian(&f).await;
    assert_eq!(started.state, RunPhase::WaitingChecks);
    // One context pending, the other not reported yet.
    forge.set(Some(&[("ci/lint", CommitStatusState::Pending)]));
    assert_eq!(poll(&f).await, 0);
    let info = run(&f, started.id).await;
    assert_eq!(info.state, RunPhase::WaitingChecks);
    assert_eq!(
        info.checks,
        Some(vec![
            check("ci/lint", CheckState::Pending, true),
            check("ci/test", CheckState::Missing, false),
        ])
    );
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(
        assigned(&mut runners[0]),
        0,
        "nothing goes out while waiting"
    );
    // Every context passes: the run moves on and runs.
    forge.set(Some(&[
        ("ci/lint", CommitStatusState::Success),
        ("ci/test", CommitStatusState::Success),
    ]));
    assert_eq!(poll(&f).await, 1);
    assert_eq!(f.ended(started.id).await.0, "succeeded");
    assert_eq!(assigned(&mut runners[0]), 1);
    assert_eq!(audits(&f, "run.checks_passed").await, 1);
    f.h.finish().await;
}

#[tokio::test]
async fn checks_come_before_approval() {
    let forge = FakeForge::default();
    forge.set(Some(&[]));
    let Some(f) = fixture("checks_approval", true, &forge, true).await else {
        return;
    };
    set_required(&f, &["ci/lint"]).await;
    let started = start_debian(&f).await;
    assert_eq!(started.state, RunPhase::WaitingChecks);
    forge.set(Some(&[("ci/lint", CommitStatusState::Success)]));
    assert_eq!(poll(&f).await, 1);
    let info = run(&f, started.id).await;
    assert_eq!(info.state, RunPhase::PendingApproval);
    let (open,): (bool,) =
        sqlx::query_as("select approval_expires_at > now() from runs where id = $1")
            .bind(started.id)
            .fetch_one(&f.h.db.owner)
            .await
            .expect("window");
    assert!(open, "the approval window starts when the checks pass");
    f.h.finish().await;
}

#[tokio::test]
async fn a_failed_check_ends_the_run_checks_failed() {
    let forge = FakeForge::default();
    forge.set(Some(&[]));
    let Some(f) = fixture("checks_fail", false, &forge, true).await else {
        return;
    };
    set_required(&f, &["ci/lint", "ci/test"]).await;
    let started = start_debian(&f).await;
    forge.set(Some(&[
        ("ci/lint", CommitStatusState::Success),
        ("ci/test", CommitStatusState::Failure),
    ]));
    assert_eq!(poll(&f).await, 1);
    assert_eq!(
        f.ended(started.id).await,
        ("failed".to_owned(), Some("checks_failed".to_owned()))
    );
    let (host,): (String,) = sqlx::query_as("select state from run_hosts where run_id = $1")
        .bind(started.id)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("host");
    assert_eq!(host, "cancelled");
    let (context,): (serde_json::Value,) = sqlx::query_as(
        "select context from audit_event where action = 'run.checks_failed' and target->>'run' = $1",
    )
    .bind(started.id.to_string())
    .fetch_one(&f.h.db.owner)
    .await
    .expect("audit");
    assert_eq!(context["failed"], serde_json::json!(["ci/test"]));
    f.h.finish().await;
}

#[tokio::test]
async fn waiting_past_the_timeout_ends_checks_timeout() {
    let forge = FakeForge::default();
    forge.set(Some(&[]));
    let Some(f) = fixture("checks_timeout", false, &forge, true).await else {
        return;
    };
    set_required(&f, &["ci/lint"]).await;
    let started = start_debian(&f).await;
    forge.set(Some(&[("ci/lint", CommitStatusState::Pending)]));
    assert_eq!(poll(&f).await, 0);
    sqlx::query("update runs set created_at = now() - interval '2 hours' where id = $1")
        .bind(started.id)
        .execute(&f.h.db.owner)
        .await
        .expect("backdate");
    assert_eq!(poll(&f).await, 1);
    assert_eq!(
        f.ended(started.id).await,
        ("failed".to_owned(), Some("checks_timeout".to_owned()))
    );
    f.h.finish().await;
}

#[tokio::test]
async fn a_converge_waits_for_the_checks_too() {
    let forge = FakeForge::default();
    forge.set(Some(&[]));
    let Some(f) = fixture("checks_converge", false, &forge, true).await else {
        return;
    };
    set_required(&f, &["ci/lint"]).await;
    let _runners = f
        .attach(|_, id| FakeRunner::new(id.to_string()).default_script(Script::finish(0)))
        .await;
    let converge = f.start().await;
    let (state,): (String,) = sqlx::query_as("select state from runs where id = $1")
        .bind(converge)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("converge");
    assert_eq!(state, "waiting_checks");
    assert!(
        f.steps(converge).await.iter().all(|s| s.3 == "blocked"),
        "no step is queued while the checks wait"
    );
    forge.set(Some(&[("ci/lint", CommitStatusState::Success)]));
    assert_eq!(poll(&f).await, 1);
    assert_eq!(f.ended(converge).await.0, "succeeded");
    // Cancelling is allowed while waiting.
    forge.set(Some(&[("ci/lint", CommitStatusState::Pending)]));
    let waiting = start_debian(&f).await;
    let (_, out) = f.h.confirm(Cmd::PreviewCancel { run: waiting.id }).await;
    assert!(
        !out.iter().any(|e| matches!(e, Event::OpError { .. })),
        "{out:?}"
    );
    let (state,): (String,) = sqlx::query_as("select state from runs where id = $1")
        .bind(waiting.id)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("cancelled");
    assert_eq!(state, "cancelled");
    assert_eq!(poll(&f).await, 0, "a cancelled run is not waiting any more");
    f.h.finish().await;
}
