//! The catalog of a workspace's primary repository (spec 066, T040, T041, T042, T045).
//!
//! After a sync of a workspace's primary binding, the catalog is rebuilt from the mirror at the
//! synced head, on the blocking pool, and stored in `binding_catalogs`. A build that fails marks
//! the stored row stale and keeps the last good document. Nothing here executes repository
//! content, and nothing stores a sops value: the core catalog drops values while parsing
//! (FR-015).
//!
//! The rest of this module reads the stored catalog: `GetCatalog` with per-host decrypt state,
//! `DraftJob`, the run preview's sops warnings (FR-018) and hosts' inventory status (FR-023).

use std::collections::BTreeSet;
use std::path::PathBuf;

use keel_core::authz::Capability;
use keel_core::catalog::{self, Catalog, Entry, EntryKind, InventoryKind, Limits, RepoFiles};
use keel_core::manifest::Manifest;
use keel_core::ports::{CommitId, GitSource};
use keel_git::Mirror;
use keel_protocol::{
    CatalogGroup, CatalogInventory, CatalogInventoryKind, CatalogPlay, CatalogPlaybook,
    CatalogProblem, CatalogRole, CatalogSecrets, CatalogSkipped, CatalogView, Event, HostDecrypt,
    HostInventoryStatus, SopsDrift, SopsFile, SopsRule,
};
use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::bindings::{db_failed, internal, op_error, require, ts};
use crate::dispatch::Request;
use crate::serve::AppState;

/// Longest build error kept on the row.
const MAX_ERROR_BYTES: usize = 512;

// ---- reading the mirror -------------------------------------------------------------------------

/// The mirror's tree at one commit, as the catalog reads it. Read failures are absence.
pub(crate) struct MirrorFiles<'a> {
    pub mirror: &'a Mirror,
    pub commit: &'a CommitId,
}

impl RepoFiles for MirrorFiles<'_> {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.mirror.read_file(self.commit, path).ok()
    }

    fn list(&self, dir: &str) -> Option<Vec<Entry>> {
        let entries = self.mirror.list_entries(self.commit, dir).ok()??;
        Some(
            entries
                .into_iter()
                .map(|(name, kind)| Entry {
                    name,
                    kind: match kind {
                        keel_git::EntryKind::File => EntryKind::File,
                        keel_git::EntryKind::Dir => EntryKind::Dir,
                        keel_git::EntryKind::Symlink(_) => EntryKind::Symlink,
                        keel_git::EntryKind::Submodule => EntryKind::Other,
                    },
                })
                .collect(),
        )
    }
}

/// Builds the catalog of `binding`'s mirror at `commit`. Blocking.
fn build_blocking(
    data_dir: &std::path::Path,
    binding: Uuid,
    commit: &str,
    manifest: Option<&Manifest>,
) -> Result<Catalog, String> {
    let mirror = Mirror::open(data_dir, binding).map_err(|e| e.to_string())?;
    let commit_id = CommitId(commit.to_owned());
    // The commit must be in the mirror; a listing of the root proves it.
    mirror
        .list_entries(&commit_id, "")
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "the commit has no tree".to_owned())?;
    let files = MirrorFiles {
        mirror: &mirror,
        commit: &commit_id,
    };
    let mut c = catalog::build(&files, manifest, Limits::default());
    c.commit = Some(commit.to_owned());
    Ok(c)
}

// ---- the sync hook (T040) -----------------------------------------------------------------------

/// Whether `binding` is its workspace's primary binding.
async fn is_primary(db: &PgPool, workspace: Uuid, binding: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "select exists (select 1 from workspaces where id = $1 and primary_binding = $2)",
    )
    .bind(workspace)
    .bind(binding)
    .fetch_one(db)
    .await
}

/// Rebuilds the catalog after a sync of `binding` applied `commit`, when it is the workspace's
/// primary binding and the stored catalog is of another commit (or stale, or missing). Never
/// fails the sync: problems are logged and recorded on the row.
pub(crate) async fn refresh(
    app: &AppState,
    binding: Uuid,
    workspace: Uuid,
    commit: &str,
    manifest: Option<Manifest>,
) {
    match is_primary(&app.db, workspace, binding).await {
        Ok(true) => {}
        Ok(false) => return,
        Err(e) => {
            tracing::error!(error = %e, "catalog: database failed");
            return;
        }
    }
    let current: Result<Option<(String, bool)>, sqlx::Error> =
        sqlx::query_as("select commit, stale from binding_catalogs where binding_id = $1")
            .bind(binding)
            .fetch_optional(&app.db)
            .await;
    match current {
        Ok(Some((c, false))) if c == commit => return,
        Ok(_) => {}
        Err(e) => {
            tracing::error!(error = %e, "catalog: database failed");
            return;
        }
    }
    let span = tracing::info_span!(
        "catalog.build",
        playbooks = tracing::field::Empty,
        roles = tracing::field::Empty,
        inventories = tracing::field::Empty,
        secret_files = tracing::field::Empty,
        problems = tracing::field::Empty,
        skipped_files = tracing::field::Empty,
        ok = tracing::field::Empty,
    );
    let dir: PathBuf = crate::sync::data_dir(app);
    let owned = commit.to_owned();
    let built = tokio::task::spawn_blocking(move || {
        build_blocking(&dir, binding, &owned, manifest.as_ref())
    })
    .await
    .unwrap_or_else(|_| Err("the catalog build stopped unexpectedly".to_owned()));
    let result = match built {
        Ok(c) => {
            span.record("playbooks", c.playbooks.len());
            span.record("roles", c.roles.len());
            span.record("inventories", c.inventories.len());
            span.record("secret_files", c.secrets.files.len());
            span.record("problems", c.problems.len());
            span.record("skipped_files", c.skipped.files);
            span.record("ok", true);
            store(&app.db, binding, commit, &c).await
        }
        Err(error) => {
            span.record("ok", false);
            tracing::warn!(parent: &span, "catalog build failed");
            mark_stale(&app.db, binding, &error).await
        }
    };
    if let Err(e) = result {
        tracing::error!(error = %e, "catalog: database failed");
    }
}

async fn store(db: &PgPool, binding: Uuid, commit: &str, c: &Catalog) -> Result<(), sqlx::Error> {
    let doc = serde_json::to_value(c).unwrap_or(Value::Null);
    sqlx::query(
        "insert into binding_catalogs (binding_id, commit, catalog, built_at, stale, error) \
         values ($1, $2, $3, now(), false, null) \
         on conflict (binding_id) do update set commit = excluded.commit, \
         catalog = excluded.catalog, built_at = now(), stale = false, error = null",
    )
    .bind(binding)
    .bind(commit)
    .bind(doc)
    .execute(db)
    .await?;
    Ok(())
}

/// Marks the stored catalog stale with `error`, keeping its document. No row: nothing to keep.
pub(crate) async fn mark_stale(db: &PgPool, binding: Uuid, error: &str) -> Result<(), sqlx::Error> {
    let mut error = error.to_owned();
    if error.len() > MAX_ERROR_BYTES {
        let mut cut = MAX_ERROR_BYTES;
        while !error.is_char_boundary(cut) {
            cut -= 1;
        }
        error.truncate(cut);
    }
    sqlx::query("update binding_catalogs set stale = true, error = $2 where binding_id = $1")
        .bind(binding)
        .bind(error)
        .execute(db)
        .await?;
    Ok(())
}

// ---- reading the stored catalog -----------------------------------------------------------------

/// A workspace's primary binding and its stored catalog.
pub(crate) struct Primary {
    pub binding: Uuid,
    pub url: String,
    pub branch: String,
    pub stored: Option<Stored>,
}

pub(crate) struct Stored {
    pub commit: String,
    pub catalog: Catalog,
    pub built_at: OffsetDateTime,
    pub stale: bool,
    pub error: Option<String>,
}

/// The workspace's primary binding, or `None` when it has none.
pub(crate) async fn primary(db: &PgPool, workspace: Uuid) -> Result<Option<Primary>, sqlx::Error> {
    type Row = (
        Uuid,
        String,
        String,
        Option<String>,
        Option<Value>,
        Option<OffsetDateTime>,
        Option<bool>,
        Option<String>,
    );
    let row: Option<Row> = sqlx::query_as(
        "select b.id, b.url, b.branch, c.commit, c.catalog, c.built_at, c.stale, c.error \
         from workspaces w join repo_bindings b on b.id = w.primary_binding \
         left join binding_catalogs c on c.binding_id = b.id where w.id = $1",
    )
    .bind(workspace)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| Primary {
        binding: r.0,
        url: r.1,
        branch: r.2,
        stored: match (r.3, r.4, r.5) {
            (Some(commit), Some(doc), Some(built_at)) => serde_json::from_value::<Catalog>(doc)
                .ok()
                .map(|catalog| Stored {
                    commit,
                    catalog,
                    built_at,
                    stale: r.6.unwrap_or(false),
                    error: r.7,
                }),
            _ => None,
        },
    }))
}

/// The host names the static inventories of `c` list, or `None` when it has no static one.
fn listed(c: &Catalog) -> Option<BTreeSet<String>> {
    let mut any = false;
    let mut out = BTreeSet::new();
    for inv in &c.inventories {
        if inv.kind == InventoryKind::Dynamic {
            continue;
        }
        any = true;
        for g in &inv.groups {
            out.extend(g.hosts.iter().cloned());
        }
    }
    any.then_some(out)
}

/// What the workspace's primary catalog says about hosts' inventory status (FR-023).
pub(crate) struct Listing(Option<BTreeSet<String>>);

impl Listing {
    /// `listed`, `pending_inventory`, or `unknown` without a primary catalog with a static
    /// inventory.
    #[must_use]
    pub fn status(&self, inventory_name: &str) -> HostInventoryStatus {
        match &self.0 {
            None => HostInventoryStatus::Unknown,
            Some(names) if names.contains(inventory_name) => HostInventoryStatus::Listed,
            Some(_) => HostInventoryStatus::PendingInventory,
        }
    }
}

/// The workspace's [`Listing`].
pub(crate) async fn listing(db: &PgPool, workspace: Uuid) -> Result<Listing, sqlx::Error> {
    let p = primary(db, workspace).await?;
    Ok(Listing(
        p.and_then(|p| p.stored).and_then(|s| listed(&s.catalog)),
    ))
}

/// The run preview's sops warnings (FR-018): the target hosts that cannot decrypt a sops file
/// job `job` reads, per the primary catalog. Empty when `binding` is not the primary binding or
/// there is no catalog. Warns, never refuses.
pub(crate) async fn run_warnings(
    db: &PgPool,
    workspace: Uuid,
    binding: Uuid,
    job: &str,
    hosts: &[String],
) -> Result<Vec<String>, sqlx::Error> {
    let Some(Primary {
        binding: primary_binding,
        stored: Some(stored),
        ..
    }) = primary(db, workspace).await?
    else {
        return Ok(Vec::new());
    };
    if primary_binding != binding {
        return Ok(Vec::new());
    }
    let files: Vec<_> = stored
        .catalog
        .secrets
        .files
        .iter()
        .filter(|f| f.read_by.iter().any(|j| j == job))
        .collect();
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let keys: Vec<(String, Option<String>)> = sqlx::query_as(
        "select inventory_name, age_recipient from hosts \
         where workspace_id = $1 and inventory_name = any($2) order by inventory_name",
    )
    .bind(workspace)
    .bind(hosts)
    .fetch_all(db)
    .await?;
    let mut warnings = Vec::new();
    let mut unknown = Vec::new();
    for (name, key) in keys {
        let Some(key) = key else {
            unknown.push(name);
            continue;
        };
        for f in &files {
            if !f.age.contains(&key) {
                warnings.push(format!(
                    "Host {name} cannot decrypt {}: its age key is not a recipient.",
                    f.path
                ));
            }
        }
    }
    if !unknown.is_empty() {
        warnings.push(format!(
            "The job reads sops files, and these hosts have not reported an age key, so Keel \
             cannot tell whether they can decrypt them: {}.",
            unknown.join(", ")
        ));
    }
    Ok(warnings)
}

// ---- GetCatalog and DraftJob (T041) -------------------------------------------------------------

fn small(n: u64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The protocol view of a stored catalog. `hosts` is the decrypt state, computed here.
fn view(binding: Uuid, s: &Stored, hosts: Vec<HostDecrypt>) -> CatalogView {
    let c = &s.catalog;
    CatalogView {
        binding,
        commit: s.commit.clone(),
        built_at: ts(s.built_at),
        stale: s.stale,
        error: s.error.clone(),
        playbooks: c
            .playbooks
            .iter()
            .map(|p| CatalogPlaybook {
                path: p.path.clone(),
                plays: p
                    .plays
                    .iter()
                    .map(|pl| CatalogPlay {
                        name: pl.name.clone(),
                        hosts: pl.hosts.clone(),
                    })
                    .collect(),
                roles: p.roles.clone(),
                imports: p.imports.clone(),
                vars_files: p.vars_files.clone(),
                unresolved: p.unresolved.clone(),
                jobs: p.jobs.clone(),
            })
            .collect(),
        roles: c
            .roles
            .iter()
            .map(|r| CatalogRole {
                name: r.name.clone(),
                path: r.path.clone().unwrap_or_default(),
                description: r.description.clone(),
                defaults: r.defaults.clone(),
                dependencies: r.dependencies.clone(),
                reached_by: r.reached_by.clone(),
                external: r.external,
            })
            .collect(),
        inventories: c
            .inventories
            .iter()
            .map(|i| CatalogInventory {
                path: i.path.clone(),
                kind: match i.kind {
                    InventoryKind::Yaml => CatalogInventoryKind::Yaml,
                    InventoryKind::Ini => CatalogInventoryKind::Ini,
                    InventoryKind::Dir => CatalogInventoryKind::Dir,
                    InventoryKind::Dynamic => CatalogInventoryKind::Dynamic,
                },
                groups: i
                    .groups
                    .iter()
                    .map(|g| CatalogGroup {
                        name: g.name.clone(),
                        parent: g
                            .parents
                            .iter()
                            .find(|p| *p != "all")
                            .or_else(|| g.parents.first())
                            .cloned(),
                        hosts: g.hosts.clone(),
                        children: g.children.clone(),
                        vars_file: g.vars_file.clone(),
                    })
                    .collect(),
            })
            .collect(),
        secrets: CatalogSecrets {
            rules_file: c.secrets.rules_file.clone(),
            rules: c
                .secrets
                .rules
                .iter()
                .map(|r| SopsRule {
                    index: small(r.index as u64),
                    path_regex: r.path_regex.clone().unwrap_or_default(),
                    age: r.age.clone(),
                    pgp: r.pgp.clone(),
                })
                .collect(),
            files: c
                .secrets
                .files
                .iter()
                .map(|f| SopsFile {
                    path: f.path.clone(),
                    rule: f.rule.map(|r| small(r as u64)),
                    age: f.age.clone(),
                    pgp: f.pgp.clone(),
                    lastmodified: f.lastmodified.clone(),
                    keys: f.keys.clone(),
                    drift: f.drift.as_ref().map(|d| SopsDrift {
                        missing: d.missing.clone(),
                        extra: d.extra.clone(),
                    }),
                    problem: f.problem.clone(),
                    read_by: f.read_by.clone(),
                })
                .collect(),
        },
        problems: c
            .problems
            .iter()
            .map(|p| CatalogProblem {
                path: p.path.clone(),
                line: p.line,
                message: p.message.clone(),
            })
            .collect(),
        skipped: CatalogSkipped {
            files: small(c.skipped.files),
            bytes: c.skipped.bytes,
            reason: c.skipped.reason.clone(),
        },
        hosts,
    }
}

/// Each enrolled host's decrypt state against `c`: the secret files whose recipients include
/// its age key. Computed on read, so a new key shows at once.
async fn decrypt_state(
    db: &PgPool,
    workspace: Uuid,
    c: &Catalog,
) -> Result<Vec<HostDecrypt>, sqlx::Error> {
    let rows: Vec<(Uuid, String, Option<String>)> = sqlx::query_as(
        "select id, inventory_name, age_recipient from hosts \
         where workspace_id = $1 and state <> 'revoked' order by inventory_name",
    )
    .bind(workspace)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(host, inventory_name, age_recipient)| {
            let decrypts = age_recipient
                .as_deref()
                .map(|key| {
                    c.secrets
                        .files
                        .iter()
                        .filter(|f| f.age.iter().any(|a| a == key))
                        .map(|f| f.path.clone())
                        .collect()
                })
                .unwrap_or_default();
            HostDecrypt {
                host,
                inventory_name,
                age_recipient,
                decrypts,
            }
        })
        .collect())
}

/// The primary catalog, or the refusal: no primary binding, or no catalog yet.
pub(crate) async fn require_catalog(
    db: &PgPool,
    workspace: Uuid,
) -> Result<(Primary, Stored), Vec<Event>> {
    let Some(mut p) = primary(db, workspace).await.map_err(|e| db_failed(&e))? else {
        return Err(op_error(
            "no_primary_repository",
            "This workspace has no primary repository.",
        ));
    };
    let Some(stored) = p.stored.take() else {
        return Err(op_error(
            "no_catalog",
            "The primary repository has not been catalogued yet. Sync it first.",
        ));
    };
    Ok((p, stored))
}

/// `GetCatalog` (View).
pub async fn get_catalog(app: &AppState, req: &Request<'_>, workspace: Uuid) -> Vec<Event> {
    const ACTION: &str = "get_catalog";
    if let Err(d) = require(app, req, ACTION, Capability::View, workspace).await {
        return d;
    }
    let (p, stored) = match require_catalog(&app.db, workspace).await {
        Ok(x) => x,
        Err(e) => return e,
    };
    let hosts = match decrypt_state(&app.db, workspace, &stored.catalog).await {
        Ok(h) => h,
        Err(e) => return db_failed(&e),
    };
    vec![Event::Catalog(view(p.binding, &stored, hosts))]
}

/// The binding's manifest as the database holds it (profiles and repository inventories), for
/// drafting. Job definitions are not needed by a draft.
pub(crate) async fn stored_manifest(db: &PgPool, binding: Uuid) -> Result<Manifest, sqlx::Error> {
    let workspace: Uuid = sqlx::query_scalar(
        "select p.workspace_id from repo_bindings b join projects p on p.id = b.project_id \
         where b.id = $1",
    )
    .bind(binding)
    .fetch_one(db)
    .await?;
    let profiles: Vec<(String, Option<String>)> =
        sqlx::query_as("select name, description from profiles where workspace_id = $1")
            .bind(workspace)
            .fetch_all(db)
            .await?;
    let inventories: Vec<(String, String)> = sqlx::query_as(
        "select name, path from inventories where binding_id = $1 and source = 'repo' \
         and path is not null",
    )
    .bind(binding)
    .fetch_all(db)
    .await?;
    Ok(Manifest {
        profiles: profiles
            .into_iter()
            .map(|(n, description)| (n, keel_core::manifest::Profile { description }))
            .collect(),
        inventories: inventories
            .into_iter()
            .map(|(n, path)| (n, keel_core::manifest::RepoInventory { path, schema: None }))
            .collect(),
        jobs: std::collections::BTreeMap::new(),
    })
}

/// The draft for `playbook`, or the refusal.
pub(crate) async fn draft(
    db: &PgPool,
    workspace: Uuid,
    playbook: &str,
) -> Result<(Primary, Stored, catalog::Draft), Vec<Event>> {
    let (p, stored) = require_catalog(db, workspace).await?;
    let manifest = stored_manifest(db, p.binding)
        .await
        .map_err(|e| db_failed(&e))?;
    let Some(d) = catalog::draft_job(&stored.catalog, Some(&manifest), playbook) else {
        return Err(op_error(
            "playbook_not_found",
            &format!("The catalog has no playbook {playbook}."),
        ));
    };
    Ok((p, stored, d))
}

/// `DraftJob` (View). Stores nothing.
pub async fn draft_job(
    app: &AppState,
    req: &Request<'_>,
    workspace: Uuid,
    playbook: &str,
) -> Vec<Event> {
    const ACTION: &str = "draft_job";
    if let Err(d) = require(app, req, ACTION, Capability::View, workspace).await {
        return d;
    }
    match draft(&app.db, workspace, playbook).await {
        Ok((_, _, d)) => vec![Event::JobDraft {
            yaml: d.yaml,
            notes: d.notes,
        }],
        Err(e) => e,
    }
}

/// Reads `path` at `commit` from `binding`'s mirror, with its blob id. `None` when absent.
pub(crate) async fn read_at(
    app: &AppState,
    binding: Uuid,
    commit: &str,
    path: &str,
) -> Result<Option<(Vec<u8>, String)>, Vec<Event>> {
    let dir = crate::sync::data_dir(app);
    let commit = CommitId(commit.to_owned());
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        let mirror = Mirror::open(&dir, binding).map_err(|_| ())?;
        let Some(blob) = mirror.blob_id(&commit, &path).map_err(|_| ())? else {
            return Ok(None);
        };
        let bytes = mirror.read_file(&commit, &path).map_err(|_| ())?;
        Ok(Some((bytes, blob)))
    })
    .await
    .map_err(|_| internal())?
    .map_err(|()| {
        op_error(
            "mirror_unavailable",
            "Keel could not read the repository mirror. Sync the binding and try again.",
        )
    })
}

#[cfg(test)]
mod tests {
    use keel_core::catalog::{Group, Inventory};

    use super::*;

    fn inv(kind: InventoryKind, hosts: &[&str]) -> Inventory {
        Inventory {
            path: "inventory/hosts.yml".into(),
            kind,
            groups: vec![Group {
                name: "web".into(),
                parents: vec!["all".into()],
                hosts: hosts.iter().map(|h| (*h).to_owned()).collect(),
                children: vec![],
                vars_file: None,
            }],
        }
    }

    #[test]
    fn inventory_status_follows_the_static_inventories() {
        let mut c = Catalog::default();
        assert_eq!(
            Listing(listed(&c)).status("a"),
            HostInventoryStatus::Unknown
        );
        c.inventories.push(inv(InventoryKind::Dynamic, &["a"]));
        assert_eq!(
            Listing(listed(&c)).status("a"),
            HostInventoryStatus::Unknown
        );
        c.inventories.push(inv(InventoryKind::Yaml, &["a"]));
        let l = Listing(listed(&c));
        assert_eq!(l.status("a"), HostInventoryStatus::Listed);
        assert_eq!(l.status("b"), HostInventoryStatus::PendingInventory);
    }
}
