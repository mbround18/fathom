//! Roles: discovery under `roles/` and `roles_path`, and what each one says about itself (R4).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::playbook::{Playbook, role_name};
use super::{Repo, get, parent, scalar};

/// A role in the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    pub name: String,
    /// Its directory; `None` for an external (collection) role.
    #[serde(default)]
    pub path: Option<String>,
    /// `meta/main.yml` `galaxy_info.description`.
    #[serde(default)]
    pub description: Option<String>,
    /// Names of the variables in `defaults/main.yml`. Never their values.
    pub defaults: Vec<String>,
    /// `meta/main.yml` dependencies, as written.
    pub dependencies: Vec<String>,
    /// Playbooks that reach it.
    pub reached_by: Vec<String>,
    /// From a collection: listed, never followed.
    pub external: bool,
}

/// What the walk needs to follow a role.
#[derive(Debug, Clone)]
pub(crate) struct RoleInfo {
    pub(crate) path: String,
    pub(crate) tasks_main: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) defaults: Vec<String>,
    pub(crate) dependencies: Vec<String>,
}

fn first_of(repo: &Repo<'_>, dir: &str, stem: &str) -> Option<String> {
    ["yml", "yaml"]
        .iter()
        .map(|ext| format!("{dir}/{stem}.{ext}"))
        .find(|p| repo.exists(p))
}

/// Every `<dir>/<name>/` with `tasks/main.y(a)ml`, for each of `dirs` in order; the first wins.
pub(crate) fn discover(repo: &Repo<'_>, dirs: &[String]) -> BTreeMap<String, RoleInfo> {
    let mut out = BTreeMap::new();
    for dir in dirs {
        for path in repo.dirs.iter().filter(|d| parent(d) == dir) {
            let name = path.rsplit('/').next().unwrap_or(path);
            if out.contains_key(name) {
                continue;
            }
            let Some(tasks_main) = first_of(repo, &format!("{path}/tasks"), "main") else {
                continue;
            };
            let meta = first_of(repo, &format!("{path}/meta"), "main").and_then(|p| repo.yaml(&p));
            let description = meta
                .as_ref()
                .and_then(|m| get(m, "galaxy_info"))
                .and_then(|g| get(g, "description"))
                .and_then(scalar);
            let dependencies = meta
                .as_ref()
                .and_then(|m| get(m, "dependencies"))
                .and_then(Value::as_sequence)
                .into_iter()
                .flatten()
                .filter_map(role_name)
                .collect();
            let defaults = first_of(repo, &format!("{path}/defaults"), "main")
                .and_then(|p| repo.yaml(&p))
                .and_then(|d| {
                    d.as_mapping().map(|m| {
                        m.keys()
                            .filter_map(scalar)
                            .collect::<BTreeSet<_>>()
                            .into_iter()
                            .collect()
                    })
                })
                .unwrap_or_default();
            out.insert(
                name.to_owned(),
                RoleInfo {
                    path: path.clone(),
                    tasks_main: Some(tasks_main),
                    description,
                    defaults,
                    dependencies,
                },
            );
        }
    }
    out
}

/// The roles list: every repository role, then each external role some playbook reaches.
pub(crate) fn assemble(index: &BTreeMap<String, RoleInfo>, playbooks: &[Playbook]) -> Vec<Role> {
    let reached = |name: &str| -> Vec<String> {
        playbooks
            .iter()
            .filter(|p| p.roles.iter().any(|r| r == name))
            .map(|p| p.path.clone())
            .collect()
    };
    let mut roles: Vec<Role> = index
        .iter()
        .map(|(name, info)| Role {
            name: name.clone(),
            path: Some(info.path.clone()),
            description: info.description.clone(),
            defaults: info.defaults.clone(),
            dependencies: info.dependencies.clone(),
            reached_by: reached(name),
            external: false,
        })
        .collect();
    let external: BTreeSet<&String> = playbooks
        .iter()
        .flat_map(|p| &p.roles)
        .filter(|r| !index.contains_key(*r))
        .collect();
    roles.extend(external.into_iter().map(|name| Role {
        name: name.clone(),
        path: None,
        description: None,
        defaults: Vec::new(),
        dependencies: Vec::new(),
        reached_by: reached(name),
        external: true,
    }));
    roles.sort_by(|a, b| a.name.cmp(&b.name));
    roles
}

#[cfg(test)]
mod tests {
    use super::super::{Limits, MemFiles, build};

    #[test]
    fn roles_path_adds_directories_and_defaults_keep_names_only() {
        let files = MemFiles::new()
            .with(
                "ansible.cfg",
                "[defaults]\nroles_path = roles:vendor/roles\n",
            )
            .with("roles/a/tasks/main.yml", "- debug: {}\n")
            .with(
                "roles/a/defaults/main.yml",
                "a_password: s3cr3t-default\na_port: 1\n",
            )
            .with("roles/notarole/README.md", "x")
            .with("vendor/roles/v/tasks/main.yaml", "- debug: {}\n")
            .with(
                "vendor/roles/v/meta/main.yml",
                "galaxy_info: {description: Vendored}\ndependencies:\n  - {role: a}\n  - name: b\n",
            )
            .with("vendor/roles/a/tasks/main.yml", "- debug: {}\n");
        let c = build(&files, None, Limits::default());
        let names: Vec<&str> = c.roles.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["a", "v"]);
        assert_eq!(c.roles[0].path.as_deref(), Some("roles/a"), "roles/ wins");
        assert_eq!(c.roles[0].defaults, ["a_password", "a_port"]);
        assert_eq!(c.roles[1].description.as_deref(), Some("Vendored"));
        assert_eq!(c.roles[1].dependencies, ["a", "b"]);
        assert!(c.roles.iter().all(|r| r.reached_by.is_empty()));
        assert!(!serde_json::to_string(&c).unwrap().contains("s3cr3t"));
    }
}
