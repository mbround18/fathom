//! Inventories as group trees: YAML, INI, a directory of either, or dynamic (listed only).
//!
//! Only the shape is read — groups, hosts, children — never variables. Validation of an
//! inventory's content stays with `crate::inventory`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::{Repo, get, join, parent, scalar};

/// How deep `children:` nesting is followed.
const MAX_NESTING: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryKind {
    Yaml,
    Ini,
    /// A directory of inventory files, merged.
    Dir,
    /// An inventory plugin config or a script: listed, never run.
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    pub path: String,
    pub kind: InventoryKind,
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    /// Groups that list it under `children` (`all` for a top-level group).
    pub parents: Vec<String>,
    pub hosts: Vec<String>,
    pub children: Vec<String>,
    /// `group_vars/<name>.yml` next to the inventory, or `group_vars/<name>/**`.
    #[serde(default)]
    pub vars_file: Option<String>,
}

#[derive(Default)]
struct Tree(BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)>);

impl Tree {
    fn group(&mut self, name: &str) -> &mut (BTreeSet<String>, BTreeSet<String>) {
        self.0.entry(name.to_owned()).or_default()
    }
}

/// Reads each inventory source once, in order.
pub(crate) fn discover(repo: &Repo<'_>, sources: &[String]) -> Vec<Inventory> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for source in sources {
        let Some(path) = join("", source) else {
            continue;
        };
        if !seen.insert(path.clone()) {
            continue;
        }
        match read(repo, &path) {
            Some(inv) => out.push(inv),
            None => repo.problem(&path, None, "inventory is not in the repository"),
        }
    }
    out
}

fn read(repo: &Repo<'_>, path: &str) -> Option<Inventory> {
    if repo.dirs.contains(path) {
        let mut tree = Tree::default();
        let members: Vec<&String> = repo.paths.iter().filter(|f| parent(f) == path).collect();
        for f in members {
            let name = f.rsplit('/').next().unwrap_or(f);
            if name.starts_with('.')
                || name.starts_with("README")
                || name.ends_with(".md")
                || name.ends_with('~')
            {
                continue;
            }
            fill(repo, f, file_kind(repo, f), &mut tree);
        }
        return Some(finish(repo, path, path, InventoryKind::Dir, tree));
    }
    if !repo.exists(path) {
        return None;
    }
    let kind = file_kind(repo, path);
    let mut tree = Tree::default();
    fill(repo, path, kind, &mut tree);
    Some(finish(repo, path, parent(path), kind, tree))
}

fn file_kind(repo: &Repo<'_>, path: &str) -> InventoryKind {
    let ext = path.rsplit_once('.').map_or("", |(_, e)| e);
    if matches!(ext, "py" | "sh" | "rb" | "pl" | "ps1") {
        return InventoryKind::Dynamic;
    }
    let text = repo.text(path).unwrap_or_default();
    if text.starts_with("#!") {
        return InventoryKind::Dynamic;
    }
    if matches!(ext, "yml" | "yaml" | "json") {
        let plugin = super::parse_yaml(&text)
            .ok()
            .is_some_and(|d| get(&d, "plugin").is_some());
        return if plugin {
            InventoryKind::Dynamic
        } else {
            InventoryKind::Yaml
        };
    }
    InventoryKind::Ini
}

fn fill(repo: &Repo<'_>, path: &str, kind: InventoryKind, tree: &mut Tree) {
    match kind {
        InventoryKind::Yaml => {
            if let Some(doc) = repo.yaml(path) {
                yaml_into(&doc, tree);
            }
        }
        InventoryKind::Ini => {
            if let Some(text) = repo.text(path) {
                ini_into(&text, tree);
            }
        }
        InventoryKind::Dir | InventoryKind::Dynamic => {}
    }
}

fn yaml_into(doc: &Value, tree: &mut Tree) {
    for (k, v) in doc.as_mapping().into_iter().flatten() {
        if let Some(name) = scalar(k) {
            yaml_group(&name, v, tree, 0);
        }
    }
}

fn yaml_group(name: &str, v: &Value, tree: &mut Tree, depth: usize) {
    tree.group(name);
    if depth > MAX_NESTING {
        return;
    }
    if let Some(hosts) = get(v, "hosts").and_then(Value::as_mapping) {
        let hosts: Vec<String> = hosts.keys().filter_map(scalar).collect();
        tree.group(name).0.extend(hosts);
    }
    if let Some(children) = get(v, "children").and_then(Value::as_mapping) {
        for (ck, cv) in children {
            if let Some(child) = scalar(ck) {
                tree.group(name).1.insert(child.clone());
                yaml_group(&child, cv, tree, depth + 1);
            }
        }
    }
}

fn ini_into(text: &str, tree: &mut Tree) {
    #[derive(Clone, Copy)]
    enum Section {
        Hosts,
        Children,
        Vars,
    }
    let mut group = "ungrouped".to_owned();
    let mut section = Section::Hosts;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(inner) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let (g, s) = match inner.split_once(':') {
                Some((g, "children")) => (g, Section::Children),
                Some((g, "vars")) => (g, Section::Vars),
                _ => (inner, Section::Hosts),
            };
            group = g.trim().to_owned();
            section = s;
            tree.group(&group);
            continue;
        }
        let Some(token) = line.split_whitespace().next() else {
            continue;
        };
        match section {
            Section::Hosts => {
                tree.group(&group).0.insert(token.to_owned());
            }
            Section::Children => {
                tree.group(&group).1.insert(token.to_owned());
                tree.group(token);
            }
            Section::Vars => {}
        }
    }
}

fn finish(
    repo: &Repo<'_>,
    path: &str,
    dir: &str,
    kind: InventoryKind,
    mut tree: Tree,
) -> Inventory {
    if kind != InventoryKind::Dynamic {
        tree.group("all");
    }
    let mut parents: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (g, (_, children)) in &tree.0 {
        for c in children {
            parents.entry(c.clone()).or_default().insert(g.clone());
        }
    }
    let orphans: Vec<String> = tree
        .0
        .keys()
        .filter(|g| *g != "all" && parents.get(*g).is_none_or(BTreeSet::is_empty))
        .cloned()
        .collect();
    for g in orphans {
        parents
            .entry(g.clone())
            .or_default()
            .insert("all".to_owned());
        tree.group("all").1.insert(g);
    }
    let groups = tree
        .0
        .into_iter()
        .map(|(name, (hosts, children))| Group {
            vars_file: vars_file(repo, dir, &name),
            parents: parents
                .get(&name)
                .map(|p| p.iter().cloned().collect())
                .unwrap_or_default(),
            hosts: hosts.into_iter().collect(),
            children: children.into_iter().collect(),
            name,
        })
        .collect();
    Inventory {
        path: path.to_owned(),
        kind,
        groups,
    }
}

fn vars_file(repo: &Repo<'_>, dir: &str, group: &str) -> Option<String> {
    let base = join(dir, "group_vars")?;
    for ext in [".yml", ".yaml", ".json", ""] {
        let p = format!("{base}/{group}{ext}");
        if repo.exists(&p) {
            return Some(p);
        }
    }
    let d = format!("{base}/{group}");
    repo.dirs.contains(&d).then(|| format!("{d}/**"))
}

#[cfg(test)]
mod tests {
    use super::super::{Limits, MemFiles, build};
    use super::*;

    fn inv(files: MemFiles) -> Vec<Inventory> {
        build(&files, None, Limits::default()).inventories
    }

    fn cfg(path: &str) -> String {
        format!("[defaults]\ninventory = {path}\n")
    }

    #[test]
    fn ini_groups_children_and_vars_sections() {
        let got = inv(MemFiles::new()
            .with("ansible.cfg", cfg("hosts.ini"))
            .with(
                "hosts.ini",
                "loose ansible_host=x\n[web]\nw1 ansible_port=22\nw2\n\n[edge:children]\nweb\n\
                 [edge:vars]\nsecret=nope\n# comment\n",
            )
            .with("group_vars/web.yml", "a: 1\n")
            .with("group_vars/edge/main.yml", "b: 1\n"));
        let i = &got[0];
        assert_eq!(i.kind, InventoryKind::Ini);
        let g = |n: &str| i.groups.iter().find(|g| g.name == n).unwrap();
        assert_eq!(g("ungrouped").hosts, ["loose"]);
        assert_eq!(g("web").hosts, ["w1", "w2"]);
        assert_eq!(g("web").parents, ["edge"]);
        assert_eq!(g("edge").parents, ["all"]);
        assert_eq!(g("web").vars_file.as_deref(), Some("group_vars/web.yml"));
        assert_eq!(g("edge").vars_file.as_deref(), Some("group_vars/edge/**"));
        assert!(
            !i.groups
                .iter()
                .any(|g| g.hosts.iter().any(|h| h.contains("nope")))
        );
    }

    #[test]
    fn directory_merges_members() {
        let got = inv(MemFiles::new()
            .with("ansible.cfg", cfg("inv"))
            .with("inv/a.yml", "web:\n  hosts:\n    w1:\n")
            .with("inv/b.ini", "[db]\nd1\n")
            .with("inv/README.md", "[nope]\nx\n")
            .with("inv/group_vars/db.yml", "x: 1\n"));
        let i = &got[0];
        assert_eq!(i.kind, InventoryKind::Dir);
        let names: Vec<&str> = i.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["all", "db", "web"]);
        assert_eq!(
            i.groups[1].vars_file.as_deref(),
            Some("inv/group_vars/db.yml")
        );
    }

    #[test]
    fn dynamic_inventories_are_listed_not_read() {
        for (path, body) in [
            ("aws_ec2.yml", "plugin: amazon.aws.aws_ec2\n"),
            ("inv.py", "print('{}')\n"),
            ("script", "#!/bin/sh\necho {}\n"),
        ] {
            let got = inv(MemFiles::new()
                .with("ansible.cfg", cfg(path))
                .with(path, body));
            assert_eq!(got[0].kind, InventoryKind::Dynamic, "{path}");
            assert!(got[0].groups.is_empty());
        }
    }

    #[test]
    fn missing_inventory_is_a_problem() {
        let c = build(
            &MemFiles::new().with("ansible.cfg", cfg("nowhere.yml")),
            None,
            Limits::default(),
        );
        assert!(c.inventories.is_empty());
        assert_eq!(c.problems[0].path, "nowhere.yml");
    }
}
