//! A manifest job drafted from the catalog (R6): what a playbook reads, as `paths` globs.
//!
//! The draft is a suggestion for the operator to commit, never written anywhere by Keel.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{Catalog, group_vars_for};
use crate::manifest::Manifest;

/// The timeout a drafted job starts with.
pub const DEFAULT_TIMEOUT: &str = "30m";

/// The job fields a draft fills.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftJob {
    pub playbook: String,
    pub profiles: Vec<String>,
    /// A manifest inventory name.
    #[serde(default)]
    pub inventory: Option<String>,
    /// Sorted globs: shared files, the playbook and its imports, its roles, the group vars of
    /// the groups it targets, the files it names and the sops files it reads.
    pub paths: Vec<String>,
    pub timeout: String,
}

/// A drafted job and its YAML, ready to paste under `jobs:` in `.keel/jobs.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub name: String,
    pub job: DraftJob,
    pub yaml: String,
    /// What the operator should check before committing it.
    pub notes: Vec<String>,
}

/// A job name from the playbook's file stem: lowercase letters, digits and `-`.
fn job_name(playbook: &str) -> String {
    let file = playbook.rsplit('/').next().unwrap_or(playbook);
    let stem = file
        .strip_suffix(".yaml")
        .or_else(|| file.strip_suffix(".yml"))
        .unwrap_or(file);
    let mut name = String::new();
    for c in stem.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            name.push(c);
        } else if !name.ends_with('-') {
            name.push('-');
        }
    }
    let name = name.trim_matches('-');
    if name.is_empty() {
        "job".to_owned()
    } else {
        name.to_owned()
    }
}

/// A YAML scalar: plain when that reads back as the same string, JSON-quoted otherwise.
fn scalar(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-'))
        && !s.starts_with(['-', '.'])
        && !matches!(
            s.to_ascii_lowercase().as_str(),
            "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "y" | "n"
        )
        && s.parse::<f64>().is_err();
    if plain {
        s.to_owned()
    } else {
        serde_json::Value::String(s.to_owned()).to_string()
    }
}

/// Drafts a job for `playbook`. `None` when the catalog has no such playbook.
#[must_use]
pub fn draft_job(catalog: &Catalog, manifest: Option<&Manifest>, playbook: &str) -> Option<Draft> {
    let pb = catalog.playbooks.iter().find(|p| p.path == playbook)?;
    let name = job_name(playbook);
    let mut notes = Vec::new();

    let profiles = if manifest.is_some_and(|m| m.profiles.contains_key(&name)) {
        vec![name.clone()]
    } else {
        notes.push(format!(
            "no profile named `{name}` is declared: add one under `profiles:` or name an existing profile"
        ));
        Vec::new()
    };

    let inventories = manifest.map(|m| &m.inventories);
    let (inventory, inventory_path) = match inventories.map(|i| i.iter().collect::<Vec<_>>()) {
        Some(list) if list.len() == 1 => (Some(list[0].0.clone()), Some(list[0].1.path.clone())),
        Some(list) if !list.is_empty() => {
            notes.push(format!(
                "several inventories are declared: `{}` was chosen, check it",
                list[0].0
            ));
            (Some(list[0].0.clone()), Some(list[0].1.path.clone()))
        }
        _ => {
            notes.push("no inventory is declared: add one under `inventories:`".to_owned());
            (None, None)
        }
    };

    let mut paths: BTreeSet<String> = catalog.shared_files.iter().cloned().collect();
    paths.insert(pb.path.clone());
    paths.extend(pb.imports.iter().cloned());
    for role in &pb.roles {
        if let Some(path) = catalog
            .roles
            .iter()
            .find(|r| &r.name == role && !r.external)
            .and_then(|r| r.path.as_deref())
        {
            paths.insert(format!("{path}/**"));
        }
    }
    paths.extend(group_vars_for(
        &catalog.inventories,
        inventory_path.as_deref(),
        pb.all_hosts.iter().map(String::as_str),
    ));
    paths.extend(pb.reads.iter().cloned());
    paths.extend(pb.secrets.iter().cloned());
    // The inventory itself changes with every host added; it does not trigger runs.
    for inv in &catalog.inventories {
        paths.remove(&inv.path);
    }
    notes.extend(
        pb.unresolved
            .iter()
            .map(|u| format!("not followed, so not in `paths`: {u}")),
    );

    let job = DraftJob {
        playbook: pb.path.clone(),
        profiles,
        inventory,
        paths: paths.into_iter().collect(),
        timeout: DEFAULT_TIMEOUT.to_owned(),
    };
    let yaml = render(&name, &job, &notes);
    Some(Draft {
        name,
        job,
        yaml,
        notes,
    })
}

fn render(name: &str, job: &DraftJob, notes: &[String]) -> String {
    let mut y = String::new();
    for n in notes {
        for line in n.lines() {
            y.push_str("# ");
            y.push_str(line);
            y.push('\n');
        }
    }
    y.push_str(&format!("{}:\n", scalar(name)));
    y.push_str(&format!("  playbook: {}\n", scalar(&job.playbook)));
    let profiles: Vec<String> = job.profiles.iter().map(|p| scalar(p)).collect();
    y.push_str(&format!("  profiles: [{}]\n", profiles.join(", ")));
    if let Some(inv) = &job.inventory {
        y.push_str(&format!("  inventory: {}\n", scalar(inv)));
    }
    y.push_str("  paths:\n");
    for p in &job.paths {
        y.push_str(&format!("    - {}\n", scalar(p)));
    }
    y.push_str(&format!("  timeout: {}\n", scalar(&job.timeout)));
    y
}

#[cfg(test)]
mod tests {
    use super::super::{Limits, MemFiles, build};
    use super::*;

    #[test]
    fn names_and_scalars() {
        assert_eq!(job_name("playbooks/Prox VM.yml"), "prox-vm");
        assert_eq!(job_name("site.yaml"), "site");
        assert_eq!(job_name("__.yml"), "job");
        assert_eq!(scalar("roles/base/**"), "\"roles/base/**\"");
        assert_eq!(scalar("ansible.cfg"), "ansible.cfg");
        assert_eq!(scalar("yes"), "\"yes\"");
        assert_eq!(scalar("30m"), "30m");
        assert_eq!(scalar("1"), "\"1\"");
    }

    #[test]
    fn yaml_reads_back_as_the_draft() {
        let c = build(
            &MemFiles::new()
                .with("site.yml", "- hosts: all\n  roles: [r, ext.coll.role]\n")
                .with("roles/r/tasks/main.yml", "- debug: {}\n"),
            None,
            Limits::default(),
        );
        let d = draft_job(&c, None, "site.yml").unwrap();
        assert_eq!(d.job.paths, ["roles/r/**", "site.yml"]);
        assert!(d.notes.iter().any(|n| n.contains("inventory")));
        let back: serde_yaml::Value = serde_yaml::from_str(&d.yaml).unwrap();
        let job = &back["site"];
        assert_eq!(job["playbook"].as_str(), Some("site.yml"));
        assert_eq!(job["paths"][0].as_str(), Some("roles/r/**"));
        assert_eq!(job["timeout"].as_str(), Some("30m"));
        assert!(job["profiles"].as_sequence().unwrap().is_empty());
    }
}
