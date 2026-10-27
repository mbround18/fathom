//! Repository sync (spec 055 T051, contracts/receivers.md "After a sync", research R5).
//!
//! A sync fetches the binding's branch into its mirror, classifies the new head against the
//! last one, and reads `.keel/jobs.yaml` at the fetched head (never at a commit a payload
//! named). A valid manifest replaces the binding's jobs and declares its profiles; an invalid one
//! leaves the previous jobs in place and records `manifest_invalid` with the problems.
//!
//! Polling runs only for bindings with a `poll_interval`: with none, the poller waits for a
//! binding change and sets no timer.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use keel_core::domain::WorkspaceId;
use keel_core::manifest::{self, Context, Manifest, ManifestIssue};
use keel_core::ports::{CommitId, GitError, GitSource};
use keel_git::{Ancestry, Auth, CommitInfo, FetchRequest, Mirror, MirrorError, TreeAt};
use keel_protocol::Event;
use secrecy::{ExposeSecret, SecretSlice};
use serde_json::{Value, json};
use sqlx::PgConnection;
use time::OffsetDateTime;
use tokio::sync::{Notify, broadcast};
use uuid::Uuid;

use crate::audit::{self, Actor, Decision, Entry};
use crate::seal::MasterKey;
use crate::serve::AppState;

/// How many recent commits a signed-mode preview inspects.
pub const RECENT_COMMITS: usize = 20;
/// The wait after the poller could not read the database.
const POLL_RETRY: Duration = Duration::from_secs(60);

/// Repository state shared by the binding, receiver and hook handlers.
#[derive(Clone)]
pub struct Repos {
    /// Opens sealed credentials and receiver secrets. Without it those fail closed.
    pub master_key: Option<Arc<MasterKey>>,
    /// Allow local paths and `file://` URLs (tests only).
    pub allow_local: bool,
    /// Pushed events, with the workspace whose viewers may see them.
    pub events: broadcast::Sender<(WorkspaceId, Event)>,
    /// Wakes the poller after a binding changed.
    pub poll_wake: Arc<Notify>,
    /// One sync at a time per binding (its mirror is not shared).
    locks: Arc<Mutex<HashMap<Uuid, Arc<tokio::sync::Mutex<()>>>>>,
    /// Per-receiver and per-address limits on `POST /hooks/{id}`.
    pub hook_limits: Arc<crate::hooks::Limiter>,
}

impl Default for Repos {
    fn default() -> Self {
        Self {
            master_key: None,
            allow_local: false,
            events: broadcast::channel(256).0,
            poll_wake: Arc::new(Notify::new()),
            locks: Arc::default(),
            hook_limits: Arc::new(crate::hooks::Limiter::default()),
        }
    }
}

impl Repos {
    fn lock(&self, binding: Uuid) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
        locks.retain(|_, l| Arc::strong_count(l) > 1);
        locks.entry(binding).or_default().clone()
    }

    /// Sends a pushed event to the workspace's viewers (none listening is fine).
    pub fn push(&self, workspace: Uuid, event: Event) {
        let _ = self.events.send((WorkspaceId(workspace), event));
    }
}

/// Why a sync did not complete. [`SyncError::code`] is stored in `last_sync_error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncError {
    NotFound,
    Disabled,
    /// The binding's credential is missing, the wrong kind, or cannot be opened.
    CredentialUnavailable,
    /// The URL is not allowed (scheme, local path) or malformed.
    UrlRefused,
    BranchNotFound,
    KnownHostRequired,
    FetchFailed,
    Internal,
}

impl SyncError {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            SyncError::NotFound => "not_found",
            SyncError::Disabled => "binding_disabled",
            SyncError::CredentialUnavailable => "credential_unavailable",
            SyncError::UrlRefused => "url_refused",
            SyncError::BranchNotFound => "branch_not_found",
            SyncError::KnownHostRequired => "known_host_required",
            SyncError::FetchFailed => "fetch_failed",
            SyncError::Internal => "internal",
        }
    }

    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            SyncError::NotFound => "That binding does not exist.",
            SyncError::Disabled => "That binding is disabled.",
            SyncError::CredentialUnavailable => {
                "The binding's credential is missing or cannot be used for this transport."
            }
            SyncError::UrlRefused => "Keel does not fetch from that URL.",
            SyncError::BranchNotFound => "That branch does not exist on the remote.",
            SyncError::KnownHostRequired => "SSH needs the host key pinned.",
            SyncError::FetchFailed => {
                "Keel could not fetch from the repository. Check the URL, credential and host key."
            }
            SyncError::Internal => "Keel could not complete that. Try again.",
        }
    }
}

impl From<MirrorError> for SyncError {
    fn from(e: MirrorError) -> Self {
        // The detail can carry remote output; keep it out of logs at info and of replies.
        tracing::debug!(kind = mirror_kind(&e), "mirror operation failed");
        match e {
            MirrorError::UrlRefused(_) | MirrorError::BranchInvalid(_) => SyncError::UrlRefused,
            MirrorError::BranchNotFound(_) => SyncError::BranchNotFound,
            MirrorError::KnownHostRequired => SyncError::KnownHostRequired,
            MirrorError::Fetch(_) => SyncError::FetchFailed,
            MirrorError::Scratch(_) | MirrorError::Repo(_) | MirrorError::NotFound(_) => {
                SyncError::Internal
            }
        }
    }
}

const fn mirror_kind(e: &MirrorError) -> &'static str {
    match e {
        MirrorError::UrlRefused(_) => "url_refused",
        MirrorError::BranchInvalid(_) => "branch_invalid",
        MirrorError::BranchNotFound(_) => "branch_not_found",
        MirrorError::KnownHostRequired => "known_host_required",
        MirrorError::Scratch(_) => "scratch",
        MirrorError::Fetch(_) => "fetch",
        MirrorError::Repo(_) => "repo",
        MirrorError::NotFound(_) => "not_found",
    }
}

/// What to fetch, and with which key material.
pub struct Source {
    pub url: String,
    pub branch: String,
    /// `https` or `ssh`.
    pub transport: String,
    pub known_host: Option<String>,
    /// The opened credential: (kind, value).
    pub credential: Option<(String, SecretSlice<u8>)>,
}

/// What a fetch found.
pub struct Fetched {
    pub head: CommitId,
    pub ancestry: Ancestry,
    pub manifest: Result<Manifest, Vec<ManifestIssue>>,
    /// The most recent commits, newest first (only when asked for).
    pub recent: Vec<CommitInfo>,
}

/// Fetches into `binding`'s mirror and reads the manifest at the fetched head. Blocking.
///
/// # Errors
///
/// The fetch failed or the credential does not fit the transport.
pub fn fetch_blocking(
    data_dir: &std::path::Path,
    binding: Uuid,
    src: &Source,
    allow_local: bool,
    previous: Option<&CommitId>,
    ctx: &Context,
    recent: usize,
) -> Result<Fetched, SyncError> {
    let token_parts;
    let auth = match (src.transport.as_str(), &src.credential) {
        ("https", None) => Auth::None,
        ("https", Some((kind, value))) if kind == "git_https_token" => {
            let text = std::str::from_utf8(value.expose_secret())
                .map_err(|_| SyncError::CredentialUnavailable)?
                .trim();
            // `username:token`, or a bare token (username `x-access-token`).
            token_parts = match text.split_once(':') {
                Some((u, t)) if !u.is_empty() && !t.is_empty() => (Some(u), t),
                _ => (None, text),
            };
            Auth::HttpsToken {
                username: token_parts.0,
                token: token_parts.1,
            }
        }
        ("ssh", Some((kind, value))) if kind == "git_ssh_key" => Auth::SshKey {
            private_key: value.expose_secret(),
            known_host: src
                .known_host
                .as_deref()
                .ok_or(SyncError::KnownHostRequired)?,
        },
        ("ssh", None) => return Err(SyncError::CredentialUnavailable),
        _ => return Err(SyncError::CredentialUnavailable),
    };
    let mirror = Mirror::open(data_dir, binding)?;
    let scratch = data_dir.join("scratch");
    std::fs::create_dir_all(&scratch).map_err(|_| SyncError::Internal)?;
    let head = mirror.fetch(&FetchRequest {
        url: &src.url,
        branch: &src.branch,
        auth,
        scratch: &scratch,
        allow_local,
    })?;
    let ancestry = mirror.classify(previous, &head)?;
    let manifest = read_manifest(&mirror, &head, ctx);
    let recent = if recent > 0 {
        mirror.recent_commits(&head, recent)?
    } else {
        Vec::new()
    };
    Ok(Fetched {
        head,
        ancestry,
        manifest,
        recent,
    })
}

/// `.keel/jobs.yaml` at `head`, validated against the tree at `head`.
fn read_manifest(
    mirror: &Mirror,
    head: &CommitId,
    ctx: &Context,
) -> Result<Manifest, Vec<ManifestIssue>> {
    let bytes = match mirror.read_file(head, manifest::MANIFEST_PATH) {
        Ok(b) => b,
        Err(GitError::PathNotFound { .. }) => {
            return Err(vec![ManifestIssue {
                at: manifest::MANIFEST_PATH.to_owned(),
                message: "the manifest does not exist at this commit".to_owned(),
            }]);
        }
        Err(_) => {
            return Err(vec![ManifestIssue {
                at: manifest::MANIFEST_PATH.to_owned(),
                message: "the manifest could not be read".to_owned(),
            }]);
        }
    };
    let tree = TreeAt {
        mirror,
        commit: head,
    };
    manifest::validate(&bytes, &tree, ctx)
}

/// The mirrors' parent directory.
#[must_use]
pub fn data_dir(app: &AppState) -> PathBuf {
    app.cfg.data_dir.clone()
}

/// What started a sync (audited as context).
#[derive(Debug, Clone)]
pub enum Trigger {
    /// The binding was just created.
    Create(Actor),
    /// `SyncBinding`.
    Manual(Actor),
    /// An accepted delivery.
    Delivery { receiver: Uuid, delivery: Uuid },
    /// The poller.
    Poll,
}

impl Trigger {
    fn actor(&self) -> Actor {
        match self {
            Trigger::Create(a) | Trigger::Manual(a) => a.clone(),
            Trigger::Delivery { receiver, .. } => Actor::Receiver(*receiver),
            Trigger::Poll => Actor::System,
        }
    }

    const fn name(&self) -> &'static str {
        match self {
            Trigger::Create(_) => "create",
            Trigger::Manual(_) => "manual",
            Trigger::Delivery { .. } => "delivery",
            Trigger::Poll => "poll",
        }
    }
}

/// A completed sync.
#[derive(Debug, Clone)]
pub struct Synced {
    pub binding: Uuid,
    pub workspace: Uuid,
    pub commit: String,
    /// The `deliveries.sync_state` this sync means.
    pub state: &'static str,
    /// The binding's head moved to `commit`.
    pub moved: bool,
    pub jobs_changed: bool,
}

struct BindingRow {
    workspace: Uuid,
    url: String,
    branch: String,
    transport: String,
    credential: Option<Uuid>,
    known_host: Option<String>,
    allow_force_push: bool,
    signed_mode: bool,
    status: String,
    last_commit: Option<String>,
}

async fn load(app: &AppState, binding: Uuid) -> Result<Option<BindingRow>, sqlx::Error> {
    type Row = (
        Uuid,
        String,
        String,
        String,
        Option<Uuid>,
        Option<String>,
        bool,
        bool,
        String,
        Option<String>,
    );
    let row: Option<Row> = sqlx::query_as(
        "select p.workspace_id, b.url, b.branch, b.transport, b.credential_id, b.known_host, \
         b.allow_force_push, b.signed_mode, b.status, b.last_commit \
         from repo_bindings b join projects p on p.id = b.project_id where b.id = $1",
    )
    .bind(binding)
    .fetch_optional(&app.db)
    .await?;
    Ok(row.map(|r| BindingRow {
        workspace: r.0,
        url: r.1,
        branch: r.2,
        transport: r.3,
        credential: r.4,
        known_host: r.5,
        allow_force_push: r.6,
        signed_mode: r.7,
        status: r.8,
        last_commit: r.9,
    }))
}

/// Credential names granted to `binding`.
///
/// # Errors
///
/// The database failed.
pub async fn granted_names(app: &AppState, binding: Uuid) -> Result<BTreeSet<String>, sqlx::Error> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "select c.name from credential_grants g join credentials c on c.id = g.credential_id \
         where g.binding_id = $1",
    )
    .bind(binding)
    .fetch_all(&app.db)
    .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

/// Fetches `binding`, records the result and pushes `BindingSynced` when the head moved.
///
/// # Errors
///
/// The binding is gone or disabled, or the fetch failed (recorded in `last_sync_error`).
pub async fn sync_binding(
    app: &AppState,
    binding: Uuid,
    trigger: Trigger,
) -> Result<Synced, SyncError> {
    let lock = app.repos.lock(binding);
    let _held = lock.lock().await;
    let row = load(app, binding)
        .await
        .map_err(|e| db_error(&e))?
        .ok_or(SyncError::NotFound)?;
    if row.status != "active" {
        return Err(SyncError::Disabled);
    }
    let credential = match row.credential {
        Some(id) => Some(
            crate::credentials::open(app, id)
                .await
                .map_err(|_| SyncError::CredentialUnavailable)?,
        ),
        None => None,
    };
    let ctx = Context {
        signed_mode: row.signed_mode,
        granted_credentials: granted_names(app, binding)
            .await
            .map_err(|e| db_error(&e))?,
    };
    let src = Source {
        url: row.url.clone(),
        branch: row.branch.clone(),
        transport: row.transport.clone(),
        known_host: row.known_host.clone(),
        credential,
    };
    let dir = data_dir(app);
    let allow_local = app.repos.allow_local;
    let previous = row.last_commit.clone().map(CommitId);
    let fetched = tokio::task::spawn_blocking(move || {
        fetch_blocking(&dir, binding, &src, allow_local, previous.as_ref(), &ctx, 0)
    })
    .await
    .map_err(|_| SyncError::Internal)?;
    let fetched = match fetched {
        Ok(f) => f,
        Err(e) => {
            record_failure(app, binding, row.workspace, &trigger, e).await;
            return Err(e);
        }
    };
    let apply = match fetched.ancestry {
        Ancestry::First | Ancestry::FastForward | Ancestry::Same => true,
        Ancestry::Rewritten => row.allow_force_push,
        Ancestry::Behind => false,
    };
    let state = fetched.ancestry.sync_state();
    let commit = fetched.head.0.clone();
    let moved = apply && row.last_commit.as_deref() != Some(commit.as_str());
    let jobs_changed = record(
        app,
        binding,
        row.workspace,
        &trigger,
        &fetched,
        apply,
        state,
    )
    .await
    .map_err(|e| db_error(&e))?;
    if moved {
        app.repos.push(
            row.workspace,
            Event::BindingSynced {
                binding,
                commit: commit.clone(),
                jobs_changed,
                inventories_changed: false,
            },
        );
    }
    Ok(Synced {
        binding,
        workspace: row.workspace,
        commit,
        state,
        moved,
        jobs_changed,
    })
}

fn db_error(e: &sqlx::Error) -> SyncError {
    tracing::error!(error = %e, "sync: database failed");
    SyncError::Internal
}

async fn record_failure(
    app: &AppState,
    binding: Uuid,
    workspace: Uuid,
    trigger: &Trigger,
    e: SyncError,
) {
    let result = async {
        let mut tx = app.db.begin().await?;
        sqlx::query("update repo_bindings set last_sync_error = $2 where id = $1")
            .bind(binding)
            .bind(e.code())
            .execute(&mut *tx)
            .await?;
        audit::record(
            &mut tx,
            Entry {
                actor: &trigger.actor(),
                action: "binding_sync_failed",
                workspace: Some(WorkspaceId(workspace)),
                target: json!({ "binding": binding }),
                decision: Decision::Allowed,
                reason: Some(e.code()),
                context: json!({ "trigger": trigger.name() }),
            },
        )
        .await?;
        tx.commit().await
    }
    .await;
    if let Err(e) = result {
        tracing::error!(error = %e, "sync: recording a failure failed");
    }
}

/// Writes the outcome; returns whether the job set changed.
async fn record(
    app: &AppState,
    binding: Uuid,
    workspace: Uuid,
    trigger: &Trigger,
    fetched: &Fetched,
    apply: bool,
    state: &'static str,
) -> Result<bool, sqlx::Error> {
    let now = app.clock.now();
    let commit = fetched.head.0.as_str();
    let mut tx = app.db.begin().await?;
    let mut jobs_changed = false;
    let mut problems = 0usize;
    if apply {
        match &fetched.manifest {
            Ok(m) => {
                jobs_changed = upsert_jobs(&mut tx, binding, commit, m).await?;
                upsert_profiles(&mut tx, workspace, commit, m).await?;
                sqlx::query(
                    "update repo_bindings set last_commit = $2, last_synced_at = $3, \
                     last_sync_error = null, manifest_problems = '[]' where id = $1",
                )
                .bind(binding)
                .bind(commit)
                .bind(now)
                .execute(&mut *tx)
                .await?;
            }
            Err(issues) => {
                problems = issues.len();
                // The previous jobs stay active.
                sqlx::query(
                    "update repo_bindings set last_commit = $2, last_synced_at = $3, \
                     last_sync_error = 'manifest_invalid', manifest_problems = $4 where id = $1",
                )
                .bind(binding)
                .bind(commit)
                .bind(now)
                .bind(json!(issues))
                .execute(&mut *tx)
                .await?;
            }
        }
    } else {
        let error = (fetched.ancestry == Ancestry::Rewritten).then_some("history_rewritten");
        sqlx::query(
            "update repo_bindings set last_synced_at = $2, \
             last_sync_error = coalesce($3, last_sync_error) where id = $1",
        )
        .bind(binding)
        .bind(now)
        .bind(error)
        .execute(&mut *tx)
        .await?;
    }
    audit::record(
        &mut tx,
        Entry {
            actor: &trigger.actor(),
            action: "binding_synced",
            workspace: Some(WorkspaceId(workspace)),
            target: json!({ "binding": binding, "commit": commit }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({
                "trigger": trigger.name(),
                "state": state,
                "applied": apply,
                "manifest_problems": problems,
                "jobs_changed": jobs_changed,
            }),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(jobs_changed)
}

async fn upsert_jobs(
    tx: &mut PgConnection,
    binding: Uuid,
    commit: &str,
    m: &Manifest,
) -> Result<bool, sqlx::Error> {
    let existing: Vec<(String, Value, Option<String>)> = sqlx::query_as(
        "select name, definition, blocked_reason from jobs \
         where binding_id = $1 and removed_at_commit is null",
    )
    .bind(binding)
    .fetch_all(&mut *tx)
    .await?;
    let before: BTreeMap<String, (Value, Option<String>)> =
        existing.into_iter().map(|(n, d, b)| (n, (d, b))).collect();
    let mut after = BTreeMap::new();
    for (name, job) in &m.jobs {
        let def = serde_json::to_value(&job.definition).unwrap_or(Value::Null);
        let blocked = job.blocked_reason().map(str::to_owned);
        sqlx::query(
            "insert into jobs (id, binding_id, name, definition, blocked_reason, \
             defined_at_commit) values ($1, $2, $3, $4, $5, $6) \
             on conflict (binding_id, name) do update set definition = excluded.definition, \
             blocked_reason = excluded.blocked_reason, \
             defined_at_commit = case when jobs.definition = excluded.definition \
                 and jobs.removed_at_commit is null then jobs.defined_at_commit \
                 else excluded.defined_at_commit end, \
             removed_at_commit = null",
        )
        .bind(Uuid::now_v7())
        .bind(binding)
        .bind(name)
        .bind(&def)
        .bind(&blocked)
        .bind(commit)
        .execute(&mut *tx)
        .await?;
        after.insert(name.clone(), (def, blocked));
    }
    let names: Vec<String> = m.jobs.keys().cloned().collect();
    sqlx::query(
        "update jobs set removed_at_commit = $2 where binding_id = $1 \
         and removed_at_commit is null and not (name = any($3))",
    )
    .bind(binding)
    .bind(commit)
    .bind(&names)
    .execute(&mut *tx)
    .await?;
    Ok(before != after)
}

async fn upsert_profiles(
    tx: &mut PgConnection,
    workspace: Uuid,
    commit: &str,
    m: &Manifest,
) -> Result<(), sqlx::Error> {
    for (name, p) in &m.profiles {
        sqlx::query(
            "insert into profiles (id, workspace_id, name, description, declared_at_commit) \
             values ($1, $2, $3, $4, $5) on conflict (workspace_id, name) do update \
             set description = excluded.description, \
             declared_at_commit = excluded.declared_at_commit",
        )
        .bind(Uuid::now_v7())
        .bind(workspace)
        .bind(name)
        .bind(&p.description)
        .bind(commit)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

/// Polls bindings with a `poll_interval`. With none, it waits for a binding change.
pub fn spawn_poller(app: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let wait = match poll_once(&app).await {
                Ok(w) => w,
                Err(e) => {
                    tracing::warn!(error = %e, "poller: database unavailable");
                    Some(POLL_RETRY)
                }
            };
            let wake = app.repos.poll_wake.notified();
            match wait {
                Some(d) => {
                    tokio::select! {
                        () = tokio::time::sleep(d) => {}
                        () = wake => {}
                    }
                }
                None => wake.await,
            }
        }
    })
}

/// Syncs every due binding; returns how long until the next is due (`None`: none poll).
async fn poll_once(app: &AppState) -> Result<Option<Duration>, sqlx::Error> {
    let rows: Vec<(Uuid, i32, Option<OffsetDateTime>)> = sqlx::query_as(
        "select id, poll_interval, last_synced_at from repo_bindings \
         where status = 'active' and poll_interval is not null",
    )
    .fetch_all(&app.db)
    .await?;
    let now = app.clock.now();
    let mut next: Option<Duration> = None;
    for (id, interval, last) in rows {
        let interval = time::Duration::seconds(i64::from(interval));
        let due = last.map_or(now, |l| l + interval);
        let wait = if due <= now {
            if let Err(e) = sync_binding(app, id, Trigger::Poll).await {
                tracing::info!(binding = %id, code = e.code(), "poll sync failed");
            }
            interval
        } else {
            due - now
        };
        let wait = Duration::try_from(wait).unwrap_or(Duration::from_secs(60));
        next = Some(next.map_or(wait, |n| n.min(wait)));
    }
    Ok(next)
}
