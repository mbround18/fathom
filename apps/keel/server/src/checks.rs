//! Spec 068 US7 (FR-013, T059): required forge checks. A binding may name commit-status
//! contexts; a run from it waits in `waiting_checks` until every one is `success` at its commit,
//! then moves on to where it would have gone (review, approval or the queue). A `failure` or
//! `error` ends it `failed` (`checks_failed`); so does the binding's check timeout
//! (`checks_timeout`). The required contexts are snapshotted onto the run when it is made.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use keel_core::authz::Capability;
use keel_core::domain::WorkspaceId;
use keel_core::ports::{CommitStatusState, Forge, ForgeError, ForgeStatus};
use keel_core::run::{RunState, RunStatus, StateReason};
use keel_protocol::{CheckState, Event, RunCheckView};
use serde_json::json;
use sqlx::PgConnection;
use tokio::task::JoinHandle;
use tracing::Instrument;
use uuid::Uuid;

use crate::audit::{self, Decision, Entry};
use crate::bindings::{binding_event, channel, db_failed, not_found, op_error, preview, require};
use crate::dispatch::{PendingOp, Request};
use crate::identity::Caller;
use crate::preview::Draft;
use crate::proposals::ForgeTarget;
use crate::serve::AppState;

/// data-model.md: at most 16 required contexts per binding.
const MAX_CONTEXTS: usize = 16;
const MAX_CONTEXT_LEN: usize = 200;
const TIMEOUT_RANGE: std::ops::RangeInclusive<u32> = 60..=86_400;

/// The binding's required contexts, in name order.
///
/// # Errors
///
/// The database failed.
pub(crate) async fn required(
    conn: &mut PgConnection,
    binding: Uuid,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "select context from binding_required_checks where binding_id = $1 order by context",
    )
    .bind(binding)
    .fetch_all(conn)
    .await
}

/// Whether a run that would start in `status` waits for checks first: only one that has not
/// already ended (spec 068 data-model: `waiting_checks` comes before review, approval and queue).
pub(crate) fn gates(status: RunStatus, contexts: &[String]) -> bool {
    !contexts.is_empty()
        && matches!(
            status.state,
            RunState::NeedsReview | RunState::PendingApproval | RunState::Queued
        )
}

/// Records each required context as `pending` for a run made in `waiting_checks`.
///
/// # Errors
///
/// The database failed.
pub(crate) async fn hold(
    conn: &mut PgConnection,
    run: Uuid,
    contexts: &[String],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "insert into run_check_states (run_id, context, state) \
         select $1, c, 'pending' from unnest($2::text[]) as c",
    )
    .bind(run)
    .bind(contexts)
    .execute(conn)
    .await?;
    Ok(())
}

pub(crate) fn waiting() -> RunStatus {
    RunStatus {
        state: RunState::WaitingChecks,
        reason: None,
    }
}

// ---- the forge -------------------------------------------------------------------------------

/// Why the binding's forge cannot be asked about commit statuses.
enum NoForge {
    /// No forge connection, no connector, or the forge cannot report statuses.
    Unsupported(String),
    /// Reading the connection or its credential failed.
    Failed(String),
}

/// A forge client for `binding`, from its workspace's forge connection.
async fn forge_for(app: &AppState, binding: Uuid) -> Result<Arc<dyn Forge>, NoForge> {
    let row: Option<(String, String, Uuid, String)> = sqlx::query_as(
        "select fc.provider, fc.api_base, fc.credential_id, b.url from repo_bindings b \
         join projects p on p.id = b.project_id \
         join forge_connections fc on fc.workspace_id = p.workspace_id where b.id = $1",
    )
    .bind(binding)
    .fetch_optional(&app.db)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "checks: database failed");
        NoForge::Failed("Keel could not read the forge connection.".to_owned())
    })?;
    let Some((provider, api_base, credential, repo_url)) = row else {
        return Err(NoForge::Unsupported(
            "This workspace has no forge connection, so Keel cannot read commit statuses."
                .to_owned(),
        ));
    };
    let Some(connect) = app.forges.connector() else {
        return Err(NoForge::Unsupported(
            "This server cannot reach forges, so it cannot read commit statuses.".to_owned(),
        ));
    };
    let Ok((_, token)) = crate::credentials::open(app, credential).await else {
        return Err(NoForge::Failed(
            "The forge connection's credential could not be opened.".to_owned(),
        ));
    };
    connect(ForgeTarget {
        provider,
        api_base,
        repo_url,
        token,
    })
    .map_err(|e| NoForge::Failed(e.to_string()))
}

// ---- the command -----------------------------------------------------------------------------

/// `PreviewRequiredChecks`: replaces the binding's required contexts (and its timeout, when
/// given). Step-up. Refused `checks_unsupported` when the forge cannot report statuses.
pub async fn preview_required_checks(
    app: &AppState,
    req: &Request<'_>,
    binding: Uuid,
    contexts: Vec<String>,
    timeout_secs: Option<u32>,
) -> Vec<Event> {
    const ACTION: &str = "preview_required_checks";
    type BindingRow = (Uuid, String, String, Option<String>, i32);
    let row: Result<Option<BindingRow>, sqlx::Error> = sqlx::query_as(
        "select p.workspace_id, b.url, b.branch, b.last_commit, b.checks_timeout_secs \
             from repo_bindings b join projects p on p.id = b.project_id where b.id = $1",
    )
    .bind(binding)
    .fetch_optional(&app.db)
    .await;
    let (ws, url, branch, head, timeout_now) = match row {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("binding"),
        Err(e) => return db_failed(&e),
    };
    if let Err(e) = require(app, req, ACTION, Capability::Administer, ws).await {
        return e;
    }
    let mut set = BTreeSet::new();
    for c in &contexts {
        let c = c.trim();
        if c.is_empty() || c.chars().count() > MAX_CONTEXT_LEN || c.chars().any(char::is_control) {
            return op_error(
                "invalid_context",
                "A check context is 1 to 200 printable characters.",
            );
        }
        set.insert(c.to_owned());
    }
    if set.len() > MAX_CONTEXTS {
        return op_error(
            "too_many_contexts",
            &format!("A binding can require at most {MAX_CONTEXTS} checks."),
        );
    }
    if let Some(t) = timeout_secs
        && !TIMEOUT_RANGE.contains(&t)
    {
        return op_error(
            "invalid_timeout",
            "The check timeout is 60 to 86400 seconds.",
        );
    }
    let contexts: Vec<String> = set.into_iter().collect();
    let mut conn = match app.db.acquire().await {
        Ok(c) => c,
        Err(e) => return db_failed(&e),
    };
    let current = match required(&mut conn, binding).await {
        Ok(c) => c,
        Err(e) => return db_failed(&e),
    };
    drop(conn);
    let timeout_same = timeout_secs.is_none_or(|t| i64::from(t) == i64::from(timeout_now));
    if current == contexts && timeout_same {
        return op_error("no_change", "Nothing would change.");
    }
    // FR-013: a forge without commit statuses cannot gate runs; ask it now, at the head.
    if !contexts.is_empty() {
        let Some(head) = head else {
            return op_error(
                "binding_not_synced",
                "Sync the binding first: Keel checks the forge reports statuses on its commit.",
            );
        };
        let forge = match forge_for(app, binding).await {
            Ok(f) => f,
            Err(NoForge::Unsupported(m)) => return op_error("checks_unsupported", &m),
            Err(NoForge::Failed(m)) => return op_error("forge_refused", &m),
        };
        match forge.commit_statuses(&head).await {
            Ok(_) => {}
            Err(ForgeError::Unsupported) => {
                return op_error(
                    "checks_unsupported",
                    "This forge cannot report commit statuses, so runs cannot wait on them.",
                );
            }
            Err(e) => return op_error(e.code(), &e.to_string()),
        }
    }
    let summary = if contexts.is_empty() {
        format!("Require no checks for {url} ({branch}): runs no longer wait on the forge.")
    } else {
        let timeout = timeout_secs.map_or(i64::from(timeout_now), i64::from);
        format!(
            "Require the checks {} to pass on {url} ({branch}) before a run from a commit moves \
             on; runs wait up to {timeout} seconds. Runs already waiting keep the checks they \
             were made with.",
            contexts.join(", ")
        )
    };
    let affected = vec![format!("{url}@{branch}")];
    let ctx = channel(req);
    let op: PendingOp = Box::new(move |app: AppState, caller: Caller| {
        Box::pin(async move {
            match set_required(&app, &caller, ws, binding, &contexts, timeout_secs, ctx).await {
                Ok(true) => {}
                Ok(false) => return not_found("binding"),
                Err(e) => return db_failed(&e),
            }
            match binding_event(&app, binding).await {
                Ok(Some(ev)) => vec![ev],
                Ok(None) => not_found("binding"),
                Err(e) => db_failed(&e),
            }
        })
    });
    preview(
        app,
        req,
        Draft {
            action: "binding.required_checks_set".to_owned(),
            summary,
            affected,
            step_up: true,
        },
        op,
    )
}

async fn set_required(
    app: &AppState,
    caller: &Caller,
    ws: Uuid,
    binding: Uuid,
    contexts: &[String],
    timeout_secs: Option<u32>,
    mut ctx: serde_json::Value,
) -> Result<bool, sqlx::Error> {
    let mut tx = app.db.begin().await?;
    let found: Option<(i32,)> =
        sqlx::query_as("select checks_timeout_secs from repo_bindings where id = $1 for update")
            .bind(binding)
            .fetch_optional(&mut *tx)
            .await?;
    if found.is_none() {
        return Ok(false);
    }
    let before = required(&mut tx, binding).await?;
    sqlx::query("delete from binding_required_checks where binding_id = $1 and context <> all($2)")
        .bind(binding)
        .bind(contexts)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "insert into binding_required_checks (binding_id, context, created_by) \
         select $1, c, $3 from unnest($2::text[]) as c on conflict do nothing",
    )
    .bind(binding)
    .bind(contexts)
    .bind(caller.user.id)
    .execute(&mut *tx)
    .await?;
    if let Some(t) = timeout_secs {
        sqlx::query("update repo_bindings set checks_timeout_secs = $2 where id = $1")
            .bind(binding)
            .bind(i32::try_from(t).unwrap_or(i32::MAX))
            .execute(&mut *tx)
            .await?;
    }
    ctx["before"] = json!(before);
    ctx["contexts"] = json!(contexts);
    ctx["timeout_secs"] = json!(timeout_secs);
    audit::record(
        &mut tx,
        Entry {
            actor: &caller.actor(),
            action: "binding.required_checks_set",
            workspace: Some(WorkspaceId(ws)),
            target: json!({ "binding": binding }),
            decision: Decision::Allowed,
            reason: None,
            context: ctx,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

// ---- waiting ---------------------------------------------------------------------------------

/// Checks waiting runs once now, then every `period` (the sync poller's cadence).
pub fn spawn_poller(app: AppState, period: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(period);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            if let Err(e) = poll_once(&app).await {
                tracing::warn!(error = %e, "checks: database unavailable");
            }
        }
    })
}

#[derive(sqlx::FromRow)]
struct Waiting {
    id: Uuid,
    workspace_id: Uuid,
    binding_id: Uuid,
    commit: String,
    overdue: bool,
}

/// What became of a waiting run on one pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Waiting,
    Passed,
    Failed,
    TimedOut,
}

impl Outcome {
    const fn as_str(self) -> &'static str {
        match self {
            Outcome::Waiting => "waiting",
            Outcome::Passed => "passed",
            Outcome::Failed => "failed",
            Outcome::TimedOut => "timeout",
        }
    }
}

/// Reads the forge for every run in `waiting_checks` (once per binding and commit) and moves
/// each on, ends it, or records what changed. Returns how many runs left `waiting_checks`.
///
/// # Errors
///
/// The database failed.
pub async fn poll_once(app: &AppState) -> Result<usize, sqlx::Error> {
    let runs: Vec<Waiting> = sqlx::query_as(
        "select r.id, r.workspace_id, r.binding_id, r.commit, \
           r.created_at + b.checks_timeout_secs * interval '1 second' < now() as overdue \
         from runs r join repo_bindings b on b.id = r.binding_id \
         where r.state = 'waiting_checks' order by r.created_at",
    )
    .fetch_all(&app.db)
    .await?;
    let mut statuses: BTreeMap<(Uuid, String), Option<Vec<ForgeStatus>>> = BTreeMap::new();
    let mut moved = 0;
    for run in runs {
        let key = (run.binding_id, run.commit.clone());
        if !statuses.contains_key(&key) {
            let read = read_statuses(app, run.binding_id, &run.commit).await;
            statuses.insert(key.clone(), read);
        }
        let seen = statuses.get(&key).cloned().flatten();
        let span = tracing::info_span!(
            "checks.wait",
            run = %run.id,
            binding = %run.binding_id,
            outcome = tracing::field::Empty,
        );
        let outcome = step(app, &run, seen.as_deref())
            .instrument(span.clone())
            .await?;
        span.record("outcome", outcome.as_str());
        if outcome != Outcome::Waiting {
            moved += 1;
        }
    }
    Ok(moved)
}

/// The commit's statuses, or `None` when the forge could not be read (the run keeps waiting
/// until its timeout).
async fn read_statuses(app: &AppState, binding: Uuid, commit: &str) -> Option<Vec<ForgeStatus>> {
    let forge = match forge_for(app, binding).await {
        Ok(f) => f,
        Err(NoForge::Unsupported(m) | NoForge::Failed(m)) => {
            tracing::warn!(binding = %binding, reason = %m, "checks: no forge to ask");
            return None;
        }
    };
    match forge.commit_statuses(commit).await {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!(binding = %binding, error = %e, "checks: forge read failed");
            None
        }
    }
}

fn state_of(s: CommitStatusState) -> CheckState {
    match s {
        CommitStatusState::Pending => CheckState::Pending,
        CommitStatusState::Success => CheckState::Success,
        CommitStatusState::Failure => CheckState::Failure,
        CommitStatusState::Error => CheckState::Error,
    }
}

fn state_str(s: CheckState) -> &'static str {
    match s {
        CheckState::Pending => "pending",
        CheckState::Success => "success",
        CheckState::Failure => "failure",
        CheckState::Error => "error",
        CheckState::Missing => "missing",
    }
}

/// A run's required checks as last recorded, in context order.
pub(crate) async fn views(
    conn: &mut PgConnection,
    run: Uuid,
) -> Result<Vec<RunCheckView>, sqlx::Error> {
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "select context, state, target_url from run_check_states where run_id = $1 \
         order by context",
    )
    .bind(run)
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(context, state, target_url)| RunCheckView {
            context,
            state: crate::bindings::enum_from(&state).unwrap_or(CheckState::Missing),
            target_url,
        })
        .collect())
}

async fn step(
    app: &AppState,
    run: &Waiting,
    seen: Option<&[ForgeStatus]>,
) -> Result<Outcome, sqlx::Error> {
    let mut tx = app.db.begin().await?;
    let state: Option<String> =
        sqlx::query_scalar("select state from runs where id = $1 for no key update")
            .bind(run.id)
            .fetch_optional(&mut *tx)
            .await?;
    if state.as_deref() != Some("waiting_checks") {
        return Ok(Outcome::Waiting);
    }
    let before = views(&mut tx, run.id).await?;
    let after: Vec<RunCheckView> = match seen {
        // The forge could not be read: nothing new is known.
        None => before.clone(),
        Some(seen) => before
            .iter()
            .map(|c| {
                // The forge lists the latest status of each context first.
                match seen.iter().find(|s| s.context == c.context) {
                    Some(s) => RunCheckView {
                        context: c.context.clone(),
                        state: state_of(s.state),
                        target_url: s.target_url.clone(),
                    },
                    None => RunCheckView {
                        context: c.context.clone(),
                        state: CheckState::Missing,
                        target_url: None,
                    },
                }
            })
            .collect(),
    };
    for c in after.iter().filter(|c| !before.contains(c)) {
        sqlx::query(
            "update run_check_states set state = $3, target_url = $4, updated_at = now() \
             where run_id = $1 and context = $2",
        )
        .bind(run.id)
        .bind(&c.context)
        .bind(state_str(c.state))
        .bind(&c.target_url)
        .execute(&mut *tx)
        .await?;
    }
    let failed: Vec<&str> = after
        .iter()
        .filter(|c| matches!(c.state, CheckState::Failure | CheckState::Error))
        .map(|c| c.context.as_str())
        .collect();
    let outcome = if !failed.is_empty() {
        Outcome::Failed
    } else if after.iter().all(|c| c.state == CheckState::Success) {
        Outcome::Passed
    } else if run.overdue {
        Outcome::TimedOut
    } else {
        Outcome::Waiting
    };
    let status = match outcome {
        Outcome::Waiting => None,
        Outcome::Passed => Some(release(&mut tx, run.id, run.workspace_id).await?),
        Outcome::Failed | Outcome::TimedOut => {
            let reason = if outcome == Outcome::Failed {
                StateReason::ChecksFailed
            } else {
                StateReason::ChecksTimeout
            };
            Some(fail(&mut tx, run, reason, &after, &failed).await?)
        }
    };
    tx.commit().await?;
    if after != before {
        app.repos.push(
            run.workspace_id,
            Event::RunChecksChanged {
                run_id: run.id,
                checks: after,
            },
        );
    }
    if let Some(status) = status {
        crate::queue::push_state(app, run.workspace_id, run.id, status);
        if status.state == RunState::Queued {
            crate::queue::kick(app);
        }
    }
    Ok(outcome)
}

/// Moves a run whose checks all passed to where it would have gone when it was made: review
/// when it has review groups, approval when it was held (its window starts now), else the queue.
async fn release(tx: &mut PgConnection, run: Uuid, ws: Uuid) -> Result<RunStatus, sqlx::Error> {
    let next: String = sqlx::query_scalar(
        "update runs r set state = n.next, \
           queued_at = case when n.next = 'queued' then now() end, \
           approval_expires_at = case when n.next = 'pending_approval' \
             then now() + (r.approval_expires_at - r.created_at) end \
         from (select case \
                 when exists (select 1 from review_groups g where g.run_id = $1) \
                   then 'needs_review' \
                 when x.approvals_needed is not null then 'pending_approval' \
                 else 'queued' end as next \
               from runs x where x.id = $1) n \
         where r.id = $1 returning n.next",
    )
    .bind(run)
    .fetch_one(&mut *tx)
    .await?;
    let state = next.parse::<RunState>().unwrap_or(RunState::Queued);
    if state == RunState::Queued {
        // A converge's first steps go out with it (spec 068).
        crate::converge::release(tx, run).await?;
    }
    audit::record(
        tx,
        Entry {
            actor: &audit::Actor::System,
            action: "run.checks_passed",
            workspace: Some(WorkspaceId(ws)),
            target: json!({ "run": run }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({ "state": next }),
        },
    )
    .await?;
    Ok(RunStatus {
        state,
        reason: None,
    })
}

/// Ends a waiting run `failed`; its hosts never started, so they are cancelled with it.
async fn fail(
    tx: &mut PgConnection,
    run: &Waiting,
    reason: StateReason,
    checks: &[RunCheckView],
    failed: &[&str],
) -> Result<RunStatus, sqlx::Error> {
    sqlx::query(
        "update runs set state = 'failed', state_reason = $2, ended_at = now() where id = $1",
    )
    .bind(run.id)
    .bind(reason.as_str())
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "update run_hosts set state = 'cancelled', ended_at = now() \
         where run_id = $1 and state in ('queued', 'blocked')",
    )
    .bind(run.id)
    .execute(&mut *tx)
    .await?;
    crate::converge::abandon(tx, run.id).await?;
    let pending: Vec<&str> = checks
        .iter()
        .filter(|c| c.state != CheckState::Success)
        .map(|c| c.context.as_str())
        .collect();
    audit::record(
        tx,
        Entry {
            actor: &audit::Actor::System,
            action: "run.checks_failed",
            workspace: Some(WorkspaceId(run.workspace_id)),
            target: json!({ "run": run.id }),
            decision: Decision::Allowed,
            reason: Some(reason.as_str()),
            context: json!({
                "commit": run.commit,
                "failed": failed,
                "not_passed": pending,
            }),
        },
    )
    .await?;
    Ok(RunStatus {
        state: RunState::Failed,
        reason: Some(reason),
    })
}
