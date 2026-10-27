//! Workspaces, projects, teams, grants and users (spec 055 US6, T036). API tokens are in
//! [`crate::api`].
//!
//! - Instance-wide changes (workspaces, teams, members, users) are for instance owners and
//!   admins, from a session. Workspace-scoped ones (rename, projects, grants) need `ws_admin`
//!   there; an API token reaches only its own workspace.
//! - Every change is a Preview/Confirm pair. The confirmed operation re-checks authorization
//!   with the caller as of the `Confirm`, and re-checks the last-owner rule under a row lock.
//! - Every change and every refusal is audited in its own transaction.
//! - A narrowing change bumps each affected user's `authz_epoch` in the same transaction and
//!   calls [`AppState::epoch_changed`] after commit, so their sockets get `Revoked`. When the
//!   caller narrows themselves, the confirming session is carried to the new epoch.
//! - Role changes, user removal and grants need step-up.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;

use keel_core::authz::{self, Capability, Refusal, Subject};
use keel_core::domain::WorkspaceId;
use keel_protocol::{
    Cmd, Event, GrantInfo, GrantProfiles, InstanceRole, Label, ProjectInfo, Slug, TeamInfo,
    TeamMemberInfo, TeamRole, UserInfo, WorkspaceInfo,
};
use serde_json::{Value, json};
use sqlx::PgConnection;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::audit::{self, Decision, Entry};
use crate::dispatch::{PendingOp, Request};
use crate::identity::{self, Caller};
use crate::preview::Draft;
use crate::serve::AppState;

/// Routes one governance command (dispatch.rs sends every `// Governance (US6)` command here).
pub async fn handle(app: &AppState, req: &Request<'_>, cmd: Cmd) -> Vec<Event> {
    match cmd {
        Cmd::ListWorkspaces => list_workspaces(app, req.caller).await,
        Cmd::PreviewWorkspaceCreate { slug, name } => workspace_create(app, req, slug, name).await,
        Cmd::PreviewWorkspaceUpdate { workspace, name } => {
            workspace_update(app, req, workspace, name).await
        }
        Cmd::PreviewWorkspaceArchive { workspace } => workspace_archive(app, req, workspace).await,
        Cmd::PreviewProjectCreate {
            workspace,
            slug,
            name,
        } => project_create(app, req, workspace, slug, name).await,
        Cmd::ListTeams => list_teams(app, req).await,
        Cmd::PreviewTeamCreate { name } => team_create(app, req, name).await,
        Cmd::PreviewTeamMemberAdd { team, user } => member_add(app, req, team, user).await,
        Cmd::PreviewTeamMemberRemove { team, user } => member_remove(app, req, team, user).await,
        Cmd::PreviewGrantAdd {
            team,
            workspace,
            role,
            profiles,
        } => grant_add(app, req, team, workspace, role, profiles).await,
        Cmd::PreviewGrantRemove { grant } => grant_remove(app, req, grant).await,
        Cmd::ListUsers => list_users(app, req).await,
        Cmd::PreviewUserRole { user, role } => user_role(app, req, user, role).await,
        Cmd::PreviewUserRemove { user } => user_remove(app, req, user).await,
        Cmd::ListApiTokens => crate::api::list_tokens(app, req).await,
        Cmd::PreviewApiTokenCreate {
            name,
            workspace,
            actions,
            expires_in_days,
        } => crate::api::token_create(app, req, name, workspace, actions, expires_in_days).await,
        Cmd::PreviewApiTokenRevoke { token } => crate::api::token_revoke(app, req, token).await,
        // Not a governance command: dispatch.rs never sends one here.
        _ => vec![internal()],
    }
}

// ---- shared helpers (also used by api.rs) ---------------------------------------------------

pub(crate) fn internal() -> Event {
    Event::op_error("internal", "Keel could not complete that. Try again.")
}

pub(crate) fn not_found(what: &str) -> Event {
    Event::op_error("not_found", format!("That {what} does not exist."))
}

pub(crate) fn ts(t: OffsetDateTime) -> String {
    t.format(&Rfc3339).unwrap_or_default()
}

/// What the confirmed operation keeps from the request.
#[derive(Clone)]
pub(crate) struct Origin {
    /// The confirming session, carried forward when the caller narrows themselves.
    pub session: Option<String>,
    /// Audit context: the channel, and the token for token callers.
    pub context: Value,
}

pub(crate) fn origin(req: &Request<'_>) -> Origin {
    let mut context = json!({ "channel": req.channel.as_str() });
    if let Some(t) = &req.caller.token {
        context["token"] = json!(t.id);
    }
    Origin {
        session: req.session.map(str::to_owned),
        context,
    }
}

fn subject(c: &Caller) -> Subject<'_> {
    Subject {
        instance_role: c.user.instance_role,
        grants: &c.grants,
    }
}

/// Instance owners and admins, from a session.
pub(crate) fn check_admin(c: &Caller) -> Result<(), &'static str> {
    if c.token.is_some() {
        return Err("token_scope");
    }
    authz::require_instance_admin(&subject(c)).map_err(|r| r.code())
}

/// `cap` on `workspace`; a token only on its own.
pub(crate) fn check_ws(c: &Caller, workspace: Uuid, cap: Capability) -> Result<(), &'static str> {
    if let Some(t) = &c.token
        && t.workspace.0 != workspace
    {
        return Err("token_scope");
    }
    authz::require(&subject(c), cap, WorkspaceId(workspace), &[]).map_err(|r| r.code())
}

fn is_owner(c: &Caller) -> bool {
    c.user.instance_role == keel_core::InstanceRole::Owner
}

/// Audits a refusal in its own transaction and replies `Denied`.
pub(crate) async fn deny(
    app: &AppState,
    caller: &Caller,
    action: &str,
    workspace: Option<Uuid>,
    reason: &str,
    context: Value,
) -> Vec<Event> {
    let actor = caller.actor();
    let result = async {
        let mut tx = app.db.begin().await?;
        audit::record(
            &mut tx,
            Entry {
                actor: &actor,
                action,
                workspace: workspace.map(WorkspaceId),
                target: json!({}),
                decision: Decision::Denied,
                reason: Some(reason),
                context,
            },
        )
        .await?;
        tx.commit().await
    }
    .await;
    if let Err(e) = result {
        tracing::error!(error = %e, "audit write failed");
    }
    vec![Event::Denied {
        action: action.to_owned(),
        reason: reason.to_owned(),
    }]
}

/// Records an allowed change in the change's transaction.
pub(crate) async fn record(
    tx: &mut PgConnection,
    caller: &Caller,
    action: &str,
    workspace: Option<Uuid>,
    target: Value,
    context: &Value,
) -> Result<(), sqlx::Error> {
    audit::record(
        tx,
        Entry {
            actor: &caller.actor(),
            action,
            workspace: workspace.map(WorkspaceId),
            target,
            decision: Decision::Allowed,
            reason: None,
            context: context.clone(),
        },
    )
    .await
    .map(|_| ())
}

/// Bumps each user's `authz_epoch`; when one is the caller, moves the confirming session to the
/// new epoch so only it survives.
pub(crate) async fn bump(
    tx: &mut PgConnection,
    users: &[Uuid],
    caller: &Caller,
    session: Option<&str>,
) -> Result<(), sqlx::Error> {
    for &u in users {
        let epoch = identity::bump_epoch(&mut *tx, u).await?;
        if u == caller.user.id
            && let Some(s) = session
        {
            sqlx::query("update sessions set authz_epoch = $3 where id = $1 and user_id = $2")
                .bind(s)
                .bind(u)
                .bind(epoch)
                .execute(&mut *tx)
                .await?;
        }
    }
    Ok(())
}

/// Why a confirmed operation did not happen.
pub(crate) enum Fail {
    /// Authorization refused it (audited, `Denied`).
    Denied(&'static str),
    /// It could not be done (`OpError`, shown verbatim).
    Op(&'static str, String),
}

/// A confirmed operation's transaction: the users whose epoch it bumped, or why it failed.
pub(crate) type Step = Result<Result<Vec<Uuid>, Fail>, sqlx::Error>;

/// Settles a transaction's outcome: announces bumped epochs, or replies the failure.
pub(crate) async fn settle(
    app: &AppState,
    caller: &Caller,
    action: &str,
    workspace: Option<Uuid>,
    origin: &Origin,
    step: Step,
) -> Result<(), Vec<Event>> {
    match step {
        Ok(Ok(bumped)) => {
            for u in bumped {
                app.epoch_changed(u);
            }
            Ok(())
        }
        Ok(Err(Fail::Denied(code))) => {
            Err(deny(app, caller, action, workspace, code, origin.context.clone()).await)
        }
        Ok(Err(Fail::Op(code, message))) => Err(vec![Event::op_error(code, message)]),
        Err(e) => {
            tracing::error!(error = %e, action, "governance change failed");
            Err(vec![internal()])
        }
    }
}

/// Wraps an async operation as a [`PendingOp`].
pub(crate) fn pending<F, Fut>(f: F) -> PendingOp
where
    F: FnOnce(AppState, Caller) -> Fut + Send + 'static,
    Fut: Future<Output = Vec<Event>> + Send + 'static,
{
    Box::new(move |app, caller| Box::pin(f(app, caller)))
}

/// Stores a preview for the request's session (or token) and replies it.
pub(crate) fn preview(
    app: &AppState,
    req: &Request<'_>,
    draft: Draft,
    op: PendingOp,
) -> Vec<Event> {
    let key = req.caller.preview_key(req.session);
    if key.is_empty() {
        // Neither a session nor a token: nothing to bind the preview to.
        return vec![internal()];
    }
    vec![Event::Preview(app.previews.create(&key, draft, op))]
}

fn draft(action: &str, summary: String, affected: Vec<String>, step_up: bool) -> Draft {
    Draft {
        action: action.to_owned(),
        summary,
        affected,
        step_up,
    }
}

fn unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .and_then(|d| d.code())
        .is_some_and(|c| c == "23505")
}

fn core_team_role(r: TeamRole) -> keel_core::TeamRole {
    match r {
        TeamRole::Viewer => keel_core::TeamRole::Viewer,
        TeamRole::Operator => keel_core::TeamRole::Operator,
        TeamRole::Approver => keel_core::TeamRole::Approver,
        TeamRole::WsAdmin => keel_core::TeamRole::WsAdmin,
    }
}

fn core_instance_role(r: InstanceRole) -> keel_core::InstanceRole {
    match r {
        InstanceRole::Owner => keel_core::InstanceRole::Owner,
        InstanceRole::Admin => keel_core::InstanceRole::Admin,
        InstanceRole::User => keel_core::InstanceRole::User,
    }
}

fn decode<T: std::str::FromStr<Err = keel_core::domain::ParseError>>(
    s: &str,
) -> Result<T, sqlx::Error> {
    s.parse::<T>().map_err(|e| sqlx::Error::Decode(Box::new(e)))
}

// ---- lookups --------------------------------------------------------------------------------

/// `(name, archived)` of a workspace.
async fn workspace_row(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<Option<(String, bool)>, sqlx::Error> {
    sqlx::query_as("select name, archived_at is not null from workspaces where id = $1")
        .bind(id)
        .fetch_optional(conn)
        .await
}

/// Active members of a team: `(id, display_name)`.
async fn team_members(
    conn: &mut PgConnection,
    team: Uuid,
) -> Result<Vec<(Uuid, String)>, sqlx::Error> {
    sqlx::query_as(
        "select u.id, u.display_name from team_members m join users u on u.id = m.user_id \
         where m.team_id = $1 and u.disabled_at is null order by u.display_name, u.id",
    )
    .bind(team)
    .fetch_all(conn)
    .await
}

/// Active users holding any grant on a workspace: `(id, display_name)`.
async fn workspace_users(
    conn: &mut PgConnection,
    ws: Uuid,
) -> Result<Vec<(Uuid, String)>, sqlx::Error> {
    sqlx::query_as(
        "select distinct u.id, u.display_name from grants g \
         join team_members m on m.team_id = g.team_id join users u on u.id = m.user_id \
         where g.workspace_id = $1 and u.disabled_at is null order by u.display_name, u.id",
    )
    .bind(ws)
    .fetch_all(conn)
    .await
}

fn names(users: &[(Uuid, String)]) -> Vec<String> {
    users.iter().map(|(_, n)| n.clone()).collect()
}

fn ids(users: &[(Uuid, String)]) -> Vec<Uuid> {
    users.iter().map(|(u, _)| *u).collect()
}

/// `(display_name, instance_role, disabled)` of a user.
async fn user_row(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<Option<(String, keel_core::InstanceRole, bool)>, sqlx::Error> {
    let row: Option<(String, String, bool)> = sqlx::query_as(
        "select display_name, instance_role, disabled_at is not null from users where id = $1",
    )
    .bind(id)
    .fetch_optional(conn)
    .await?;
    row.map(|(n, r, d)| Ok((n, decode(&r)?, d))).transpose()
}

/// Locks the active owners' rows and counts them.
async fn lock_owners(conn: &mut PgConnection) -> Result<usize, sqlx::Error> {
    let owners: Vec<(Uuid,)> = sqlx::query_as(
        "select id from users where instance_role = 'owner' and disabled_at is null \
         order by id for update",
    )
    .fetch_all(conn)
    .await?;
    Ok(owners.len())
}

/// Runs a closure against a pooled connection (previews read outside any change).
async fn read<T>(
    app: &AppState,
    f: impl AsyncFnOnce(&mut PgConnection) -> Result<T, sqlx::Error>,
) -> Result<T, Vec<Event>> {
    let result = async {
        let mut conn = app.db.acquire().await?;
        f(&mut conn).await
    }
    .await;
    result.map_err(|e| {
        tracing::error!(error = %e, "governance read failed");
        vec![internal()]
    })
}

// ---- workspaces and projects ----------------------------------------------------------------

type WorkspaceRow = (Uuid, String, String, OffsetDateTime, bool);

/// `Workspaces`: instance admins see every workspace, a token its own, others their grants'.
pub async fn list_workspaces(app: &AppState, caller: &Caller) -> Vec<Event> {
    let result: Result<Vec<WorkspaceInfo>, sqlx::Error> = async {
        let rows: Vec<WorkspaceRow> = sqlx::query_as(
            "select id, slug, name, created_at, archived_at is not null from workspaces \
             order by slug",
        )
        .fetch_all(&app.db)
        .await?;
        let projects: Vec<(Uuid, Uuid, String, String)> =
            sqlx::query_as("select id, workspace_id, slug, name from projects order by slug")
                .fetch_all(&app.db)
                .await?;
        let visible = |id: Uuid, archived: bool| match &caller.token {
            Some(t) => t.workspace.0 == id && !archived,
            None if subject(caller).is_instance_admin() => true,
            None => !archived && caller.grants.iter().any(|g| g.workspace.0 == id),
        };
        Ok(rows
            .into_iter()
            .filter(|r| visible(r.0, r.4))
            .map(|(id, slug, name, created, archived)| WorkspaceInfo {
                id,
                slug,
                name,
                created: ts(created),
                archived,
                projects: projects
                    .iter()
                    .filter(|p| p.1 == id)
                    .map(|p| ProjectInfo {
                        id: p.0,
                        slug: p.2.clone(),
                        name: p.3.clone(),
                    })
                    .collect(),
            })
            .collect())
    }
    .await;
    match result {
        Ok(items) => vec![Event::Workspaces { items }],
        Err(e) => {
            tracing::error!(error = %e, "list workspaces failed");
            vec![internal()]
        }
    }
}

async fn workspace_create(
    app: &AppState,
    req: &Request<'_>,
    slug: Slug,
    name: Label,
) -> Vec<Event> {
    const CMD: &str = "preview_workspace_create";
    const ACTION: &str = "workspace_create";
    let o = origin(req);
    if let Err(code) = check_admin(req.caller) {
        return deny(app, req.caller, CMD, None, code, o.context).await;
    }
    let (slug, name) = (slug.as_str().to_owned(), name.as_str().to_owned());
    let taken = read(app, async |c| {
        sqlx::query_scalar::<_, bool>("select exists(select 1 from workspaces where slug = $1)")
            .bind(&slug)
            .fetch_one(c)
            .await
    })
    .await;
    match taken {
        Err(e) => return e,
        Ok(true) => return vec![slug_taken()],
        Ok(false) => {}
    }
    let summary = format!("Create the workspace \"{name}\" ({slug}).");
    let op = pending(move |app, caller| async move {
        let step: Step = async {
            if let Err(code) = check_admin(&caller) {
                return Ok(Err(Fail::Denied(code)));
            }
            let mut tx = app.db.begin().await?;
            let id = Uuid::now_v7();
            let inserted =
                sqlx::query("insert into workspaces (id, slug, name) values ($1, $2, $3)")
                    .bind(id)
                    .bind(&slug)
                    .bind(&name)
                    .execute(&mut *tx)
                    .await;
            match inserted {
                Err(e) if unique_violation(&e) => {
                    return Ok(Err(Fail::Op("slug_taken", SLUG_TAKEN.into())));
                }
                other => other?,
            };
            let target = json!({ "slug": slug, "name": name });
            record(&mut tx, &caller, ACTION, Some(id), target, &o.context).await?;
            tx.commit().await?;
            Ok(Ok(vec![]))
        }
        .await;
        if let Err(events) = settle(&app, &caller, ACTION, None, &o, step).await {
            return events;
        }
        list_workspaces(&app, &caller).await
    });
    preview(app, req, draft(ACTION, summary, vec![], false), op)
}

const SLUG_TAKEN: &str = "That slug is already in use.";

fn slug_taken() -> Event {
    Event::op_error("slug_taken", SLUG_TAKEN)
}

/// Checks the workspace exists and is not archived, for a change to it.
async fn live_workspace(app: &AppState, ws: Uuid) -> Result<String, Vec<Event>> {
    match read(app, async |c| workspace_row(c, ws).await).await? {
        None => Err(vec![not_found("workspace")]),
        Some((_, true)) => Err(vec![archived()]),
        Some((name, false)) => Ok(name),
    }
}

fn archived() -> Event {
    Event::op_error("workspace_archived", "That workspace is archived.")
}

/// Inside a change: `Some(fail)` unless the workspace exists and is live.
async fn live_in_tx(conn: &mut PgConnection, ws: Uuid) -> Result<Option<Fail>, sqlx::Error> {
    let row: Option<(bool,)> =
        sqlx::query_as("select archived_at is not null from workspaces where id = $1 for update")
            .bind(ws)
            .fetch_optional(conn)
            .await?;
    Ok(match row {
        None => Some(Fail::Op(
            "not_found",
            "That workspace does not exist.".into(),
        )),
        Some((true,)) => Some(Fail::Op(
            "workspace_archived",
            "That workspace is archived.".into(),
        )),
        Some((false,)) => None,
    })
}

async fn workspace_update(app: &AppState, req: &Request<'_>, ws: Uuid, name: Label) -> Vec<Event> {
    const CMD: &str = "preview_workspace_update";
    const ACTION: &str = "workspace_update";
    let o = origin(req);
    if let Err(code) = check_ws(req.caller, ws, Capability::Administer) {
        return deny(app, req.caller, CMD, Some(ws), code, o.context).await;
    }
    let old = match live_workspace(app, ws).await {
        Ok(n) => n,
        Err(e) => return e,
    };
    let name = name.as_str().to_owned();
    let summary = format!("Rename the workspace \"{old}\" to \"{name}\".");
    let op = pending(move |app, caller| async move {
        let step: Step = async {
            if let Err(code) = check_ws(&caller, ws, Capability::Administer) {
                return Ok(Err(Fail::Denied(code)));
            }
            let mut tx = app.db.begin().await?;
            if let Some(f) = live_in_tx(&mut tx, ws).await? {
                return Ok(Err(f));
            }
            sqlx::query("update workspaces set name = $2 where id = $1")
                .bind(ws)
                .bind(&name)
                .execute(&mut *tx)
                .await?;
            record(
                &mut tx,
                &caller,
                ACTION,
                Some(ws),
                json!({ "name": name }),
                &o.context,
            )
            .await?;
            tx.commit().await?;
            Ok(Ok(vec![]))
        }
        .await;
        if let Err(events) = settle(&app, &caller, ACTION, Some(ws), &o, step).await {
            return events;
        }
        list_workspaces(&app, &caller).await
    });
    preview(app, req, draft(ACTION, summary, vec![], false), op)
}

async fn workspace_archive(app: &AppState, req: &Request<'_>, ws: Uuid) -> Vec<Event> {
    const CMD: &str = "preview_workspace_archive";
    const ACTION: &str = "workspace_archive";
    let o = origin(req);
    if let Err(code) = check_admin(req.caller) {
        return deny(app, req.caller, CMD, Some(ws), code, o.context).await;
    }
    let name = match live_workspace(app, ws).await {
        Ok(n) => n,
        Err(e) => return e,
    };
    let affected = match read(app, async |c| workspace_users(c, ws).await).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let summary = format!(
        "Archive the workspace \"{name}\". Everyone with a grant on it loses access, and its API \
         tokens are revoked."
    );
    let op = pending(move |app, caller| async move {
        let step: Step = async {
            if let Err(code) = check_admin(&caller) {
                return Ok(Err(Fail::Denied(code)));
            }
            let mut tx = app.db.begin().await?;
            if let Some(f) = live_in_tx(&mut tx, ws).await? {
                return Ok(Err(f));
            }
            let users = ids(&workspace_users(&mut tx, ws).await?);
            sqlx::query("update workspaces set archived_at = now() where id = $1")
                .bind(ws)
                .execute(&mut *tx)
                .await?;
            let revoked = sqlx::query(
                "update api_tokens set revoked_at = now() \
                 where workspace_id = $1 and revoked_at is null",
            )
            .bind(ws)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            bump(&mut tx, &users, &caller, o.session.as_deref()).await?;
            let target = json!({ "tokens_revoked": revoked, "epoch_bumped": users });
            record(&mut tx, &caller, ACTION, Some(ws), target, &o.context).await?;
            tx.commit().await?;
            Ok(Ok(users))
        }
        .await;
        if let Err(events) = settle(&app, &caller, ACTION, Some(ws), &o, step).await {
            return events;
        }
        list_workspaces(&app, &caller).await
    });
    preview(
        app,
        req,
        draft(ACTION, summary, names(&affected), false),
        op,
    )
}

async fn project_create(
    app: &AppState,
    req: &Request<'_>,
    ws: Uuid,
    slug: Slug,
    name: Label,
) -> Vec<Event> {
    const CMD: &str = "preview_project_create";
    const ACTION: &str = "project_create";
    let o = origin(req);
    if let Err(code) = check_ws(req.caller, ws, Capability::Administer) {
        return deny(app, req.caller, CMD, Some(ws), code, o.context).await;
    }
    let ws_name = match live_workspace(app, ws).await {
        Ok(n) => n,
        Err(e) => return e,
    };
    let (slug, name) = (slug.as_str().to_owned(), name.as_str().to_owned());
    let taken = read(app, async |c| {
        sqlx::query_scalar::<_, bool>(
            "select exists(select 1 from projects where workspace_id = $1 and slug = $2)",
        )
        .bind(ws)
        .bind(&slug)
        .fetch_one(c)
        .await
    })
    .await;
    match taken {
        Err(e) => return e,
        Ok(true) => return vec![slug_taken()],
        Ok(false) => {}
    }
    let summary = format!("Create the project \"{name}\" ({slug}) in \"{ws_name}\".");
    let op = pending(move |app, caller| async move {
        let step: Step = async {
            if let Err(code) = check_ws(&caller, ws, Capability::Administer) {
                return Ok(Err(Fail::Denied(code)));
            }
            let mut tx = app.db.begin().await?;
            if let Some(f) = live_in_tx(&mut tx, ws).await? {
                return Ok(Err(f));
            }
            let id = Uuid::now_v7();
            let inserted = sqlx::query(
                "insert into projects (id, workspace_id, slug, name) values ($1, $2, $3, $4)",
            )
            .bind(id)
            .bind(ws)
            .bind(&slug)
            .bind(&name)
            .execute(&mut *tx)
            .await;
            match inserted {
                Err(e) if unique_violation(&e) => {
                    return Ok(Err(Fail::Op("slug_taken", SLUG_TAKEN.into())));
                }
                other => other?,
            };
            let target = json!({ "project": id, "slug": slug, "name": name });
            record(&mut tx, &caller, ACTION, Some(ws), target, &o.context).await?;
            tx.commit().await?;
            Ok(Ok(vec![]))
        }
        .await;
        if let Err(events) = settle(&app, &caller, ACTION, Some(ws), &o, step).await {
            return events;
        }
        list_workspaces(&app, &caller).await
    });
    preview(app, req, draft(ACTION, summary, vec![], false), op)
}

// ---- teams, members and grants --------------------------------------------------------------

type GrantRow = (Uuid, Uuid, Uuid, String, String, Vec<String>);

/// `Teams`: instance admins see every team. Others see their own teams and those granted on a
/// workspace they administer, with grants limited to workspaces they can see.
async fn list_teams(app: &AppState, req: &Request<'_>) -> Vec<Event> {
    let caller = req.caller;
    if caller.token.is_some() {
        return deny(
            app,
            caller,
            "list_teams",
            None,
            "token_scope",
            origin(req).context,
        )
        .await;
    }
    teams(app, caller).await
}

async fn teams(app: &AppState, caller: &Caller) -> Vec<Event> {
    let result: Result<Vec<TeamInfo>, sqlx::Error> = async {
        let teams: Vec<(Uuid, String)> = sqlx::query_as("select id, name from teams order by name")
            .fetch_all(&app.db)
            .await?;
        let members: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
            "select m.team_id, u.id, u.display_name from team_members m \
             join users u on u.id = m.user_id where u.disabled_at is null \
             order by u.display_name, u.id",
        )
        .fetch_all(&app.db)
        .await?;
        let grants: Vec<GrantRow> = sqlx::query_as(
            "select g.id, g.team_id, g.workspace_id, w.slug, g.role, g.profiles from grants g \
             join workspaces w on w.id = g.workspace_id where w.archived_at is null \
             order by w.slug, g.role, g.id",
        )
        .fetch_all(&app.db)
        .await?;
        let admin = subject(caller).is_instance_admin();
        let sees: BTreeSet<Uuid> = caller.grants.iter().map(|g| g.workspace.0).collect();
        let administers: BTreeSet<Uuid> = sees
            .iter()
            .copied()
            .filter(|w| check_ws(caller, *w, Capability::Administer).is_ok())
            .collect();
        let mut out = Vec::new();
        for (id, name) in teams {
            let member = members.iter().any(|m| m.0 == id && m.1 == caller.user.id);
            let administered = grants
                .iter()
                .any(|g| g.1 == id && administers.contains(&g.2));
            if !(admin || member || administered) {
                continue;
            }
            let mut gs = Vec::new();
            for g in grants.iter().filter(|g| g.1 == id) {
                if admin || sees.contains(&g.2) {
                    gs.push(GrantInfo {
                        id: g.0,
                        workspace: g.2,
                        workspace_slug: g.3.clone(),
                        role: decode::<keel_core::TeamRole>(&g.4)?.into(),
                        profiles: g.5.clone(),
                    });
                }
            }
            out.push(TeamInfo {
                id,
                name,
                members: members
                    .iter()
                    .filter(|m| m.0 == id)
                    .map(|m| TeamMemberInfo {
                        user: m.1,
                        display_name: m.2.clone(),
                    })
                    .collect(),
                grants: gs,
            });
        }
        Ok(out)
    }
    .await;
    match result {
        Ok(items) => vec![Event::Teams { items }],
        Err(e) => {
            tracing::error!(error = %e, "list teams failed");
            vec![internal()]
        }
    }
}

async fn team_create(app: &AppState, req: &Request<'_>, name: Label) -> Vec<Event> {
    const CMD: &str = "preview_team_create";
    const ACTION: &str = "team_create";
    let o = origin(req);
    if let Err(code) = check_admin(req.caller) {
        return deny(app, req.caller, CMD, None, code, o.context).await;
    }
    let name = name.as_str().to_owned();
    let taken = read(app, async |c| {
        sqlx::query_scalar::<_, bool>("select exists(select 1 from teams where name = $1)")
            .bind(&name)
            .fetch_one(c)
            .await
    })
    .await;
    match taken {
        Err(e) => return e,
        Ok(true) => return vec![name_taken()],
        Ok(false) => {}
    }
    let summary = format!("Create the team \"{name}\".");
    let op = pending(move |app, caller| async move {
        let step: Step = async {
            if let Err(code) = check_admin(&caller) {
                return Ok(Err(Fail::Denied(code)));
            }
            let mut tx = app.db.begin().await?;
            let id = Uuid::now_v7();
            let inserted = sqlx::query("insert into teams (id, name) values ($1, $2)")
                .bind(id)
                .bind(&name)
                .execute(&mut *tx)
                .await;
            match inserted {
                Err(e) if unique_violation(&e) => {
                    return Ok(Err(Fail::Op("name_taken", NAME_TAKEN.into())));
                }
                other => other?,
            };
            let target = json!({ "team": id, "name": name });
            record(&mut tx, &caller, ACTION, None, target, &o.context).await?;
            tx.commit().await?;
            Ok(Ok(vec![]))
        }
        .await;
        if let Err(events) = settle(&app, &caller, ACTION, None, &o, step).await {
            return events;
        }
        teams(&app, &caller).await
    });
    preview(app, req, draft(ACTION, summary, vec![], false), op)
}

const NAME_TAKEN: &str = "A team with that name already exists.";

fn name_taken() -> Event {
    Event::op_error("name_taken", NAME_TAKEN)
}

async fn team_name(app: &AppState, team: Uuid) -> Result<Option<String>, Vec<Event>> {
    read(app, async |c| {
        sqlx::query_scalar::<_, String>("select name from teams where id = $1")
            .bind(team)
            .fetch_optional(c)
            .await
    })
    .await
}

async fn member_add(app: &AppState, req: &Request<'_>, team: Uuid, user: Uuid) -> Vec<Event> {
    const CMD: &str = "preview_team_member_add";
    const ACTION: &str = "team_member_add";
    let o = origin(req);
    if let Err(code) = check_admin(req.caller) {
        return deny(app, req.caller, CMD, None, code, o.context).await;
    }
    let team_name = match team_name(app, team).await {
        Ok(Some(n)) => n,
        Ok(None) => return vec![not_found("team")],
        Err(e) => return e,
    };
    let looked = read(app, async |c| {
        let u = user_row(&mut *c, user).await?;
        let member = sqlx::query_scalar::<_, bool>(
            "select exists(select 1 from team_members where team_id = $1 and user_id = $2)",
        )
        .bind(team)
        .bind(user)
        .fetch_one(&mut *c)
        .await?;
        let grants: Vec<(String, String)> = sqlx::query_as(
            "select w.slug, g.role from grants g join workspaces w on w.id = g.workspace_id \
             where g.team_id = $1 and w.archived_at is null order by w.slug, g.role",
        )
        .bind(team)
        .fetch_all(&mut *c)
        .await?;
        Ok((u, member, grants))
    })
    .await;
    let (display, grants) = match looked {
        Err(e) => return e,
        Ok((None | Some((_, _, true)), _, _)) => return vec![not_found("user")],
        Ok((_, true, _)) => return vec![already_member()],
        Ok((Some((n, _, false)), false, g)) => (n, g),
    };
    let gains = if grants.is_empty() {
        "no workspace access yet".to_owned()
    } else {
        grants
            .iter()
            .map(|(w, r)| format!("{r} on {w}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let summary = format!("Add {display} to the team \"{team_name}\". They gain {gains}.");
    let op = pending(move |app, caller| async move {
        let step: Step = async {
            if let Err(code) = check_admin(&caller) {
                return Ok(Err(Fail::Denied(code)));
            }
            let mut tx = app.db.begin().await?;
            match user_row(&mut tx, user).await? {
                Some((_, _, false)) => {}
                _ => {
                    return Ok(Err(Fail::Op(
                        "not_found",
                        "That user does not exist.".into(),
                    )));
                }
            }
            let added = sqlx::query(
                "insert into team_members (team_id, user_id) select $1, $2 \
                 where exists(select 1 from teams where id = $1) on conflict do nothing",
            )
            .bind(team)
            .bind(user)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if added == 0 {
                return Ok(Err(Fail::Op("already_member", ALREADY_MEMBER.into())));
            }
            let target = json!({ "team": team, "user": user });
            record(&mut tx, &caller, ACTION, None, target, &o.context).await?;
            tx.commit().await?;
            Ok(Ok(vec![]))
        }
        .await;
        if let Err(events) = settle(&app, &caller, ACTION, None, &o, step).await {
            return events;
        }
        teams(&app, &caller).await
    });
    preview(app, req, draft(ACTION, summary, vec![display], false), op)
}

const ALREADY_MEMBER: &str = "That user is already in the team.";

fn already_member() -> Event {
    Event::op_error("already_member", ALREADY_MEMBER)
}

async fn member_remove(app: &AppState, req: &Request<'_>, team: Uuid, user: Uuid) -> Vec<Event> {
    const CMD: &str = "preview_team_member_remove";
    const ACTION: &str = "team_member_remove";
    let o = origin(req);
    if let Err(code) = check_admin(req.caller) {
        return deny(app, req.caller, CMD, None, code, o.context).await;
    }
    let team_name = match team_name(app, team).await {
        Ok(Some(n)) => n,
        Ok(None) => return vec![not_found("team")],
        Err(e) => return e,
    };
    let display = read(app, async |c| {
        sqlx::query_scalar::<_, String>(
            "select u.display_name from team_members m join users u on u.id = m.user_id \
             where m.team_id = $1 and m.user_id = $2",
        )
        .bind(team)
        .bind(user)
        .fetch_optional(c)
        .await
    })
    .await;
    let display = match display {
        Err(e) => return e,
        Ok(None) => return vec![not_member()],
        Ok(Some(n)) => n,
    };
    let summary = format!(
        "Remove {display} from the team \"{team_name}\". They lose what the team's grants give \
         them, and their sessions end."
    );
    let op = pending(move |app, caller| async move {
        let step: Step = async {
            if let Err(code) = check_admin(&caller) {
                return Ok(Err(Fail::Denied(code)));
            }
            let mut tx = app.db.begin().await?;
            let removed =
                sqlx::query("delete from team_members where team_id = $1 and user_id = $2")
                    .bind(team)
                    .bind(user)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected();
            if removed == 0 {
                return Ok(Err(Fail::Op("not_member", NOT_MEMBER.into())));
            }
            bump(&mut tx, &[user], &caller, o.session.as_deref()).await?;
            let target = json!({ "team": team, "user": user, "epoch_bumped": [user] });
            record(&mut tx, &caller, ACTION, None, target, &o.context).await?;
            tx.commit().await?;
            Ok(Ok(vec![user]))
        }
        .await;
        if let Err(events) = settle(&app, &caller, ACTION, None, &o, step).await {
            return events;
        }
        teams(&app, &caller).await
    });
    preview(app, req, draft(ACTION, summary, vec![display], false), op)
}

const NOT_MEMBER: &str = "That user is not in the team.";

fn not_member() -> Event {
    Event::op_error("not_member", NOT_MEMBER)
}

async fn grant_add(
    app: &AppState,
    req: &Request<'_>,
    team: Uuid,
    ws: Uuid,
    role: TeamRole,
    profiles: GrantProfiles,
) -> Vec<Event> {
    const CMD: &str = "preview_grant_add";
    const ACTION: &str = "grant_add";
    let o = origin(req);
    if let Err(code) = check_ws(req.caller, ws, Capability::Administer) {
        return deny(app, req.caller, CMD, Some(ws), code, o.context).await;
    }
    let ws_name = match live_workspace(app, ws).await {
        Ok(n) => n,
        Err(e) => return e,
    };
    let team_name = match team_name(app, team).await {
        Ok(Some(n)) => n,
        Ok(None) => return vec![not_found("team")],
        Err(e) => return e,
    };
    let mut profiles: Vec<String> = profiles.0.iter().map(|p| p.as_str().to_owned()).collect();
    profiles.sort();
    let role = core_team_role(role);
    let members = match read(app, async |c| team_members(c, team).await).await {
        Ok(m) => m,
        Err(e) => return e,
    };
    let scope = if profiles.is_empty() {
        "the whole workspace".to_owned()
    } else {
        format!("hosts with the profiles {}", profiles.join(", "))
    };
    let summary = format!(
        "Grant the team \"{team_name}\" {role} on \"{ws_name}\" ({scope}). Its members listed \
         here gain this access."
    );
    let op = pending(move |app, caller| async move {
        let step: Step = async {
            if let Err(code) = check_ws(&caller, ws, Capability::Administer) {
                return Ok(Err(Fail::Denied(code)));
            }
            let mut tx = app.db.begin().await?;
            if let Some(f) = live_in_tx(&mut tx, ws).await? {
                return Ok(Err(f));
            }
            let team_exists: bool =
                sqlx::query_scalar("select exists(select 1 from teams where id = $1)")
                    .bind(team)
                    .fetch_one(&mut *tx)
                    .await?;
            if !team_exists {
                return Ok(Err(Fail::Op(
                    "not_found",
                    "That team does not exist.".into(),
                )));
            }
            let duplicate: bool = sqlx::query_scalar(
                "select exists(select 1 from grants where team_id = $1 and workspace_id = $2 \
                 and role = $3 and profiles = $4)",
            )
            .bind(team)
            .bind(ws)
            .bind(role.as_str())
            .bind(&profiles)
            .fetch_one(&mut *tx)
            .await?;
            if duplicate {
                return Ok(Err(Fail::Op(
                    "grant_exists",
                    "The team already holds that grant.".into(),
                )));
            }
            let id = Uuid::now_v7();
            sqlx::query(
                "insert into grants (id, team_id, workspace_id, role, profiles) \
                 values ($1, $2, $3, $4, $5)",
            )
            .bind(id)
            .bind(team)
            .bind(ws)
            .bind(role.as_str())
            .bind(&profiles)
            .execute(&mut *tx)
            .await?;
            let target = json!({
                "grant": id, "team": team, "role": role.as_str(), "profiles": profiles,
            });
            record(&mut tx, &caller, ACTION, Some(ws), target, &o.context).await?;
            tx.commit().await?;
            Ok(Ok(vec![]))
        }
        .await;
        if let Err(events) = settle(&app, &caller, ACTION, Some(ws), &o, step).await {
            return events;
        }
        teams(&app, &caller).await
    });
    preview(app, req, draft(ACTION, summary, names(&members), true), op)
}

async fn grant_remove(app: &AppState, req: &Request<'_>, grant: Uuid) -> Vec<Event> {
    const CMD: &str = "preview_grant_remove";
    const ACTION: &str = "grant_remove";
    let o = origin(req);
    let row = read(app, async |c| {
        sqlx::query_as::<_, (Uuid, Uuid, String, String, String)>(
            "select g.team_id, g.workspace_id, w.name, t.name, g.role from grants g \
             join workspaces w on w.id = g.workspace_id join teams t on t.id = g.team_id \
             where g.id = $1",
        )
        .bind(grant)
        .fetch_optional(c)
        .await
    })
    .await;
    let (team, ws, ws_name, team_name, role) = match row {
        Err(e) => return e,
        Ok(Some(r)) => r,
        // Fail closed: someone who could not see it learns nothing.
        Ok(None) if check_admin(req.caller).is_err() => {
            return deny(
                app,
                req.caller,
                CMD,
                None,
                Refusal::NoGrant.code(),
                o.context,
            )
            .await;
        }
        Ok(None) => return vec![not_found("grant")],
    };
    if let Err(code) = check_ws(req.caller, ws, Capability::Administer) {
        return deny(app, req.caller, CMD, Some(ws), code, o.context).await;
    }
    let members = match read(app, async |c| team_members(c, team).await).await {
        Ok(m) => m,
        Err(e) => return e,
    };
    let summary = format!(
        "Remove the team \"{team_name}\"'s {role} grant on \"{ws_name}\". Its members listed here \
         lose this access, and their sessions end."
    );
    let op = pending(move |app, caller| async move {
        let step: Step = async {
            if let Err(code) = check_ws(&caller, ws, Capability::Administer) {
                return Ok(Err(Fail::Denied(code)));
            }
            let mut tx = app.db.begin().await?;
            let removed = sqlx::query("delete from grants where id = $1")
                .bind(grant)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            if removed == 0 {
                return Ok(Err(Fail::Op(
                    "not_found",
                    "That grant does not exist.".into(),
                )));
            }
            let users = ids(&team_members(&mut tx, team).await?);
            bump(&mut tx, &users, &caller, o.session.as_deref()).await?;
            let target = json!({
                "grant": grant, "team": team, "role": role, "epoch_bumped": users,
            });
            record(&mut tx, &caller, ACTION, Some(ws), target, &o.context).await?;
            tx.commit().await?;
            Ok(Ok(users))
        }
        .await;
        if let Err(events) = settle(&app, &caller, ACTION, Some(ws), &o, step).await {
            return events;
        }
        teams(&app, &caller).await
    });
    preview(app, req, draft(ACTION, summary, names(&members), false), op)
}

// ---- users ----------------------------------------------------------------------------------

type UserListRow = (Uuid, String, Option<String>, String, bool, OffsetDateTime);

async fn list_users(app: &AppState, req: &Request<'_>) -> Vec<Event> {
    if let Err(code) = check_admin(req.caller) {
        return deny(
            app,
            req.caller,
            "list_users",
            None,
            code,
            origin(req).context,
        )
        .await;
    }
    users(app).await
}

async fn users(app: &AppState) -> Vec<Event> {
    let result: Result<Vec<UserInfo>, sqlx::Error> = async {
        let rows: Vec<UserListRow> = sqlx::query_as(
            "select id, display_name, email, instance_role, disabled_at is not null, created_at \
             from users order by display_name, id",
        )
        .fetch_all(&app.db)
        .await?;
        let teams: Vec<(Uuid, String)> = sqlx::query_as(
            "select m.user_id, t.name from team_members m join teams t on t.id = m.team_id \
             order by t.name",
        )
        .fetch_all(&app.db)
        .await?;
        let mut by_user: BTreeMap<Uuid, Vec<String>> = BTreeMap::new();
        for (u, t) in teams {
            by_user.entry(u).or_default().push(t);
        }
        rows.into_iter()
            .map(|(id, display_name, email, role, disabled, created)| {
                Ok(UserInfo {
                    id,
                    display_name,
                    email,
                    instance_role: decode::<keel_core::InstanceRole>(&role)?.into(),
                    disabled,
                    teams: by_user.remove(&id).unwrap_or_default(),
                    created: ts(created),
                })
            })
            .collect()
    }
    .await;
    match result {
        Ok(items) => vec![Event::Users { items }],
        Err(e) => {
            tracing::error!(error = %e, "list users failed");
            vec![internal()]
        }
    }
}

/// The ownership rules for changing `target` from `from` to `to` (`None`: removal).
fn owner_rule(
    caller: &Caller,
    from: keel_core::InstanceRole,
    to: Option<keel_core::InstanceRole>,
    owners: usize,
) -> Result<(), &'static str> {
    let owner = keel_core::InstanceRole::Owner;
    if (from == owner || to == Some(owner)) && !is_owner(caller) {
        return Err("owner_only");
    }
    authz::check_instance_role_change(from, to, owners).map_err(|r| r.code())
}

async fn owners(app: &AppState) -> Result<usize, Vec<Event>> {
    read(app, async |c| {
        sqlx::query_scalar::<_, i64>(
            "select count(*) from users where instance_role = 'owner' and disabled_at is null",
        )
        .fetch_one(c)
        .await
    })
    .await
    .map(|n| usize::try_from(n).unwrap_or(0))
}

/// Narrowing when the role administers less than before.
fn narrows(from: keel_core::InstanceRole, to: keel_core::InstanceRole) -> bool {
    use keel_core::InstanceRole::{Admin, Owner, User};
    matches!((from, to), (Owner, Admin | User) | (Admin, User))
}

async fn user_role(
    app: &AppState,
    req: &Request<'_>,
    user: Uuid,
    role: InstanceRole,
) -> Vec<Event> {
    const CMD: &str = "preview_user_role";
    const ACTION: &str = "user_role";
    let o = origin(req);
    if let Err(code) = check_admin(req.caller) {
        return deny(app, req.caller, CMD, None, code, o.context).await;
    }
    let to = core_instance_role(role);
    let (display, from) = match read(app, async |c| user_row(c, user).await).await {
        Err(e) => return e,
        Ok(None | Some((_, _, true))) => return vec![not_found("user")],
        Ok(Some((n, r, false))) => (n, r),
    };
    if from == to {
        return vec![no_change()];
    }
    let n = match owners(app).await {
        Ok(n) => n,
        Err(e) => return e,
    };
    if let Err(code) = owner_rule(req.caller, from, Some(to), n) {
        return deny(app, req.caller, CMD, None, code, o.context).await;
    }
    let summary = if narrows(from, to) {
        format!("Change {display}'s instance role from {from} to {to}. Their sessions end.")
    } else {
        format!("Change {display}'s instance role from {from} to {to}.")
    };
    let op = pending(move |app, caller| async move {
        let step: Step = async {
            if let Err(code) = check_admin(&caller) {
                return Ok(Err(Fail::Denied(code)));
            }
            let mut tx = app.db.begin().await?;
            let owners = lock_owners(&mut tx).await?;
            let from = match user_row(&mut tx, user).await? {
                Some((_, r, false)) => r,
                _ => {
                    return Ok(Err(Fail::Op(
                        "not_found",
                        "That user does not exist.".into(),
                    )));
                }
            };
            if from == to {
                return Ok(Err(Fail::Op("no_change", NO_CHANGE.into())));
            }
            if let Err(code) = owner_rule(&caller, from, Some(to), owners) {
                return Ok(Err(Fail::Denied(code)));
            }
            sqlx::query("update users set instance_role = $2 where id = $1")
                .bind(user)
                .bind(to.as_str())
                .execute(&mut *tx)
                .await?;
            let bumped = if narrows(from, to) {
                vec![user]
            } else {
                vec![]
            };
            bump(&mut tx, &bumped, &caller, o.session.as_deref()).await?;
            let target = json!({
                "user": user, "from": from.as_str(), "to": to.as_str(), "epoch_bumped": bumped,
            });
            record(&mut tx, &caller, ACTION, None, target, &o.context).await?;
            tx.commit().await?;
            Ok(Ok(bumped))
        }
        .await;
        if let Err(events) = settle(&app, &caller, ACTION, None, &o, step).await {
            return events;
        }
        users(&app).await
    });
    preview(app, req, draft(ACTION, summary, vec![display], true), op)
}

const NO_CHANGE: &str = "The user already has that role.";

fn no_change() -> Event {
    Event::op_error("no_change", NO_CHANGE)
}

async fn user_remove(app: &AppState, req: &Request<'_>, user: Uuid) -> Vec<Event> {
    const CMD: &str = "preview_user_remove";
    const ACTION: &str = "user_remove";
    let o = origin(req);
    if let Err(code) = check_admin(req.caller) {
        return deny(app, req.caller, CMD, None, code, o.context).await;
    }
    if user == req.caller.user.id {
        return deny(app, req.caller, CMD, None, "self", o.context).await;
    }
    let (display, from) = match read(app, async |c| user_row(c, user).await).await {
        Err(e) => return e,
        Ok(None | Some((_, _, true))) => return vec![not_found("user")],
        Ok(Some((n, r, false))) => (n, r),
    };
    let n = match owners(app).await {
        Ok(n) => n,
        Err(e) => return e,
    };
    if let Err(code) = owner_rule(req.caller, from, None, n) {
        return deny(app, req.caller, CMD, None, code, o.context).await;
    }
    let summary = format!(
        "Remove {display}: they are signed out everywhere, leave every team, and their API \
         tokens are revoked. Their audit history stays."
    );
    let op = pending(move |app, caller| async move {
        let step: Step = async {
            if let Err(code) = check_admin(&caller) {
                return Ok(Err(Fail::Denied(code)));
            }
            if user == caller.user.id {
                return Ok(Err(Fail::Denied("self")));
            }
            let mut tx = app.db.begin().await?;
            let owners = lock_owners(&mut tx).await?;
            let from = match user_row(&mut tx, user).await? {
                Some((_, r, false)) => r,
                _ => {
                    return Ok(Err(Fail::Op(
                        "not_found",
                        "That user does not exist.".into(),
                    )));
                }
            };
            if let Err(code) = owner_rule(&caller, from, None, owners) {
                return Ok(Err(Fail::Denied(code)));
            }
            sqlx::query("update users set disabled_at = now() where id = $1")
                .bind(user)
                .execute(&mut *tx)
                .await?;
            bump(&mut tx, &[user], &caller, None).await?;
            let teams = sqlx::query("delete from team_members where user_id = $1")
                .bind(user)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            let tokens = sqlx::query(
                "update api_tokens set revoked_at = now() \
                 where owner_user_id = $1 and revoked_at is null",
            )
            .bind(user)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            sqlx::query("delete from sessions where user_id = $1")
                .bind(user)
                .execute(&mut *tx)
                .await?;
            let target = json!({
                "user": user, "role": from.as_str(), "teams_left": teams,
                "tokens_revoked": tokens, "epoch_bumped": [user],
            });
            record(&mut tx, &caller, ACTION, None, target, &o.context).await?;
            tx.commit().await?;
            Ok(Ok(vec![user]))
        }
        .await;
        if let Err(events) = settle(&app, &caller, ACTION, None, &o, step).await {
            return events;
        }
        users(&app).await
    });
    preview(app, req, draft(ACTION, summary, vec![display], true), op)
}
