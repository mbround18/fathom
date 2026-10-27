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
}

/// One validation problem: where it is and what is wrong. Never contains file contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestIssue {
    /// A dotted location, such as `jobs.web-deploy.playbook`.
    pub at: String,
    pub message: String,
}

impl ManifestIssue {
    fn new(at: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            at: at.into(),
            message: message.into(),
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Manifest {
    pub profiles: BTreeMap<String, Profile>,
    pub inventories: BTreeMap<String, RepoInventory>,
    pub jobs: BTreeMap<String, Job>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    #[serde(default)]
    pub description: Option<String>,
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
        profiles.insert(name, profile.unwrap_or(Profile { description: None }));
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

    let mut jobs = BTreeMap::new();
    for (name, job) in raw.jobs {
        let at = format!("jobs.{name}");
        if !is_name(&name) {
            issues.push(ManifestIssue::new(
                &at,
                "job names use a-z, 0-9, '-' and '_' (at most 63)",
            ));
        }
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

    if issues.is_empty() {
        Ok(Manifest {
            profiles,
            inventories: raw.inventories,
            jobs,
        })
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
        },
        missing_credentials,
    })
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
