//! Approving and denying runs held by an approval policy (spec 064).
//!
//! A held run carries the terms it was held with (`approvals_needed`, `four_eyes`,
//! `approval_expires_at`; see [`crate::runs`]). Approving or denying needs `Approve` in the run's
//! workspace and a fresh passkey step-up. The decision runs in one transaction that locks the
//! run, so two answers to the same run are serialized: one decides it, the other is refused
//! `run_answered`. The rules themselves live in [`keel_core::approval`].

use keel_core::approval::{self, HeldTerms, Refusal};
use keel_core::authz::{self, Capability};
use keel_core::run::{RunEvent, RunState, RunStatus};
use keel_core::{InstanceRole, TeamRole, WorkspaceId};
use keel_protocol::{Cmd, Event, WorkspaceCount};
use serde_json::json;
use sqlx::PgConnection;
use std::time::Duration;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::audit::{self, Decision, Entry};
use crate::bindings::{db_failed, internal, not_found, op_error, require, subject};
use crate::dispatch::{PendingOp, Request};
use crate::identity::Caller;
use crate::preview::Draft;
use crate::serve::AppState;

const APPROVE: &str = "preview_run_approve";
const DENY: &str = "preview_run_deny";

/// Handles the run-approval commands.
pub async fn handle(app: &AppState, req: &Request<'_>, cmd: Cmd) -> Vec<Event> {
    match cmd {
        Cmd::PreviewRunApprove { run } => preview_run_approve(app, req, run).await,
        Cmd::PreviewRunDeny { run, reason } => {
            let reason = reason.map(|r| r.as_str().trim().to_owned());
            preview_run_deny(app, req, run, reason).await
        }
        Cmd::CountPendingApprovals => count_pending_approvals(app, req.caller).await,
        _ => internal(),
    }
}

// ---- reading a held run ----------------------------------------------------------------------

/// What a decision needs to know about a run.
#[derive(sqlx::FromRow)]
struct Held {
    workspace_id: Uuid,
    job_name: String,
    inventory_name: String,
    resolved_hosts: Vec<String>,
    commit: String,
    policy_id: Option<Uuid>,
    requested_by: Option<Uuid>,
    state: String,
    approvals_needed: Option<i32>,
    four_eyes: Option<bool>,
    expires_at: Option<i64>,
    now: i64,
    approvals: i64,
}

impl Held {
    fn state(&self) -> RunState {
        self.state.parse().unwrap_or(RunState::Failed)
    }

    /// The held terms. A run that was never held has none: it is answered as not pending.
    fn terms(&self) -> HeldTerms {
        HeldTerms {
            needed: u16::try_from(self.approvals_needed.unwrap_or(1).max(1)).unwrap_or(u16::MAX),
            four_eyes: self.four_eyes.unwrap_or(false),
            expires_at: self.expires_at.unwrap_or(i64::MAX),
        }
    }

    fn expired(&self) -> bool {
        self.state() == RunState::PendingApproval && self.now >= self.terms().expires_at
    }

    fn needed(&self) -> u32 {
        u32::from(self.terms().needed)
    }
}

/// Reads `run` as `caller` sees it; with `lock`, locks it for the decision.
async fn held(conn: &mut PgConnection, run: Uuid, lock: bool) -> Result<Option<Held>, sqlx::Error> {
    let sql = format!(
        "select r.workspace_id, r.job_name, r.inventory_name, r.resolved_hosts, r.commit, \
         r.policy_id, r.requested_by, r.state, r.approvals_needed, r.four_eyes, \
         floor(extract(epoch from r.approval_expires_at))::bigint as expires_at, \
         floor(extract(epoch from now()))::bigint as now, \
         (select count(distinct a.approver_user_id) from approvals a where a.run_id = r.id \
            and a.kind = 'approval' and a.decision = 'approve') as approvals \
         from runs r where r.id = $1{}",
        if lock { " for update of r" } else { "" }
    );
    sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(run)
        .fetch_optional(&mut *conn)
        .await
}

async fn answered(conn: &mut PgConnection, run: Uuid, user: Uuid) -> Result<bool, sqlx::Error> {
    let (n,): (i64,) = sqlx::query_as(
        "select count(*) from approvals where run_id = $1 and approver_user_id = $2 \
         and kind = 'approval'",
    )
    .bind(run)
    .bind(user)
    .fetch_one(&mut *conn)
    .await?;
    Ok(n > 0)
}

/// The caller's effective role in `ws` for the record: the instance role for owners and admins,
/// otherwise the strongest team role that grants `Approve` on the whole workspace.
fn effective_role(caller: &Caller, ws: Uuid) -> Option<&'static str> {
    match caller.user.instance_role {
        InstanceRole::Owner => return Some("owner"),
        InstanceRole::Admin => return Some("admin"),
        _ => {}
    }
    let roles = caller
        .grants
        .iter()
        .filter(|g| g.workspace == WorkspaceId(ws) && g.profiles.is_empty())
        .map(|g| g.role);
    let mut best = None;
    for r in roles {
        match r {
            TeamRole::WsAdmin => return Some("ws_admin"),
            TeamRole::Approver => best = Some("approver"),
            _ => {}
        }
    }
    best
}

// ---- refusals --------------------------------------------------------------------------------

/// Audits a refusal (`run_approval_refused`, denied) and answers it.
async fn refuse(
    app: &AppState,
    caller: &Caller,
    run: Uuid,
    ws: Uuid,
    refusal: Refusal,
    state: RunState,
) -> Vec<Event> {
    // Found overdue before the sweeper got to it: expire it now.
    if refusal == Refusal::Expired
        && state == RunState::PendingApproval
        && let Err(e) = expire_overdue(app, run).await
    {
        tracing::error!(error = %e, run = %run, "expiring a held run failed");
    }
    let result = async {
        let mut tx = app.db.begin().await?;
        audit::record(
            &mut tx,
            Entry {
                actor: &caller.actor(),
                action: "run_approval_refused",
                workspace: Some(WorkspaceId(ws)),
                target: json!({ "run": run }),
                decision: Decision::Denied,
                reason: Some(refusal.code()),
                context: json!({ "code": refusal.code(), "state": state.as_str() }),
            },
        )
        .await?;
        tx.commit().await
    }
    .await;
    if let Err(e) = result {
        tracing::error!(error = %e, "audit write failed");
    }
    op_error(refusal.code(), &refusal.message())
}

/// Expires `run` if it is still held past its window.
async fn expire_overdue(app: &AppState, run: Uuid) -> Result<(), sqlx::Error> {
    let mut tx = app.db.begin().await?;
    let Some(h) = held(&mut tx, run, true).await? else {
        return Ok(());
    };
    if !h.expired() {
        return Ok(());
    }
    let ws = h.workspace_id;
    let next = expire(&mut tx, run, ws, &h).await?;
    tx.commit().await?;
    crate::queue::push_state(app, ws, run, next);
    Ok(())
}

/// Expires a held run whose window has closed, inside the caller's transaction.
async fn expire(
    tx: &mut PgConnection,
    run: Uuid,
    ws: Uuid,
    h: &Held,
) -> Result<RunStatus, sqlx::Error> {
    let next = expired()?;
    sqlx::query("update runs set state = $2, state_reason = $3, ended_at = now() where id = $1")
        .bind(run)
        .bind(next.state.as_str())
        .bind(next.reason.map(|r| r.as_str()))
        .execute(&mut *tx)
        .await?;
    cancel_hosts(tx, run).await?;
    audit_expired(tx, run, ws, h.policy_id, h.expires_at).await?;
    Ok(next)
}

/// A held run's hosts never started: they are cancelled with it.
async fn cancel_hosts(tx: &mut PgConnection, run: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update run_hosts set state = 'cancelled', ended_at = now() \
         where run_id = $1 and state = 'queued'",
    )
    .bind(run)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

// ---- approving -------------------------------------------------------------------------------

fn pending<F, Fut>(f: F) -> PendingOp
where
    F: FnOnce(AppState, Caller) -> Fut + Send + 'static,
    Fut: Future<Output = Vec<Event>> + Send + 'static,
{
    Box::new(move |app, caller| Box::pin(f(app, caller)))
}

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(12)]
}

async fn preview_run_approve(app: &AppState, req: &Request<'_>, run: Uuid) -> Vec<Event> {
    let mut conn = match app.db.acquire().await {
        Ok(c) => c,
        Err(e) => return db_failed(&e),
    };
    let h = match held(&mut conn, run, false).await {
        Ok(Some(h)) => h,
        Ok(None) => return not_found("run"),
        Err(e) => return db_failed(&e),
    };
    let ws = h.workspace_id;
    if let Err(d) = require(app, req, APPROVE, Capability::Approve, ws).await {
        return d;
    }
    let caller = req.caller;
    let already = match answered(&mut conn, run, caller.user.id).await {
        Ok(a) => a,
        Err(e) => return db_failed(&e),
    };
    drop(conn);
    if let Err(r) = approval::check_approve(
        h.state(),
        &h.terms(),
        h.now,
        caller.user.id,
        h.requested_by,
        already,
    ) {
        return refuse(app, caller, run, ws, r, h.state()).await;
    }
    let k = u32::try_from(h.approvals)
        .unwrap_or(u32::MAX)
        .saturating_add(1);
    let effect = if approval::after_approve(&h.terms(), k).is_some() {
        "this queues the run".to_owned()
    } else {
        format!("this adds approval {k} of {}", h.needed())
    };
    let op = pending(move |app, caller| async move {
        if let Err(d) = crate::runs::recheck(&caller, APPROVE, Capability::Approve, ws) {
            return d;
        }
        match approve(&app, &caller, run).await {
            Ok(Ok(())) => reply(&app, run).await,
            Ok(Err((refusal, state))) => refuse(&app, &caller, run, ws, refusal, state).await,
            Err(e) => db_failed(&e),
        }
    });
    let hosts = h.resolved_hosts.len();
    crate::bindings::preview(
        app,
        req,
        Draft {
            action: APPROVE.to_owned(),
            summary: format!(
                "Approve run of job {} at {} on {} host(s) from inventory {}: {effect}.",
                h.job_name,
                short(&h.commit),
                hosts,
                h.inventory_name
            ),
            affected: h.resolved_hosts,
            step_up: true,
        },
        op,
    )
}

/// The run as it now is.
async fn reply(app: &AppState, run: Uuid) -> Vec<Event> {
    match crate::runs::get_run(app, run).await {
        Ok(Some(ev)) => vec![ev],
        Ok(None) => not_found("run"),
        Err(e) => db_failed(&e),
    }
}

type Outcome = Result<(), (Refusal, RunState)>;

fn unique_violation(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(d) if d.code().as_deref() == Some("23505"))
}

/// Records `caller`'s approval of `run` and queues it once it has enough.
async fn approve(app: &AppState, caller: &Caller, run: Uuid) -> Result<Outcome, sqlx::Error> {
    let mut tx = app.db.begin().await?;
    let Some(h) = held(&mut tx, run, true).await? else {
        return Ok(Err((
            Refusal::NotPending(RunState::Failed),
            RunState::Failed,
        )));
    };
    let ws = h.workspace_id;
    if h.expired() {
        let next = expire(&mut tx, run, ws, &h).await?;
        tx.commit().await?;
        crate::queue::push_state(app, ws, run, next);
        return Ok(Err((Refusal::Expired, RunState::Expired)));
    }
    let already = answered(&mut tx, run, caller.user.id).await?;
    if let Err(r) = approval::check_approve(
        h.state(),
        &h.terms(),
        h.now,
        caller.user.id,
        h.requested_by,
        already,
    ) {
        return Ok(Err((r, h.state())));
    }
    let role = effective_role(caller, ws);
    let inserted = sqlx::query(
        "insert into approvals (id, run_id, approver_user_id, kind, decision, step_up_at, \
         approver_role) values ($1, $2, $3, 'approval', 'approve', now(), $4)",
    )
    .bind(Uuid::now_v7())
    .bind(run)
    .bind(caller.user.id)
    .bind(role)
    .execute(&mut *tx)
    .await;
    match inserted {
        Ok(_) => {}
        Err(e) if unique_violation(&e) => {
            return Ok(Err((Refusal::AlreadyAnswered, h.state())));
        }
        Err(e) => return Err(e),
    }
    let count = u32::try_from(h.approvals)
        .unwrap_or(u32::MAX)
        .saturating_add(1);
    let queued = match approval::after_approve(&h.terms(), count) {
        Some(event) => match keel_core::run::transition(RunState::PendingApproval, &event) {
            Ok(next) => {
                sqlx::query(
                    "update runs set state = $2, state_reason = $3, queued_at = now() \
                     where id = $1",
                )
                .bind(run)
                .bind(next.state.as_str())
                .bind(next.reason.map(|r| r.as_str()))
                .execute(&mut *tx)
                .await?;
                Some(next)
            }
            Err(_) => None,
        },
        None => None,
    };
    let actor = caller.actor();
    audit::record(
        &mut tx,
        Entry {
            actor: &actor,
            action: "run_approved",
            workspace: Some(WorkspaceId(ws)),
            target: json!({ "run": run }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({
                "policy": h.policy_id,
                "role": role,
                "approvals": count,
                "needed": h.needed(),
            }),
        },
    )
    .await?;
    if queued.is_some() {
        audit::record(
            &mut tx,
            Entry {
                actor: &actor,
                action: "run_approval_queued",
                workspace: Some(WorkspaceId(ws)),
                target: json!({ "run": run }),
                decision: Decision::Allowed,
                reason: None,
                context: json!({ "policy": h.policy_id, "approvals": count }),
            },
        )
        .await?;
    }
    tx.commit().await?;
    if let Some(next) = queued {
        crate::queue::push_state(app, ws, run, next);
        crate::queue::kick(app);
    }
    Ok(Ok(()))
}

// ---- counting --------------------------------------------------------------------------------

/// Per workspace, the held runs `caller` could answer now: still in their window, not already
/// answered by them, and not four-eyes runs they asked for. Workspaces where they lack `Approve`
/// are skipped, as are workspaces with nothing waiting.
async fn count_pending_approvals(app: &AppState, caller: &Caller) -> Vec<Event> {
    let rows: Vec<(Uuid, i64)> = match sqlx::query_as(
        "select r.workspace_id, count(*) from runs r \
         where r.state = 'pending_approval' and r.approval_expires_at > now() \
           and not (r.four_eyes and r.requested_by is not distinct from $1) \
           and not exists (select 1 from approvals a where a.run_id = r.id \
                           and a.approver_user_id = $1 and a.kind = 'approval') \
         group by r.workspace_id order by r.workspace_id",
    )
    .bind(caller.user.id)
    .fetch_all(&app.db)
    .await
    {
        Ok(r) => r,
        Err(e) => return db_failed(&e),
    };
    let me = subject(caller);
    let counts = rows
        .into_iter()
        .filter(|(ws, _)| {
            caller.token.as_ref().is_none_or(|t| t.workspace.0 == *ws)
                && authz::require(&me, Capability::Approve, WorkspaceId(*ws), &[]).is_ok()
        })
        .map(|(workspace, n)| WorkspaceCount {
            workspace,
            count: u32::try_from(n).unwrap_or(u32::MAX),
        })
        .collect();
    vec![Event::PendingApprovals { counts }]
}

// ---- expiring --------------------------------------------------------------------------------

/// Sweeps once now, then every `period`, so runs whose window closed while Keel was down expire
/// on start-up rather than a period later.
pub fn spawn_approval_sweeper(app: AppState, period: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(period);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            if let Err(e) = sweep(&app).await {
                tracing::error!(error = %e, "expiring held runs failed");
            }
        }
    })
}

/// Expires every held run whose approval window has closed; returns how many.
///
/// # Errors
///
/// The database failed; nothing is expired then.
pub async fn sweep(app: &AppState) -> Result<usize, sqlx::Error> {
    let next = expired()?;
    let mut tx = app.db.begin().await?;
    let expired: Vec<(Uuid, Uuid, Option<Uuid>, Option<i64>)> = sqlx::query_as(
        "update runs set state = $1, state_reason = $2, ended_at = now() \
         where state = 'pending_approval' and approval_expires_at <= now() \
         returning id, workspace_id, policy_id, \
           floor(extract(epoch from approval_expires_at))::bigint",
    )
    .bind(next.state.as_str())
    .bind(next.reason.map(|r| r.as_str()))
    .fetch_all(&mut *tx)
    .await?;
    for (run, ws, policy, expires_at) in &expired {
        cancel_hosts(&mut tx, *run).await?;
        audit_expired(&mut tx, *run, *ws, *policy, *expires_at).await?;
    }
    tx.commit().await?;
    for (run, ws, ..) in &expired {
        crate::queue::push_state(app, *ws, *run, next);
    }
    Ok(expired.len())
}

/// What a held run becomes when its window closes.
fn expired() -> Result<RunStatus, sqlx::Error> {
    keel_core::run::transition(RunState::PendingApproval, &RunEvent::Expired)
        .map_err(|e| sqlx::Error::Protocol(format!("expire: {e:?}")))
}

async fn audit_expired(
    tx: &mut PgConnection,
    run: Uuid,
    ws: Uuid,
    policy: Option<Uuid>,
    expires_at: Option<i64>,
) -> Result<(), sqlx::Error> {
    audit::record(
        tx,
        Entry {
            actor: &audit::Actor::System,
            action: "run_approval_expired",
            workspace: Some(WorkspaceId(ws)),
            target: json!({ "run": run }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({ "policy": policy, "expires_at": expires_at }),
        },
    )
    .await
    .map(|_| ())
}

// ---- denying ---------------------------------------------------------------------------------

async fn preview_run_deny(
    app: &AppState,
    req: &Request<'_>,
    run: Uuid,
    reason: Option<String>,
) -> Vec<Event> {
    let mut conn = match app.db.acquire().await {
        Ok(c) => c,
        Err(e) => return db_failed(&e),
    };
    let h = match held(&mut conn, run, false).await {
        Ok(Some(h)) => h,
        Ok(None) => return not_found("run"),
        Err(e) => return db_failed(&e),
    };
    let ws = h.workspace_id;
    if let Err(d) = require(app, req, DENY, Capability::Approve, ws).await {
        return d;
    }
    let caller = req.caller;
    let already = match answered(&mut conn, run, caller.user.id).await {
        Ok(a) => a,
        Err(e) => return db_failed(&e),
    };
    drop(conn);
    if let Err(r) = approval::check_deny(h.state(), &h.terms(), h.now, already) {
        return refuse(app, caller, run, ws, r, h.state()).await;
    }
    let op = pending(move |app, caller| async move {
        if let Err(d) = crate::runs::recheck(&caller, DENY, Capability::Approve, ws) {
            return d;
        }
        match deny(&app, &caller, run, reason.as_deref()).await {
            Ok(Ok(())) => reply(&app, run).await,
            Ok(Err((refusal, state))) => refuse(&app, &caller, run, ws, refusal, state).await,
            Err(e) => db_failed(&e),
        }
    });
    let hosts = h.resolved_hosts.len();
    crate::bindings::preview(
        app,
        req,
        Draft {
            action: DENY.to_owned(),
            summary: format!(
                "Deny run of job {} at {} on {} host(s) from inventory {}: it will not run.",
                h.job_name,
                short(&h.commit),
                hosts,
                h.inventory_name
            ),
            affected: h.resolved_hosts,
            step_up: true,
        },
        op,
    )
}

/// Records `caller`'s denial of `run`, which ends it.
async fn deny(
    app: &AppState,
    caller: &Caller,
    run: Uuid,
    reason: Option<&str>,
) -> Result<Outcome, sqlx::Error> {
    let mut tx = app.db.begin().await?;
    let Some(h) = held(&mut tx, run, true).await? else {
        return Ok(Err((
            Refusal::NotPending(RunState::Failed),
            RunState::Failed,
        )));
    };
    let ws = h.workspace_id;
    if h.expired() {
        let next = expire(&mut tx, run, ws, &h).await?;
        tx.commit().await?;
        crate::queue::push_state(app, ws, run, next);
        return Ok(Err((Refusal::Expired, RunState::Expired)));
    }
    let already = answered(&mut tx, run, caller.user.id).await?;
    if let Err(r) = approval::check_deny(h.state(), &h.terms(), h.now, already) {
        return Ok(Err((r, h.state())));
    }
    let role = effective_role(caller, ws);
    let inserted = sqlx::query(
        "insert into approvals (id, run_id, approver_user_id, kind, decision, comment, \
         step_up_at, approver_role) values ($1, $2, $3, 'approval', 'deny', $4, now(), $5)",
    )
    .bind(Uuid::now_v7())
    .bind(run)
    .bind(caller.user.id)
    .bind(reason)
    .bind(role)
    .execute(&mut *tx)
    .await;
    match inserted {
        Ok(_) => {}
        Err(e) if unique_violation(&e) => {
            return Ok(Err((Refusal::AlreadyAnswered, h.state())));
        }
        Err(e) => return Err(e),
    }
    let next = keel_core::run::transition(RunState::PendingApproval, &RunEvent::Denied)
        .map_err(|e| sqlx::Error::Protocol(format!("deny: {e:?}")))?;
    sqlx::query("update runs set state = $2, state_reason = $3, ended_at = now() where id = $1")
        .bind(run)
        .bind(next.state.as_str())
        .bind(next.reason.map(|r| r.as_str()))
        .execute(&mut *tx)
        .await?;
    cancel_hosts(&mut tx, run).await?;
    audit::record(
        &mut tx,
        Entry {
            actor: &caller.actor(),
            action: "run_denied",
            workspace: Some(WorkspaceId(ws)),
            target: json!({ "run": run }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({
                "policy": h.policy_id,
                "role": role,
                "approvals": h.approvals,
                "needed": h.needed(),
                "reason": reason,
            }),
        },
    )
    .await?;
    tx.commit().await?;
    crate::queue::push_state(app, ws, run, next);
    Ok(Ok(()))
}
