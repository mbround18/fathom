//! Changes Keel proposes to a workspace's primary repository (spec 066, T044): adding a host to
//! the inventory, adding a host's age key to a sops creation rule, and adding a drafted job to
//! the manifest.
//!
//! Every change is computed from the catalogued commit, and goes out as a pull request on a new
//! `keel/…` branch (FR-021), never as a write to an existing branch. Without a forge connection,
//! with writes off, or when the forge refuses, the reply carries the diff to apply by hand.
//! Re-encrypting sops files is always the operator's step (FR-022).
//!
//! The forge is reached through [`Forges`], over the `keel_core::ports::Forge` port only.

use std::sync::Arc;

use keel_core::authz::Capability;
use keel_core::catalog::InventoryKind;
use keel_core::domain::WorkspaceId;
use keel_core::inventory_edit::{self, Edit};
use keel_core::manifest::MANIFEST_PATH;
use keel_core::ports::{ChangeRequest, FileEdit, Forge, ForgeError};
use keel_protocol::{ChangeOutcome, Cmd, EnrollmentIssued, Event};
use secrecy::SecretSlice;
use serde_json::json;
use tracing::Instrument;
use uuid::Uuid;

use crate::audit::{self, Decision, Entry};
use crate::bindings::{db_failed, internal, not_found, op_error, require};
use crate::dispatch::Request;
use crate::governance::{pending, preview};
use crate::hosts::{NewEnrollment, check_enroll, enrollment_event, insert_enrollment};
use crate::identity::Caller;
use crate::preview::Draft;
use crate::repo_catalog::{self, Primary, Stored};
use crate::runs::recheck;
use crate::serve::AppState;

// ---- the forge seam -----------------------------------------------------------------------------

/// What a [`Connector`] connects to: a workspace's forge connection, opened.
pub struct ForgeTarget {
    /// `forgejo`, `gitea` or `github`.
    pub provider: String,
    pub api_base: String,
    /// The primary binding's clone URL; the forge client derives owner and repository from it.
    pub repo_url: String,
    /// The connection's credential, opened. Never logged.
    pub token: SecretSlice<u8>,
}

/// Builds a [`Forge`] client for a target.
pub type Connector = Arc<dyn Fn(ForgeTarget) -> Result<Arc<dyn Forge>, ForgeError> + Send + Sync>;

/// How the server reaches forges. Without a connector every change is `Manual`.
#[derive(Clone)]
pub struct Forges {
    connect: Option<Connector>,
    /// `false`: no forge writes, every change is `Manual` (the kill switch).
    writes: bool,
}

impl Default for Forges {
    fn default() -> Self {
        Self {
            connect: None,
            writes: true,
        }
    }
}

impl Forges {
    #[must_use]
    pub fn with_connector(mut self, connect: Connector) -> Self {
        self.connect = Some(connect);
        self
    }

    #[must_use]
    pub fn with_writes(mut self, writes: bool) -> Self {
        self.writes = writes;
        self
    }
}

/// A short suffix that keeps proposal branches unique.
fn short_id() -> String {
    let s = Uuid::new_v4().simple().to_string();
    s[s.len() - 8..].to_owned()
}

/// Opens `change` on the workspace's forge, or says why it is the operator's to apply.
async fn open_change(
    app: &AppState,
    workspace: Uuid,
    primary: &Primary,
    change: &ChangeRequest,
    diff: String,
) -> ChangeOutcome {
    let manual = |reason: &str| ChangeOutcome::Manual {
        diff: diff.clone(),
        reason: reason.to_owned(),
    };
    if !app.forges.writes {
        return manual("Forge writes are turned off on this server. Apply the change by hand.");
    }
    let row: Result<Option<(String, String, Uuid)>, sqlx::Error> = sqlx::query_as(
        "select provider, api_base, credential_id from forge_connections where workspace_id = $1",
    )
    .bind(workspace)
    .fetch_optional(&app.db)
    .await;
    let (provider, api_base, credential) = match row {
        Ok(Some(r)) => r,
        Ok(None) => {
            return manual(
                "This workspace has no forge connection. Apply the change by hand, or connect a \
                 forge in the repository settings.",
            );
        }
        Err(e) => {
            tracing::error!(error = %e, "forge: database failed");
            return ChangeOutcome::Failed {
                diff,
                error: "Keel could not read the forge connection.".to_owned(),
            };
        }
    };
    let Some(connect) = app.forges.connect.clone() else {
        return manual("This server cannot reach forges. Apply the change by hand.");
    };
    let span = tracing::info_span!(
        "forge.open_change",
        provider = %provider,
        files = change.files.len(),
        outcome = tracing::field::Empty,
    );
    let outcome = async {
        let token = match crate::credentials::open(app, credential).await {
            Ok((_, token)) => token,
            Err(_) => {
                return ChangeOutcome::Failed {
                    diff: diff.clone(),
                    error: "The forge connection's credential could not be opened.".to_owned(),
                };
            }
        };
        let forge = match connect(ForgeTarget {
            provider,
            api_base,
            repo_url: primary.url.clone(),
            token,
        }) {
            Ok(f) => f,
            Err(e) => {
                return ChangeOutcome::Failed {
                    diff: diff.clone(),
                    error: forge_error(&e),
                };
            }
        };
        match forge.open_change(change).await {
            Ok(opened) => ChangeOutcome::Opened {
                url: opened.url,
                branch: opened.branch,
            },
            Err(e) => ChangeOutcome::Failed {
                diff: diff.clone(),
                error: forge_error(&e),
            },
        }
    }
    .instrument(span.clone())
    .await;
    span.record("outcome", outcome_kind(&outcome));
    outcome
}

fn forge_error(e: &ForgeError) -> String {
    match e {
        ForgeError::Conflict(_) => "The file changed since Keel read it, or the branch already \
                                    exists. Sync the repository and try again."
            .to_owned(),
        other => other.to_string(),
    }
}

fn outcome_kind(o: &ChangeOutcome) -> &'static str {
    match o {
        ChangeOutcome::Opened { .. } => "opened",
        ChangeOutcome::Manual { .. } => "manual",
        ChangeOutcome::Failed { .. } => "failed",
    }
}

/// Audits a proposal's outcome. The diff stays out of the log.
async fn audit_outcome(
    app: &AppState,
    caller: &Caller,
    action: &str,
    workspace: Uuid,
    target: serde_json::Value,
    branch: &str,
    outcome: &ChangeOutcome,
) -> Result<(), sqlx::Error> {
    let (url, opened_branch, error) = match outcome {
        ChangeOutcome::Opened { url, branch } => (Some(url.as_str()), Some(branch.as_str()), None),
        ChangeOutcome::Manual { .. } => (None, None, None),
        ChangeOutcome::Failed { error, .. } => (None, None, Some(error.as_str())),
    };
    let mut tx = app.db.begin().await?;
    audit::record(
        &mut tx,
        Entry {
            actor: &caller.actor(),
            action,
            workspace: Some(WorkspaceId(workspace)),
            target,
            decision: Decision::Allowed,
            reason: None,
            context: json!({
                "outcome": outcome_kind(outcome),
                "branch": opened_branch.unwrap_or(branch),
                "url": url,
                "error": error,
            }),
        },
    )
    .await?;
    tx.commit().await
}

/// The one-file change request for `edit` of `path`.
fn request(
    p: &Primary,
    path: &str,
    blob: Option<String>,
    edit: &Edit,
    branch: String,
    title: String,
    body: String,
) -> ChangeRequest {
    ChangeRequest {
        base_branch: p.branch.clone(),
        new_branch: branch,
        title,
        body,
        files: vec![FileEdit {
            path: path.to_owned(),
            expected_blob_sha: blob,
            new_content: edit.new_text.clone().into_bytes(),
        }],
    }
}

fn edit_refused(e: &inventory_edit::EditError) -> Vec<Event> {
    op_error(e.code(), &e.to_string())
}

// ---- dispatch -----------------------------------------------------------------------------------

pub async fn handle(app: &AppState, req: &Request<'_>, cmd: Cmd) -> Vec<Event> {
    match cmd {
        Cmd::PreviewHostAdd {
            workspace,
            inventory_name,
            groups,
            profiles,
            inventory,
        } => {
            let add = HostAdd {
                workspace,
                name: inventory_name.as_str().to_owned(),
                groups: groups.0.iter().map(|g| g.as_str().to_owned()).collect(),
                profiles: profiles.0.iter().map(|p| p.as_str().to_owned()).collect(),
                inventory: inventory.map(|p| p.as_str().to_owned()),
            };
            preview_host_add(app, req, add).await
        }
        Cmd::PreviewSopsRecipient { host } => preview_sops_recipient(app, req, host).await,
        Cmd::PreviewJobPropose {
            workspace,
            playbook,
            name,
        } => {
            preview_job_propose(
                app,
                req,
                workspace,
                playbook.as_str().to_owned(),
                name.as_str().to_owned(),
            )
            .await
        }
        _ => internal(),
    }
}

// ---- PreviewHostAdd -----------------------------------------------------------------------------

#[derive(Clone)]
struct HostAdd {
    workspace: Uuid,
    name: String,
    groups: Vec<String>,
    profiles: Vec<String>,
    inventory: Option<String>,
}

/// The inventory edit for `add`: the primary binding, the file, its blob and the edit.
async fn host_edit(
    app: &AppState,
    add: &HostAdd,
) -> Result<(Primary, String, String, Edit), Vec<Event>> {
    let (p, stored) = repo_catalog::require_catalog(&app.db, add.workspace).await?;
    let path = choose_inventory(&stored, add)?;
    let Some((bytes, blob)) = repo_catalog::read_at(app, p.binding, &stored.commit, &path).await?
    else {
        return Err(op_error(
            "inventory_not_found",
            &format!("{path} is not in the repository at the catalogued commit."),
        ));
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return Err(op_error(
            "unparseable",
            &format!("{path} is not UTF-8 text."),
        ));
    };
    let groups: Vec<&str> = add.groups.iter().map(String::as_str).collect();
    let edit =
        inventory_edit::add_host_groups(&text, &groups, &add.name).map_err(|e| edit_refused(&e))?;
    Ok((p, path, blob, edit))
}

/// The inventory to edit: the one asked for, else the YAML inventory that has every group.
fn choose_inventory(stored: &Stored, add: &HostAdd) -> Result<String, Vec<Event>> {
    let invs = &stored.catalog.inventories;
    if let Some(path) = &add.inventory {
        let Some(inv) = invs.iter().find(|i| &i.path == path) else {
            return Err(op_error(
                "inventory_not_found",
                &format!("The catalog has no inventory {path}."),
            ));
        };
        if inv.kind != InventoryKind::Yaml {
            return Err(op_error(
                "inventory_not_yaml",
                &format!("{path} is not a YAML inventory; Keel edits only YAML inventories."),
            ));
        }
        return Ok(path.clone());
    }
    let has_all = |inv: &keel_core::catalog::Inventory| {
        add.groups
            .iter()
            .all(|g| inv.groups.iter().any(|ig| &ig.name == g))
    };
    if let Some(inv) = invs
        .iter()
        .find(|i| i.kind == InventoryKind::Yaml && has_all(i))
    {
        return Ok(inv.path.clone());
    }
    if invs.iter().any(|i| i.kind == InventoryKind::Yaml) {
        Err(op_error(
            "group_not_found",
            &format!(
                "No YAML inventory has every group of: {}.",
                add.groups.join(", ")
            ),
        ))
    } else {
        Err(op_error(
            "inventory_not_yaml",
            "The repository has no YAML inventory Keel can edit.",
        ))
    }
}

async fn preview_host_add(app: &AppState, req: &Request<'_>, add: HostAdd) -> Vec<Event> {
    const ACTION: &str = "preview_host_add";
    if let Err(d) = require(app, req, ACTION, Capability::Operate, add.workspace).await {
        return d;
    }
    if app.repos.master_key.is_none() || app.ssh.get().is_none() {
        return op_error(
            "ssh_ca_unavailable",
            "The SSH certificate authorities are not loaded.",
        );
    }
    if let Err(e) = check_enroll(app, add.workspace, &add.name, &add.profiles).await {
        return e;
    }
    let path = match host_edit(app, &add).await {
        Ok((_, path, _, _)) => path,
        Err(e) => return e,
    };
    let summary = format!(
        "Enrol host {} with profiles {}, and propose adding it to group(s) {} in {path} on a new \
         branch. The token is shown once and expires in 15 minutes.",
        add.name,
        if add.profiles.is_empty() {
            "(none)".to_owned()
        } else {
            add.profiles.join(", ")
        },
        add.groups.join(", "),
    );
    let affected = vec![add.name.clone(), path];
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Operate, add.workspace) {
            return d;
        }
        confirm_host_add(&app, &caller, add).await
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary,
            affected,
            step_up: false,
        },
        op,
    )
}

async fn confirm_host_add(app: &AppState, caller: &Caller, add: HostAdd) -> Vec<Event> {
    if let Err(e) = check_enroll(app, add.workspace, &add.name, &add.profiles).await {
        return e;
    }
    let (p, path, blob, edit) = match host_edit(app, &add).await {
        Ok(x) => x,
        Err(e) => return e,
    };
    let new = match NewEnrollment::mint(app) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let slug: Result<String, sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        let slug = insert_enrollment(
            &mut tx,
            caller,
            add.workspace,
            &add.name,
            &add.profiles,
            &new,
            None,
        )
        .await?;
        tx.commit().await?;
        Ok(slug)
    }
    .await;
    let slug = match slug {
        Ok(s) => s,
        Err(e) => return db_failed(&e),
    };
    let branch = format!("keel/add-host-{}-{}", add.name, short_id());
    let change = request(
        &p,
        &path,
        Some(blob),
        &edit,
        branch.clone(),
        format!("Add host {} to the inventory", add.name),
        format!(
            "Keel proposes adding host `{}` to group(s) {} in `{path}`.\n\nThe host was enrolled \
             in Keel; it runs jobs once this is merged and synced.",
            add.name,
            add.groups.join(", ")
        ),
    );
    let outcome = open_change(app, add.workspace, &p, &change, edit.unified_diff(&path)).await;
    if let Err(e) = audit_outcome(
        app,
        caller,
        "host_add_proposed",
        add.workspace,
        json!({ "inventory_name": add.name, "path": path, "groups": add.groups }),
        &branch,
        &outcome,
    )
    .await
    {
        return db_failed(&e);
    }
    let enrollment = match enrollment_event(
        app,
        new,
        add.workspace,
        &slug,
        add.name.clone(),
        add.profiles.clone(),
    )
    .await
    {
        Event::HostEnrollment {
            id,
            workspace,
            inventory_name,
            profiles,
            expires_at,
            token,
            snippet,
            instructions,
            instructions_unavailable,
        } => EnrollmentIssued {
            id,
            workspace,
            inventory_name,
            profiles,
            expires_at,
            token,
            snippet,
            instructions,
            instructions_unavailable,
        },
        _ => return internal(),
    };
    vec![Event::HostAdded {
        enrollment,
        change: outcome,
    }]
}

// ---- PreviewSopsRecipient -----------------------------------------------------------------------

const REENCRYPT: &str = "Re-encrypting the files the rule covers is a manual step: once this is \
                         merged, run `sops updatekeys` on each of them with a key that can \
                         already decrypt them.";

/// The host's workspace, name and age key.
async fn host_key(
    app: &AppState,
    host: Uuid,
) -> Result<(Uuid, String, Option<String>), Vec<Event>> {
    let row: Option<(Uuid, String, Option<String>)> = sqlx::query_as(
        "select workspace_id, inventory_name, age_recipient from hosts where id = $1",
    )
    .bind(host)
    .fetch_optional(&app.db)
    .await
    .map_err(|e| db_failed(&e))?;
    row.ok_or_else(|| not_found("host"))
}

/// The rules file edit adding `key`: the rule of the first file the host cannot decrypt, else
/// the first rule without the key.
async fn sops_edit(
    app: &AppState,
    workspace: Uuid,
    key: &str,
) -> Result<(Primary, String, String, usize, Edit), Vec<Event>> {
    let (p, stored) = repo_catalog::require_catalog(&app.db, workspace).await?;
    let secrets = &stored.catalog.secrets;
    let (Some(rules_file), false) = (&secrets.rules_file, secrets.rules.is_empty()) else {
        return Err(op_error(
            "no_sops_rule",
            "The repository has no sops creation rules.",
        ));
    };
    let index = secrets
        .files
        .iter()
        .filter(|f| !f.age.iter().any(|a| a == key))
        .find_map(|f| f.rule)
        .or_else(|| {
            secrets
                .rules
                .iter()
                .find(|r| !r.age.iter().any(|a| a == key))
                .map(|r| r.index)
        })
        .ok_or_else(|| {
            op_error(
                "recipient_exists",
                "Every creation rule already lists this host's age key.",
            )
        })?;
    let Some((bytes, blob)) =
        repo_catalog::read_at(app, p.binding, &stored.commit, rules_file).await?
    else {
        return Err(op_error(
            "no_sops_rule",
            &format!("{rules_file} is not in the repository at the catalogued commit."),
        ));
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return Err(op_error(
            "unparseable",
            &format!("{rules_file} is not UTF-8 text."),
        ));
    };
    let edit =
        inventory_edit::add_sops_recipient(&text, index, key).map_err(|e| edit_refused(&e))?;
    Ok((p, rules_file.clone(), blob, index, edit))
}

async fn preview_sops_recipient(app: &AppState, req: &Request<'_>, host: Uuid) -> Vec<Event> {
    const ACTION: &str = "preview_sops_recipient";
    let (workspace, name, key) = match host_key(app, host).await {
        Ok(x) => x,
        Err(e) => return e,
    };
    if let Err(d) = require(app, req, ACTION, Capability::Administer, workspace).await {
        return d;
    }
    let Some(key) = key else {
        return op_error(
            "no_age_recipient",
            "The host has not reported an age key. Install the sops role on it and reconnect \
             its runner.",
        );
    };
    let (path, index) = match sops_edit(app, workspace, &key).await {
        Ok((_, path, _, index, _)) => (path, index),
        Err(e) => return e,
    };
    let summary = format!(
        "Propose adding host {name}'s age key to creation rule {index} of {path} on a new \
         branch. {REENCRYPT}"
    );
    let affected = vec![name.clone(), path];
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Administer, workspace) {
            return d;
        }
        let (p, path, blob, index, edit) = match sops_edit(&app, workspace, &key).await {
            Ok(x) => x,
            Err(e) => return e,
        };
        let branch = format!("keel/sops-recipient-{name}-{}", short_id());
        let change = request(
            &p,
            &path,
            Some(blob),
            &edit,
            branch.clone(),
            format!("Add host {name}'s age key to sops rule {index}"),
            format!(
                "Keel proposes adding host `{name}`'s age key to creation rule {index} of \
                 `{path}`.\n\n{REENCRYPT}"
            ),
        );
        let outcome = open_change(&app, workspace, &p, &change, edit.unified_diff(&path)).await;
        let outcome = match outcome {
            ChangeOutcome::Manual { diff, reason } => ChangeOutcome::Manual {
                diff,
                reason: format!("{reason} {REENCRYPT}"),
            },
            other => other,
        };
        if let Err(e) = audit_outcome(
            &app,
            &caller,
            "sops_recipient_proposed",
            workspace,
            json!({ "host": host, "path": path, "rule": index }),
            &branch,
            &outcome,
        )
        .await
        {
            return db_failed(&e);
        }
        vec![Event::ChangeProposed(outcome)]
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary,
            affected,
            step_up: false,
        },
        op,
    )
}

// ---- PreviewJobPropose --------------------------------------------------------------------------

/// The body of a drafted job: its YAML without the comments and the `name:` line, unindented.
fn job_body(yaml: &str) -> String {
    let mut out = String::new();
    for line in yaml.lines().filter(|l| !l.starts_with('#')).skip(1) {
        out.push_str(line.strip_prefix("  ").unwrap_or(line));
        out.push('\n');
    }
    out
}

/// The manifest edit adding the draft for `playbook` as job `name`.
async fn job_edit(
    app: &AppState,
    workspace: Uuid,
    playbook: &str,
    name: &str,
) -> Result<(Primary, Option<String>, Edit, Vec<String>), Vec<Event>> {
    let (p, stored, draft) = repo_catalog::draft(&app.db, workspace, playbook).await?;
    let (text, blob) =
        match repo_catalog::read_at(app, p.binding, &stored.commit, MANIFEST_PATH).await? {
            Some((bytes, blob)) => match String::from_utf8(bytes) {
                Ok(t) => (t, Some(blob)),
                Err(_) => {
                    return Err(op_error(
                        "unparseable",
                        &format!("{MANIFEST_PATH} is not UTF-8 text."),
                    ));
                }
            },
            None => (String::new(), None),
        };
    let edit = inventory_edit::add_job(&text, name, &job_body(&draft.yaml))
        .map_err(|e| edit_refused(&e))?;
    Ok((p, blob, edit, draft.notes))
}

async fn preview_job_propose(
    app: &AppState,
    req: &Request<'_>,
    workspace: Uuid,
    playbook: String,
    name: String,
) -> Vec<Event> {
    const ACTION: &str = "preview_job_propose";
    if let Err(d) = require(app, req, ACTION, Capability::Administer, workspace).await {
        return d;
    }
    if let Err(e) = job_edit(app, workspace, &playbook, &name).await {
        return e;
    }
    let summary = format!(
        "Propose adding job {name} for playbook {playbook} to {MANIFEST_PATH} on a new branch."
    );
    let affected = vec![name.clone(), MANIFEST_PATH.to_owned()];
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Administer, workspace) {
            return d;
        }
        let (p, blob, edit, notes) = match job_edit(&app, workspace, &playbook, &name).await {
            Ok(x) => x,
            Err(e) => return e,
        };
        let branch = format!("keel/add-job-{name}-{}", short_id());
        let mut body = format!(
            "Keel proposes adding job `{name}` for playbook `{playbook}` to `{MANIFEST_PATH}`."
        );
        if !notes.is_empty() {
            body.push_str("\n\nCheck before merging:\n");
            for n in &notes {
                body.push_str(&format!("\n- {n}"));
            }
        }
        let change = request(
            &p,
            MANIFEST_PATH,
            blob,
            &edit,
            branch.clone(),
            format!("Add job {name}"),
            body,
        );
        let outcome = open_change(
            &app,
            workspace,
            &p,
            &change,
            edit.unified_diff(MANIFEST_PATH),
        )
        .await;
        if let Err(e) = audit_outcome(
            &app,
            &caller,
            "job_proposed",
            workspace,
            json!({ "job": name, "playbook": playbook }),
            &branch,
            &outcome,
        )
        .await
        {
            return db_failed(&e);
        }
        vec![Event::ChangeProposed(outcome)]
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary,
            affected,
            step_up: false,
        },
        op,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_body_drops_comments_and_the_name() {
        let yaml = "# check the profiles\nsite:\n  playbook: site.yml\n  paths:\n    - site.yml\n";
        assert_eq!(job_body(yaml), "playbook: site.yml\npaths:\n  - site.yml\n");
    }

    #[test]
    fn short_ids_are_eight_hex_digits() {
        let s = short_id();
        assert_eq!(s.len(), 8);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
