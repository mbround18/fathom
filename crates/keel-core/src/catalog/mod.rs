//! The static catalog of an Ansible repository (spec 066, R4–R6).
//!
//! Built from a [`RepoFiles`] view of one commit: playbooks, roles, inventories, sops metadata
//! and the jobs that read them. Pure: nothing here executes repository content, follows a
//! symbolic link, or leaves the tree, and no sops value is ever kept (R5).

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::manifest::Manifest;

pub mod draft;
pub mod inventory;
pub mod playbook;
pub mod role;
pub mod sops;

pub use draft::{Draft, DraftJob, draft_job};
pub use inventory::{Group, Inventory, InventoryKind};
pub use playbook::{Play, Playbook};
pub use role::Role;
pub use sops::{Drift, Rule, SecretFile, Secrets};

/// The document version written into [`Catalog::version`].
pub const CATALOG_VERSION: u32 = 1;

/// Files every run reads, listed in [`Catalog::shared_files`] when they exist (R6).
pub const SHARED_FILES: [&str; 3] = [
    "ansible.cfg",
    "inventory/group_vars/all.yml",
    "requirements.yml",
];

/// Directory names whose YAML is never a playbook (R4).
const NOT_PLAYBOOK_DIRS: [&str; 9] = [
    "roles",
    "inventory",
    "group_vars",
    "host_vars",
    ".github",
    ".forgejo",
    ".keel",
    "molecule",
    ".git",
];

/// Directory nesting walked before giving up on a subtree.
const MAX_DIR_DEPTH: usize = 32;

// ---- the file port ---------------------------------------------------------------------------

/// What a directory entry is. Symbolic links are listed but never followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Other,
}

/// One directory entry: its name (no `/`) and kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
}

/// A read-only view of a repository at one commit. Paths are relative and `/`-separated.
pub trait RepoFiles {
    /// The bytes of the regular file at `path`, or `None`. Must not follow symbolic links.
    fn read(&self, path: &str) -> Option<Vec<u8>>;
    /// The entries of directory `dir` (`""` is the root), or `None` when it is not one.
    fn list(&self, dir: &str) -> Option<Vec<Entry>>;
}

/// An in-memory repository: regular files only, directories implied by their children.
#[derive(Debug, Clone, Default)]
pub struct MemFiles(pub BTreeMap<String, Vec<u8>>);

impl MemFiles {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a file.
    #[must_use]
    pub fn with(mut self, path: &str, content: impl AsRef<[u8]>) -> Self {
        self.0.insert(path.to_owned(), content.as_ref().to_vec());
        self
    }
}

impl RepoFiles for MemFiles {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.0.get(path).cloned()
    }

    fn list(&self, dir: &str) -> Option<Vec<Entry>> {
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let mut out = BTreeMap::new();
        for key in self.0.keys() {
            if let Some(rest) = key.strip_prefix(&prefix) {
                match rest.split_once('/') {
                    Some((d, _)) => out.insert(d.to_owned(), EntryKind::Dir),
                    None => out.insert(rest.to_owned(), EntryKind::File),
                };
            }
        }
        if out.is_empty() && !dir.is_empty() {
            return None;
        }
        Some(
            out.into_iter()
                .map(|(name, kind)| Entry { name, kind })
                .collect(),
        )
    }
}

// ---- the document ----------------------------------------------------------------------------

/// How much of a repository the catalog reads before it stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    /// Regular files listed; the rest are counted in [`Skipped`].
    pub max_files: usize,
    /// Bytes of any one file read.
    pub max_file_bytes: usize,
    /// Bytes read in total.
    pub max_total_bytes: usize,
    /// How deep role dependencies, task includes and playbook imports are followed.
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_files: 5000,
            max_file_bytes: 1024 * 1024,
            max_total_bytes: 32 * 1024 * 1024,
            max_depth: 8,
        }
    }
}

/// The catalog of one commit of a repository.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub version: u32,
    /// The commit it was built from; the caller sets it.
    #[serde(default)]
    pub commit: Option<String>,
    /// Which of [`SHARED_FILES`] exist.
    #[serde(default)]
    pub shared_files: Vec<String>,
    pub playbooks: Vec<Playbook>,
    pub roles: Vec<Role>,
    pub inventories: Vec<Inventory>,
    pub secrets: Secrets,
    pub problems: Vec<Problem>,
    pub skipped: Skipped,
}

/// Something in the repository the catalog could not make sense of. Never holds file content.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Problem {
    pub path: String,
    #[serde(default)]
    pub line: Option<u32>,
    pub message: String,
}

/// What the limits left out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skipped {
    pub files: u64,
    pub bytes: u64,
    /// The first limit that was hit.
    #[serde(default)]
    pub reason: Option<String>,
}

// ---- reading the tree ------------------------------------------------------------------------

/// The walked tree plus a size-limited, cached reader. Only paths found by the walk are read.
pub(crate) struct Repo<'a> {
    files: &'a dyn RepoFiles,
    pub(crate) limits: Limits,
    pub(crate) paths: BTreeSet<String>,
    pub(crate) dirs: BTreeSet<String>,
    cache: RefCell<BTreeMap<String, Option<String>>>,
    total: Cell<usize>,
    skipped: RefCell<Skipped>,
    problems: RefCell<Vec<Problem>>,
}

impl<'a> Repo<'a> {
    pub(crate) fn walk(files: &'a dyn RepoFiles, limits: Limits) -> Self {
        let repo = Self {
            files,
            limits,
            paths: BTreeSet::new(),
            dirs: BTreeSet::new(),
            cache: RefCell::new(BTreeMap::new()),
            total: Cell::new(0),
            skipped: RefCell::new(Skipped::default()),
            problems: RefCell::new(Vec::new()),
        };
        let mut paths = BTreeSet::new();
        let mut dirs = BTreeSet::new();
        let mut stack = vec![(String::new(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            let Some(mut entries) = files.list(&dir) else {
                continue;
            };
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            for e in entries {
                if e.name.is_empty() || e.name.contains('/') || e.name == "." || e.name == ".." {
                    continue;
                }
                let path = if dir.is_empty() {
                    e.name.clone()
                } else {
                    format!("{dir}/{}", e.name)
                };
                match e.kind {
                    EntryKind::File => {
                        if paths.len() < limits.max_files {
                            paths.insert(path);
                        } else {
                            repo.skip(0, format!("more than {} files", limits.max_files));
                        }
                    }
                    EntryKind::Dir if e.name != ".git" => {
                        if depth + 1 < MAX_DIR_DEPTH {
                            dirs.insert(path.clone());
                            stack.push((path, depth + 1));
                        } else {
                            repo.problem(&path, None, "nested too deep; not read");
                        }
                    }
                    _ => {}
                }
            }
        }
        Self {
            paths,
            dirs,
            ..repo
        }
    }

    fn skip(&self, bytes: usize, reason: String) {
        let mut s = self.skipped.borrow_mut();
        s.files += 1;
        s.bytes += bytes as u64;
        s.reason.get_or_insert(reason);
    }

    pub(crate) fn problem(&self, path: &str, line: Option<u32>, message: impl Into<String>) {
        self.problems.borrow_mut().push(Problem {
            path: path.to_owned(),
            line,
            message: message.into(),
        });
    }

    pub(crate) fn exists(&self, path: &str) -> bool {
        self.paths.contains(path)
    }

    /// The first of `candidates` that is a file.
    pub(crate) fn first<'p>(&self, candidates: &[&'p str]) -> Option<&'p str> {
        candidates.iter().copied().find(|p| self.exists(p))
    }

    /// Regular files under `dir`, recursively.
    pub(crate) fn files_under<'s>(&'s self, dir: &'s str) -> impl Iterator<Item = &'s String> + 's {
        let prefix = format!("{dir}/");
        self.paths
            .range(prefix.clone()..)
            .take_while(move |p| p.starts_with(&prefix))
    }

    /// The UTF-8 text of a walked file, within the limits, or `None`.
    pub(crate) fn text(&self, path: &str) -> Option<String> {
        if !self.exists(path) {
            return None;
        }
        if let Some(hit) = self.cache.borrow().get(path) {
            return hit.clone();
        }
        let got = self.load(path);
        self.cache.borrow_mut().insert(path.to_owned(), got.clone());
        got
    }

    fn load(&self, path: &str) -> Option<String> {
        let bytes = self.files.read(path)?;
        let len = bytes.len();
        if len > self.limits.max_file_bytes {
            self.skip(
                len,
                format!("files larger than {} bytes", self.limits.max_file_bytes),
            );
            return None;
        }
        if self.total.get() + len > self.limits.max_total_bytes {
            self.skip(
                len,
                format!("more than {} bytes in total", self.limits.max_total_bytes),
            );
            return None;
        }
        self.total.set(self.total.get() + len);
        String::from_utf8(bytes).ok()
    }

    /// Parses a walked YAML file. A parse failure is a problem naming the line only, since the
    /// parser's message can quote the document.
    pub(crate) fn yaml(&self, path: &str) -> Option<serde_yaml::Value> {
        let text = self.text(path)?;
        match parse_yaml(&text) {
            Ok(v) => Some(v),
            Err(line) => {
                self.problem(path, line, "not valid YAML");
                None
            }
        }
    }
}

/// Parses YAML, returning the failing line (1-based) on error.
pub(crate) fn parse_yaml(text: &str) -> Result<serde_yaml::Value, Option<u32>> {
    serde_yaml::from_str(text).map_err(|e| e.location().and_then(|l| u32::try_from(l.line()).ok()))
}

// ---- small helpers ---------------------------------------------------------------------------

/// The directory part of `path` (`""` for a top-level file).
pub(crate) fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// `rel` resolved against directory `base`, or `None` when it is absolute or leaves the tree.
pub(crate) fn join(base: &str, rel: &str) -> Option<String> {
    if rel.starts_with('/') || rel.contains('\\') || rel.chars().any(char::is_control) {
        return None;
    }
    let mut parts: Vec<&str> = base
        .split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .collect();
    for c in rel.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            c => parts.push(c),
        }
    }
    let out = parts.join("/");
    (!out.is_empty()).then_some(out)
}

/// A value Ansible would template.
pub(crate) fn templated(s: &str) -> bool {
    s.contains("{{") || s.contains("{%")
}

/// `{{ playbook_dir }}/rest` → `rest`, when `rest` itself is literal.
pub(crate) fn playbook_dir_relative(s: &str) -> Option<&str> {
    let rest = s.trim().strip_prefix("{{")?.trim_start();
    let rest = rest.strip_prefix("playbook_dir")?.trim_start();
    let rest = rest.strip_prefix("}}")?.strip_prefix('/')?;
    (!templated(rest)).then_some(rest)
}

/// A scalar as text; `None` for mappings, sequences and null.
pub(crate) fn scalar(v: &serde_yaml::Value) -> Option<String> {
    use serde_yaml::Value;
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Tagged(t) => scalar(&t.value),
        _ => None,
    }
}

/// `map[key]` for a string key.
pub(crate) fn get<'v>(v: &'v serde_yaml::Value, key: &str) -> Option<&'v serde_yaml::Value> {
    v.as_mapping()?.get(key)
}

/// A string, or every string in a sequence (nested sequences flattened).
pub(crate) fn strings(v: &serde_yaml::Value) -> Vec<String> {
    match v {
        serde_yaml::Value::Sequence(items) => items.iter().flat_map(strings).collect(),
        other => scalar(other).into_iter().collect(),
    }
}

fn is_yaml(path: &str) -> bool {
    path.ends_with(".yml") || path.ends_with(".yaml")
}

// ---- ansible.cfg -----------------------------------------------------------------------------

#[derive(Debug, Default)]
pub(crate) struct AnsibleCfg {
    pub(crate) roles_path: Vec<String>,
    pub(crate) inventory: Vec<String>,
}

/// The `[defaults]` keys the catalog uses: relative `roles_path` and `inventory` entries.
pub(crate) fn ansible_cfg(text: &str) -> AnsibleCfg {
    let mut cfg = AnsibleCfg::default();
    let mut in_defaults = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(section) = line.strip_prefix('[') {
            in_defaults = section.trim_end_matches(']').trim() == "defaults";
            continue;
        }
        if !in_defaults {
            continue;
        }
        let Some((key, value)) = line.split_once(['=', ':']) else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        let relative = |s: &str| -> Option<String> {
            let s = s.trim();
            (!s.is_empty() && !s.starts_with('~') && !s.starts_with('$') && !templated(s))
                .then(|| join("", s))
                .flatten()
        };
        match key {
            "roles_path" => cfg.roles_path.extend(value.split(':').filter_map(relative)),
            "inventory" | "hostfile" => cfg.inventory.extend(value.split(',').filter_map(relative)),
            _ => {}
        }
    }
    cfg
}

// ---- build -----------------------------------------------------------------------------------

/// Builds the catalog of the repository behind `files`. `manifest` links jobs to what they read.
#[must_use]
pub fn build(files: &impl RepoFiles, manifest: Option<&Manifest>, limits: Limits) -> Catalog {
    let repo = Repo::walk(files, limits);
    let cfg = repo
        .text("ansible.cfg")
        .map(|t| ansible_cfg(&t))
        .unwrap_or_default();

    let mut role_dirs = vec!["roles".to_owned()];
    for d in cfg.roles_path {
        if !role_dirs.contains(&d) {
            role_dirs.push(d);
        }
    }
    let role_index = role::discover(&repo, &role_dirs);

    // Playbooks: their own plays and reach, then imports folded in.
    let mut scans = BTreeMap::new();
    for path in &repo.paths {
        if !is_yaml(path)
            || path
                .split('/')
                .any(|c| NOT_PLAYBOOK_DIRS.contains(&c) || role_dirs.iter().any(|d| d == c))
            || path == ".sops.yaml"
        {
            continue;
        }
        let Some(doc) = repo.yaml(path) else { continue };
        if playbook::is_playbook(&doc) {
            scans.insert(path.clone(), playbook::scan(&repo, &role_index, path, &doc));
        }
    }
    let mut playbooks = playbook::assemble(&scans, limits.max_depth);

    let mut sources: Vec<String> = Vec::new();
    if let Some(m) = manifest {
        sources.extend(m.inventories.values().map(|i| i.path.clone()));
    }
    sources.extend(cfg.inventory);
    let inventories = inventory::discover(&repo, &sources);

    let mut secrets = sops::scan(&repo);

    // Which sops files each playbook reads: named in vars_files / load_vars, or named in the
    // group vars of the groups it targets.
    let sops_paths: BTreeSet<&str> = secrets.files.iter().map(|f| f.path.as_str()).collect();
    for pb in &mut playbooks {
        let mut found: BTreeSet<String> = pb
            .vars_files
            .iter()
            .filter(|v| sops_paths.contains(v.as_str()))
            .cloned()
            .collect();
        for vars in group_vars_for(&inventories, None, pb.all_hosts.iter().map(String::as_str)) {
            for text in vars_texts(&repo, &vars) {
                found.extend(
                    sops_paths
                        .iter()
                        .filter(|s| text.contains(**s))
                        .map(|s| (*s).to_owned()),
                );
            }
        }
        pb.secrets = found.into_iter().collect();
    }

    if let Some(m) = manifest {
        for pb in &mut playbooks {
            pb.jobs = m
                .jobs
                .iter()
                .filter(|(_, j)| j.definition.playbook == pb.path)
                .map(|(n, _)| n.clone())
                .collect();
        }
        for file in &mut secrets.files {
            let mut readers = BTreeSet::new();
            for (name, job) in &m.jobs {
                let by_glob = job.definition.paths.iter().flatten().any(|g| {
                    globset::Glob::new(g).is_ok_and(|g| g.compile_matcher().is_match(&file.path))
                });
                let by_name = playbooks
                    .iter()
                    .find(|p| p.path == job.definition.playbook)
                    .is_some_and(|p| p.secrets.contains(&file.path));
                if by_glob || by_name {
                    readers.insert(name.clone());
                }
            }
            file.read_by = readers.into_iter().collect();
        }
    }

    let roles = role::assemble(&role_index, &playbooks);
    let shared_files = SHARED_FILES
        .iter()
        .filter(|f| repo.exists(f))
        .map(|f| (*f).to_owned())
        .collect();

    let mut problems = repo.problems.take();
    problems.sort();
    problems.dedup();
    Catalog {
        version: CATALOG_VERSION,
        commit: None,
        shared_files,
        playbooks,
        roles,
        inventories,
        secrets,
        problems,
        skipped: repo.skipped.take(),
    }
}

/// The texts of a group's vars (a file, or every file under a `group_vars/<g>/` directory).
fn vars_texts(repo: &Repo<'_>, vars: &str) -> Vec<String> {
    match vars.strip_suffix("/**") {
        Some(dir) => repo.files_under(dir).filter_map(|p| repo.text(p)).collect(),
        None => repo.text(vars).into_iter().collect(),
    }
}

/// The vars files of the groups that `patterns` (play `hosts:` values) name, plus those of
/// every ancestor group. `inventory` limits it to one inventory path when that is catalogued.
pub(crate) fn group_vars_for<'p>(
    inventories: &[Inventory],
    inventory: Option<&str>,
    patterns: impl IntoIterator<Item = &'p str>,
) -> BTreeSet<String> {
    let chosen: Vec<&Inventory> = match inventory {
        Some(p) if inventories.iter().any(|i| i.path == p) => {
            inventories.iter().filter(|i| i.path == p).collect()
        }
        _ => inventories.iter().collect(),
    };
    let names: Vec<&str> = patterns
        .into_iter()
        .filter(|p| !templated(p))
        .flat_map(|p| p.split([':', ',']))
        .map(|t| t.trim().trim_start_matches(['&', '!']))
        .filter(|t| !t.is_empty() && !t.contains(['*', '?', '[']) && !t.starts_with('~'))
        .collect();
    let mut out = BTreeSet::new();
    for inv in chosen {
        let by_name: BTreeMap<&str, &Group> =
            inv.groups.iter().map(|g| (g.name.as_str(), g)).collect();
        let mut stack: Vec<&str> = names.clone();
        let mut seen = BTreeSet::new();
        while let Some(n) = stack.pop() {
            if !seen.insert(n) {
                continue;
            }
            if let Some(g) = by_name.get(n) {
                out.extend(g.vars_file.clone());
                stack.extend(g.parents.iter().map(String::as_str));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_stays_inside() {
        assert_eq!(
            join("playbooks", "../secrets/a.yaml").as_deref(),
            Some("secrets/a.yaml")
        );
        assert_eq!(join("playbooks", "../../x"), None);
        assert_eq!(join("", "/etc/passwd"), None);
        assert_eq!(join("a", "./b/./c/").as_deref(), Some("a/b/c"));
        assert_eq!(join("", ".."), None);
    }

    #[test]
    fn playbook_dir_prefix() {
        assert_eq!(
            playbook_dir_relative("{{ playbook_dir }}/files/x/"),
            Some("files/x/")
        );
        assert_eq!(playbook_dir_relative("{{playbook_dir}}/a"), Some("a"));
        assert_eq!(playbook_dir_relative("{{ playbook_dir }}/{{ x }}"), None);
        assert_eq!(playbook_dir_relative("files/a"), None);
    }

    #[test]
    fn cfg_relative_entries_only() {
        let cfg = ansible_cfg(
            "[defaults]\nroles_path = roles:~/.ansible/roles:/etc/ansible/roles:extra\n\
             inventory = inventory/hosts.yml,/etc/hosts\n[ssh_connection]\nroles_path = no\n",
        );
        assert_eq!(cfg.roles_path, ["roles", "extra"]);
        assert_eq!(cfg.inventory, ["inventory/hosts.yml"]);
    }

    #[test]
    fn mem_files_lists_implied_dirs() {
        let m = MemFiles::new().with("a/b/c.yml", "x").with("top.yml", "y");
        let root = m.list("").unwrap();
        assert_eq!(root.len(), 2);
        assert_eq!(m.list("a").unwrap()[0].kind, EntryKind::Dir);
        assert!(m.list("missing").is_none());
    }

    #[test]
    fn limits_count_skipped_files_and_bytes() {
        let files = MemFiles::new()
            .with("a.yml", "- hosts: all\n")
            .with("b.yml", "- hosts: all\n")
            .with("big.yml", format!("- hosts: all\n#{}\n", "x".repeat(100)));
        let c = build(
            &files,
            None,
            Limits {
                max_files: 2,
                ..Limits::default()
            },
        );
        assert_eq!(c.playbooks.len(), 2);
        assert_eq!(c.skipped.files, 1);
        assert!(c.skipped.reason.as_deref().unwrap().contains("2 files"));

        let c = build(
            &files,
            None,
            Limits {
                max_file_bytes: 50,
                ..Limits::default()
            },
        );
        assert_eq!(c.playbooks.len(), 2);
        assert_eq!(c.skipped.files, 1);
        assert!(c.skipped.bytes > 100);

        let c = build(
            &files,
            None,
            Limits {
                max_total_bytes: 20,
                ..Limits::default()
            },
        );
        assert_eq!(c.playbooks.len(), 1);
        assert_eq!(c.skipped.files, 2);
    }

    #[test]
    fn invalid_yaml_is_a_problem_without_content() {
        let files = MemFiles::new().with("site.yml", "- hosts: all\n  bad: [secretvalue\n");
        let c = build(&files, None, Limits::default());
        assert!(c.playbooks.is_empty());
        assert_eq!(c.problems.len(), 1);
        assert_eq!(c.problems[0].path, "site.yml");
        assert!(!c.problems[0].message.contains("secretvalue"));
    }

    #[test]
    fn group_vars_walk_ancestors() {
        let inv = Inventory {
            path: "inv/hosts.yml".into(),
            kind: InventoryKind::Yaml,
            groups: vec![
                Group {
                    name: "all".into(),
                    parents: vec![],
                    hosts: vec![],
                    children: vec!["web".into()],
                    vars_file: Some("inv/group_vars/all.yml".into()),
                },
                Group {
                    name: "web".into(),
                    parents: vec!["all".into()],
                    hosts: vec![],
                    children: vec!["edge".into()],
                    vars_file: None,
                },
                Group {
                    name: "edge".into(),
                    parents: vec!["web".into()],
                    hosts: vec![],
                    children: vec![],
                    vars_file: Some("inv/group_vars/edge.yml".into()),
                },
            ],
        };
        let got = group_vars_for(&[inv], None, ["edge:!web", "{{ x }}"]);
        assert_eq!(
            got.into_iter().collect::<Vec<_>>(),
            ["inv/group_vars/all.yml", "inv/group_vars/edge.yml"]
        );
    }
}
