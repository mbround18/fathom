//! Repository bindings (spec 055 T048, data-model `repo_bindings`).
//!
//! A binding ties a project to one branch of one repository. Creating one test-fetches first, so
//! the preview shows the head it found, the manifest's state and, in signed mode, which recent
//! commits are unsigned. Everything here needs `Administer` on the workspace, except reading
//! (`View`) and `SyncBinding` (`Operate`). Turning signed mode off needs step-up.
//!
//! Also the access helpers the receiver and credential commands share.

use keel_core::authz::{self, Capability, Subject};
use keel_core::domain::WorkspaceId;
use keel_core::manifest::Context;
use keel_core::ports::CommitId;
use keel_core::repo_url;
use keel_git::Mirror;
use keel_protocol::{
    ActiveStatus, BindingInfo, BindingSpec, BindingUpdate, Event, JobSummary, ManifestProblem,
    Transport,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::audit::{self, Decision, Entry};
use crate::dispatch::{PendingOp, Request};
use crate::identity::Caller;
use crate::preview::Draft;
use crate::serve::AppState;
use crate::sync::{self, Source, SyncError, Trigger};

// ---- Shared helpers (bindings, receivers, credentials) ----

pub(crate) fn internal() -> Vec<Event> {
    vec![Event::op_error(
        "internal",
        "Keel could not complete that. Try again.",
    )]
}

pub(crate) fn op_error(code: &str, message: &str) -> Vec<Event> {
    vec![Event::op_error(code, message)]
}

pub(crate) fn not_found(what: &str) -> Vec<Event> {
    op_error("not_found", &format!("That {what} does not exist."))
}

pub(crate) fn db_failed(e: &sqlx::Error) -> Vec<Event> {
    tracing::error!(error = %e, "database failed");
    internal()
}

pub(crate) fn ts(t: OffsetDateTime) -> String {
    t.format(&Rfc3339).unwrap_or_default()
}

/// A snake_case protocol enum from its database text.
pub(crate) fn enum_from<T: DeserializeOwned>(s: &str) -> Option<T> {
    serde_json::from_value(Value::String(s.to_owned())).ok()
}

/// A protocol enum's database text.
pub(crate) fn enum_str<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(Value::String(s)) => s,
        _ => String::new(),
    }
}

pub(crate) fn channel(req: &Request<'_>) -> Value {
    json!({ "channel": req.channel.as_str() })
}

/// Requires `cap` on `workspace`; a refusal is audited and answered `Denied`.
pub(crate) async fn require(
    app: &AppState,
    req: &Request<'_>,
    action: &str,
    cap: Capability,
    workspace: Uuid,
) -> Result<(), Vec<Event>> {
    let caller = req.caller;
    let refused = match &caller.token {
        Some(t) if t.workspace.0 != workspace => Some("token_scope"),
        _ => authz::require(&subject(caller), cap, WorkspaceId(workspace), &[])
            .err()
            .map(|r| r.code()),
    };
    let Some(reason) = refused else {
        return Ok(());
    };
    let actor = caller.actor();
    let result = async {
        let mut tx = app.db.begin().await?;
        audit::record(
            &mut tx,
            Entry {
                actor: &actor,
                action,
                workspace: Some(WorkspaceId(workspace)),
                target: json!({}),
                decision: Decision::Denied,
                reason: Some(reason),
                context: channel(req),
            },
        )
        .await?;
        tx.commit().await
    }
    .await;
    if let Err(e) = result {
        tracing::error!(error = %e, "audit write failed");
    }
    Err(vec![Event::Denied {
        action: action.to_owned(),
        reason: reason.to_owned(),
    }])
}

pub(crate) fn subject(caller: &Caller) -> Subject<'_> {
    Subject {
        instance_role: caller.user.instance_role,
        grants: &caller.grants,
    }
}

/// Stores a preview for this caller and answers with it.
pub(crate) fn preview(
    app: &AppState,
    req: &Request<'_>,
    draft: Draft,
    op: PendingOp,
) -> Vec<Event> {
    let key = req.caller.preview_key(req.session);
    vec![Event::Preview(app.previews.create(&key, draft, op))]
}

/// The workspace a binding lives in.
pub(crate) async fn binding_workspace(
    app: &AppState,
    binding: Uuid,
) -> Result<Option<Uuid>, sqlx::Error> {
    let row: Option<(Uuid,)> = sqlx::query_as(
        "select p.workspace_id from repo_bindings b join projects p on p.id = b.project_id \
         where b.id = $1",
    )
    .bind(binding)
    .fetch_optional(&app.db)
    .await?;
    Ok(row.map(|r| r.0))
}

// ---- Reading ----

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    project_id: Uuid,
    workspace_id: Uuid,
    url: String,
    branch: String,
    transport: String,
    credential_id: Option<Uuid>,
    known_host: Option<String>,
    poll_interval: Option<i32>,
    allow_force_push: bool,
    signed_mode: bool,
    override_quorum: i32,
    status: String,
    last_commit: Option<String>,
    last_synced_at: Option<OffsetDateTime>,
    last_sync_error: Option<String>,
    manifest_problems: Value,
}

const SELECT: &str = "select b.id, b.project_id, p.workspace_id, b.url, b.branch, b.transport, \
     b.credential_id, b.known_host, b.poll_interval, b.allow_force_push, b.signed_mode, \
     b.override_quorum, b.status, b.last_commit, b.last_synced_at, b.last_sync_error, \
     b.manifest_problems from repo_bindings b join projects p on p.id = b.project_id";

impl Row {
    fn info(self) -> BindingInfo {
        BindingInfo {
            id: self.id,
            project: self.project_id,
            workspace: self.workspace_id,
            url: self.url,
            branch: self.branch,
            transport: enum_from(&self.transport).unwrap_or(Transport::Https),
            credential: self.credential_id,
            known_host: self.known_host,
            poll_interval: self.poll_interval.and_then(|p| u32::try_from(p).ok()),
            allow_force_push: self.allow_force_push,
            signed_mode: self.signed_mode,
            override_quorum: u32::try_from(self.override_quorum).unwrap_or(1),
            status: enum_from(&self.status).unwrap_or(ActiveStatus::Disabled),
            last_commit: self.last_commit,
            last_synced_at: self.last_synced_at.map(ts),
            last_sync_error: self.last_sync_error,
            manifest_problems: serde_json::from_value::<Vec<ManifestProblem>>(
                self.manifest_problems,
            )
            .unwrap_or_default(),
        }
    }
}

async fn row(app: &AppState, binding: Uuid) -> Result<Option<Row>, sqlx::Error> {
    sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(format!("{SELECT} where b.id = $1")))
        .bind(binding)
        .fetch_optional(&app.db)
        .await
}

/// The binding and its active jobs.
pub(crate) async fn binding_event(
    app: &AppState,
    binding: Uuid,
) -> Result<Option<Event>, sqlx::Error> {
    let Some(r) = row(app, binding).await? else {
        return Ok(None);
    };
    let jobs: Vec<(String, Value, Option<String>, String)> = sqlx::query_as(
        "select name, definition, blocked_reason, defined_at_commit from jobs \
         where binding_id = $1 and removed_at_commit is null order by name",
    )
    .bind(binding)
    .fetch_all(&app.db)
    .await?;
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    let jobs = jobs
        .into_iter()
        .map(|(name, def, blocked, at)| JobSummary {
            name,
            playbook: text(&def, "playbook"),
            profiles: def
                .get("profiles")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|p| p.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            inventory: text(&def, "inventory"),
            blocked,
            defined_at_commit: at,
        })
        .collect();
    Ok(Some(Event::Binding {
        binding: r.info(),
        jobs,
    }))
}

pub async fn list(app: &AppState, req: &Request<'_>, workspace: Uuid) -> Vec<Event> {
    if let Err(e) = require(app, req, "list_bindings", Capability::View, workspace).await {
        return e;
    }
    let rows = sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(format!(
        "{SELECT} where p.workspace_id = $1 order by b.url, b.branch"
    )))
    .bind(workspace)
    .fetch_all(&app.db)
    .await;
    match rows {
        Ok(rows) => vec![Event::Bindings {
            items: rows.into_iter().map(Row::info).collect(),
        }],
        Err(e) => db_failed(&e),
    }
}

pub async fn get(app: &AppState, req: &Request<'_>, binding: Uuid) -> Vec<Event> {
    let ws = match binding_workspace(app, binding).await {
        Ok(Some(ws)) => ws,
        Ok(None) => return not_found("binding"),
        Err(e) => return db_failed(&e),
    };
    if let Err(e) = require(app, req, "get_binding", Capability::View, ws).await {
        return e;
    }
    match binding_event(app, binding).await {
        Ok(Some(ev)) => vec![ev],
        Ok(None) => not_found("binding"),
        Err(e) => db_failed(&e),
    }
}

// ---- Validation ----

/// Checks the credential belongs to `workspace` and fits `transport`.
async fn check_credential(
    app: &AppState,
    workspace: Uuid,
    transport: Transport,
    credential: Option<Uuid>,
    known_host: Option<&str>,
) -> Result<(), Vec<Event>> {
    if transport == Transport::Ssh {
        if known_host.is_none() {
            return Err(op_error(
                "known_host_required",
                "SSH needs the server's host key pinned (a known_hosts line).",
            ));
        }
        if credential.is_none() {
            return Err(op_error(
                "credential_unavailable",
                "SSH needs a git_ssh_key credential.",
            ));
        }
    }
    let Some(id) = credential else {
        return Ok(());
    };
    let row: Option<(Uuid, String)> =
        sqlx::query_as("select workspace_id, kind from credentials where id = $1")
            .bind(id)
            .fetch_optional(&app.db)
            .await
            .map_err(|e| db_failed(&e))?;
    let want = match transport {
        Transport::Https => "git_https_token",
        Transport::Ssh => "git_ssh_key",
    };
    match row {
        Some((ws, kind)) if ws == workspace && kind == want => Ok(()),
        _ => Err(op_error(
            "credential_unavailable",
            &format!("The credential must be a {want} in this workspace."),
        )),
    }
}

fn check_numbers(poll_interval: Option<u32>, override_quorum: u32) -> Result<(), Vec<Event>> {
    if let Some(p) = poll_interval
        && !(60..=86_400).contains(&p)
    {
        return Err(op_error(
            "poll_interval_invalid",
            "The poll interval is between 60 seconds and 24 hours.",
        ));
    }
    if !(1..=16).contains(&override_quorum) {
        return Err(op_error(
            "override_quorum_invalid",
            "The override quorum is between 1 and 16.",
        ));
    }
    Ok(())
}

pub(crate) fn url_key(app: &AppState, url: &str) -> Option<String> {
    repo_url::normalize(url).or_else(|| app.repos.allow_local.then(|| url.to_owned()))
}

fn short(commit: &str) -> &str {
    commit.get(..12).unwrap_or(commit)
}

// ---- Create ----

pub async fn preview_create(app: &AppState, req: &Request<'_>, spec: BindingSpec) -> Vec<Event> {
    const ACTION: &str = "preview_binding_create";
    let project: Option<(Uuid, String)> =
        match sqlx::query_as("select workspace_id, slug from projects where id = $1")
            .bind(spec.project)
            .fetch_optional(&app.db)
            .await
        {
            Ok(r) => r,
            Err(e) => return db_failed(&e),
        };
    let Some((ws, slug)) = project else {
        return not_found("project");
    };
    if let Err(e) = require(app, req, ACTION, Capability::Administer, ws).await {
        return e;
    }
    let Some(key) = url_key(app, spec.url.as_str()) else {
        return op_error(
            "url_invalid",
            "Use an https:// or SSH repository URL, such as git@host:owner/repo.git.",
        );
    };
    if let Err(e) = check_numbers(spec.poll_interval, spec.override_quorum) {
        return e;
    }
    let known_host = spec.known_host.as_ref().map(|k| k.as_str().to_owned());
    if let Err(e) = check_credential(
        app,
        ws,
        spec.transport,
        spec.credential,
        known_host.as_deref(),
    )
    .await
    {
        return e;
    }
    let exists: Result<Option<(Uuid,)>, _> = sqlx::query_as(
        "select id from repo_bindings where url = $1 and branch = $2 and project_id = $3",
    )
    .bind(spec.url.as_str())
    .bind(spec.branch.as_str())
    .bind(spec.project)
    .fetch_optional(&app.db)
    .await;
    match exists {
        Ok(Some(_)) => {
            return op_error(
                "binding_exists",
                "This project already binds that repository and branch.",
            );
        }
        Ok(None) => {}
        Err(e) => return db_failed(&e),
    }

    // The test fetch, into the mirror the binding will keep.
    let id = Uuid::now_v7();
    let credential = match spec.credential {
        Some(c) => match crate::credentials::open(app, c).await {
            Ok(v) => Some(v),
            Err(_) => {
                return op_error(
                    SyncError::CredentialUnavailable.code(),
                    SyncError::CredentialUnavailable.message(),
                );
            }
        },
        None => None,
    };
    let src = Source {
        url: spec.url.as_str().to_owned(),
        branch: spec.branch.as_str().to_owned(),
        transport: enum_str(&spec.transport),
        known_host: known_host.clone(),
        credential,
    };
    let dir = sync::data_dir(app);
    let allow_local = app.repos.allow_local;
    let recent = if spec.signed_mode {
        sync::RECENT_COMMITS
    } else {
        1
    };
    let signed_mode = spec.signed_mode;
    let fetched = tokio::task::spawn_blocking(move || {
        let ctx = Context {
            signed_mode,
            ..Context::default()
        };
        let r = sync::fetch_blocking(&dir, id, &src, allow_local, None, &ctx, recent);
        if r.is_err() {
            let _ = Mirror::remove(&dir, id);
        }
        r
    })
    .await;
    let fetched = match fetched {
        Ok(Ok(f)) => f,
        Ok(Err(e)) => return op_error(e.code(), e.message()),
        Err(_) => return internal(),
    };

    let mut summary = format!(
        "Bind {} ({}) to project {slug}. Test fetch found {}",
        spec.url.as_str(),
        spec.branch.as_str(),
        short(&fetched.head.0),
    );
    if let Some(c) = fetched.recent.first() {
        summary.push_str(&format!(" \"{}\"", c.summary));
    }
    summary.push('.');
    match &fetched.manifest {
        Ok(m) => summary.push_str(&format!(" The manifest is valid: {} job(s).", m.jobs.len())),
        Err(issues) => summary.push_str(&format!(
            " The manifest has {} problem(s); no jobs run until it is fixed (first: {}: {}).",
            issues.len(),
            issues[0].at,
            issues[0].message,
        )),
    }
    if spec.signed_mode {
        let unsigned: Vec<&str> = fetched
            .recent
            .iter()
            .filter(|c| !c.signed)
            .map(|c| short(&c.id.0))
            .collect();
        if unsigned.is_empty() {
            summary.push_str(" Signed mode: every recent commit is signed.");
        } else {
            summary.push_str(&format!(
                " Signed mode: unsigned recent commits: {}.",
                unsigned.join(", ")
            ));
        }
    }
    let affected = vec![format!("{}@{}", spec.url.as_str(), spec.branch.as_str())];
    let ctx = channel(req);
    let op: PendingOp = Box::new(move |app: AppState, caller: Caller| {
        Box::pin(async move { create(&app, &caller, id, ws, &key, &spec, ctx).await })
    });
    preview(
        app,
        req,
        Draft {
            action: "binding_create".to_owned(),
            summary,
            affected,
            step_up: false,
        },
        op,
    )
}

async fn create(
    app: &AppState,
    caller: &Caller,
    id: Uuid,
    ws: Uuid,
    key: &str,
    spec: &BindingSpec,
    ctx: Value,
) -> Vec<Event> {
    let result: Result<bool, sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        let inserted = sqlx::query(
            "insert into repo_bindings (id, project_id, url, url_key, branch, transport, \
             credential_id, known_host, allow_force_push, poll_interval, signed_mode, \
             override_quorum, created_by) \
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
             on conflict (url, branch, project_id) do nothing",
        )
        .bind(id)
        .bind(spec.project)
        .bind(spec.url.as_str())
        .bind(key)
        .bind(spec.branch.as_str())
        .bind(enum_str(&spec.transport))
        .bind(spec.credential)
        .bind(spec.known_host.as_ref().map(|k| k.as_str()))
        .bind(spec.allow_force_push)
        .bind(spec.poll_interval.and_then(|p| i32::try_from(p).ok()))
        .bind(spec.signed_mode)
        .bind(i32::try_from(spec.override_quorum).unwrap_or(1))
        .bind(caller.user.id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        if !inserted {
            return Ok(false);
        }
        audit::record(
            &mut tx,
            Entry {
                actor: &caller.actor(),
                action: "binding_created",
                workspace: Some(WorkspaceId(ws)),
                target: json!({
                    "binding": id, "project": spec.project, "url": spec.url.as_str(),
                    "branch": spec.branch.as_str(),
                }),
                decision: Decision::Allowed,
                reason: None,
                context: json!({
                    "transport": enum_str(&spec.transport),
                    "credential": spec.credential,
                    "signed_mode": spec.signed_mode,
                    "poll_interval": spec.poll_interval,
                    "allow_force_push": spec.allow_force_push,
                    "override_quorum": spec.override_quorum,
                    "request": ctx,
                }),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(true)
    }
    .await;
    match result {
        Ok(true) => {}
        Ok(false) => {
            return op_error(
                "binding_exists",
                "This project already binds that repository and branch.",
            );
        }
        Err(e) => return db_failed(&e),
    }
    app.repos.poll_wake.notify_one();
    let mut out = Vec::new();
    if let Err(e) = sync::sync_binding(app, id, Trigger::Create(caller.actor())).await {
        out.push(Event::op_error(e.code(), e.message()));
    }
    match binding_event(app, id).await {
        Ok(Some(ev)) => out.insert(0, ev),
        Ok(None) => {}
        Err(e) => return db_failed(&e),
    }
    out
}

// ---- Update, signed mode, disable, enable, remove ----

pub async fn preview_update(app: &AppState, req: &Request<'_>, upd: BindingUpdate) -> Vec<Event> {
    const ACTION: &str = "preview_binding_update";
    let r = match row(app, upd.binding).await {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("binding"),
        Err(e) => return db_failed(&e),
    };
    if let Err(e) = require(app, req, ACTION, Capability::Administer, r.workspace_id).await {
        return e;
    }
    if let Err(e) = check_numbers(upd.poll_interval, upd.override_quorum) {
        return e;
    }
    let transport = enum_from(&r.transport).unwrap_or(Transport::Https);
    let known_host = upd.known_host.as_ref().map(|k| k.as_str().to_owned());
    if let Err(e) = check_credential(
        app,
        r.workspace_id,
        transport,
        upd.credential,
        known_host.as_deref(),
    )
    .await
    {
        return e;
    }
    let mut changes = Vec::new();
    if upd.branch.as_str() != r.branch {
        changes.push(format!("branch {} → {}", r.branch, upd.branch.as_str()));
    }
    if upd.credential != r.credential_id {
        changes.push("credential changed".to_owned());
    }
    if known_host != r.known_host {
        changes.push("host key changed".to_owned());
    }
    let poll = upd.poll_interval.and_then(|p| i32::try_from(p).ok());
    if poll != r.poll_interval {
        changes.push(match upd.poll_interval {
            Some(p) => format!("poll every {p}s"),
            None => "polling off".to_owned(),
        });
    }
    if upd.allow_force_push != r.allow_force_push {
        changes.push(format!("allow force-push: {}", upd.allow_force_push));
    }
    if i64::from(upd.override_quorum) != i64::from(r.override_quorum) {
        changes.push(format!("override quorum {}", upd.override_quorum));
    }
    if changes.is_empty() {
        return op_error("no_change", "Nothing would change.");
    }
    let summary = format!(
        "Update the binding {} ({}): {}.",
        r.url,
        r.branch,
        changes.join("; ")
    );
    let affected = vec![format!("{}@{}", r.url, r.branch)];
    let (ws, branch_changed, ctx) = (
        r.workspace_id,
        upd.branch.as_str() != r.branch,
        channel(req),
    );
    let op: PendingOp = Box::new(move |app: AppState, caller: Caller| {
        Box::pin(async move { update(&app, &caller, ws, &upd, branch_changed, changes, ctx).await })
    });
    preview(
        app,
        req,
        Draft {
            action: "binding_update".to_owned(),
            summary,
            affected,
            step_up: false,
        },
        op,
    )
}

async fn update(
    app: &AppState,
    caller: &Caller,
    ws: Uuid,
    upd: &BindingUpdate,
    branch_changed: bool,
    changes: Vec<String>,
    ctx: Value,
) -> Vec<Event> {
    let result: Result<u64, sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        let n = sqlx::query(
            "update repo_bindings set branch = $2, credential_id = $3, known_host = $4, \
             poll_interval = $5, allow_force_push = $6, override_quorum = $7, \
             last_commit = case when $8 then null else last_commit end where id = $1",
        )
        .bind(upd.binding)
        .bind(upd.branch.as_str())
        .bind(upd.credential)
        .bind(upd.known_host.as_ref().map(|k| k.as_str()))
        .bind(upd.poll_interval.and_then(|p| i32::try_from(p).ok()))
        .bind(upd.allow_force_push)
        .bind(i32::try_from(upd.override_quorum).unwrap_or(1))
        .bind(branch_changed)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if n == 1 {
            audit::record(
                &mut tx,
                Entry {
                    actor: &caller.actor(),
                    action: "binding_updated",
                    workspace: Some(WorkspaceId(ws)),
                    target: json!({ "binding": upd.binding }),
                    decision: Decision::Allowed,
                    reason: None,
                    context: json!({ "changes": changes, "request": ctx }),
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(n)
    }
    .await;
    match result {
        Ok(1) => {}
        Ok(_) => return not_found("binding"),
        Err(e) => return db_failed(&e),
    }
    app.repos.poll_wake.notify_one();
    match binding_event(app, upd.binding).await {
        Ok(Some(ev)) => vec![ev],
        Ok(None) => not_found("binding"),
        Err(e) => db_failed(&e),
    }
}

/// Signed mode on or off. Off needs step-up; on reports the unsigned recent commits.
pub async fn preview_signed_mode(
    app: &AppState,
    req: &Request<'_>,
    binding: Uuid,
    on: bool,
) -> Vec<Event> {
    const ACTION: &str = "preview_signed_mode";
    let r = match row(app, binding).await {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("binding"),
        Err(e) => return db_failed(&e),
    };
    if let Err(e) = require(app, req, ACTION, Capability::Administer, r.workspace_id).await {
        return e;
    }
    if r.signed_mode == on {
        return op_error("no_change", "Nothing would change.");
    }
    let mut summary = if on {
        format!(
            "Turn signed mode on for {} ({}): runs need every new commit signed by a trusted key.",
            r.url, r.branch
        )
    } else {
        format!(
            "Turn signed mode off for {} ({}): hosts stop requiring signed commits from it.",
            r.url, r.branch
        )
    };
    if on && let Some(head) = r.last_commit.clone() {
        let dir = sync::data_dir(app);
        let recent = tokio::task::spawn_blocking(move || {
            Mirror::open(&dir, binding)
                .and_then(|m| m.recent_commits(&CommitId(head), sync::RECENT_COMMITS))
        })
        .await;
        if let Ok(Ok(recent)) = recent {
            let unsigned: Vec<String> = recent
                .iter()
                .filter(|c| !c.signed)
                .map(|c| short(&c.id.0).to_owned())
                .collect();
            if !unsigned.is_empty() {
                summary.push_str(&format!(
                    " Unsigned recent commits: {}.",
                    unsigned.join(", ")
                ));
            }
        }
    }
    let affected = vec![format!("{}@{}", r.url, r.branch)];
    let (ws, ctx) = (r.workspace_id, channel(req));
    let op: PendingOp = Box::new(move |app: AppState, caller: Caller| {
        Box::pin(async move {
            let action = if on {
                "signed_mode_on"
            } else {
                "signed_mode_off"
            };
            set_column(
                &app,
                &caller,
                ws,
                binding,
                Change::SignedMode(on),
                action,
                ctx,
            )
            .await
        })
    });
    preview(
        app,
        req,
        Draft {
            action: if on {
                "signed_mode_on"
            } else {
                "signed_mode_off"
            }
            .to_owned(),
            summary,
            affected,
            step_up: !on,
        },
        op,
    )
}

pub async fn preview_status(
    app: &AppState,
    req: &Request<'_>,
    binding: Uuid,
    active: bool,
) -> Vec<Event> {
    let action = if active {
        "preview_binding_enable"
    } else {
        "preview_binding_disable"
    };
    let r = match row(app, binding).await {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("binding"),
        Err(e) => return db_failed(&e),
    };
    if let Err(e) = require(app, req, action, Capability::Administer, r.workspace_id).await {
        return e;
    }
    let status = if active { "active" } else { "disabled" };
    if r.status == status {
        return op_error("no_change", "Nothing would change.");
    }
    let summary = if active {
        format!(
            "Enable the binding {} ({}): deliveries and polling resume.",
            r.url, r.branch
        )
    } else {
        // Runs arrive with US2; until then there is nothing queued or pending to cancel.
        format!(
            "Disable the binding {} ({}): deliveries are refused, polling stops, and its queued \
             and pending runs are cancelled (binding_disabled). Runs to cancel: none.",
            r.url, r.branch
        )
    };
    let affected = vec![format!("{}@{}", r.url, r.branch)];
    let (ws, ctx) = (r.workspace_id, channel(req));
    let audited = if active {
        "binding_enabled"
    } else {
        "binding_disabled"
    };
    let op: PendingOp = Box::new(move |app: AppState, caller: Caller| {
        Box::pin(async move {
            set_column(
                &app,
                &caller,
                ws,
                binding,
                Change::Status(status),
                audited,
                ctx,
            )
            .await
        })
    });
    preview(
        app,
        req,
        Draft {
            action: if active {
                "binding_enable"
            } else {
                "binding_disable"
            }
            .to_owned(),
            summary,
            affected,
            step_up: false,
        },
        op,
    )
}

/// A one-field change to a binding.
enum Change {
    SignedMode(bool),
    Status(&'static str),
}

/// Applies `change`, audits it and answers with the binding.
async fn set_column(
    app: &AppState,
    caller: &Caller,
    ws: Uuid,
    binding: Uuid,
    change: Change,
    action: &'static str,
    ctx: Value,
) -> Vec<Event> {
    let result: Result<u64, sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        let q = match change {
            Change::SignedMode(on) => {
                sqlx::query("update repo_bindings set signed_mode = $2 where id = $1")
                    .bind(binding)
                    .bind(on)
            }
            Change::Status(s) => sqlx::query("update repo_bindings set status = $2 where id = $1")
                .bind(binding)
                .bind(s),
        };
        let n = q.execute(&mut *tx).await?.rows_affected();
        if n == 1 {
            audit::record(
                &mut tx,
                Entry {
                    actor: &caller.actor(),
                    action,
                    workspace: Some(WorkspaceId(ws)),
                    target: json!({ "binding": binding }),
                    decision: Decision::Allowed,
                    reason: None,
                    context: ctx,
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(n)
    }
    .await;
    match result {
        Ok(1) => {}
        Ok(_) => return not_found("binding"),
        Err(e) => return db_failed(&e),
    }
    app.repos.poll_wake.notify_one();
    match binding_event(app, binding).await {
        Ok(Some(ev)) => vec![ev],
        Ok(None) => not_found("binding"),
        Err(e) => db_failed(&e),
    }
}

pub async fn preview_remove(app: &AppState, req: &Request<'_>, binding: Uuid) -> Vec<Event> {
    const ACTION: &str = "preview_binding_remove";
    let r = match row(app, binding).await {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("binding"),
        Err(e) => return db_failed(&e),
    };
    if let Err(e) = require(app, req, ACTION, Capability::Administer, r.workspace_id).await {
        return e;
    }
    let summary = format!(
        "Remove the binding {} ({}): its jobs, receiver links and credential grants go with it, \
         and its mirror is deleted.",
        r.url, r.branch
    );
    let affected = vec![format!("{}@{}", r.url, r.branch)];
    let (ws, ctx) = (r.workspace_id, channel(req));
    let op: PendingOp = Box::new(move |app: AppState, caller: Caller| {
        Box::pin(async move { remove(&app, &caller, ws, binding, ctx).await })
    });
    preview(
        app,
        req,
        Draft {
            action: "binding_remove".to_owned(),
            summary,
            affected,
            step_up: false,
        },
        op,
    )
}

async fn remove(
    app: &AppState,
    caller: &Caller,
    ws: Uuid,
    binding: Uuid,
    ctx: Value,
) -> Vec<Event> {
    let result: Result<u64, sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        let n = sqlx::query("delete from repo_bindings where id = $1")
            .bind(binding)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if n == 1 {
            audit::record(
                &mut tx,
                Entry {
                    actor: &caller.actor(),
                    action: "binding_removed",
                    workspace: Some(WorkspaceId(ws)),
                    target: json!({ "binding": binding }),
                    decision: Decision::Allowed,
                    reason: None,
                    context: ctx,
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(n)
    }
    .await;
    match result {
        Ok(1) => {}
        Ok(_) => return not_found("binding"),
        Err(e) => return db_failed(&e),
    }
    let dir = sync::data_dir(app);
    let lock_free = tokio::task::spawn_blocking(move || Mirror::remove(&dir, binding)).await;
    if !matches!(lock_free, Ok(Ok(()))) {
        tracing::warn!(binding = %binding, "removing the mirror failed");
    }
    app.repos.poll_wake.notify_one();
    Vec::new()
}

/// `SyncBinding`: fetch now (`Operate`).
pub async fn sync_now(app: &AppState, req: &Request<'_>, binding: Uuid) -> Vec<Event> {
    let ws = match binding_workspace(app, binding).await {
        Ok(Some(ws)) => ws,
        Ok(None) => return not_found("binding"),
        Err(e) => return db_failed(&e),
    };
    if let Err(e) = require(app, req, "sync_binding", Capability::Operate, ws).await {
        return e;
    }
    match sync::sync_binding(app, binding, Trigger::Manual(req.caller.actor())).await {
        Ok(s) => vec![Event::BindingSynced {
            binding,
            commit: s.commit,
            jobs_changed: s.jobs_changed,
            inventories_changed: false,
        }],
        Err(e) => op_error(e.code(), e.message()),
    }
}
