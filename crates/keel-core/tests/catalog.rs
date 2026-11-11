//! The repository catalog against a synthetic copy of the operator's layout (spec 066, R4–R6).
//!
//! Every test is named `catalog_*` so `cargo test -p keel-core catalog` runs them.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use keel_core::catalog::{
    self, Catalog, Entry, EntryKind, InventoryKind, Limits, RepoFiles, draft_job,
};
use keel_core::manifest::{self, Context, Manifest, PathKind, RepoTree};

/// The fixture on disk. Reads never follow symbolic links and never leave the root.
struct Disk(PathBuf);

impl Disk {
    fn fixture() -> Self {
        Self(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ansible-repo"))
    }
    fn at(&self, path: &str) -> Option<PathBuf> {
        if path.split('/').any(|c| c == "..") || path.starts_with('/') {
            return None;
        }
        Some(if path.is_empty() {
            self.0.clone()
        } else {
            self.0.join(path)
        })
    }
}

impl RepoFiles for Disk {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        let p = self.at(path)?;
        let meta = fs::symlink_metadata(&p).ok()?;
        meta.is_file().then(|| fs::read(&p).ok()).flatten()
    }
    fn list(&self, dir: &str) -> Option<Vec<Entry>> {
        let p = self.at(dir)?;
        if !fs::symlink_metadata(&p).ok()?.is_dir() {
            return None;
        }
        let mut out = Vec::new();
        for e in fs::read_dir(p).ok()? {
            let e = e.ok()?;
            let ft = e.file_type().ok()?;
            let kind = if ft.is_symlink() {
                EntryKind::Symlink
            } else if ft.is_dir() {
                EntryKind::Dir
            } else if ft.is_file() {
                EntryKind::File
            } else {
                EntryKind::Other
            };
            out.push(Entry {
                name: e.file_name().to_string_lossy().into_owned(),
                kind,
            });
        }
        Some(out)
    }
}

impl RepoTree for Disk {
    fn kind(&self, path: &str) -> Option<PathKind> {
        let meta = fs::symlink_metadata(self.at(path)?).ok()?;
        Some(if meta.is_dir() {
            PathKind::Dir
        } else if meta.is_file() {
            PathKind::File
        } else {
            PathKind::Other
        })
    }
}

fn manifest(disk: &Disk) -> Manifest {
    let bytes = disk
        .read(manifest::MANIFEST_PATH)
        .expect("fixture manifest");
    manifest::validate(&bytes, disk, &Context::default()).expect("fixture manifest is valid")
}

fn fixture() -> (Catalog, Manifest) {
    let disk = Disk::fixture();
    let m = manifest(&disk);
    (catalog::build(&disk, Some(&m), Limits::default()), m)
}

fn names<T>(items: &[T], f: impl Fn(&T) -> &str) -> Vec<String> {
    items.iter().map(|i| f(i).to_owned()).collect()
}

#[test]
fn catalog_detects_playbooks_only() {
    let (c, _) = fixture();
    assert_eq!(
        names(&c.playbooks, |p| &p.path),
        [
            "playbooks/cleanup.yaml",
            "playbooks/debian.yaml",
            "playbooks/gameserver.yaml",
            "playbooks/k3s-master.yaml",
            "playbooks/keel-server.yaml",
            "playbooks/prox-vm.yaml",
            "playbooks/upgrade.yaml",
        ]
    );
}

#[test]
fn catalog_follows_roles_imports_and_includes() {
    let (c, _) = fixture();
    let pb = |p: &str| c.playbooks.iter().find(|x| x.path == p).unwrap();

    let prox = pb("playbooks/prox-vm.yaml");
    assert_eq!(prox.roles, ["base", "docker", "tailscale"]);
    assert_eq!(prox.plays[0].hosts, "compose_hosts");
    assert_eq!(prox.jobs, ["prox-vm"]);

    let game = pb("playbooks/gameserver.yaml");
    assert_eq!(game.imports, ["playbooks/prox-vm.yaml"]);
    assert_eq!(game.roles, ["base", "docker", "tailscale"]);
    assert_eq!(game.reads, ["playbooks/files/gamestacks/**"]);

    // include_tasks inside a role, then import_role from that task file.
    let k3s = pb("playbooks/k3s-master.yaml");
    assert_eq!(k3s.roles, ["base", "k3s_node", "sops"]);
    assert_eq!(k3s.vars_files, ["secrets/k3s_servers.sops.yaml"]);

    // meta dependency: keel_server -> docker -> base.
    let keel = pb("playbooks/keel-server.yaml");
    assert_eq!(keel.roles, ["base", "docker", "keel_server", "sops"]);
    assert_eq!(keel.vars_files, ["secrets/keel_servers.sops.yaml"]);

    // Templated names are reported, collection roles are external and not followed.
    let cleanup = pb("playbooks/cleanup.yaml");
    assert_eq!(cleanup.roles, ["cleanup", "community.general.nothing"]);
    assert_eq!(cleanup.unresolved.len(), 1, "{:?}", cleanup.unresolved);
    assert!(cleanup.unresolved[0].contains("{{ cleanup_extra_role }}"));
    assert!(cleanup.jobs.is_empty());
}

#[test]
fn catalog_describes_roles() {
    let (c, _) = fixture();
    let role = |n: &str| c.roles.iter().find(|r| r.name == n).unwrap();
    let base = role("base");
    assert_eq!(base.path.as_deref(), Some("roles/base"));
    assert_eq!(
        base.description.as_deref(),
        Some("The baseline every box gets")
    );
    assert_eq!(
        base.defaults,
        ["base_packages", "base_secret_like", "base_user"]
    );
    assert!(
        base.reached_by
            .contains(&"playbooks/k3s-master.yaml".to_owned())
    );
    assert_eq!(role("keel_server").dependencies, ["docker"]);
    let ext = role("community.general.nothing");
    assert!(ext.external);
    assert_eq!(ext.path, None);
    assert_eq!(ext.reached_by, ["playbooks/cleanup.yaml"]);
}

#[test]
fn catalog_reads_the_inventory_tree() {
    let (c, _) = fixture();
    assert_eq!(c.inventories.len(), 1);
    let inv = &c.inventories[0];
    assert_eq!(inv.path, "inventory/hosts.yml");
    assert_eq!(inv.kind, InventoryKind::Yaml);
    let g = |n: &str| inv.groups.iter().find(|g| g.name == n).unwrap();
    assert_eq!(g("compose_hosts").hosts, ["alpha", "bravo"]);
    assert_eq!(g("compose_hosts").children, ["gameservers"]);
    assert_eq!(g("compose_hosts").parents, ["all"]);
    assert_eq!(g("gameservers").parents, ["compose_hosts"]);
    assert_eq!(
        g("gameservers").vars_file.as_deref(),
        Some("inventory/group_vars/gameservers.yml")
    );
    assert_eq!(
        g("all").vars_file.as_deref(),
        Some("inventory/group_vars/all.yml")
    );
}

#[test]
fn catalog_sops_rules_metadata_and_drift() {
    let (c, _) = fixture();
    let s = &c.secrets;
    assert_eq!(s.rules_file.as_deref(), Some(".sops.yaml"));
    assert_eq!(s.rules.len(), 2);
    assert_eq!(s.rules[0].age.len(), 2);

    let f = |p: &str| s.files.iter().find(|f| f.path == p).unwrap();
    let keel = f("secrets/keel_servers.sops.yaml");
    assert_eq!(keel.rule, Some(0));
    assert_eq!(keel.drift, None, "recipients match the rule");
    assert_eq!(keel.keys, ["keel_db_password", "keel_oidc.client_secret"]);
    assert_eq!(keel.lastmodified.as_deref(), Some("2026-11-01T10:00:00Z"));
    assert_eq!(keel.read_by, ["keel-server"]);

    let k3s = f("secrets/k3s_servers.sops.yaml");
    assert_eq!(k3s.rule, Some(1));
    let drift = k3s.drift.as_ref().expect("one recipient is missing");
    assert_eq!(drift.missing.len(), 1);
    assert!(drift.extra.is_empty());
    assert_eq!(k3s.read_by, ["k3s-master"]);

    let broken = f("secrets/broken.sops.yaml");
    assert!(broken.problem.is_some());
    assert!(broken.keys.is_empty());
}

/// SC-004: nothing a sops file encrypts, and none of its MAC or data keys, reaches the catalog.
#[test]
fn catalog_never_contains_sops_values() {
    let (c, _) = fixture();
    let json = serde_json::to_string(&c).unwrap();
    for needle in [
        "ENC[",
        "AES256_GCM",
        "BEGIN AGE",
        "RkFLRVNUQU5aQUZPUktFRUxURVNUUw",
        "Zm9vYmFyS0VFTERC",
        "TUFDTUFDTUFDS0VFTA",
        "not-a-secret",
        "hunter2",
        "\"mac\"",
        "\"enc\"",
    ] {
        assert!(!json.contains(needle), "catalog contains {needle}");
    }
    // Every fixture file under secrets/: no value line survives.
    let disk = Disk::fixture();
    for name in ["keel_servers.sops.yaml", "k3s_servers.sops.yaml"] {
        let text = String::from_utf8(disk.read(&format!("secrets/{name}")).unwrap()).unwrap();
        for v in text
            .split(['[', ',', ']'])
            .filter(|t| t.starts_with("data:"))
        {
            assert!(!json.contains(&v[5..]), "{name}: ciphertext leaked");
        }
    }
}

/// R6 / SC-003: drafting a declared playbook reproduces its declared `paths`.
#[test]
fn catalog_drafts_equal_declared_paths() {
    let (c, m) = fixture();
    let mut checked = 0;
    for (name, job) in &m.jobs {
        let Some(declared) = &job.definition.paths else {
            continue;
        };
        let d = draft_job(&c, Some(&m), &job.definition.playbook).expect("a draft");
        let want: BTreeSet<&str> = declared.iter().map(String::as_str).collect();
        let got: BTreeSet<&str> = d.job.paths.iter().map(String::as_str).collect();
        assert_eq!(got, want, "job {name}");
        assert_eq!(d.job.profiles, job.definition.profiles, "job {name}");
        assert_eq!(d.job.inventory.as_deref(), Some("fleet"));
        checked += 1;
    }
    assert_eq!(checked, 5);
}

#[test]
fn catalog_draft_for_an_undeclared_playbook() {
    let (c, m) = fixture();
    let d = draft_job(&c, Some(&m), "playbooks/cleanup.yaml").unwrap();
    assert_eq!(d.name, "cleanup");
    assert!(d.job.profiles.is_empty());
    assert!(d.notes.iter().any(|n| n.contains("profile")));
    assert!(d.yaml.starts_with("cleanup:\n") || d.yaml.contains("\ncleanup:\n"));
    assert!(d.yaml.contains("  timeout: 30m\n"));
    assert!(!d.job.paths.iter().any(|p| p == "inventory/hosts.yml"));
    assert!(draft_job(&c, Some(&m), "playbooks/nope.yaml").is_none());
}

#[test]
fn catalog_round_trips_through_json() {
    let (c, _) = fixture();
    let back: Catalog = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
    assert_eq!(back, c);
    assert_eq!(c.version, 1);
    assert_eq!(c.skipped.files, 0);
    assert_eq!(
        c.shared_files,
        [
            "ansible.cfg",
            "inventory/group_vars/all.yml",
            "requirements.yml"
        ]
    );
}

#[test]
fn catalog_without_a_manifest_still_builds() {
    let disk = Disk::fixture();
    let c = catalog::build(&disk, None, Limits::default());
    assert_eq!(c.playbooks.len(), 7);
    // ansible.cfg names the inventory, so it is still found.
    assert_eq!(c.inventories.len(), 1);
    assert!(c.playbooks.iter().all(|p| p.jobs.is_empty()));
    let d = draft_job(&c, None, "playbooks/debian.yaml").unwrap();
    assert_eq!(d.job.inventory, None);
}
