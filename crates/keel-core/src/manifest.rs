//! The repository manifest, `.keel/jobs.yaml` (spec 055 T046, contracts/repo-manifest.md).
//!
//! The repository declares jobs; Keel decides whether they run. Parsing is strict (`version: 1`,
//! unknown keys refused) and validation collects every problem, so a manifest that fails leaves
//! the previous job set active and the binding shows `manifest_invalid` with all the errors.
//!
//! Nothing here reads a repository: the caller passes a [`RepoTree`] for the fetched head.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Where the manifest lives in the repository.
pub const MANIFEST_PATH: &str = ".keel/jobs.yaml";

/// The largest manifest Keel parses.
pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;

/// Symbolic links followed while resolving one path before it counts as a loop.
const MAX_SYMLINKS: usize = 16;

/// What a path is at the commit being validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathKind {
    File,
    Dir,
    /// A symbolic link, with its target as stored.
    Symlink(String),
    /// A submodule or anything else that is not a file in this tree.
    Other,
}

/// A read-only view of the repository tree at one commit.
pub trait RepoTree {
    /// What `path` (relative, `/`-separated, no symlinks followed) is, or `None` when absent.
    fn kind(&self, path: &str) -> Option<PathKind>;

    /// The names of the directories directly inside directory `path` (not symbolic links to
    /// directories), in name order; `None` when `path` is absent or not a directory. Used by
    /// `choice` params with `from:` (spec 068). The default cannot list, so a `from:` choice
    /// fails closed on a tree that does not implement it.
    fn list_dirs(&self, path: &str) -> Option<Vec<String>> {
        let _ = path;
        None
    }
}

/// One validation problem: where it is and what is wrong. Never contains file contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestIssue {
    /// A dotted location, such as `jobs.web-deploy.playbook`.
    pub at: String,
    pub message: String,
    /// A stable code for the rules added by spec 068 (`profile_cycle`, `env_name_reserved`, …;
    /// contracts/manifest.md). Absent for older rules, and omitted from JSON when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl ManifestIssue {
    fn new(at: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            at: at.into(),
            message: message.into(),
            code: None,
        }
    }

    fn coded(at: impl Into<String>, code: &str, message: impl Into<String>) -> Self {
        Self {
            at: at.into(),
            message: message.into(),
            code: Some(code.to_owned()),
        }
    }
}

/// What validation needs to know besides the tree.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// The binding is in signed mode: jobs may only use repository inventories.
    pub signed_mode: bool,
    /// Credential names granted to this binding (for any profile).
    pub granted_credentials: BTreeSet<String>,
}

/// A validated manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Manifest {
    pub profiles: BTreeMap<String, Profile>,
    pub inventories: BTreeMap<String, RepoInventory>,
    pub jobs: BTreeMap<String, Job>,
    /// Inventory group → profiles it suggests (spec 068 `groups:`). Never assigns.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub groups: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    #[serde(default)]
    pub description: Option<String>,
    /// Profiles applied first, in order, recursively (spec 068 recipe).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub includes: Vec<String>,
    /// Jobs applied after the includes, in order (spec 068 recipe).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub jobs: Vec<String>,
}

impl Profile {
    /// Whether the profile declares a recipe. Without one, a converge runs every job whose
    /// `profiles` names it, as before spec 068.
    #[must_use]
    pub fn has_recipe(&self) -> bool {
        !self.includes.is_empty() || !self.jobs.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoInventory {
    pub path: String,
    #[serde(default)]
    pub schema: Option<String>,
}

/// A validated job: what is stored in `jobs.definition`, and whether it can run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Job {
    pub definition: JobDefinition,
    /// Credentials the job names that are not granted to the binding. Non-empty means the job is
    /// `blocked (credential_unavailable)`.
    pub missing_credentials: Vec<String>,
}

impl Job {
    /// `Some("credential_unavailable")` when the job cannot run.
    #[must_use]
    pub fn blocked_reason(&self) -> Option<&'static str> {
        (!self.missing_credentials.is_empty()).then_some("credential_unavailable")
    }
}

/// Where a job's inventory comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InventorySource {
    Repo,
    Managed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobDefinition {
    pub playbook: String,
    pub profiles: Vec<String>,
    pub inventory: String,
    pub inventory_source: InventorySource,
    pub limit: Option<String>,
    pub vars: BTreeMap<String, serde_json::Value>,
    pub credentials: Vec<String>,
    /// Path globs that trigger a run; `None` means manual only.
    pub paths: Option<Vec<String>>,
    pub timeout_secs: Option<u64>,
    pub rollback: Option<Rollback>,
    /// Values Keel may fill per run (spec 061 R4). Omitted when empty, so a definition stored
    /// before 061 reads back, and one without params stores exactly as before.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, ParamSpec>,
    /// Jobs this one applies in full at the same commit (spec 068 `covers`). Omitted when empty,
    /// so a definition without it stores exactly as before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub covers: Vec<String>,
    /// Environment name → credential name (an `env_value` credential), set for
    /// `ansible-playbook` (spec 068 `env`). Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

/// The most params one job may declare.
pub const MAX_PARAMS: usize = 8;

/// One declared job parameter: what kind of value Keel may pass for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParamSpec {
    pub kind: ParamKind,
    /// `choice` only: the allowed values (1–[`MAX_CHOICE_VALUES`]). Exactly one of `values` and
    /// `from`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<String>>,
    /// `choice` only: a repo-relative directory whose subdirectory names are the allowed values
    /// at the commit (hidden and ill-formed names ignored; more than [`MAX_CHOICE_FROM`] is
    /// refused).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// `choice` only: the value a form starts with. Must be one of the values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

impl ParamSpec {
    /// A param of `kind` with no `choice` fields.
    #[must_use]
    pub const fn of(kind: ParamKind) -> Self {
        Self {
            kind,
            values: None,
            from: None,
            default: None,
        }
    }

    /// A `choice` param's allowed values at `tree`: `values`, or the matching subdirectory names
    /// of `from`, in order. `None` for other kinds.
    ///
    /// # Errors
    ///
    /// `from` is missing or not a directory at the tree, or lists more than [`MAX_CHOICE_FROM`]
    /// entries; or the spec has neither or both of `values` and `from`.
    pub fn choices(&self, tree: &dyn RepoTree) -> Option<Result<Vec<String>, ChoiceError>> {
        if self.kind != ParamKind::Choice {
            return None;
        }
        Some(match (&self.values, &self.from) {
            (Some(values), None) => Ok(values.clone()),
            (None, Some(from)) => choices_from(tree, from),
            _ => Err(ChoiceError::Shape),
        })
    }

    /// Whether `value` may be passed for this param at `tree`: of the right kind and, for a
    /// `choice`, one of the choices there. The error names no value.
    ///
    /// # Errors
    ///
    /// Why the value doesn't fit.
    pub fn check_value(&self, value: &str, tree: &dyn RepoTree) -> Result<(), String> {
        self.kind.validate(value).map_err(str::to_owned)?;
        match self.choices(tree) {
            None => Ok(()),
            Some(Err(e)) => Err(e.to_string()),
            Some(Ok(c)) if c.iter().any(|v| v == value) => Ok(()),
            Some(Ok(_)) => Err("not one of the choices at the commit".to_owned()),
        }
    }
}

/// The most `values` a `choice` param may list.
pub const MAX_CHOICE_VALUES: usize = 64;

/// The most entries a `choice` param's `from:` directory may hold, counting only matching names.
pub const MAX_CHOICE_FROM: usize = 256;

/// Why a `choice` param's values cannot be listed. `code` is the manifest validation code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ChoiceError {
    #[error("a choice has exactly one of values (1-64 valid names) or from")]
    Shape,
    #[error("the choice's from directory is missing at the commit")]
    FromMissing,
    #[error("the choice's from directory has more than 256 entries")]
    FromTooMany,
}

impl ChoiceError {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Shape => "choice_shape",
            Self::FromMissing => "choice_from_missing",
            Self::FromTooMany => "choice_from_too_many",
        }
    }
}

/// `^[A-Za-z0-9][A-Za-z0-9._-]{0,62}$`: a `choice` value.
#[must_use]
pub fn is_choice_value(v: &str) -> bool {
    let mut bytes = v.bytes();
    v.len() <= 63
        && matches!(bytes.next(), Some(b) if b.is_ascii_alphanumeric())
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn choices_from(tree: &dyn RepoTree, from: &str) -> Result<Vec<String>, ChoiceError> {
    if check_relative(from).is_err() || tree.kind(from.trim_end_matches('/')) != Some(PathKind::Dir)
    {
        return Err(ChoiceError::FromMissing);
    }
    let names = tree
        .list_dirs(from.trim_end_matches('/'))
        .ok_or(ChoiceError::FromMissing)?;
    // Hidden names fail `is_choice_value` (they start with '.').
    let names: Vec<String> = names.into_iter().filter(|n| is_choice_value(n)).collect();
    if names.len() > MAX_CHOICE_FROM {
        return Err(ChoiceError::FromTooMany);
    }
    Ok(names)
}

/// The kinds a parameter can be. Closed: anything else is refused at parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    /// `sha256:` and 64 lowercase hex characters.
    OciDigest,
    /// One of a list of names (spec 068): `values`, or the directories under `from` at the
    /// commit. [`ParamKind::validate`] checks only the name's shape; [`ParamSpec::check_value`]
    /// checks it is one of the choices.
    Choice,
}

impl ParamKind {
    /// Whether `value` is a value of this kind. The error never echoes the value.
    ///
    /// # Errors
    ///
    /// Why the value doesn't fit.
    pub fn validate(self, value: &str) -> Result<(), &'static str> {
        match self {
            Self::OciDigest => {
                let ok = value.strip_prefix("sha256:").is_some_and(|hex| {
                    hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                });
                if ok {
                    Ok(())
                } else {
                    Err("not an OCI digest (sha256: and 64 lowercase hex characters)")
                }
            }
            Self::Choice => {
                if is_choice_value(value) {
                    Ok(())
                } else {
                    Err("not a choice value ([A-Za-z0-9][A-Za-z0-9._-]{0,62})")
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rollback {
    #[serde(default)]
    pub playbook: Option<String>,
    #[serde(default)]
    pub reapply: Option<Reapply>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reapply {
    LastSuccess,
}

// ---- wire shape -------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    version: u32,
    #[serde(default)]
    profiles: BTreeMap<String, Option<Profile>>,
    #[serde(default)]
    inventories: BTreeMap<String, RepoInventory>,
    #[serde(default)]
    jobs: BTreeMap<String, RawJob>,
    #[serde(default)]
    groups: BTreeMap<String, Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawJob {
    playbook: String,
    #[serde(default)]
    profiles: Vec<String>,
    inventory: String,
    #[serde(default)]
    limit: Option<String>,
    #[serde(default)]
    vars: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    credentials: Vec<String>,
    #[serde(default)]
    paths: Option<Vec<String>>,
    #[serde(default)]
    timeout: Option<String>,
    #[serde(default)]
    rollback: Option<Rollback>,
    #[serde(default)]
    params: BTreeMap<String, ParamSpec>,
    #[serde(default)]
    covers: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

/// Parses and validates `bytes` (the manifest at the fetched head) against `tree`.
///
/// # Errors
///
/// Every problem found, in a stable order. The caller keeps the previous job set.
pub fn validate(
    bytes: &[u8],
    tree: &dyn RepoTree,
    ctx: &Context,
) -> Result<Manifest, Vec<ManifestIssue>> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(vec![ManifestIssue::new(
            "",
            format!("manifest is larger than {MAX_MANIFEST_BYTES} bytes"),
        )]);
    }
    let raw: RawManifest = match serde_yaml::from_slice(bytes) {
        Ok(raw) => raw,
        // serde_yaml's message names the key and line, not the value.
        Err(e) => return Err(vec![ManifestIssue::new("", e.to_string())]),
    };
    let mut issues = Vec::new();
    if raw.version != 1 {
        issues.push(ManifestIssue::new(
            "version",
            format!("unsupported version {} (expected 1)", raw.version),
        ));
    }

    let mut profiles = BTreeMap::new();
    for (name, profile) in raw.profiles {
        if !is_name(&name) {
            issues.push(ManifestIssue::new(
                format!("profiles.{name}"),
                "profile names use a-z, 0-9, '-' and '_' (at most 63)",
            ));
        }
        profiles.insert(name, profile.unwrap_or_default());
    }

    for (name, inv) in &raw.inventories {
        let at = format!("inventories.{name}");
        if !is_name(name) {
            issues.push(ManifestIssue::new(
                &at,
                "inventory names use a-z, 0-9, '-' and '_' (at most 63)",
            ));
        }
        require_file(tree, &inv.path, &format!("{at}.path"), &mut issues);
        if let Some(schema) = &inv.schema {
            require_file(tree, schema, &format!("{at}.schema"), &mut issues);
        }
    }

    let declared_jobs: BTreeSet<String> = raw.jobs.keys().cloned().collect();
    let mut jobs = BTreeMap::new();
    for (name, job) in raw.jobs {
        let at = format!("jobs.{name}");
        if !is_name(&name) {
            issues.push(ManifestIssue::new(
                &at,
                "job names use a-z, 0-9, '-' and '_' (at most 63)",
            ));
        }
        validate_covers(&at, &job.covers, &declared_jobs, &mut issues);
        if let Some(job) = validate_job(
            &at,
            job,
            tree,
            ctx,
            &profiles,
            &raw.inventories,
            &mut issues,
        ) {
            jobs.insert(name, job);
        }
    }

    validate_recipes(&profiles, &declared_jobs, &mut issues);
    validate_groups(&raw.groups, &profiles, &mut issues);

    if issues.is_empty() {
        let manifest = Manifest {
            profiles,
            inventories: raw.inventories,
            jobs,
            groups: raw.groups,
        };
        validate_plans(&manifest, &mut issues);
        if issues.is_empty() {
            Ok(manifest)
        } else {
            Err(issues)
        }
    } else {
        Err(issues)
    }
}

fn validate_job(
    at: &str,
    job: RawJob,
    tree: &dyn RepoTree,
    ctx: &Context,
    profiles: &BTreeMap<String, Profile>,
    inventories: &BTreeMap<String, RepoInventory>,
    issues: &mut Vec<ManifestIssue>,
) -> Option<Job> {
    let before = issues.len();
    require_file(tree, &job.playbook, &format!("{at}.playbook"), issues);

    if job.profiles.is_empty() {
        issues.push(ManifestIssue::new(
            format!("{at}.profiles"),
            "at least one profile is required",
        ));
    }
    for p in &job.profiles {
        if !profiles.contains_key(p) {
            issues.push(ManifestIssue::new(
                format!("{at}.profiles"),
                format!("profile '{p}' is not declared under profiles"),
            ));
        }
    }

    let inventory_source = if inventories.contains_key(&job.inventory) {
        InventorySource::Repo
    } else if ctx.signed_mode {
        issues.push(ManifestIssue::new(
            format!("{at}.inventory"),
            format!(
                "'{}' is not a repository inventory; signed mode only uses repository inventories",
                job.inventory
            ),
        ));
        InventorySource::Repo
    } else {
        if !is_name(&job.inventory) {
            issues.push(ManifestIssue::new(
                format!("{at}.inventory"),
                "inventory names use a-z, 0-9, '-' and '_' (at most 63)",
            ));
        }
        InventorySource::Managed
    };

    if let Some(limit) = &job.limit
        && let Err(why) = check_limit(limit)
    {
        issues.push(ManifestIssue::new(format!("{at}.limit"), why));
    }

    for key in job.vars.keys() {
        if let Err(why) = check_var_name(key) {
            issues.push(ManifestIssue::new(format!("{at}.vars.{key}"), why));
        }
    }

    if job.params.len() > MAX_PARAMS {
        issues.push(ManifestIssue::new(
            format!("{at}.params"),
            format!("a job declares at most {MAX_PARAMS} params"),
        ));
    }
    for (name, spec) in &job.params {
        let pat = format!("{at}.params.{name}");
        if let Err(why) = check_param_name(name) {
            issues.push(ManifestIssue::new(&pat, why));
        }
        validate_param(&pat, spec, tree, issues);
        if job.vars.contains_key(name) {
            issues.push(ManifestIssue::new(
                &pat,
                format!("'{name}' is also set under vars"),
            ));
        }
    }

    let mut seen = BTreeSet::new();
    for c in &job.credentials {
        if !is_name(c) {
            issues.push(ManifestIssue::new(
                format!("{at}.credentials"),
                "credential names use a-z, 0-9, '-' and '_' (at most 63)",
            ));
        }
        if !seen.insert(c) {
            issues.push(ManifestIssue::new(
                format!("{at}.credentials"),
                format!("'{c}' is listed twice"),
            ));
        }
    }

    if let Some(paths) = &job.paths {
        for (i, p) in paths.iter().enumerate() {
            let pat = format!("{at}.paths[{i}]");
            if let Err(why) = check_relative(p) {
                issues.push(ManifestIssue::new(&pat, why));
            } else if globset::Glob::new(p).is_err() {
                issues.push(ManifestIssue::new(&pat, "not a valid glob"));
            }
        }
    }

    let timeout_secs = match job.timeout.as_deref().map(parse_duration) {
        None => None,
        Some(Ok(s)) => Some(s),
        Some(Err(why)) => {
            issues.push(ManifestIssue::new(format!("{at}.timeout"), why));
            None
        }
    };

    if let Some(rb) = &job.rollback {
        if rb.playbook.is_none() && rb.reapply.is_none() {
            issues.push(ManifestIssue::new(
                format!("{at}.rollback"),
                "rollback needs a playbook, reapply, or both",
            ));
        }
        if let Some(pb) = &rb.playbook {
            require_file(tree, pb, &format!("{at}.rollback.playbook"), issues);
        }
    }

    validate_env(at, &job.env, issues);

    if issues.len() != before {
        return None;
    }
    let missing_credentials = job
        .credentials
        .iter()
        .filter(|c| !ctx.granted_credentials.contains(*c))
        .cloned()
        .collect();
    Some(Job {
        definition: JobDefinition {
            playbook: job.playbook,
            profiles: job.profiles,
            inventory: job.inventory,
            inventory_source,
            limit: job.limit,
            vars: job.vars,
            credentials: job.credentials,
            paths: job.paths,
            timeout_secs,
            rollback: job.rollback,
            params: job.params,
            covers: job.covers,
            env: job.env,
        },
        missing_credentials,
    })
}

/// `jobs.<j>.covers`: declared jobs, not itself, each once. Cycles are found by
/// [`validate_plans`] once every job is valid.
fn validate_covers(
    at: &str,
    covers: &[String],
    declared_jobs: &BTreeSet<String>,
    issues: &mut Vec<ManifestIssue>,
) {
    let me = at.strip_prefix("jobs.").unwrap_or(at);
    let cat = format!("{at}.covers");
    let mut seen = BTreeSet::new();
    for c in covers {
        if c == me {
            issues.push(ManifestIssue::coded(
                &cat,
                "self_cover",
                format!("job '{me}' covers itself"),
            ));
        } else if !declared_jobs.contains(c) {
            issues.push(ManifestIssue::coded(
                &cat,
                "unknown_cover",
                format!("'{c}' is not a job in this manifest"),
            ));
        } else if !seen.insert(c) {
            issues.push(ManifestIssue::coded(
                &cat,
                "unknown_cover",
                format!("'{c}' is listed twice"),
            ));
        }
    }
}

/// `jobs.<j>.env`: at most [`crate::env_names::MAX_JOB_ENV`] entries, each a valid, unreserved
/// name mapped to a credential name. Whether the credential exists is the server's concern
/// (a warning at sync, `credential_unavailable` at dispatch).
fn validate_env(at: &str, env: &BTreeMap<String, String>, issues: &mut Vec<ManifestIssue>) {
    use crate::env_names::{self, MAX_JOB_ENV};
    if env.len() > MAX_JOB_ENV {
        issues.push(ManifestIssue::coded(
            format!("{at}.env"),
            "env_too_many",
            format!("a job sets at most {MAX_JOB_ENV} environment names"),
        ));
    }
    for (name, credential) in env {
        let nat = format!("{at}.env.{name}");
        if let Err(e) = env_names::check(name) {
            issues.push(ManifestIssue::coded(&nat, e.code(), e.to_string()));
        }
        if !is_name(credential) {
            issues.push(ManifestIssue::coded(
                &nat,
                "env_name_invalid",
                "the value is a credential name: a-z, 0-9, '-' and '_' (at most 63)",
            ));
        }
    }
}

/// `profiles.<p>.includes` and `.jobs`: declared names, no job twice in one recipe.
fn validate_recipes(
    profiles: &BTreeMap<String, Profile>,
    declared_jobs: &BTreeSet<String>,
    issues: &mut Vec<ManifestIssue>,
) {
    for (name, p) in profiles {
        for inc in &p.includes {
            if !profiles.contains_key(inc) {
                issues.push(ManifestIssue::coded(
                    format!("profiles.{name}.includes"),
                    "unknown_include",
                    format!("profile '{inc}' is not declared under profiles"),
                ));
            }
        }
        let at = format!("profiles.{name}.jobs");
        let mut seen = BTreeSet::new();
        for job in &p.jobs {
            if !declared_jobs.contains(job) {
                issues.push(ManifestIssue::coded(
                    &at,
                    "unknown_recipe_job",
                    format!("job '{job}' is not declared under jobs"),
                ));
            } else if !seen.insert(job) {
                issues.push(ManifestIssue::coded(
                    &at,
                    "duplicate_recipe_job",
                    format!("job '{job}' is listed twice"),
                ));
            }
        }
    }
}

/// `groups:` values must be declared profiles. Keys are inventory groups and are not checked:
/// an unknown group simply matches no host.
fn validate_groups(
    groups: &BTreeMap<String, Vec<String>>,
    profiles: &BTreeMap<String, Profile>,
    issues: &mut Vec<ManifestIssue>,
) {
    for (group, targets) in groups {
        for p in targets {
            if !profiles.contains_key(p) {
                issues.push(ManifestIssue::coded(
                    format!("groups.{group}"),
                    "unknown_group_profile",
                    format!("profile '{p}' is not declared under profiles"),
                ));
            }
        }
    }
}

/// What needs the whole manifest: every profile expands (no cycle, at most 32 steps and 8
/// deep) and `covers` closes without a cycle. Each cycle is reported once.
fn validate_plans(m: &Manifest, issues: &mut Vec<ManifestIssue>) {
    use crate::converge::{self, CoverError, ExpandError};
    let mut cycles: BTreeSet<BTreeSet<String>> = BTreeSet::new();
    for name in m.profiles.keys() {
        match converge::expand(std::slice::from_ref(name), m) {
            Ok(_) => {}
            Err(ExpandError::Cycle(c)) => {
                if cycles.insert(c.iter().cloned().collect()) {
                    issues.push(ManifestIssue::coded(
                        format!("profiles.{name}.includes"),
                        "profile_cycle",
                        ExpandError::Cycle(c).to_string(),
                    ));
                }
            }
            Err(e) => issues.push(ManifestIssue::coded(
                format!("profiles.{name}"),
                e.code(),
                e.to_string(),
            )),
        }
    }
    if let Err(e) = converge::coverage(m) {
        let at = match &e {
            CoverError::SelfCover(j) | CoverError::Unknown { job: j, .. } => j.clone(),
            CoverError::Cycle(c) => c.first().cloned().unwrap_or_default(),
        };
        issues.push(ManifestIssue::coded(
            format!("jobs.{at}.covers"),
            e.code(),
            e.to_string(),
        ));
    }
}

/// Names of profiles, jobs, inventories and credentials.
fn is_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 63
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-' || *c == b'_')
}

/// A param's `choice` fields (spec 068): only on kind `choice`; exactly one of `values` (1–64
/// distinct valid names) or `from` (a directory at the commit, at most 256 matching entries); a
/// `default` that is one of the values.
fn validate_param(
    at: &str,
    spec: &ParamSpec,
    tree: &dyn RepoTree,
    issues: &mut Vec<ManifestIssue>,
) {
    if spec.kind != ParamKind::Choice {
        if spec.values.is_some() || spec.from.is_some() || spec.default.is_some() {
            issues.push(ManifestIssue::coded(
                at,
                ChoiceError::Shape.code(),
                "values, from and default are only for kind choice",
            ));
        }
        return;
    }
    if let Some(values) = &spec.values {
        let distinct: BTreeSet<&String> = values.iter().collect();
        if values.is_empty()
            || values.len() > MAX_CHOICE_VALUES
            || distinct.len() != values.len()
            || !values.iter().all(|v| is_choice_value(v))
        {
            issues.push(ManifestIssue::coded(
                format!("{at}.values"),
                ChoiceError::Shape.code(),
                format!(
                    "1 to {MAX_CHOICE_VALUES} distinct values matching [A-Za-z0-9][A-Za-z0-9._-]{{0,62}}"
                ),
            ));
            return;
        }
    }
    let choices = match spec.choices(tree) {
        Some(Ok(c)) => c,
        Some(Err(e)) => {
            let at = if e == ChoiceError::Shape {
                at.to_owned()
            } else {
                format!("{at}.from")
            };
            issues.push(ManifestIssue::coded(at, e.code(), e.to_string()));
            return;
        }
        None => return,
    };
    if let Some(d) = &spec.default
        && !choices.contains(d)
    {
        issues.push(ManifestIssue::coded(
            format!("{at}.default"),
            "choice_default",
            "the default is not one of the choices",
        ));
    }
}

/// A path relative to the repository root that cannot leave it lexically.
fn check_relative(p: &str) -> Result<(), String> {
    if p.is_empty() {
        return Err("path is empty".into());
    }
    if p.starts_with('/') {
        return Err("absolute paths are not allowed".into());
    }
    if p.chars().any(|c| c.is_control() || c == '\\') {
        return Err("path contains a control character or backslash".into());
    }
    if p.split('/').any(|c| c == "..") {
        return Err("'..' is not allowed".into());
    }
    Ok(())
}

/// Splits a relative path into its components, dropping `.` and empty ones.
fn components(p: &str) -> Vec<&str> {
    p.split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .collect()
}

/// Resolves `path` inside the tree, following symbolic links, refusing any that leave it.
/// Returns what the path finally is, or `None` when it does not exist.
fn resolve(tree: &dyn RepoTree, path: &str) -> Result<Option<PathKind>, String> {
    let mut pending: Vec<String> = components(path)
        .into_iter()
        .rev()
        .map(str::to_owned)
        .collect();
    let mut resolved: Vec<String> = Vec::new();
    let mut followed = 0;
    let mut last = PathKind::Dir;
    while let Some(part) = pending.pop() {
        if part == ".." {
            if resolved.pop().is_none() {
                return Err("a symbolic link leaves the repository".into());
            }
            last = PathKind::Dir;
            continue;
        }
        resolved.push(part);
        let here = resolved.join("/");
        match tree.kind(&here) {
            None => return Ok(None),
            Some(PathKind::Symlink(target)) => {
                followed += 1;
                if followed > MAX_SYMLINKS {
                    return Err("too many symbolic links".into());
                }
                if target.starts_with('/') {
                    return Err("a symbolic link points outside the repository".into());
                }
                resolved.pop();
                for c in components(&target).into_iter().rev() {
                    pending.push(c.to_owned());
                }
                last = PathKind::Dir;
            }
            Some(kind) => {
                if !pending.is_empty() && kind != PathKind::Dir {
                    return Ok(None);
                }
                last = kind;
            }
        }
    }
    Ok(Some(last))
}

fn require_file(tree: &dyn RepoTree, path: &str, at: &str, issues: &mut Vec<ManifestIssue>) {
    if let Err(why) = check_relative(path) {
        issues.push(ManifestIssue::new(at, why));
        return;
    }
    match resolve(tree, path) {
        Err(why) => issues.push(ManifestIssue::new(at, why)),
        Ok(Some(PathKind::File)) => {}
        Ok(Some(_)) => issues.push(ManifestIssue::new(at, format!("'{path}' is not a file"))),
        Ok(None) => issues.push(ManifestIssue::new(
            at,
            format!("'{path}' does not exist at this commit"),
        )),
    }
}

/// Extra-var names: identifiers, and never one that carries a secret Ansible reads.
fn check_var_name(key: &str) -> Result<(), String> {
    let b = key.as_bytes();
    let ident = !b.is_empty()
        && (b[0].is_ascii_alphabetic() || b[0] == b'_')
        && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_');
    if !ident {
        return Err("variable names are letters, digits and '_'".into());
    }
    let k = key.to_ascii_lowercase();
    if k.starts_with("ansible_")
        && (k.contains("password")
            || k.starts_with("ansible_ssh_private_key")
            || k.starts_with("ansible_become_pass"))
    {
        return Err("secret variables come only from credentials".into());
    }
    Ok(())
}

/// Param names: `^[a-z_][a-z0-9_]{0,63}$`, and never in Ansible's or Keel's own namespace.
fn check_param_name(name: &str) -> Result<(), &'static str> {
    let b = name.as_bytes();
    let ok = !b.is_empty()
        && b.len() <= 64
        && (b[0].is_ascii_lowercase() || b[0] == b'_')
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_');
    if !ok {
        return Err("a parameter name is a-z, 0-9 and '_', not starting with a digit (at most 64)");
    }
    if name.starts_with("ansible_") || name.starts_with("keel_internal_") {
        return Err("a parameter name may not start with ansible_ or keel_internal_");
    }
    Ok(())
}

/// An Ansible host pattern: terms separated by `:` or `,`, each optionally prefixed by `!` or
/// `&`, naming a host or group (with `*`/`?` wildcards and an optional `[a:b]` slice) or a `~`
/// regex. It is passed as one argument, so it may not start with `-` or contain whitespace.
pub fn check_limit(limit: &str) -> Result<(), String> {
    if limit.is_empty() {
        return Err("limit is empty".into());
    }
    if limit.starts_with('-') {
        return Err("limit may not start with '-'".into());
    }
    if limit.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("limit may not contain whitespace".into());
    }
    let mut terms = Vec::new();
    let mut depth = 0u32;
    let mut start = 0;
    for (i, c) in limit.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| "unbalanced ']' in limit".to_owned())?;
            }
            ':' | ',' if depth == 0 => {
                terms.push(&limit[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err("unbalanced '[' in limit".into());
    }
    terms.push(&limit[start..]);
    for term in terms {
        check_term(term).map_err(|why| format!("'{term}': {why}"))?;
    }
    Ok(())
}

fn check_term(term: &str) -> Result<(), &'static str> {
    let term = term.strip_prefix(['!', '&']).unwrap_or(term);
    if term.is_empty() {
        return Err("empty term");
    }
    if let Some(re) = term.strip_prefix('~') {
        return if re.is_empty() {
            Err("empty regex")
        } else {
            Ok(())
        };
    }
    let (name, slice) = match term.find('[') {
        Some(i) => (&term[..i], Some(&term[i..])),
        None => (term, None),
    };
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '*' | '?'))
    {
        return Err("not a host or group name");
    }
    if let Some(slice) = slice {
        let inner = slice
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .ok_or("bad slice")?;
        let ok = match inner.split_once(':') {
            Some((a, b)) => {
                (a.is_empty() || a.parse::<i64>().is_ok())
                    && (b.is_empty() || b.parse::<i64>().is_ok())
            }
            None => inner.parse::<i64>().is_ok(),
        };
        if !ok {
            return Err("bad slice");
        }
    }
    Ok(())
}

/// `90s`, `30m`, `2h` or a bare number of seconds. Between one second and 24 hours.
fn parse_duration(s: &str) -> Result<u64, String> {
    let (num, mult) = match s.as_bytes().last() {
        Some(b's') => (&s[..s.len() - 1], 1),
        Some(b'm') => (&s[..s.len() - 1], 60),
        Some(b'h') => (&s[..s.len() - 1], 3600),
        _ => (s, 1),
    };
    let n: u64 = num
        .parse()
        .map_err(|_| "timeout is a number with s, m or h (such as 30m)".to_owned())?;
    let secs = n.saturating_mul(mult);
    if secs == 0 || secs > 24 * 3600 {
        return Err("timeout must be between 1s and 24h".into());
    }
    Ok(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits() {
        for ok in [
            "web:&prod",
            "all",
            "web[0:5],db",
            "!staging",
            "~web\\d+",
            "web*",
            "a[2]",
            "a[:3]",
        ] {
            assert!(check_limit(ok).is_ok(), "{ok}");
        }
        for bad in [
            "", "-x", "web prod", "web[0:5", "web]", "a::b", "&", "web[x]", "a$b",
        ] {
            assert!(check_limit(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("30m"), Ok(1800));
        assert_eq!(parse_duration("90"), Ok(90));
        assert_eq!(parse_duration("2h"), Ok(7200));
        assert!(parse_duration("0s").is_err());
        assert!(parse_duration("25h").is_err());
        assert!(parse_duration("1d").is_err());
    }
}
