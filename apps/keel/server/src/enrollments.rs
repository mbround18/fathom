//! Every enrolment token, for instance admins: list, revoke and roll (spec 060 US3, T037;
//! research R4).
//!
//! - The state is derived, in order: `used` (a host enrolled with it), `revoked`, `burned` (any
//!   other refusal), `expired`, `pending`.
//! - `PreviewEnrollmentRevoke` (step-up): pending tokens only. It spends the token the way a
//!   refusal does (`used_at`, `burned_reason = 'revoked'`), so the runner gets the same answer
//!   as for a burned token. Audited as `host_enrollment_revoked`.
//! - `PreviewEnrollmentRoll` (step-up): any token not used. One transaction revokes the old one
//!   if still pending and mints a new one with `rolled_from`; the host name must still be free.
//!   Replies `HostEnrollment`, with the snippet.
//! - Nothing here reads, returns or audits a token or its hash.

use keel_protocol::{EnrollmentInfo, EnrollmentState, Event};
use serde_json::{Value, json};
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::bindings::{enum_from, enum_str};
use crate::dispatch::Request;
use crate::governance::{self, Fail, check_admin, deny, origin, ts};
use crate::hosts::{self, NewEnrollment};
use crate::identity::Caller;
use crate::preview::Draft;
use crate::serve::AppState;

/// The most enrolments `ListEnrollments` returns.
const LIST_LIMIT: i64 = 1000;

/// The derived state, as SQL over `host_enrollments e`.
const STATE_SQL: &str = "case when e.host_id is not null then 'used' \
     when e.burned_reason = 'revoked' then 'revoked' \
     when e.used_at is not null then 'burned' \
     when e.expires_at <= now() then 'expired' \
     else 'pending' end";

type Row = (
    Uuid,
    Uuid,
    String,
    String,
    Vec<String>,
    String,
    Option<String>,
    OffsetDateTime,
    OffsetDateTime,
    Option<Uuid>,
    Option<Uuid>,
);

fn info(r: Row) -> EnrollmentInfo {
    EnrollmentInfo {
        id: r.0,
        workspace: r.1,
        workspace_slug: r.2,
        inventory_name: r.3,
        profiles: r.4,
        // An unknown state reads as burned: fail closed.
        state: enum_from(&r.5).unwrap_or(EnrollmentState::Burned),
        created_by: r.6,
        created_at: ts(r.7),
        expires_at: ts(r.8),
        host: r.9,
        rolled_from: r.10,
    }
}

/// `$1` state, `$2` workspace, `$3` one enrolment, `$4` limit. The filters are one `where`
/// clause so a visibility rule can join it.
async fn rows(
    conn: &mut PgConnection,
    state: Option<EnrollmentState>,
    workspace: Option<Uuid>,
    enrollment: Option<Uuid>,
) -> Result<Vec<EnrollmentInfo>, sqlx::Error> {
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select * from (select e.id, e.workspace_id, w.slug, e.inventory_name, e.profiles, \
         {STATE_SQL} as state, u.display_name, e.created_at, e.expires_at, e.host_id, \
         e.rolled_from from host_enrollments e join workspaces w on w.id = e.workspace_id \
         left join users u on u.id = e.created_by \
         where ($2::uuid is null or e.workspace_id = $2) \
         and ($3::uuid is null or e.id = $3)) s \
         where ($1::text is null or s.state = $1) \
         order by s.created_at desc, s.id desc limit $4"
    )))
    .bind(state.as_ref().map(enum_str))
    .bind(workspace)
    .bind(enrollment)
    .bind(LIST_LIMIT)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(info).collect())
}

async fn enrollments_event(
    app: &AppState,
    state: Option<EnrollmentState>,
    workspace: Option<Uuid>,
) -> Vec<Event> {
    let result = async {
        let mut conn = app.db.acquire().await?;
        rows(&mut conn, state, workspace, None).await
    }
    .await;
    match result {
        Ok(items) => vec![Event::Enrollments { items }],
        Err(e) => {
            tracing::error!(error = %e, "list enrollments failed");
            vec![governance::internal()]
        }
    }
}

/// `ListEnrollments`: instance admins.
pub async fn list(
    app: &AppState,
    req: &Request<'_>,
    state: Option<EnrollmentState>,
    workspace: Option<Uuid>,
) -> Vec<Event> {
    const CMD: &str = "list_enrollments";
    if let Err(code) = check_admin(req.caller) {
        return deny(app, req.caller, CMD, workspace, code, origin(req).context).await;
    }
    enrollments_event(app, state, workspace).await
}

/// One enrolment, by id.
async fn one(app: &AppState, enrollment: Uuid) -> Result<Option<EnrollmentInfo>, Vec<Event>> {
    let result = async {
        let mut conn = app.db.acquire().await?;
        rows(&mut conn, None, None, Some(enrollment)).await
    }
    .await;
    match result {
        Ok(mut items) => Ok(items.pop()),
        Err(e) => {
            tracing::error!(error = %e, "enrollment lookup failed");
            Err(vec![governance::internal()])
        }
    }
}

fn used() -> Fail {
    Fail::Op(
        "enrollment_used",
        "A host has already enrolled with that token.".into(),
    )
}

fn not_pending(state: EnrollmentState) -> Fail {
    Fail::Op(
        "enrollment_not_pending",
        format!(
            "That token is {}, so there is nothing to revoke.",
            enum_str(&state)
        ),
    )
}

fn fail_event(f: Fail) -> Vec<Event> {
    match f {
        Fail::Op(code, message) => vec![Event::op_error(code, message)],
        Fail::Denied(code) => vec![Event::Denied {
            action: "enrollment".to_owned(),
            reason: code.to_owned(),
        }],
    }
}

/// Locks the row in the change's transaction: its state, and who minted it.
async fn lock(
    conn: &mut PgConnection,
    enrollment: Uuid,
) -> Result<Option<(EnrollmentState, Option<Uuid>)>, sqlx::Error> {
    let row: Option<(String, Option<Uuid>)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {STATE_SQL}, e.created_by from host_enrollments e where e.id = $1 for update"
    )))
    .bind(enrollment)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|(s, by)| (enum_from(&s).unwrap_or(EnrollmentState::Burned), by)))
}

fn gone() -> Fail {
    Fail::Op("not_found", "That enrolment token does not exist.".into())
}

/// Spends a pending token as revoked and audits it.
async fn revoke_in(
    conn: &mut PgConnection,
    caller: &Caller,
    e: &EnrollmentInfo,
    context: &Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update host_enrollments set used_at = now(), burned_reason = 'revoked' \
         where id = $1 and used_at is null",
    )
    .bind(e.id)
    .execute(&mut *conn)
    .await?;
    governance::record(
        conn,
        caller,
        "host_enrollment_revoked",
        Some(e.workspace),
        json!({ "enrollment": e.id, "inventory_name": e.inventory_name }),
        context,
    )
    .await
}

/// The audit context of a revoke: the workspace, and whether the token was someone else's.
fn revoke_context(
    o: &governance::Origin,
    caller: &Caller,
    e: &EnrollmentInfo,
    minted_by: Option<Uuid>,
) -> Value {
    let mut context = o.context.clone();
    context["workspace"] = json!(e.workspace_slug);
    context["by_admin"] = json!(minted_by != Some(caller.user.id));
    context
}

/// `PreviewEnrollmentRevoke` (step-up). `Confirm` replies `Enrollments`.
pub async fn preview_revoke(app: &AppState, req: &Request<'_>, enrollment: Uuid) -> Vec<Event> {
    const CMD: &str = "preview_enrollment_revoke";
    const ACTION: &str = "host_enrollment_revoked";
    let o = origin(req);
    if let Err(code) = check_admin(req.caller) {
        return deny(app, req.caller, CMD, None, code, o.context).await;
    }
    let e = match one(app, enrollment).await {
        Ok(Some(e)) => e,
        Ok(None) => return vec![governance::not_found("enrolment token")],
        Err(ev) => return ev,
    };
    match e.state {
        EnrollmentState::Pending => {}
        EnrollmentState::Used => return fail_event(used()),
        other => return fail_event(not_pending(other)),
    }
    let summary = format!(
        "Revoke the enrolment token for {} in {}. A runner using it is refused.",
        e.inventory_name, e.workspace_slug
    );
    let affected = vec![e.inventory_name.clone()];
    let op = governance::pending(move |app, caller| async move {
        let ws = Some(e.workspace);
        let step: governance::Step = async {
            if let Err(code) = check_admin(&caller) {
                return Ok(Err(Fail::Denied(code)));
            }
            let mut tx = app.db.begin().await?;
            let minted_by = match lock(&mut tx, enrollment).await? {
                Some((EnrollmentState::Pending, by)) => by,
                Some((EnrollmentState::Used, _)) => return Ok(Err(used())),
                Some((other, _)) => return Ok(Err(not_pending(other))),
                None => return Ok(Err(gone())),
            };
            let context = revoke_context(&o, &caller, &e, minted_by);
            revoke_in(&mut tx, &caller, &e, &context).await?;
            tx.commit().await?;
            Ok(Ok(vec![]))
        }
        .await;
        if let Err(events) = governance::settle(&app, &caller, ACTION, ws, &o, step).await {
            return events;
        }
        enrollments_event(&app, None, None).await
    });
    let draft = Draft {
        action: ACTION.to_owned(),
        summary,
        affected,
        step_up: true,
    };
    governance::preview(app, req, draft, op)
}

/// `PreviewEnrollmentRoll` (step-up). `Confirm` replies `HostEnrollment` with the new token.
pub async fn preview_roll(app: &AppState, req: &Request<'_>, enrollment: Uuid) -> Vec<Event> {
    const CMD: &str = "preview_enrollment_roll";
    const ACTION: &str = "host_enrollment_roll";
    let o = origin(req);
    if let Err(code) = check_admin(req.caller) {
        return deny(app, req.caller, CMD, None, code, o.context).await;
    }
    let e = match one(app, enrollment).await {
        Ok(Some(e)) => e,
        Ok(None) => return vec![governance::not_found("enrolment token")],
        Err(ev) => return ev,
    };
    if e.state == EnrollmentState::Used {
        return fail_event(used());
    }
    if app.repos.master_key.is_none() || app.ssh.get().is_none() {
        return vec![Event::op_error(
            "ssh_ca_unavailable",
            "The SSH certificate authorities are not loaded.",
        )];
    }
    if let Err(ev) = hosts::check_enroll(app, e.workspace, &e.inventory_name, &e.profiles).await {
        return ev;
    }
    let summary = format!(
        "Replace the enrolment token for {} in {}{}. The new token is shown once and expires in \
         15 minutes.",
        e.inventory_name,
        e.workspace_slug,
        if e.state == EnrollmentState::Pending {
            "; the current one is revoked"
        } else {
            ""
        }
    );
    let affected = vec![e.inventory_name.clone()];
    let op = governance::pending(move |app, caller| async move {
        let ws = Some(e.workspace);
        if let Err(code) = check_admin(&caller) {
            return deny(&app, &caller, ACTION, ws, code, o.context.clone()).await;
        }
        if let Err(ev) =
            hosts::check_enroll(&app, e.workspace, &e.inventory_name, &e.profiles).await
        {
            return ev;
        }
        let new = match NewEnrollment::mint(&app) {
            Ok(n) => n,
            Err(ev) => return ev,
        };
        let step: Result<Result<String, Fail>, sqlx::Error> = async {
            let mut tx = app.db.begin().await?;
            match lock(&mut tx, enrollment).await? {
                Some((EnrollmentState::Used, _)) => return Ok(Err(used())),
                Some((EnrollmentState::Pending, by)) => {
                    let mut context = revoke_context(&o, &caller, &e, by);
                    context["rolled_to"] = json!(new.id);
                    revoke_in(&mut tx, &caller, &e, &context).await?;
                }
                Some(_) => {}
                None => return Ok(Err(gone())),
            }
            let slug = hosts::insert_enrollment(
                &mut tx,
                &caller,
                e.workspace,
                &e.inventory_name,
                &e.profiles,
                &new,
                Some(enrollment),
            )
            .await?;
            tx.commit().await?;
            Ok(Ok(slug))
        }
        .await;
        match step {
            Ok(Ok(slug)) => {
                vec![
                    hosts::enrollment_event(
                        &app,
                        new,
                        e.workspace,
                        &slug,
                        e.inventory_name,
                        e.profiles,
                    )
                    .await,
                ]
            }
            Ok(Err(f)) => fail_event(f),
            Err(err) => {
                tracing::error!(error = %err, "enrollment roll failed");
                vec![governance::internal()]
            }
        }
    });
    let draft = Draft {
        action: ACTION.to_owned(),
        summary,
        affected,
        step_up: true,
    };
    governance::preview(app, req, draft, op)
}
