//! Playbooks: detection, plays, and what they reach (R4).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::role::RoleInfo;
use super::{Repo, get, join, parent, playbook_dir_relative, scalar, strings, templated};

/// One play: its name and `hosts:` pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Play {
    #[serde(default)]
    pub name: Option<String>,
    pub hosts: String,
}

/// A playbook and everything it reaches, imports included.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playbook {
    pub path: String,
    /// Its own plays (imported playbooks list theirs).
    pub plays: Vec<Play>,
    /// The `hosts:` patterns of its plays and of every play it imports.
    pub all_hosts: Vec<String>,
    /// Roles reached, by name. Collection roles are listed and marked external in the roles list.
    pub roles: Vec<String>,
    /// Playbooks it imports, directly or through another import.
    pub imports: Vec<String>,
    /// `vars_files` and `community.sops.load_vars` files named literally.
    pub vars_files: Vec<String>,
    /// Files and directories named through `{{ playbook_dir }}`; directories end in `/**`.
    pub reads: Vec<String>,
    /// The sops files it reads (R5).
    pub secrets: Vec<String>,
    /// What could not be followed, and why.
    pub unresolved: Vec<String>,
    /// Manifest jobs that run it.
    pub jobs: Vec<String>,
}

/// One playbook's own reach, before imports are folded in.
#[derive(Debug, Default)]
pub(crate) struct Scan {
    plays: Vec<Play>,
    imports: BTreeSet<String>,
    roles: BTreeSet<String>,
    vars_files: BTreeSet<String>,
    reads: BTreeSet<String>,
    unresolved: BTreeSet<String>,
}

fn is_import(key: &str) -> bool {
    matches!(key, "import_playbook" | "ansible.builtin.import_playbook")
}

/// A sequence of mappings, each a play (`hosts`) or an import.
pub(crate) fn is_playbook(doc: &Value) -> bool {
    doc.as_sequence().is_some_and(|plays| {
        !plays.is_empty()
            && plays.iter().all(|p| {
                p.as_mapping().is_some_and(|m| {
                    m.keys()
                        .filter_map(Value::as_str)
                        .any(|k| k == "hosts" || is_import(k))
                })
            })
    })
}

/// Reads one playbook's plays and follows roles and task includes.
pub(crate) fn scan(
    repo: &Repo<'_>,
    roles: &BTreeMap<String, RoleInfo>,
    path: &str,
    doc: &Value,
) -> Scan {
    let mut w = Walker {
        repo,
        roles,
        dir: parent(path).to_owned(),
        max_depth: repo.limits.max_depth,
        scan: Scan::default(),
        seen_roles: BTreeSet::new(),
        seen_tasks: BTreeSet::new(),
    };
    for play in doc.as_sequence().into_iter().flatten() {
        w.play(play);
    }
    let vars = w.scan.vars_files.clone();
    w.scan.reads.retain(|r| !vars.contains(r));
    w.scan
}

struct Walker<'r, 'a> {
    repo: &'r Repo<'a>,
    roles: &'r BTreeMap<String, RoleInfo>,
    /// The playbook's directory: `playbook_dir`.
    dir: String,
    max_depth: usize,
    scan: Scan,
    seen_roles: BTreeSet<String>,
    seen_tasks: BTreeSet<String>,
}

impl Walker<'_, '_> {
    fn note(&mut self, s: String) {
        self.scan.unresolved.insert(s);
    }

    /// A path written in the playbook or a task file: `{{ playbook_dir }}/x`, or relative to `base`.
    fn resolve(&mut self, base: &str, v: &str, what: &str) -> Option<String> {
        let resolved = if let Some(rel) = playbook_dir_relative(v) {
            join(&self.dir, rel)
        } else if templated(v) {
            self.note(format!("{what} `{v}` is templated"));
            return None;
        } else {
            join(base, v)
        };
        if resolved.is_none() {
            self.note(format!("{what} `{v}` is outside the repository"));
        }
        resolved
    }

    fn play(&mut self, play: &Value) {
        let Some(map) = play.as_mapping() else { return };
        let dir = self.dir.clone();
        for (k, v) in map {
            if k.as_str().is_some_and(is_import)
                && let Some(target) = scalar(v)
                && let Some(p) = self.resolve(&dir, &target, "import_playbook")
            {
                self.scan.imports.insert(p);
            }
        }
        if let Some(hosts) = get(play, "hosts") {
            let hosts = strings(hosts).join(",");
            if templated(&hosts) {
                self.note(format!("hosts `{hosts}` are templated"));
            }
            self.scan.plays.push(Play {
                name: get(play, "name").and_then(scalar),
                hosts,
            });
        }
        for r in get(play, "roles")
            .and_then(Value::as_sequence)
            .into_iter()
            .flatten()
        {
            if let Some(name) = role_name(r) {
                self.role(&name, 1);
            }
        }
        for v in get(play, "vars_files").map(strings).unwrap_or_default() {
            self.vars_file(&dir, &v);
        }
        for key in [
            "pre_tasks",
            "roles_tasks",
            "tasks",
            "post_tasks",
            "handlers",
        ] {
            if let Some(seq) = get(play, key).and_then(Value::as_sequence) {
                self.tasks(seq, &dir, 1);
            }
        }
        let mut found = Vec::new();
        literal_dir_refs(play, &mut found);
        for rel in found {
            if let Some(p) = join(&self.dir, &rel) {
                if self.repo.dirs.contains(&p) {
                    self.scan.reads.insert(format!("{p}/**"));
                } else if self.repo.exists(&p) {
                    self.scan.reads.insert(p);
                }
            }
        }
    }

    fn vars_file(&mut self, base: &str, v: &str) {
        if let Some(p) = self.resolve(base, v, "vars file") {
            if self.repo.exists(&p) {
                self.scan.vars_files.insert(p);
            } else {
                self.note(format!("vars file `{v}` is not in the repository"));
            }
        }
    }

    fn role(&mut self, name: &str, depth: usize) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if templated(name) {
            self.note(format!("role `{name}` is templated"));
            return;
        }
        let Some(info) = self.roles.get(name) else {
            if name.contains('.') {
                // A collection role: listed, never followed.
                self.scan.roles.insert(name.to_owned());
            } else {
                self.note(format!("role `{name}` is not in the repository"));
            }
            return;
        };
        if depth > self.max_depth {
            self.note(format!(
                "stopped following at depth {}: role `{name}`",
                self.max_depth
            ));
            return;
        }
        if !self.seen_roles.insert(name.to_owned()) {
            return;
        }
        self.scan.roles.insert(name.to_owned());
        for dep in info.dependencies.clone() {
            self.role(&dep, depth + 1);
        }
        if let Some(main) = info.tasks_main.clone() {
            self.task_file(&main, depth); // A role's own tasks are at its depth.
        }
    }

    fn task_file(&mut self, path: &str, depth: usize) {
        if depth > self.max_depth {
            self.note(format!(
                "stopped following at depth {}: tasks `{path}`",
                self.max_depth
            ));
            return;
        }
        if !self.seen_tasks.insert(path.to_owned()) {
            return;
        }
        let Some(doc) = self.repo.yaml(path) else {
            return;
        };
        if let Some(seq) = doc.as_sequence() {
            self.tasks(seq, parent(path), depth);
        }
    }

    fn tasks(&mut self, seq: &[Value], base: &str, depth: usize) {
        for task in seq {
            let Some(map) = task.as_mapping() else {
                continue;
            };
            for (k, v) in map {
                let Some(key) = k.as_str() else { continue };
                if matches!(key, "block" | "rescue" | "always") {
                    if let Some(inner) = v.as_sequence() {
                        self.tasks(inner, base, depth);
                    }
                    continue;
                }
                if key == "community.sops.load_vars" {
                    if let Some(f) = get(v, "file").and_then(scalar) {
                        self.vars_file(base, &f);
                    }
                    continue;
                }
                let module = key
                    .strip_prefix("ansible.builtin.")
                    .or_else(|| key.strip_prefix("ansible.legacy."))
                    .unwrap_or(key);
                match module {
                    "include_role" | "import_role" => {
                        if let Some(n) = get(v, "name").and_then(scalar) {
                            self.role(&n, depth + 1);
                        }
                    }
                    "include_tasks" | "import_tasks" | "include" => {
                        let file = scalar(v).or_else(|| get(v, "file").and_then(scalar));
                        if let Some(f) = file
                            && let Some(p) = self.resolve(base, &f, "tasks")
                        {
                            if self.repo.exists(&p) {
                                self.task_file(&p, depth + 1);
                            } else {
                                self.note(format!("tasks `{f}` are not in the repository"));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// A `roles:` entry: a name, or a mapping with `role` or `name`.
pub(crate) fn role_name(v: &Value) -> Option<String> {
    scalar(v).or_else(|| get(v, "role").or_else(|| get(v, "name")).and_then(scalar))
}

/// Every string under `v` of the form `{{ playbook_dir }}/<literal>`, as `<literal>`.
fn literal_dir_refs(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => {
            if let Some(rel) = playbook_dir_relative(s) {
                out.push(rel.to_owned());
            }
        }
        Value::Sequence(items) => items.iter().for_each(|i| literal_dir_refs(i, out)),
        Value::Mapping(m) => m.values().for_each(|i| literal_dir_refs(i, out)),
        Value::Tagged(t) => literal_dir_refs(&t.value, out),
        _ => {}
    }
}

/// Folds each playbook's imports (transitively, to `max_depth`) into what it reaches.
pub(crate) fn assemble(scans: &BTreeMap<String, Scan>, max_depth: usize) -> Vec<Playbook> {
    scans
        .iter()
        .map(|(path, own)| {
            let mut roles = own.roles.clone();
            let mut vars = own.vars_files.clone();
            let mut reads = own.reads.clone();
            let mut unresolved = own.unresolved.clone();
            let mut hosts: BTreeSet<String> = own.plays.iter().map(|p| p.hosts.clone()).collect();
            let mut imports = BTreeSet::new();
            let mut queue: Vec<(&String, usize)> = own.imports.iter().map(|i| (i, 1)).collect();
            while let Some((imp, depth)) = queue.pop() {
                if imp == path || imports.contains(imp) {
                    continue;
                }
                let Some(s) = scans.get(imp) else {
                    unresolved.insert(format!(
                        "import `{imp}` is not a playbook in the repository"
                    ));
                    continue;
                };
                if depth > max_depth {
                    unresolved.insert(format!(
                        "stopped following at depth {max_depth}: import `{imp}`"
                    ));
                    continue;
                }
                imports.insert(imp.clone());
                roles.extend(s.roles.iter().cloned());
                vars.extend(s.vars_files.iter().cloned());
                reads.extend(s.reads.iter().cloned());
                unresolved.extend(s.unresolved.iter().cloned());
                hosts.extend(s.plays.iter().map(|p| p.hosts.clone()));
                queue.extend(s.imports.iter().map(|i| (i, depth + 1)));
            }
            Playbook {
                path: path.clone(),
                plays: own.plays.clone(),
                all_hosts: hosts.into_iter().collect(),
                roles: roles.into_iter().collect(),
                imports: imports.into_iter().collect(),
                vars_files: vars.into_iter().collect(),
                reads: reads.into_iter().collect(),
                secrets: Vec::new(),
                unresolved: unresolved.into_iter().collect(),
                jobs: Vec::new(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::{Limits, MemFiles, build, parse_yaml};
    use super::*;

    fn yaml(s: &str) -> Value {
        parse_yaml(s).unwrap()
    }

    #[test]
    fn detection() {
        assert!(is_playbook(&yaml("- hosts: all\n  tasks: []\n")));
        assert!(is_playbook(&yaml(
            "- ansible.builtin.import_playbook: a.yml\n- hosts: x\n"
        )));
        assert!(!is_playbook(&yaml("[]")));
        assert!(!is_playbook(&yaml("- name: a task\n  debug: {}\n")));
        assert!(!is_playbook(&yaml("services: {}\n")));
        assert!(!is_playbook(&yaml("collections:\n  - name: a.b\n")));
    }

    #[test]
    fn excluded_directories_are_never_playbooks() {
        let play = "- hosts: all\n";
        let files = MemFiles::new()
            .with("site.yml", play)
            .with("molecule/default/converge.yml", play)
            .with("inventory/x.yml", play)
            .with(".github/workflows/x.yml", play)
            .with("roles/r/tasks/main.yml", play)
            .with("nested/deploy.yaml", play)
            .with("notes.txt", play);
        let c = build(&files, None, Limits::default());
        let got: Vec<&str> = c.playbooks.iter().map(|p| p.path.as_str()).collect();
        assert_eq!(got, ["nested/deploy.yaml", "site.yml"]);
    }

    fn chain(n: usize) -> MemFiles {
        let mut f = MemFiles::new().with("site.yml", "- hosts: all\n  roles: [r0]\n");
        for i in 0..n {
            f = f
                .with(&format!("roles/r{i}/tasks/main.yml"), "- debug: {}\n")
                .with(
                    &format!("roles/r{i}/meta/main.yml"),
                    format!("dependencies: [r{}]\n", i + 1),
                );
        }
        f.with(&format!("roles/r{n}/tasks/main.yml"), "- debug: {}\n")
    }

    #[test]
    fn depth_limit_stops_and_says_so() {
        let c = build(&chain(12), None, Limits::default());
        let pb = &c.playbooks[0];
        assert_eq!(pb.roles.len(), 8, "{:?}", pb.roles);
        assert_eq!(pb.unresolved, ["stopped following at depth 8: role `r8`"]);

        let c = build(&chain(3), None, Limits::default());
        assert_eq!(c.playbooks[0].roles, ["r0", "r1", "r2", "r3"]);
        assert!(c.playbooks[0].unresolved.is_empty());
    }

    #[test]
    fn literal_templated_and_collection_includes() {
        let files = MemFiles::new()
            .with(
                "site.yml",
                "- hosts: web:&prod\n  tasks:\n    - block:\n        - include_role: {name: a}\n      \
                 rescue:\n        - ansible.builtin.import_tasks: extra.yml\n    \
                 - include_role:\n        name: \"{{ r }}\"\n    \
                 - include_role: {name: ns.coll.thing}\n    \
                 - include_tasks: \"{{ playbook_dir }}/{{ x }}.yml\"\n    \
                 - include_tasks: ../../etc/passwd\n",
            )
            .with("extra.yml", "- import_role:\n    name: b\n")
            .with("roles/a/tasks/main.yml", "- include_tasks: more.yml\n")
            .with("roles/a/tasks/more.yml", "- debug: {}\n")
            .with("roles/b/tasks/main.yaml", "- debug: {}\n");
        let c = build(&files, None, Limits::default());
        let pb = &c.playbooks[0];
        assert_eq!(pb.roles, ["a", "b", "ns.coll.thing"]);
        assert_eq!(pb.plays[0].hosts, "web:&prod");
        assert_eq!(pb.unresolved.len(), 3, "{:?}", pb.unresolved);
        assert!(
            pb.unresolved
                .iter()
                .any(|u| u.contains("outside the repository"))
        );
        let ext = c.roles.iter().find(|r| r.name == "ns.coll.thing").unwrap();
        assert!(ext.external);
    }

    #[test]
    fn import_cycles_terminate() {
        let files = MemFiles::new()
            .with(
                "a.yml",
                "- import_playbook: b.yml\n- hosts: x\n  roles: [ra]\n",
            )
            .with("b.yml", "- import_playbook: a.yml\n- hosts: y\n")
            .with("roles/ra/tasks/main.yml", "- debug: {}\n");
        let c = build(&files, None, Limits::default());
        let b = c.playbooks.iter().find(|p| p.path == "b.yml").unwrap();
        assert_eq!(b.imports, ["a.yml"]);
        assert_eq!(b.roles, ["ra"]);
        assert_eq!(b.all_hosts, ["x", "y"]);
    }

    #[test]
    fn missing_role_is_unresolved() {
        let files = MemFiles::new().with("s.yml", "- hosts: all\n  roles: [{role: ghost}]\n");
        let c = build(&files, None, Limits::default());
        assert_eq!(
            c.playbooks[0].unresolved,
            ["role `ghost` is not in the repository"]
        );
        assert!(c.roles.is_empty());
    }
}
