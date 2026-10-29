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

// ---- the update run's prerequisites (spec 061 FR-016, T070) ----------------------------------

/// The profile Keel's own host serves, and the job that deploys Keel.
pub const KEEL_SERVER: &str = "keel-server";
/// The job param the update run fills with the new digest.
pub const DIGEST_PARAM: &str = "keel_server_image_digest";

/// Where an update run goes: the System workspace's `keel-server` job.
#[derive(Debug, Clone)]
pub(crate) struct UpdateTarget {
    pub workspace: Uuid,
    pub binding: Uuid,
    pub job: Uuid,
    pub job_name: String,
    pub def: keel_core::manifest::JobDefinition,
    /// The binding's last synced commit.
    pub commit: String,
    pub signed_mode: bool,
}

/// The update run's target, or the first missing prerequisite, in the order of FR-016:
/// `no_system_workspace`, `no_keel_host`, `profile_not_allowed`, `job_missing_param`. A job that
/// is blocked, unsynced or on a disabled binding counts as `job_missing_param`: there is no job
/// to run.
///
/// # Errors
///
/// A database error.
pub(crate) async fn update_target(
    conn: &mut PgConnection,
) -> Result<Result<UpdateTarget, &'static str>, sqlx::Error> {
    let Some((ws, allowed, _)) = config(&mut *conn, false).await? else {
        return Ok(Err("no_system_workspace"));
    };
    let hosts: Vec<(String,)> = sqlx::query_as(
        "select h.inventory_name from hosts h \
         join host_profiles hp on hp.host_id = h.id \
         join profiles p on p.id = hp.profile_id \
         where h.workspace_id = $1 and h.state <> 'revoked' and p.name = $2 \
         order by h.inventory_name",
    )
    .bind(ws)
    .bind(KEEL_SERVER)
    .fetch_all(&mut *conn)
    .await?;
    if hosts.is_empty() {
        return Ok(Err("no_keel_host"));
    }
    if !allowed.iter().any(|p| p == KEEL_SERVER) {
        return Ok(Err("profile_not_allowed"));
    }
    let job: Option<(Uuid, Uuid, Value, Option<String>, bool, String)> = sqlx::query_as(
        "select j.id, j.binding_id, j.definition, b.last_commit, b.signed_mode, b.status \
         from jobs j join repo_bindings b on b.id = j.binding_id \
         join projects p on p.id = b.project_id \
         where p.workspace_id = $1 and j.name = $2 and j.removed_at_commit is null \
           and j.blocked_reason is null \
         order by j.id limit 1",
    )
    .bind(ws)
    .bind(KEEL_SERVER)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((job, binding, def, Some(commit), signed_mode, status)) = job else {
        return Ok(Err("job_missing_param"));
    };
    let Ok(def) = serde_json::from_value::<keel_core::manifest::JobDefinition>(def) else {
        return Ok(Err("job_missing_param"));
    };
    let declared = def
        .params
        .get(DIGEST_PARAM)
        .is_some_and(|p| p.kind == keel_core::manifest::ParamKind::OciDigest);
    if !declared || status != "active" {
        return Ok(Err("job_missing_param"));
    }
    Ok(Ok(UpdateTarget {
        workspace: ws,
        binding,
        job,
        job_name: KEEL_SERVER.to_owned(),
        def,
        commit,
        signed_mode,
    }))
}

/// `UpdateStatus.update_path`: `Run` when every prerequisite is met, else the manual steps for
/// `digest` (the available image, else the running one).
///
/// # Errors
///
/// A database error.
pub(crate) async fn update_path(
    conn: &mut PgConnection,
    watched: &oci_registry::ImageRef,
    digest: Option<&str>,
) -> Result<keel_protocol::UpdatePath, sqlx::Error> {
    Ok(match update_target(conn).await? {
        Ok(_) => keel_protocol::UpdatePath::Run,
        Err(missing) => crate::updates::manual::path(missing, watched, digest),
    })
}

/// A pending, queued or running update run in the System workspace (FR-018).
///
/// # Errors
///
/// A database error.
pub(crate) async fn update_run(conn: &mut PgConnection) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "select id from runs \
         where workspace_id = (select workspace_id from system_config where id) \
           and params ? 'keel_server_image_digest' \
           and state in ('pending_approval', 'queued', 'running') \
         order by created_at desc limit 1",
    )
    .fetch_optional(conn)
    .await
}
