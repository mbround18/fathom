//! Signed mode with reviewed overrides (spec 055 US8, T100 and T101; contracts/signed-mode.md,
//! research R19 and R20).
//!
//! **The early check** ([`early_check`]) runs when a signed-mode run is made. It groups the
//! run's hosts by their base for the job (the newest `host_revisions` row Keel recorded from a
//! host's ledger) and checks each group with keel-trust against the binding's mirror and the
//! workspace's trusted signers. A group that is not fully signed (or cannot be checked) becomes
//! a `review_groups` row, with its commits and their signature status and the files that change
//! trust or targeting (`.keel/`, repo inventories and their `group_vars`/`host_vars`, any diff
//! line naming `keel_runner_*`); the run waits in `needs_review`. The check is advisory: every
//! host verifies on its own and refuses what it cannot verify, so a mistake here can only send
//! a run to review that did not need it, never let one through.
//!
//! **Overrides**: `PreviewOverride` answers with the preview and the canonical challenge
//! (keel-trust [`OverrideScope`]) for the reviewer's passkey to sign. `Confirm` must carry that
//! assertion: a recent step-up does not stand in for it (dispatch asks [`Overrides`] for the
//! verifier). The approval keeps the comment, the challenge's SHA-256 and the assertion (sent to
//! hosts unchanged in `Approvals`); the audit log keeps neither the challenge nor the
//! assertion. A group is approved at its quorum (the larger of the binding's and its hosts'
//! `override_quorum`); when every group of a run is, the run moves on (`Overridden`).
//! Four-eyes, when the run's policy asks for it (or the run has no policy), refuses the person
//! who requested the run, an author of a reviewed commit, and the owner of a signer key that
//! signed one.
//!
//! Reads need `View`; overrides, denials and baselines `Approve` (a baseline also `Operate`);
//! signer changes `Administer` with a fresh passkey.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use keel_core::WorkspaceId;
use keel_core::authz::{self, Capability};
use keel_core::manifest::InventorySource;
use keel_core::ports::CommitId;
use keel_core::run::{RunEvent, RunState, RunStatus};
use keel_git::{Change, Mirror};
use keel_protocol::{
    Cmd, DiffFile, DiffPage, DiffStatus, Event, OverrideChallenge, PasskeyAssertion,
    PasskeyRefusedReason, PreviewId, ReviewCommit, ReviewDecision, ReviewFlag, ReviewFlagKind,
    ReviewInfo, ReviewReason, ReviewState, SignatureStatus, SignerInfo, SignerKind,
};
use keel_trust::challenge::{ApprovalKind, OverrideScope, format_utc, normalize_hosts};
use keel_trust::commit_check::{CheckError, verify_commit};
use keel_trust::object::{StoreError, parse_commit};
use keel_trust::{ObjectStore, Signers, Verdict};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::audit::{self, Decision, Entry};
use crate::bindings::{db_failed, enum_from, internal, not_found, op_error, require, subject, ts};
use crate::dispatch::{PendingOp, Request};
use crate::identity::Caller;
use crate::passkeys::{BoundAssertion, Passkeys};
use crate::preview::{Draft, StepUpVerifier};
use crate::serve::AppState;

/// The most commits a review group lists.
pub const MAX_REVIEW_COMMITS: usize = 200;
/// The most changed files whose diff is scanned for `keel_runner_*`; beyond it, the group is
/// flagged as not fully scanned.
pub const MAX_SCANNED_FILES: usize = 500;
/// Diff lines per `GetReviewDiff` page.
pub const DIFF_PAGE_LINES: usize = 400;
/// The longest an override may be valid (the host's `max_override_age`).
pub const MAX_OVERRIDE_SECS: i64 = 24 * 3600;
/// The most review groups `ListReviews` returns.
const LIST_LIMIT: i64 = 200;
/// How long an override challenge is kept: the preview's own TTL.
const CHALLENGE_TTL: time::Duration = crate::preview::TTL;
/// The variable prefix whose changes are flagged.
const RUNNER_VARS: &str = "keel_runner_";

// ---- override challenges ---------------------------------------------------------------------

struct Bound {
    challenge: [u8; 32],
    user: Uuid,
    slot: Arc<Mutex<Option<BoundAssertion>>>,
    created: OffsetDateTime,
}

/// The challenges of open override and baseline previews, by preview id. `Confirm` of one of
/// them is verified against its challenge only.
#[derive(Default)]
pub struct Overrides {
    pending: Mutex<HashMap<PreviewId, Bound>>,
}

impl Overrides {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<PreviewId, Bound>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn insert(&self, id: PreviewId, bound: Bound) {
        let now = bound.created;
        let mut map = self.lock();
        map.retain(|_, b| now - b.created < CHALLENGE_TTL);
        map.insert(id, bound);
    }

    /// The verifier for `preview`, when it is an override or baseline. It accepts only an
    /// assertion over that preview's challenge, and hands the assertion to the confirmed
    /// operation.
    pub fn verifier(&self, passkeys: &Arc<Passkeys>, preview: PreviewId) -> Option<BoundVerifier> {
        let map = self.lock();
        let b = map.get(&preview)?;
        Some(BoundVerifier {
            passkeys: passkeys.clone(),
            challenge: b.challenge,
            user: b.user,
            slot: b.slot.clone(),
        })
    }
}

/// Verifies an override's passkey assertion against its challenge (see [`Overrides`]).
pub struct BoundVerifier {
    passkeys: Arc<Passkeys>,
    challenge: [u8; 32],
    user: Uuid,
    slot: Arc<Mutex<Option<BoundAssertion>>>,
}

impl StepUpVerifier for BoundVerifier {
    fn verify<'a>(
        &'a self,
        session: &'a str,
        assertion: &'a PasskeyAssertion,
    ) -> Pin<Box<dyn Future<Output = Result<(), PasskeyRefusedReason>> + Send + 'a>> {
        Box::pin(async move {
            let a = self
                .passkeys
                .verify_bound(session, self.user, assertion, &self.challenge)
                .await?;
            *self.slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(a);
            Ok(())
        })
    }
}

// ---- the early check -------------------------------------------------------------------------

/// A group of hosts that needs review.
pub(crate) struct GroupDraft {
    pub base: Option<String>,
    pub hosts: Vec<String>,
    pub reason: ReviewReason,
    pub commits: Vec<ReviewCommit>,
    pub flags: Vec<ReviewFlag>,
}

/// Each host's base, and the groups that need review.
pub(crate) type Checked = (Vec<(Uuid, Option<String>)>, Vec<GroupDraft>);

/// What [`early_check`] decided.
pub(crate) enum Early {
    /// The job uses a managed inventory, which signed mode refuses.
    Managed,
    /// Each host's base, and the groups that need review (none: every group is signed).
    Checked {
        bases: Vec<(Uuid, Option<String>)>,
        groups: Vec<GroupDraft>,
    },
}

/// The newest base Keel recorded for `job` on each host (`None`: no base). Only signed-mode
/// runs move a host's base: outside signed mode the host keeps no ledger.
pub(crate) async fn bases(
    conn: &mut PgConnection,
    job: &str,
    hosts: &[Uuid],
) -> Result<Vec<(Uuid, Option<String>)>, sqlx::Error> {
    sqlx::query_as(
        "select h.id, (select hr.commit from host_revisions hr join runs pr on pr.id = hr.run_id \
           where hr.host_id = h.id and pr.job_name = $2 and hr.authorization_kind <> 'unchecked' \
           order by hr.applied_at desc, hr.id desc limit 1) \
         from unnest($1::uuid[]) as h(id)",
    )
    .bind(hosts)
    .bind(job)
    .fetch_all(conn)
    .await
}

/// The workspace's trusted signers as a signers file: SSH lines first, then OpenPGP blocks.
async fn signers_text(conn: &mut PgConnection, ws: Uuid) -> Result<String, sqlx::Error> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "select kind, public_key from trusted_signers where workspace_id = $1 \
         and removed_at is null order by (kind = 'ssh') desc, added_at, id",
    )
    .bind(ws)
    .fetch_all(conn)
    .await?;
    let mut out = String::new();
    for (kind, key) in rows {
        if kind == "ssh" {
            out.push_str(&ssh_line(&key));
        } else {
            out.push_str(key.trim());
        }
        out.push('\n');
    }
    Ok(out)
}

fn ssh_line(key: &str) -> String {
    format!("keel namespaces=\"git\" {}", key.trim())
}

/// Runs the early check for a new signed-mode run (see the module docs).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn early_check(
    app: &AppState,
    conn: &mut PgConnection,
    ws: Uuid,
    binding: Uuid,
    job: &str,
    source: InventorySource,
    commit: &str,
    hosts: &[(Uuid, String)],
) -> Result<Early, sqlx::Error> {
    if source != InventorySource::Repo {
        return Ok(Early::Managed);
    }
    let ids: Vec<Uuid> = hosts.iter().map(|h| h.0).collect();
    let bases = bases(conn, job, &ids).await?;
    let names: HashMap<Uuid, &str> = hosts.iter().map(|h| (h.0, h.1.as_str())).collect();
    let mut by_base: BTreeMap<Option<String>, Vec<String>> = BTreeMap::new();
    for (host, base) in &bases {
        if let Some(name) = names.get(host) {
            by_base
                .entry(base.clone())
                .or_default()
                .push((*name).to_owned());
        }
    }
    let signers = signers_text(conn, ws).await?;
    let inventories: Vec<(String,)> = sqlx::query_as(
        "select path from inventories where binding_id = $1 and source = 'repo' \
         and path is not null",
    )
    .bind(binding)
    .fetch_all(&mut *conn)
    .await?;
    let inventories: Vec<String> = inventories.into_iter().map(|r| r.0).collect();
    let dir = crate::sync::data_dir(app);
    let target = commit.to_owned();
    let groups = tokio::task::spawn_blocking(move || {
        let signers = Signers::parse(&signers).ok();
        let mirror = Mirror::open(&dir, binding).ok();
        by_base
            .into_iter()
            .filter_map(|(base, hosts)| {
                check_group(
                    mirror.as_ref(),
                    signers.as_ref(),
                    base,
                    &target,
                    hosts,
                    &inventories,
                )
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_else(|_| {
        // The check panicked: fail closed, one group for every host.
        vec![GroupDraft {
            base: None,
            hosts: hosts.iter().map(|h| h.1.clone()).collect(),
            reason: ReviewReason::CheckFailed,
            commits: Vec::new(),
            flags: Vec::new(),
        }]
    });
    Ok(Early::Checked { bases, groups })
}

/// keel-trust's object store over a binding's mirror.
struct MirrorStore<'a>(&'a Mirror);

impl ObjectStore for MirrorStore<'_> {
    fn commit_bytes(&self, id: &keel_trust::CommitId) -> Result<Option<Vec<u8>>, StoreError> {
        self.0
            .commit_bytes(&CommitId(id.as_str().to_owned()))
            .map_err(|e| StoreError(e.to_string()))
    }

    fn tree_complete(&self, tree: &keel_trust::CommitId) -> Result<bool, StoreError> {
        self.0
            .tree_complete(&CommitId(tree.as_str().to_owned()))
            .map_err(|e| StoreError(e.to_string()))
    }
}

/// Checks one group; `None` when every new commit is signed by a trusted signer.
fn check_group(
    mirror: Option<&Mirror>,
    signers: Option<&Signers>,
    base: Option<String>,
    target: &str,
    hosts: Vec<String>,
    inventories: &[String],
) -> Option<GroupDraft> {
    let failed = |base: Option<String>, hosts: Vec<String>| GroupDraft {
        base,
        hosts,
        reason: ReviewReason::CheckFailed,
        commits: Vec::new(),
        flags: Vec::new(),
    };
    let Some(mirror) = mirror else {
        return Some(failed(base, hosts));
    };
    let (Ok(target_id), Ok(base_id)) = (
        keel_trust::CommitId::parse(target),
        base.as_deref().map(keel_trust::CommitId::parse).transpose(),
    ) else {
        return Some(failed(base, hosts));
    };
    let reason = match signers {
        // An unreadable signers file trusts nobody: every group needs review.
        None => ReviewReason::CheckFailed,
        Some(s) => match keel_trust::check(&MirrorStore(mirror), s, base_id.as_ref(), &target_id) {
            Ok(Verdict::Signed { .. }) => return None,
            Ok(Verdict::NeedsOverride { reason, .. }) => match reason {
                keel_trust::commit_check::OverrideReason::Unsigned => ReviewReason::Unsigned,
                keel_trust::commit_check::OverrideReason::Downgrade => ReviewReason::Downgrade,
                keel_trust::commit_check::OverrideReason::Baseline => ReviewReason::Baseline,
            },
            Err(CheckError::PackIncomplete { .. } | CheckError::Store(_)) => {
                ReviewReason::CheckFailed
            }
        },
    };
    let commits = list_commits(mirror, signers, base_id.as_ref(), &target_id);
    let flags = flags(mirror, base.as_deref(), target, inventories);
    Some(GroupDraft {
        base,
        hosts,
        reason,
        commits,
        flags,
    })
}

/// The commits reachable from `target` and not from `base`, newest first, at most
/// [`MAX_REVIEW_COMMITS`], with their signature status.
fn list_commits(
    mirror: &Mirror,
    signers: Option<&Signers>,
    base: Option<&keel_trust::CommitId>,
    target: &keel_trust::CommitId,
) -> Vec<ReviewCommit> {
    let store = MirrorStore(mirror);
    let load = |id: &keel_trust::CommitId| {
        store
            .commit_bytes(id)
            .ok()
            .flatten()
            .and_then(|raw| parse_commit(&raw, id.is_sha256()).ok())
    };
    let mut excluded = HashSet::new();
    if let Some(base) = base {
        let mut queue = VecDeque::from([base.clone()]);
        while let Some(id) = queue.pop_front() {
            if excluded.insert(id.clone())
                && let Some(c) = load(&id)
            {
                queue.extend(c.parents);
            }
        }
    }
    let mut out = Vec::new();
    let mut seen = HashSet::from([target.clone()]);
    let mut queue = VecDeque::from([target.clone()]);
    while let Some(id) = queue.pop_front() {
        if out.len() >= MAX_REVIEW_COMMITS {
            break;
        }
        let parsed = load(&id);
        let (status, signer) = match (&parsed, signers) {
            (None, _) => (SignatureStatus::Unparseable, None),
            (Some(_), None) => (SignatureStatus::Unchecked, None),
            (Some(c), Some(s)) => match verify_commit(s, c) {
                Ok(signer) => (SignatureStatus::Good, Some(signer)),
                Err(p) => (
                    serde_json::to_value(p)
                        .ok()
                        .and_then(|v| serde_json::from_value(v).ok())
                        .unwrap_or(SignatureStatus::Unparseable),
                    None,
                ),
            },
        };
        let meta = mirror.commit_meta(&CommitId(id.as_str().to_owned())).ok();
        out.push(ReviewCommit {
            id: id.as_str().to_owned(),
            author: meta.as_ref().map(|m| m.author.clone()).unwrap_or_default(),
            committer: meta
                .as_ref()
                .map(|m| m.committer.clone())
                .unwrap_or_default(),
            summary: meta.map(|m| m.summary).unwrap_or_default(),
            status,
            signer,
        });
        if let Some(c) = parsed {
            for p in c.parents {
                if !excluded.contains(&p) && seen.insert(p.clone()) {
                    queue.push_back(p);
                }
            }
        }
    }
    out
}

/// The path flags of one changed file (`.keel/` and inventories).
fn path_flags(path: &str, inventories: &[String]) -> Vec<ReviewFlagKind> {
    let mut out = Vec::new();
    if path.starts_with(".keel/") || path == ".keel" {
        out.push(ReviewFlagKind::KeelConfig);
    }
    let touches_inventory = inventories.iter().any(|inv| {
        let inv = inv.trim_end_matches('/');
        let dir = match inv.rsplit_once('/') {
            Some((d, _)) => format!("{d}/"),
            None => String::new(),
        };
        path == inv
            || path.starts_with(&format!("{inv}/"))
            || path.starts_with(&format!("{dir}group_vars/"))
            || path.starts_with(&format!("{dir}host_vars/"))
    });
    if touches_inventory {
        out.push(ReviewFlagKind::Inventory);
    }
    out
}

/// The trust and targeting changes between `base` and `target`. Anything that cannot be read
/// is flagged (a file that cannot be scanned may hide a `keel_runner_*` change).
fn flags(
    mirror: &Mirror,
    base: Option<&str>,
    target: &str,
    inventories: &[String],
) -> Vec<ReviewFlag> {
    let base = base.map(|b| CommitId(b.to_owned()));
    let target = CommitId(target.to_owned());
    let Ok(files) = mirror.changed_files(base.as_ref(), &target) else {
        return vec![ReviewFlag {
            kind: ReviewFlagKind::KeelConfig,
            path: String::new(),
        }];
    };
    let mut out = Vec::new();
    for (i, f) in files.iter().enumerate() {
        for kind in path_flags(&f.path, inventories) {
            out.push(ReviewFlag {
                kind,
                path: f.path.clone(),
            });
        }
        let runner_vars = if i >= MAX_SCANNED_FILES {
            true
        } else {
            match mirror.file_diff(base.as_ref(), &target, &f.path) {
                Ok(d) if !d.binary => d
                    .lines
                    .iter()
                    .any(|l| (l.starts_with('+') || l.starts_with('-')) && l.contains(RUNNER_VARS)),
                _ => true,
            }
        };
        if runner_vars {
            out.push(ReviewFlag {
                kind: ReviewFlagKind::RunnerVars,
                path: f.path.clone(),
            });
        }
    }
    out
}

/// Records the review groups of a new run; returns their ids.
pub(crate) async fn insert_groups(
    conn: &mut PgConnection,
    run: Uuid,
    target: &str,
    groups: &[GroupDraft],
) -> Result<Vec<Uuid>, sqlx::Error> {
    let mut ids = Vec::with_capacity(groups.len());
    for g in groups {
        let id = Uuid::now_v7();
        sqlx::query(
            "insert into review_groups (id, run_id, base_commit, target_commit, hosts, commits, \
             flags, reason) values ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(id)
        .bind(run)
        .bind(&g.base)
        .bind(target)
        .bind(&g.hosts)
        .bind(json!(g.commits))
        .bind(json!(g.flags))
        .bind(crate::bindings::enum_str(&g.reason))
        .execute(&mut *conn)
        .await?;
        ids.push(id);
    }
    Ok(ids)
}

// ---- commands --------------------------------------------------------------------------------

fn pending<F, Fut>(f: F) -> PendingOp
where
    F: FnOnce(AppState, Caller) -> Fut + Send + 'static,
    Fut: Future<Output = Vec<Event>> + Send + 'static,
{
    Box::new(move |app, caller| Box::pin(f(app, caller)))
}

/// Re-checks `cap` when a confirmed operation runs (grants may have changed since).
fn recheck(caller: &Caller, action: &str, cap: Capability, ws: Uuid) -> Result<(), Vec<Event>> {
    let refused = match &caller.token {
        Some(t) if t.workspace.0 != ws => Some("token_scope"),
        _ => authz::require(&subject(caller), cap, WorkspaceId(ws), &[])
            .err()
            .map(|r| r.code()),
    };
    match refused {
        None => Ok(()),
        Some(reason) => Err(vec![Event::Denied {
            action: action.to_owned(),
            reason: reason.to_owned(),
        }]),
    }
}

fn session_required(action: &str) -> Vec<Event> {
    vec![Event::Denied {
        action: action.to_owned(),
        reason: "session_required".to_owned(),
    }]
}

fn unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .and_then(|d| d.code())
        .is_some_and(|c| c == "23505")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Runs one review or signer command.
pub async fn handle(app: &AppState, req: &Request<'_>, cmd: Cmd) -> Vec<Event> {
    match cmd {
        Cmd::ListReviews { workspace, state } => list_reviews(app, req, workspace, state).await,
        Cmd::GetReview { group } => {
            let ws = match group_workspace(app, group).await {
                Ok(Some(ws)) => ws,
                Ok(None) => return not_found("review"),
                Err(e) => return db_failed(&e),
            };
            if let Err(d) = require(app, req, "get_review", Capability::View, ws).await {
                return d;
            }
            review_event(app, group).await
        }
        Cmd::GetReviewDiff { group, path, page } => {
            review_diff(
                app,
                req,
                group,
                path.map(|p| p.as_str().to_owned()),
                page.unwrap_or(0),
            )
            .await
        }
        Cmd::PreviewOverride { group, comment } => {
            preview_override(app, req, group, comment.as_str().to_owned()).await
        }
        Cmd::PreviewOverrideDeny { group, comment } => {
            preview_deny(app, req, group, comment.as_str().to_owned()).await
        }
        Cmd::PreviewBaseline {
            binding,
            job,
            hosts,
            commit,
            comment,
        } => {
            let hosts = normalize_hosts(hosts.0.iter().map(|h| h.as_str()));
            preview_baseline(
                app,
                req,
                binding,
                job,
                hosts,
                commit.as_str().to_owned(),
                comment.as_str().to_owned(),
            )
            .await
        }
        Cmd::ListSigners { workspace } => {
            if let Err(d) = require(app, req, "list_signers", Capability::View, workspace).await {
                return d;
            }
            signers_event(app, workspace).await
        }
        Cmd::PreviewSignerAdd {
            workspace,
            kind,
            public_key,
            label,
            user,
        } => {
            preview_signer_add(
                app,
                req,
                workspace,
                kind,
                public_key.as_str().to_owned(),
                label.as_str().to_owned(),
                user,
            )
            .await
        }
        Cmd::PreviewSignerRemove { signer } => preview_signer_remove(app, req, signer).await,
        _ => internal(),
    }
}

async fn group_workspace(app: &AppState, group: Uuid) -> Result<Option<Uuid>, sqlx::Error> {
    let row: Option<(Uuid,)> = sqlx::query_as(
        "select r.workspace_id from review_groups g join runs r on r.id = g.run_id \
         where g.id = $1",
    )
    .bind(group)
    .fetch_optional(&app.db)
    .await?;
    Ok(row.map(|r| r.0))
}

const INFO_COLUMNS: &str = "g.id, g.run_id, r.workspace_id, r.binding_id, r.job_name, \
     g.base_commit, g.target_commit, g.hosts, g.reason, g.state, \
     (select count(*) from approvals a where a.review_group_id = g.id and a.kind = 'override' \
        and a.decision = 'approve'), \
     greatest(b.override_quorum, (select max(h.override_quorum) from hosts h \
        where h.workspace_id = r.workspace_id and h.inventory_name = any(g.hosts)), 1), \
     coalesce(p.four_eyes, true), jsonb_array_length(g.flags), g.created_at";

const INFO_FROM: &str = "from review_groups g join runs r on r.id = g.run_id \
     left join repo_bindings b on b.id = r.binding_id \
     left join approval_policies p on p.id = r.policy_id";

type InfoRow = (
    Uuid,
    Uuid,
    Uuid,
    Option<Uuid>,
    String,
    Option<String>,
    String,
    Vec<String>,
    String,
    String,
    i64,
    Option<i32>,
    bool,
    i32,
    OffsetDateTime,
);

fn info(r: InfoRow) -> ReviewInfo {
    let (
        id,
        run,
        workspace,
        binding,
        job_name,
        base,
        target,
        hosts,
        reason,
        state,
        approvals,
        quorum,
        four_eyes,
        flagged,
        created,
    ) = r;
    ReviewInfo {
        id,
        run,
        workspace,
        binding,
        job_name,
        base,
        target,
        hosts,
        reason: enum_from(&reason).unwrap_or(ReviewReason::CheckFailed),
        state: enum_from(&state).unwrap_or(ReviewState::Open),
        approvals: u32::try_from(approvals).unwrap_or(0),
        quorum: quorum
            .and_then(|q| u32::try_from(q).ok())
            .unwrap_or(1)
            .max(1),
        four_eyes,
        flagged: u32::try_from(flagged).unwrap_or(0),
        created_at: ts(created),
    }
}

async fn list_reviews(
    app: &AppState,
    req: &Request<'_>,
    workspace: Option<Uuid>,
    state: Option<ReviewState>,
) -> Vec<Event> {
    const ACTION: &str = "list_reviews";
    let workspace = workspace.or_else(|| req.caller.token.as_ref().map(|t| t.workspace.0));
    if let Some(ws) = workspace
        && let Err(d) = require(app, req, ACTION, Capability::View, ws).await
    {
        return d;
    }
    let rows: Result<Vec<InfoRow>, sqlx::Error> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {INFO_COLUMNS} {INFO_FROM} \
         where ($1::uuid is null or r.workspace_id = $1) and ($2::text is null or g.state = $2) \
         order by g.created_at desc, g.id desc limit $3"
    )))
    .bind(workspace)
    .bind(state.map(ReviewState::as_str))
    .bind(LIST_LIMIT)
    .fetch_all(&app.db)
    .await;
    match rows {
        Ok(rows) => {
            let s = subject(req.caller);
            let items = rows
                .into_iter()
                .map(info)
                // Across workspaces: only those the caller may see.
                .filter(|i| {
                    authz::require(&s, Capability::View, WorkspaceId(i.workspace), &[]).is_ok()
                })
                .collect();
            vec![Event::Reviews { items }]
        }
        Err(e) => db_failed(&e),
    }
}

async fn review_event(app: &AppState, group: Uuid) -> Vec<Event> {
    let row: Result<Option<(InfoRow, Value, Value)>, sqlx::Error> = async {
        let info: Option<InfoRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select {INFO_COLUMNS} {INFO_FROM} where g.id = $1"
        )))
        .bind(group)
        .fetch_optional(&app.db)
        .await?;
        let Some(info) = info else { return Ok(None) };
        let (commits, flags): (Value, Value) =
            sqlx::query_as("select commits, flags from review_groups where id = $1")
                .bind(group)
                .fetch_one(&app.db)
                .await?;
        Ok(Some((info, commits, flags)))
    }
    .await;
    let (row, commits, flags) = match row {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("review"),
        Err(e) => return db_failed(&e),
    };
    type DecisionRow = (
        String,
        String,
        Option<String>,
        OffsetDateTime,
        Option<OffsetDateTime>,
    );
    let decisions: Vec<DecisionRow> = match sqlx::query_as(
        "select coalesce(u.email, u.display_name), a.decision, a.comment, a.at, a.expires_at \
         from approvals a join users u on u.id = a.approver_user_id \
         where a.review_group_id = $1 and a.kind = 'override' order by a.at, a.id",
    )
    .bind(group)
    .fetch_all(&app.db)
    .await
    {
        Ok(d) => d,
        Err(e) => return db_failed(&e),
    };
    vec![Event::Review {
        review: info(row),
        commits: serde_json::from_value(commits).unwrap_or_default(),
        flags: serde_json::from_value(flags).unwrap_or_default(),
        decisions: decisions
            .into_iter()
            .map(
                |(approver, decision, comment, at, expires)| ReviewDecision {
                    approver,
                    approve: decision == "approve",
                    comment,
                    at: ts(at),
                    expires_at: expires.map(ts),
                },
            )
            .collect(),
    }]
}

async fn review_diff(
    app: &AppState,
    req: &Request<'_>,
    group: Uuid,
    path: Option<String>,
    page: u32,
) -> Vec<Event> {
    type Row = (Uuid, Option<Uuid>, Option<String>, String, Value);
    let row: Option<Row> = match sqlx::query_as(
        "select r.workspace_id, r.binding_id, g.base_commit, g.target_commit, g.flags \
         from review_groups g join runs r on r.id = g.run_id where g.id = $1",
    )
    .bind(group)
    .fetch_optional(&app.db)
    .await
    {
        Ok(r) => r,
        Err(e) => return db_failed(&e),
    };
    let Some((ws, binding, base, target, flags)) = row else {
        return not_found("review");
    };
    if let Err(d) = require(app, req, "get_review_diff", Capability::View, ws).await {
        return d;
    }
    let Some(binding) = binding else {
        return op_error("artifact_missing", "The run's binding is gone.");
    };
    let flags: Vec<ReviewFlag> = serde_json::from_value(flags).unwrap_or_default();
    let dir = crate::sync::data_dir(app);
    let result = tokio::task::spawn_blocking(move || {
        let mirror = Mirror::open(&dir, binding).ok()?;
        let base = base.map(CommitId);
        let target = CommitId(target);
        let changed = mirror.changed_files(base.as_ref(), &target).ok()?;
        let files: Vec<DiffFile> = changed
            .iter()
            .map(|c| {
                let mut kinds: Vec<ReviewFlagKind> = flags
                    .iter()
                    .filter(|f| f.path == c.path)
                    .map(|f| f.kind)
                    .collect();
                kinds.dedup();
                DiffFile {
                    path: c.path.clone(),
                    status: match c.change {
                        Change::Added => DiffStatus::Added,
                        Change::Modified => DiffStatus::Modified,
                        Change::Deleted => DiffStatus::Deleted,
                    },
                    flags: kinds,
                }
            })
            .collect();
        let page = match path {
            None => Ok(None),
            Some(p) if !files.iter().any(|f| f.path == p) => Err("not_changed"),
            Some(p) => {
                let Ok(d) = mirror.file_diff(base.as_ref(), &target, &p) else {
                    return None;
                };
                let pages = d.lines.len().div_ceil(DIFF_PAGE_LINES).max(1);
                let n = page as usize;
                if n >= pages {
                    Err("bad_page")
                } else {
                    Ok(Some(DiffPage {
                        path: p,
                        page,
                        pages: u32::try_from(pages).unwrap_or(u32::MAX),
                        lines: d
                            .lines
                            .into_iter()
                            .skip(n * DIFF_PAGE_LINES)
                            .take(DIFF_PAGE_LINES)
                            .collect(),
                        binary: d.binary,
                    }))
                }
            }
        };
        Some((files, page))
    })
    .await;
    match result {
        Ok(Some((files, Ok(page)))) => vec![Event::ReviewDiff { group, files, page }],
        Ok(Some((_, Err("not_changed")))) => {
            op_error("not_changed", "That file does not change in this review.")
        }
        Ok(Some((_, Err(_)))) => op_error("bad_page", "That page does not exist."),
        Ok(None) | Err(_) => op_error(
            "artifact_missing",
            "The commits under review are not in Keel's copy of the repository.",
        ),
    }
}

// ---- overrides -------------------------------------------------------------------------------

/// What an override decides on, read when it is previewed and again when it is confirmed.
#[derive(Clone)]
struct GroupCtx {
    run: Uuid,
    ws: Uuid,
    binding: Option<Uuid>,
    job_name: String,
    base: Option<String>,
    target: String,
    hosts: Vec<String>,
    state: String,
    run_state: String,
    four_eyes: bool,
    ttl_secs: i64,
}

async fn group_ctx(
    conn: &mut PgConnection,
    group: Uuid,
    lock: bool,
) -> Result<Option<GroupCtx>, sqlx::Error> {
    type Row = (
        Uuid,
        Uuid,
        Option<Uuid>,
        String,
        Option<String>,
        String,
        Vec<String>,
        String,
        String,
        bool,
        i64,
    );
    // Constants only: no caller input reaches the text.
    let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select g.run_id, r.workspace_id, r.binding_id, r.job_name, g.base_commit, \
         g.target_commit, g.hosts, g.state, r.state, coalesce(p.four_eyes, true), \
         coalesce(extract(epoch from p.approval_ttl)::bigint, {MAX_OVERRIDE_SECS}) \
         from review_groups g join runs r on r.id = g.run_id \
         left join approval_policies p on p.id = r.policy_id where g.id = $1{}",
        if lock { " for update of g, r" } else { "" }
    )))
    .bind(group)
    .fetch_optional(conn)
    .await?;
    Ok(row.map(
        |(run, ws, binding, job_name, base, target, hosts, state, run_state, four_eyes, ttl)| {
            GroupCtx {
                run,
                ws,
                binding,
                job_name,
                base,
                target,
                hosts,
                state,
                run_state,
                four_eyes,
                ttl_secs: ttl,
            }
        },
    ))
}

/// Why four-eyes refuses `user` for this group, if it does: they requested the run, wrote one
/// of its commits, or own a signer key that signed one.
async fn four_eyes(
    conn: &mut PgConnection,
    user: Uuid,
    group: Uuid,
) -> Result<Option<&'static str>, sqlx::Error> {
    let (requested, author, signer): (bool, bool, bool) = sqlx::query_as(
        "select coalesce(r.requested_by = $1, false), \
           exists (select 1 from users u, jsonb_array_elements(g.commits) c \
             where u.id = $1 and u.email is not null \
               and lower(c->>'author') like '%<' || lower(u.email) || '>'), \
           exists (select 1 from trusted_signers s, jsonb_array_elements(g.commits) c \
             where s.workspace_id = r.workspace_id and s.user_id = $1 \
               and c->>'signer' = s.fingerprint) \
         from review_groups g join runs r on r.id = g.run_id where g.id = $2",
    )
    .bind(user)
    .bind(group)
    .fetch_one(conn)
    .await?;
    Ok(if requested {
        Some("four_eyes_requester")
    } else if author {
        Some("four_eyes_author")
    } else if signer {
        Some("four_eyes_signer")
    } else {
        None
    })
}

fn four_eyes_refused(reason: &str) -> Vec<Event> {
    op_error(
        reason,
        "Four-eyes: someone who requested the run, wrote or signed one of its commits cannot \
         review it.",
    )
}

/// The challenge scope for a group (or a baseline run about to be made).
#[allow(clippy::too_many_arguments)]
fn scope(
    ws: Uuid,
    binding: Uuid,
    run: Uuid,
    job: &str,
    base: Option<&str>,
    target: &str,
    hosts: &[String],
    expires_at: &str,
) -> Option<OverrideScope> {
    let s = OverrideScope {
        kind: if base.is_none() {
            ApprovalKind::Baseline
        } else {
            ApprovalKind::Override
        },
        workspace: ws.to_string(),
        binding: binding.to_string(),
        run: run.to_string(),
        job: job.to_owned(),
        base: match base {
            Some(b) => Some(keel_trust::CommitId::parse(b).ok()?),
            None => None,
        },
        target: keel_trust::CommitId::parse(target).ok()?,
        hosts: normalize_hosts(hosts.iter().map(String::as_str)),
        expires_at: expires_at.to_owned(),
    };
    s.validate().ok()?;
    Some(s)
}

/// `now + min(ttl, 24h)`, whole seconds, as Unix seconds and `YYYY-MM-DDTHH:MM:SSZ`.
fn expiry(now: OffsetDateTime, ttl_secs: i64) -> Option<(i64, String)> {
    let at = now.unix_timestamp() + ttl_secs.clamp(60, MAX_OVERRIDE_SECS);
    Some((at, format_utc(at)?))
}

/// Opens the passkey ceremony over `scope`'s challenge and stores the preview; replies
/// `Preview` then `OverrideChallenge`.
async fn challenge_preview(
    app: &AppState,
    req: &Request<'_>,
    session: &str,
    scope: &OverrideScope,
    draft: Draft,
    slot: Arc<Mutex<Option<BoundAssertion>>>,
    op: PendingOp,
) -> Vec<Event> {
    let (Ok(challenge), Ok(canonical)) = (scope.challenge(), scope.canonical_bytes()) else {
        return internal();
    };
    let Ok(canonical) = String::from_utf8(canonical) else {
        return internal();
    };
    let request = match app
        .passkeys
        .bound_begin(session, req.caller, &challenge)
        .await
    {
        Ok(r) => r,
        Err(events) => return events,
    };
    let key = req.caller.preview_key(req.session);
    let preview = app.previews.create(&key, draft, op);
    app.reviews.insert(
        preview.preview_id,
        Bound {
            challenge,
            user: req.caller.user.id,
            slot,
            created: app.clock.now(),
        },
    );
    let expires_at = scope.expires_at.clone();
    let preview_id = preview.preview_id;
    vec![
        Event::Preview(preview),
        Event::OverrideChallenge(OverrideChallenge {
            preview_id,
            request,
            canonical,
            expires_at,
        }),
    ]
}

async fn preview_override(
    app: &AppState,
    req: &Request<'_>,
    group: Uuid,
    comment: String,
) -> Vec<Event> {
    const ACTION: &str = "preview_override";
    let Some(session) = req.session.filter(|_| req.caller.token.is_none()) else {
        return session_required(ACTION);
    };
    let mut conn = match app.db.acquire().await {
        Ok(c) => c,
        Err(e) => return db_failed(&e),
    };
    let ctx = match group_ctx(&mut conn, group, false).await {
        Ok(Some(c)) => c,
        Ok(None) => return not_found("review"),
        Err(e) => return db_failed(&e),
    };
    if let Err(d) = require(app, req, ACTION, Capability::Approve, ctx.ws).await {
        return d;
    }
    if ctx.state != "open" || ctx.run_state != "needs_review" {
        return op_error("review_closed", "This review is no longer open.");
    }
    if ctx.four_eyes {
        match four_eyes(&mut conn, req.caller.user.id, group).await {
            Ok(Some(r)) => return four_eyes_refused(r),
            Ok(None) => {}
            Err(e) => return db_failed(&e),
        }
    }
    drop(conn);
    let Some(binding) = ctx.binding else {
        return op_error("artifact_missing", "The run's binding is gone.");
    };
    let Some((_, expires_at)) = expiry(app.clock.now(), ctx.ttl_secs) else {
        return internal();
    };
    let Some(scope) = scope(
        ctx.ws,
        binding,
        ctx.run,
        &ctx.job_name,
        ctx.base.as_deref(),
        &ctx.target,
        &ctx.hosts,
        &expires_at,
    ) else {
        return internal();
    };
    let Ok(challenge) = scope.challenge() else {
        return internal();
    };
    let slot = Arc::new(Mutex::new(None));
    let taken = slot.clone();
    let hosts = scope.hosts.clone();
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Approve, ctx.ws) {
            return d;
        }
        let Some(assertion) = taken.lock().unwrap_or_else(PoisonError::into_inner).take() else {
            return op_error(
                "override_assertion_required",
                "Sign the override challenge with your passkey.",
            );
        };
        let approval = Approval {
            group,
            comment,
            challenge,
            assertion,
            expires_at,
        };
        match approve(&app, &caller, &approval).await {
            Ok(Ok(())) => review_event(&app, group).await,
            Ok(Err(events)) => events,
            Err(e) => db_failed(&e),
        }
    });
    let kind = if ctx.base.is_none() {
        "baseline"
    } else {
        "override"
    };
    let draft = Draft {
        action: ACTION.to_owned(),
        summary: format!(
            "Approve a reviewed {kind} of {} on {} host(s) for job {}. Your passkey signs the \
             exact run, commits and hosts; hosts check it themselves.",
            &ctx.target[..ctx.target.len().min(12)],
            hosts.len(),
            ctx.job_name
        ),
        affected: hosts,
        step_up: true,
    };
    challenge_preview(app, req, session, &scope, draft, slot, op).await
}

struct Approval {
    group: Uuid,
    comment: String,
    challenge: [u8; 32],
    assertion: BoundAssertion,
    expires_at: String,
}

/// Records an approved override; at quorum approves the group, and when every group of the
/// run is approved moves the run on. `Ok(Err(events))` is a refusal.
async fn approve(
    app: &AppState,
    caller: &Caller,
    a: &Approval,
) -> Result<Result<(), Vec<Event>>, sqlx::Error> {
    let mut tx = app.db.begin().await?;
    let Some(ctx) = group_ctx(&mut tx, a.group, true).await? else {
        return Ok(Err(not_found("review")));
    };
    if ctx.state != "open" || ctx.run_state != "needs_review" {
        return Ok(Err(op_error(
            "review_closed",
            "This review is no longer open.",
        )));
    }
    if ctx.four_eyes
        && let Some(r) = four_eyes(&mut tx, caller.user.id, a.group).await?
    {
        return Ok(Err(four_eyes_refused(r)));
    }
    let Some(expires) = keel_trust::challenge::parse_utc(&a.expires_at)
        .and_then(|s| OffsetDateTime::from_unix_timestamp(s).ok())
    else {
        return Ok(Err(internal()));
    };
    if expires <= app.clock.now() {
        return Ok(Err(op_error(
            "override_expired",
            "The override challenge expired.",
        )));
    }
    let assertion = json!({
        "credential_id": a.assertion.credential_id,
        "authenticator_data": a.assertion.authenticator_data,
        "client_data_json": a.assertion.client_data_json,
        "signature": a.assertion.signature,
    });
    let inserted = sqlx::query(
        "insert into approvals (id, run_id, approver_user_id, kind, decision, comment, \
         step_up_at, review_group_id, challenge_sha256, assertion, expires_at) \
         values ($1, $2, $3, 'override', 'approve', $4, now(), $5, $6, $7, $8)",
    )
    .bind(Uuid::now_v7())
    .bind(ctx.run)
    .bind(caller.user.id)
    .bind(&a.comment)
    .bind(a.group)
    .bind(hex(&Sha256::digest(a.challenge)))
    .bind(assertion)
    .bind(expires)
    .execute(&mut *tx)
    .await;
    if let Err(e) = inserted {
        if unique_violation(&e) {
            return Ok(Err(op_error(
                "already_reviewed",
                "You have already reviewed this group.",
            )));
        }
        return Err(e);
    }
    let (count, quorum): (i64, i32) = sqlx::query_as(
        "select (select count(*) from approvals a where a.review_group_id = g.id \
            and a.kind = 'override' and a.decision = 'approve'), \
          greatest(b.override_quorum, (select max(h.override_quorum) from hosts h \
            where h.workspace_id = r.workspace_id and h.inventory_name = any(g.hosts)), 1) \
         from review_groups g join runs r on r.id = g.run_id \
         left join repo_bindings b on b.id = r.binding_id where g.id = $1",
    )
    .bind(a.group)
    .fetch_one(&mut *tx)
    .await?;
    let group_done = count >= i64::from(quorum.max(1));
    audit::record(
        &mut tx,
        Entry {
            actor: &caller.actor(),
            action: "override_approved",
            workspace: Some(WorkspaceId(ctx.ws)),
            target: json!({ "review_group": a.group, "run": ctx.run }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({
                "base": ctx.base,
                "target": ctx.target,
                "hosts": ctx.hosts,
                "expires_at": a.expires_at,
                "approvals": count,
                "quorum": quorum,
            }),
        },
    )
    .await?;
    let mut next = None;
    let mut cleared = Vec::new();
    if group_done {
        sqlx::query("update review_groups set state = 'approved' where id = $1")
            .bind(a.group)
            .execute(&mut *tx)
            .await?;
        // A baseline approval vouches for these hosts as they are (058 A2).
        if ctx.base.is_none() {
            cleared = crate::outside::clear(
                &mut tx,
                ctx.ws,
                &ctx.hosts,
                "baseline approved",
                &caller.actor(),
            )
            .await?;
        }
        let (open,): (i64,) = sqlx::query_as(
            "select count(*) from review_groups where run_id = $1 and state <> 'approved'",
        )
        .bind(ctx.run)
        .fetch_one(&mut *tx)
        .await?;
        if open == 0 {
            next = Some(overridden(&mut tx, caller, ctx.run, ctx.ws).await?);
        }
    }
    tx.commit().await?;
    crate::outside::pushed(app, ctx.ws, &cleared);
    if let Some(Some(status)) = next {
        crate::queue::push_state(app, ctx.ws, ctx.run, status);
        if status.state == RunState::Queued {
            crate::queue::kick(app);
        }
    }
    Ok(Ok(()))
}

/// Every group of `run` is approved: the run leaves `needs_review`. The approval policy is
/// satisfied when it requires none, or when the reviewers (distinct, and four-eyes already
/// checked) number at least `approvals_needed`. A run without a policy requires one approval.
async fn overridden(
    tx: &mut PgConnection,
    caller: &Caller,
    run: Uuid,
    ws: Uuid,
) -> Result<Option<RunStatus>, sqlx::Error> {
    let (requires, needed, reviewers): (bool, i32, i64) = sqlx::query_as(
        "select coalesce(p.requires_approval, true), coalesce(p.approvals_needed, 1), \
           (select count(distinct a.approver_user_id) from approvals a where a.run_id = r.id \
              and a.kind = 'override' and a.decision = 'approve') \
         from runs r left join approval_policies p on p.id = r.policy_id where r.id = $1",
    )
    .bind(run)
    .fetch_one(&mut *tx)
    .await?;
    let satisfied = !requires || reviewers >= i64::from(needed.max(1));
    let Ok(next) = keel_core::run::transition(
        RunState::NeedsReview,
        &RunEvent::Overridden {
            approval_satisfied: satisfied,
        },
    ) else {
        return Ok(None);
    };
    sqlx::query(
        "update runs set state = $2, state_reason = $3, \
         queued_at = case when $2 = 'queued' then now() else queued_at end where id = $1",
    )
    .bind(run)
    .bind(next.state.as_str())
    .bind(next.reason.map(|r| r.as_str()))
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        Entry {
            actor: &caller.actor(),
            action: "run_overridden",
            workspace: Some(WorkspaceId(ws)),
            target: json!({ "run": run }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({ "state": next.state.as_str(), "approval_satisfied": satisfied }),
        },
    )
    .await?;
    Ok(Some(next))
}

async fn preview_deny(
    app: &AppState,
    req: &Request<'_>,
    group: Uuid,
    comment: String,
) -> Vec<Event> {
    const ACTION: &str = "preview_override_deny";
    let mut conn = match app.db.acquire().await {
        Ok(c) => c,
        Err(e) => return db_failed(&e),
    };
    let ctx = match group_ctx(&mut conn, group, false).await {
        Ok(Some(c)) => c,
        Ok(None) => return not_found("review"),
        Err(e) => return db_failed(&e),
    };
    drop(conn);
    if let Err(d) = require(app, req, ACTION, Capability::Approve, ctx.ws).await {
        return d;
    }
    if ctx.state != "open" || ctx.run_state != "needs_review" {
        return op_error("review_closed", "This review is no longer open.");
    }
    let ws = ctx.ws;
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Approve, ws) {
            return d;
        }
        match deny(&app, &caller, group, &comment).await {
            Ok(Ok(())) => review_event(&app, group).await,
            Ok(Err(events)) => events,
            Err(e) => db_failed(&e),
        }
    });
    crate::bindings::preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary: format!(
                "Deny the review of {} on {} host(s) for job {}: the run is denied and will not \
                 run anywhere.",
                &ctx.target[..ctx.target.len().min(12)],
                ctx.hosts.len(),
                ctx.job_name
            ),
            affected: ctx.hosts,
            step_up: false,
        },
        op,
    )
}

async fn deny(
    app: &AppState,
    caller: &Caller,
    group: Uuid,
    comment: &str,
) -> Result<Result<(), Vec<Event>>, sqlx::Error> {
    let mut tx = app.db.begin().await?;
    let Some(ctx) = group_ctx(&mut tx, group, true).await? else {
        return Ok(Err(not_found("review")));
    };
    if ctx.state != "open" || ctx.run_state != "needs_review" {
        return Ok(Err(op_error(
            "review_closed",
            "This review is no longer open.",
        )));
    }
    let inserted = sqlx::query(
        "insert into approvals (id, run_id, approver_user_id, kind, decision, comment, \
         review_group_id) values ($1, $2, $3, 'override', 'deny', $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(ctx.run)
    .bind(caller.user.id)
    .bind(comment)
    .bind(group)
    .execute(&mut *tx)
    .await;
    if let Err(e) = inserted {
        if unique_violation(&e) {
            return Ok(Err(op_error(
                "already_reviewed",
                "You have already reviewed this group.",
            )));
        }
        return Err(e);
    }
    sqlx::query("update review_groups set state = 'denied' where id = $1")
        .bind(group)
        .execute(&mut *tx)
        .await?;
    let Ok(next) = keel_core::run::transition(RunState::NeedsReview, &RunEvent::Denied) else {
        return Ok(Err(internal()));
    };
    sqlx::query("update runs set state = $2, state_reason = $3 where id = $1")
        .bind(ctx.run)
        .bind(next.state.as_str())
        .bind(next.reason.map(|r| r.as_str()))
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "update run_hosts set state = 'cancelled', ended_at = now() \
         where run_id = $1 and state = 'queued'",
    )
    .bind(ctx.run)
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut tx,
        Entry {
            actor: &caller.actor(),
            action: "override_denied",
            workspace: Some(WorkspaceId(ctx.ws)),
            target: json!({ "review_group": group, "run": ctx.run }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({ "base": ctx.base, "target": ctx.target, "hosts": ctx.hosts }),
        },
    )
    .await?;
    tx.commit().await?;
    crate::queue::push_state(app, ctx.ws, ctx.run, next);
    crate::queue::maybe_finish(app, ctx.run).await;
    Ok(Ok(()))
}

// ---- baselines -------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn preview_baseline(
    app: &AppState,
    req: &Request<'_>,
    binding: Uuid,
    job: Uuid,
    hosts: Vec<String>,
    commit: String,
    comment: String,
) -> Vec<Event> {
    const ACTION: &str = "preview_baseline";
    let Some(session) = req.session.filter(|_| req.caller.token.is_none()) else {
        return session_required(ACTION);
    };
    let Some((job_binding, ws, name, def, blocked)) = (match crate::runs::load_job(app, job).await {
        Ok(j) => j,
        Err(e) => return db_failed(&e),
    }) else {
        return not_found("job");
    };
    if job_binding != binding {
        return not_found("job");
    }
    if let Err(d) = require(app, req, ACTION, Capability::Operate, ws).await {
        return d;
    }
    if let Err(d) = require(app, req, ACTION, Capability::Approve, ws).await {
        return d;
    }
    if let Some(why) = blocked {
        return op_error("job_blocked", &format!("The job is blocked: {why}."));
    }
    let row: Option<(Option<String>, bool, String)> = match sqlx::query_as(
        "select last_commit, signed_mode, status from repo_bindings where id = $1",
    )
    .bind(binding)
    .fetch_optional(&app.db)
    .await
    {
        Ok(r) => r,
        Err(e) => return db_failed(&e),
    };
    let Some((last, signed, status)) = row else {
        return not_found("binding");
    };
    if !signed {
        return op_error("not_signed_mode", "Baselines are for signed-mode bindings.");
    }
    if status != "active" {
        return op_error("binding_disabled", "The binding is disabled.");
    }
    // Fail closed: only the binding's synced head can be a baseline.
    if last.as_deref() != Some(commit.as_str()) {
        return op_error(
            "not_head",
            "A baseline must use the binding's latest synced commit.",
        );
    }
    let mut conn = match app.db.acquire().await {
        Ok(c) => c,
        Err(e) => return db_failed(&e),
    };
    let plan = match crate::runs::plan_hosts(&mut conn, ws, &name, &def).await {
        Ok(Some(p)) => p,
        Ok(None) => return op_error("inventory_invalid", "The job's hosts cannot be resolved."),
        Err(e) => return db_failed(&e),
    };
    if plan.1 {
        return op_error(
            "four_eyes",
            "The job's approval policy asks for four-eyes: a baseline must be approved by \
             someone else, in the review of a run.",
        );
    }
    let known: HashMap<&str, Uuid> = plan.0.iter().map(|h| (h.1.as_str(), h.0)).collect();
    let mut ids = Vec::new();
    for h in &hosts {
        match known.get(h.as_str()) {
            Some(id) => ids.push(*id),
            None => {
                return op_error(
                    "host_not_targeted",
                    &format!("{h} is not an enrolled host this job reaches."),
                );
            }
        }
    }
    match bases(&mut conn, &name, &ids).await {
        Ok(b) if b.iter().all(|(_, base)| base.is_none()) => {}
        Ok(_) => {
            return op_error(
                "has_base",
                "A host already has a base for this job: review its runs instead.",
            );
        }
        Err(e) => return db_failed(&e),
    }
    drop(conn);
    let ttl = plan.2;
    let Some((_, expires_at)) = expiry(app.clock.now(), ttl) else {
        return internal();
    };
    let run = Uuid::now_v7();
    let Some(scope) = scope(ws, binding, run, &name, None, &commit, &hosts, &expires_at) else {
        return internal();
    };
    let Ok(challenge) = scope.challenge() else {
        return internal();
    };
    let slot = Arc::new(Mutex::new(None));
    let taken = slot.clone();
    let covered = scope.hosts.clone();
    let summary = format!(
        "Baseline job {name} at {} on {} host(s): their first signed-mode run. Your passkey \
         signs the exact run, commit and hosts.",
        &scope.target.as_str()[..scope.target.as_str().len().min(12)],
        scope.hosts.len()
    );
    let op = pending(move |app, caller| async move {
        for cap in [Capability::Operate, Capability::Approve] {
            if let Err(d) = recheck(&caller, ACTION, cap, ws) {
                return d;
            }
        }
        let Some(assertion) = taken.lock().unwrap_or_else(PoisonError::into_inner).take() else {
            return op_error(
                "override_assertion_required",
                "Sign the baseline challenge with your passkey.",
            );
        };
        let made = crate::runs::create_baseline_run(
            &app,
            &caller,
            crate::runs::BaselineRun {
                id: run,
                workspace: ws,
                binding,
                job,
                job_name: &name,
                def: &def,
                commit: &commit,
                hosts: &covered,
            },
        )
        .await;
        if let Err(e) = made {
            return db_failed(&e);
        }
        let groups: Vec<(Uuid, Option<String>, String, Vec<String>)> = match sqlx::query_as(
            "select id, base_commit, target_commit, hosts from review_groups where run_id = $1",
        )
        .bind(run)
        .fetch_all(&app.db)
        .await
        {
            Ok(g) => g,
            Err(e) => return db_failed(&e),
        };
        // The run must have come out exactly as signed: one baseline group over these hosts.
        let group = match groups.as_slice() {
            [(g, None, target, h)]
                if *target == commit
                    && normalize_hosts(h.iter().map(String::as_str)) == covered =>
            {
                *g
            }
            _ => {
                return op_error(
                    "baseline_mismatch",
                    "The run Keel made does not match what you signed; it waits for review.",
                );
            }
        };
        let approval = Approval {
            group,
            comment,
            challenge,
            assertion,
            expires_at,
        };
        match approve(&app, &caller, &approval).await {
            Ok(Ok(())) => match crate::runs::get_run(&app, run).await {
                Ok(Some(e)) => vec![e],
                Ok(None) => internal(),
                Err(e) => db_failed(&e),
            },
            Ok(Err(events)) => events,
            Err(e) => db_failed(&e),
        }
    });
    let draft = Draft {
        action: ACTION.to_owned(),
        summary,
        affected: scope.hosts.clone(),
        step_up: true,
    };
    challenge_preview(app, req, session, &scope, draft, slot, op).await
}

// ---- trusted signers -------------------------------------------------------------------------

async fn signers_event(app: &AppState, ws: Uuid) -> Vec<Event> {
    type Row = (
        Uuid,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        OffsetDateTime,
    );
    let rows: Vec<Row> = match sqlx::query_as(
        "select s.id, s.kind, s.fingerprint, s.label, coalesce(u.email, u.display_name), \
         coalesce(a.email, a.display_name), s.added_at \
         from trusted_signers s left join users u on u.id = s.user_id \
         left join users a on a.id = s.added_by \
         where s.workspace_id = $1 and s.removed_at is null order by s.added_at, s.id",
    )
    .bind(ws)
    .fetch_all(&app.db)
    .await
    {
        Ok(r) => r,
        Err(e) => return db_failed(&e),
    };
    vec![Event::Signers {
        workspace: ws,
        items: rows
            .into_iter()
            .map(
                |(id, kind, fingerprint, label, user, added_by, at)| SignerInfo {
                    id,
                    kind: enum_from(&kind).unwrap_or(SignerKind::Ssh),
                    fingerprint,
                    label,
                    user,
                    added_by,
                    added_at: ts(at),
                },
            )
            .collect(),
    }]
}

/// The signer id of exactly one key of `kind`, as keel-trust names it.
fn signer_id(kind: SignerKind, key: &str) -> Option<String> {
    let text = match kind {
        SignerKind::Ssh => {
            if key.trim().contains('\n') {
                return None;
            }
            ssh_line(key)
        }
        SignerKind::Openpgp => key.trim().to_owned(),
    };
    let s = Signers::parse(&text).ok()?;
    match (kind, s.ssh.as_slice(), s.pgp.as_slice()) {
        (SignerKind::Ssh, [one], []) => Some(one.id.clone()),
        (SignerKind::Openpgp, [], [one]) => Some(one.id.clone()),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
async fn preview_signer_add(
    app: &AppState,
    req: &Request<'_>,
    ws: Uuid,
    kind: SignerKind,
    key: String,
    label: String,
    user: Option<Uuid>,
) -> Vec<Event> {
    const ACTION: &str = "preview_signer_add";
    if let Err(d) = require(app, req, ACTION, Capability::Administer, ws).await {
        return d;
    }
    let Some(fingerprint) = signer_id(kind, &key) else {
        return op_error(
            "invalid_signer_key",
            "That is not exactly one valid public key of that kind.",
        );
    };
    if let Some(u) = user {
        match sqlx::query_as::<_, (bool,)>("select exists(select 1 from users where id = $1)")
            .bind(u)
            .fetch_one(&app.db)
            .await
        {
            Ok((true,)) => {}
            Ok((false,)) => return not_found("user"),
            Err(e) => return db_failed(&e),
        }
    }
    let summary = format!(
        "Trust {} key {fingerprint} ({label}) for Keel's early signature check. Hosts trust \
         only the signers pinned on them.",
        kind.as_str()
    );
    let affected = vec![fingerprint.clone()];
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Administer, ws) {
            return d;
        }
        let result: Result<bool, sqlx::Error> = async {
            let mut tx = app.db.begin().await?;
            let row: Option<(Uuid,)> = sqlx::query_as(
                "insert into trusted_signers (id, workspace_id, kind, fingerprint, public_key, \
                 label, user_id, added_by) values ($1, $2, $3, $4, $5, $6, $7, $8) \
                 on conflict (workspace_id, fingerprint) do update set kind = excluded.kind, \
                   public_key = excluded.public_key, label = excluded.label, \
                   user_id = excluded.user_id, added_by = excluded.added_by, added_at = now(), \
                   removed_at = null \
                 where trusted_signers.removed_at is not null returning id",
            )
            .bind(Uuid::now_v7())
            .bind(ws)
            .bind(kind.as_str())
            .bind(&fingerprint)
            .bind(key.trim())
            .bind(&label)
            .bind(user)
            .bind(caller.user.id)
            .fetch_optional(&mut *tx)
            .await?;
            let Some((id,)) = row else { return Ok(false) };
            audit::record(
                &mut tx,
                Entry {
                    actor: &caller.actor(),
                    action: "signer_added",
                    workspace: Some(WorkspaceId(ws)),
                    target: json!({ "signer": id }),
                    decision: Decision::Allowed,
                    reason: None,
                    context: json!({
                        "kind": kind.as_str(),
                        "fingerprint": fingerprint,
                        "label": label,
                        "user": user,
                    }),
                },
            )
            .await?;
            tx.commit().await?;
            Ok(true)
        }
        .await;
        match result {
            Ok(true) => signers_event(&app, ws).await,
            Ok(false) => op_error("signer_exists", "That key is already trusted."),
            Err(e) => db_failed(&e),
        }
    });
    crate::bindings::preview(
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

async fn preview_signer_remove(app: &AppState, req: &Request<'_>, signer: Uuid) -> Vec<Event> {
    const ACTION: &str = "preview_signer_remove";
    let row: Option<(Uuid, String, String)> = match sqlx::query_as(
        "select workspace_id, fingerprint, label from trusted_signers \
         where id = $1 and removed_at is null",
    )
    .bind(signer)
    .fetch_optional(&app.db)
    .await
    {
        Ok(r) => r,
        Err(e) => return db_failed(&e),
    };
    let Some((ws, fingerprint, label)) = row else {
        return not_found("signer");
    };
    if let Err(d) = require(app, req, ACTION, Capability::Administer, ws).await {
        return d;
    }
    let summary = format!(
        "Stop trusting key {fingerprint} ({label}) in Keel's early check. Commits it signed \
         will need review."
    );
    let affected = vec![fingerprint.clone()];
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Administer, ws) {
            return d;
        }
        let result: Result<bool, sqlx::Error> = async {
            let mut tx = app.db.begin().await?;
            let n = sqlx::query(
                "update trusted_signers set removed_at = now() \
                 where id = $1 and removed_at is null",
            )
            .bind(signer)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if n != 1 {
                return Ok(false);
            }
            audit::record(
                &mut tx,
                Entry {
                    actor: &caller.actor(),
                    action: "signer_removed",
                    workspace: Some(WorkspaceId(ws)),
                    target: json!({ "signer": signer }),
                    decision: Decision::Allowed,
                    reason: None,
                    context: json!({ "fingerprint": fingerprint }),
                },
            )
            .await?;
            tx.commit().await?;
            Ok(true)
        }
        .await;
        match result {
            Ok(true) => signers_event(&app, ws).await,
            Ok(false) => not_found("signer"),
            Err(e) => db_failed(&e),
        }
    });
    crate::bindings::preview(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_flags_cover_keel_inventories_and_their_vars() {
        let inv = vec!["inventories/prod/hosts.yaml".to_owned()];
        assert_eq!(
            path_flags(".keel/keel.yaml", &inv),
            [ReviewFlagKind::KeelConfig]
        );
        assert_eq!(
            path_flags("inventories/prod/hosts.yaml", &inv),
            [ReviewFlagKind::Inventory]
        );
        assert_eq!(
            path_flags("inventories/prod/group_vars/all.yaml", &inv),
            [ReviewFlagKind::Inventory]
        );
        assert_eq!(
            path_flags("inventories/prod/host_vars/web-1.yaml", &inv),
            [ReviewFlagKind::Inventory]
        );
        assert!(path_flags("roles/web/tasks/main.yaml", &inv).is_empty());
        let root = vec!["hosts.yaml".to_owned()];
        assert_eq!(
            path_flags("group_vars/all.yaml", &root),
            [ReviewFlagKind::Inventory]
        );
    }

    #[test]
    fn expiry_is_capped_at_a_day() {
        let now = OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();
        assert_eq!(expiry(now, 7 * 86_400).unwrap().0, 1_800_000_000 + 86_400);
        assert_eq!(expiry(now, 3600).unwrap().0, 1_800_000_000 + 3600);
    }

    #[test]
    fn a_signer_key_must_be_exactly_one_key_of_its_kind() {
        assert!(signer_id(SignerKind::Ssh, "not a key").is_none());
        assert!(signer_id(SignerKind::Openpgp, "ssh-ed25519 AAAA").is_none());
        let key =
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";
        let id = signer_id(SignerKind::Ssh, key).expect("valid key");
        assert!(id.starts_with("SHA256:"), "{id}");
        assert!(signer_id(SignerKind::Ssh, &format!("{key}\n{key}")).is_none());
    }
}
