//! The owner-only System workspace (spec 060 US7, contracts/system-workspace.md, research R8).
//!
//! - `GetSystem` reports the configuration; with no `system_config` row there is no System
//!   workspace as far as anyone can see (FR-039).
//! - `PreviewSystemConfigure` creates the workspace `keel-system` (or un-archives it), and writes
//!   `system_config` under an optimistic `version`.
//! - `PreviewSystemRemove`, in one transaction, revokes the workspace's hosts, burns its pending
//!   enrolment tokens, disables its bindings, archives it and deletes the configuration. The
//!   workspace and its history stay (FR-038).
//!
//! Every command is for instance owners, from a session (dispatch refuses tokens), and both
//! changes need step-up. The access rule itself lives in `authz::require`
//! ([`keel_core::authz::Refusal::OwnerOnly`]); [`AppState::system_workspace`] feeds it and is
//! updated here after each commit.

use keel_core::InstanceRole;
use keel_protocol::{Cmd, Event, HostStatus};
use serde_json::{Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::dispatch::Request;
use crate::governance::{Fail, Step, deny, internal, origin, pending, preview, record, settle};
use crate::identity::Caller;
use crate::preview::Draft;
use crate::serve::AppState;

/// The System workspace's reserved slug; workspace create refuses it.
pub const SLUG: &str = "keel-system";
/// Its display name.
const NAME: &str = "System";
/// The most profiles `system_config.allowed_profiles` holds.
const MAX_PROFILES: usize = 16;

/// Routes `GetSystem`, `PreviewSystemConfigure` and `PreviewSystemRemove`.
pub async fn handle(app: &AppState, req: &Request<'_>, cmd: Cmd) -> Vec<Event> {
    match cmd {
        Cmd::GetSystem => get(app, req).await,
        Cmd::PreviewSystemConfigure {
            allowed_profiles,
            version,
        } => {
            let profiles: Vec<String> = allowed_profiles
                .0
                .iter()
                .map(|p| p.as_str().to_owned())
                .collect();
            configure(app, req, profiles, version).await
        }
        Cmd::PreviewSystemRemove => remove(app, req).await,
        _ => vec![internal()],
    }
}

/// An instance owner, from a session.
fn is_owner(c: &Caller) -> bool {
    c.token.is_none() && c.user.instance_role == InstanceRole::Owner
}

/// The System workspace to leave out of a cross-workspace list for `caller`: `Some(id)` for
/// anyone but an instance owner while one is configured, `None` otherwise.
///
/// In SQL: `and ($n::uuid is null or workspace_id is distinct from $n)`.
#[must_use]
pub fn hide_system(caller: &Caller) -> Option<Uuid> {
    if caller.token.is_none() && caller.user.instance_role == InstanceRole::Owner {
        None
    } else {
        caller.system_workspace.map(|w| w.0)
    }
}

/// `system_config` as stored: workspace, allowed profiles, version.
type ConfigRow = (Uuid, Vec<String>, i64);

async fn config(conn: &mut PgConnection, lock: bool) -> Result<Option<ConfigRow>, sqlx::Error> {
    let sql = if lock {
        "select workspace_id, allowed_profiles, version from system_config where id for update"
    } else {
        "select workspace_id, allowed_profiles, version from system_config where id"
    };
    sqlx::query_as(sql).fetch_optional(conn).await
}

/// The `System` event for the current configuration.
async fn system_event(app: &AppState) -> Result<Event, sqlx::Error> {
    let mut conn = app.db.acquire().await?;
    let Some((ws, allowed_profiles, version)) = config(&mut conn, false).await? else {
        return Ok(Event::System {
            configured: false,
            workspace: None,
            allowed_profiles: Vec::new(),
            version: 0,
            bindings: Vec::new(),
            hosts: Vec::new(),
        });
    };
    let bindings: Vec<(Uuid,)> = sqlx::query_as(
        "select id from repo_bindings \
             where project_id in (select id from projects where workspace_id = $1) order by id",
    )
    .bind(ws)
    .fetch_all(&mut *conn)
    .await?;
    let hosts: Vec<(Uuid,)> = sqlx::query_as(
        "select id from hosts where workspace_id = $1 and state <> 'revoked' order by id",
    )
    .bind(ws)
    .fetch_all(&mut *conn)
    .await?;
    Ok(Event::System {
        configured: true,
        workspace: Some(ws),
        allowed_profiles,
        version,
        bindings: bindings.into_iter().map(|r| r.0).collect(),
        hosts: hosts.into_iter().map(|r| r.0).collect(),
    })
}

async fn reply(app: &AppState) -> Vec<Event> {
    match system_event(app).await {
        Ok(e) => vec![e],
        Err(e) => {
            tracing::error!(error = %e, "reading the System configuration failed");
            vec![internal()]
        }
    }
}

async fn get(app: &AppState, req: &Request<'_>) -> Vec<Event> {
    if !is_owner(req.caller) {
        let o = origin(req);
        return deny(app, req.caller, "get_system", None, "owner_only", o.context).await;
    }
    reply(app).await
}

const CONFLICT: &str =
    "Someone changed the System configuration since you loaded it. Reload and try again.";

fn conflict() -> Fail {
    Fail::Op("system_conflict", CONFLICT.into())
}

async fn configure(
    app: &AppState,
    req: &Request<'_>,
    profiles: Vec<String>,
    version: i64,
) -> Vec<Event> {
    const CMD: &str = "preview_system_configure";
    const ACTION: &str = "system_configure";
    let o = origin(req);
    if !is_owner(req.caller) {
        return deny(app, req.caller, CMD, None, "owner_only", o.context).await;
    }
    if profiles.is_empty() || profiles.len() > MAX_PROFILES {
        return vec![Event::op_error(
            "invalid_profiles",
            format!("Allow 1 to {MAX_PROFILES} profiles."),
        )];
    }
    let current = async {
        let mut conn = app.db.acquire().await?;
        config(&mut conn, false).await
    }
    .await;
    let current = match current {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "reading the System configuration failed");
            return vec![internal()];
        }
    };
    if current.as_ref().map_or(0, |c| c.2) != version {
        return vec![Event::op_error("system_conflict", CONFLICT)];
    }
    let summary = if current.is_some() {
        format!(
            "Change the profiles the System workspace may run to {}.",
            profiles.join(", ")
        )
    } else {
        format!(
            "Create the owner-only System workspace ({SLUG}), allowing the profiles {}. Only \
             instance owners can see or use it.",
            profiles.join(", ")
        )
    };
    let op = pending(move |app, caller| async move {
        let mut configured = None;
        let step: Step = async {
            if !is_owner(&caller) {
                return Ok(Err(Fail::Denied("owner_only")));
            }
            let mut tx = app.db.begin().await?;
            let before = config(&mut tx, true).await?;
            if before.as_ref().map_or(0, |c| c.2) != version {
                return Ok(Err(conflict()));
            }
            let existing: Option<(Uuid,)> =
                sqlx::query_as("select id from workspaces where system for update")
                    .fetch_optional(&mut *tx)
                    .await?;
            let ws = match existing {
                Some((id,)) => {
                    sqlx::query("update workspaces set archived_at = null where id = $1")
                        .bind(id)
                        .execute(&mut *tx)
                        .await?;
                    id
                }
                None => {
                    let id = Uuid::now_v7();
                    let r = sqlx::query(
                        "insert into workspaces (id, slug, name, system) values ($1, $2, $3, true)",
                    )
                    .bind(id)
                    .bind(SLUG)
                    .bind(NAME)
                    .execute(&mut *tx)
                    .await;
                    match r {
                        // Another configure won the race, or the slug predates the reservation.
                        Err(e) if unique_violation(&e) => return Ok(Err(conflict())),
                        other => other?,
                    };
                    id
                }
            };
            let n = if version == 0 {
                sqlx::query(
                    "insert into system_config (id, workspace_id, allowed_profiles, updated_by) \
                     values (true, $1, $2, $3) on conflict (id) do nothing",
                )
                .bind(ws)
                .bind(&profiles)
                .bind(caller.user.id)
                .execute(&mut *tx)
                .await?
                .rows_affected()
            } else {
                sqlx::query(
                    "update system_config set workspace_id = $1, allowed_profiles = $2, \
                     updated_by = $3, updated_at = now(), version = version + 1 \
                     where id and version = $4",
                )
                .bind(ws)
                .bind(&profiles)
                .bind(caller.user.id)
                .bind(version)
                .execute(&mut *tx)
                .await?
                .rows_affected()
            };
            if n != 1 {
                return Ok(Err(conflict()));
            }
            let before: Value = before.map_or(Value::Null, |c| json!(c.1));
            let context = merge(
                &o.context,
                json!({ "allowed_profiles": profiles, "before": before }),
            );
            record(
                &mut tx,
                &caller,
                ACTION,
                Some(ws),
                json!({ "workspace": ws }),
                &context,
            )
            .await?;
            tx.commit().await?;
            configured = Some(ws);
            Ok(Ok(vec![]))
        }
        .await;
        if let Err(events) = settle(&app, &caller, ACTION, None, &o, step).await {
            return events;
        }
        app.set_system_workspace(configured);
        reply(&app).await
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary,
            affected: vec![],
            step_up: true,
        },
        op,
    )
}

async fn remove(app: &AppState, req: &Request<'_>) -> Vec<Event> {
    const CMD: &str = "preview_system_remove";
    const ACTION: &str = "system_remove";
    let o = origin(req);
    if !is_owner(req.caller) {
        return deny(app, req.caller, CMD, None, "owner_only", o.context).await;
    }
    let found: Result<Option<(Uuid, Vec<String>)>, sqlx::Error> = async {
        let mut conn = app.db.acquire().await?;
        let Some((ws, _, _)) = config(&mut conn, false).await? else {
            return Ok(None);
        };
        let hosts: Vec<(String,)> = sqlx::query_as(
            "select inventory_name from hosts where workspace_id = $1 and state <> 'revoked' \
             order by inventory_name",
        )
        .bind(ws)
        .fetch_all(&mut *conn)
        .await?;
        let bindings: Vec<(String, String)> = sqlx::query_as(
            "select url, branch from repo_bindings \
             where project_id in (select id from projects where workspace_id = $1) and status <> 'disabled' \
             order by url",
        )
        .bind(ws)
        .fetch_all(&mut *conn)
        .await?;
        let affected = hosts
            .into_iter()
            .map(|h| h.0)
            .chain(bindings.into_iter().map(|(u, b)| format!("{u}@{b}")))
            .collect();
        Ok(Some((ws, affected)))
    }
    .await;
    let affected = match found {
        Ok(Some((_, a))) => a,
        Ok(None) => return vec![not_configured()],
        Err(e) => {
            tracing::error!(error = %e, "reading the System configuration failed");
            return vec![internal()];
        }
    };
    let summary = format!(
        "Remove the System configuration: revoke the Keel host, disable the System workspace's \
         bindings and archive {SLUG}. Its runs and audit history are kept."
    );
    let op = pending(move |app, caller| async move {
        let mut done: Option<(Uuid, Vec<Uuid>)> = None;
        let step: Step = async {
            if !is_owner(&caller) {
                return Ok(Err(Fail::Denied("owner_only")));
            }
            let mut tx = app.db.begin().await?;
            let Some((ws, _, _)) = config(&mut tx, true).await? else {
                return Ok(Err(Fail::Op(
                    "not_configured",
                    "There is no System configuration to remove.".into(),
                )));
            };
            let hosts: Vec<(Uuid,)> = sqlx::query_as(
                "update hosts set state = 'revoked', revoked_at = now() \
                 where workspace_id = $1 and state <> 'revoked' returning id",
            )
            .bind(ws)
            .fetch_all(&mut *tx)
            .await?;
            let hosts: Vec<Uuid> = hosts.into_iter().map(|h| h.0).collect();
            let serials = sqlx::query(
                "update ssh_certs set revoked_at = now() \
                 where host_id = any($1) and revoked_at is null",
            )
            .bind(&hosts)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            let burned = sqlx::query(
                "update host_enrollments set used_at = now(), burned_reason = 'system_removed' \
                 where workspace_id = $1 and used_at is null",
            )
            .bind(ws)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            let bindings: Vec<(Uuid,)> = sqlx::query_as(
                "update repo_bindings set status = 'disabled' \
                 where project_id in (select id from projects where workspace_id = $1) \
                 and status <> 'disabled' returning id",
            )
            .bind(ws)
            .fetch_all(&mut *tx)
            .await?;
            sqlx::query(
                "update workspaces set archived_at = coalesce(archived_at, now()) where id = $1",
            )
            .bind(ws)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "update api_tokens set revoked_at = now() \
                 where workspace_id = $1 and revoked_at is null",
            )
            .bind(ws)
            .execute(&mut *tx)
            .await?;
            sqlx::query("delete from system_config where id")
                .execute(&mut *tx)
                .await?;
            let context = merge(
                &o.context,
                json!({
                    "hosts_revoked": hosts.len(),
                    "bindings_disabled": bindings.len(),
                    "certs_revoked": serials,
                    "enrollments_burned": burned,
                }),
            );
            record(
                &mut tx,
                &caller,
                ACTION,
                Some(ws),
                json!({ "workspace": ws, "hosts": hosts }),
                &context,
            )
            .await?;
            tx.commit().await?;
            done = Some((ws, hosts));
            Ok(Ok(vec![]))
        }
        .await;
        if let Err(events) = settle(&app, &caller, ACTION, None, &o, step).await {
            return events;
        }
        app.set_system_workspace(None);
        if let Some((ws, hosts)) = done {
            if !hosts.is_empty() {
                crate::hosts::publish_krl(&app).await;
            }
            for host in hosts {
                app.runs.close(host);
                app.repos.push(
                    ws,
                    Event::HostState {
                        host,
                        state: HostStatus::Revoked,
                    },
                );
            }
            app.repos.poll_wake.notify_one();
        }
        reply(&app).await
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary,
            affected,
            step_up: true,
        },
        op,
    )
}

fn not_configured() -> Event {
    Event::op_error(
        "not_configured",
        "There is no System configuration to remove.",
    )
}

fn unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .and_then(|d| d.code())
        .is_some_and(|c| c == "23505")
}

/// `base` (the request's audit context) with `extra`'s keys added.
fn merge(base: &Value, extra: Value) -> Value {
    let mut out = base.clone();
    if let (Some(o), Value::Object(e)) = (out.as_object_mut(), extra) {
        o.extend(e);
    }
    out
}

/// Whether `profiles` are within the System workspace's allowed profiles. `Ok(None)` when
/// `workspace` isn't the System workspace (or none is configured).
///
/// # Errors
///
/// A database error.
pub async fn disallowed_profile(
    conn: &mut PgConnection,
    workspace: Uuid,
    profiles: &[String],
) -> Result<Option<String>, sqlx::Error> {
    let allowed: Option<(Vec<String>,)> =
        sqlx::query_as("select allowed_profiles from system_config where id and workspace_id = $1")
            .bind(workspace)
            .fetch_optional(conn)
            .await?;
    Ok(allowed.and_then(|(allowed,)| profiles.iter().find(|p| !allowed.contains(p)).cloned()))
}
