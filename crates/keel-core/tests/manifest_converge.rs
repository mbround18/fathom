//! Spec 068 manifest additions (contracts/manifest.md): recipes, `groups`, `covers` and `env`,
//! and FR-016 / SC-006: a manifest without them validates exactly as before.

use std::collections::{BTreeMap, BTreeSet};

use keel_core::converge::{self, MAX_PLAN_STEPS};
use keel_core::manifest::{Context, Manifest, ManifestIssue, PathKind, RepoTree, validate};

/// Every path with an extension is a file and every other path a directory: these tests are
/// about the new keys, not paths.
struct AnyFile;

impl RepoTree for AnyFile {
    fn kind(&self, path: &str) -> Option<PathKind> {
        let last = path.rsplit('/').next().unwrap_or(path);
        Some(if last.contains('.') {
            PathKind::File
        } else {
            PathKind::Dir
        })
    }
}

fn ctx() -> Context {
    Context {
        signed_mode: false,
        granted_credentials: BTreeSet::new(),
    }
}

fn ok(yaml: &str) -> Manifest {
    validate(yaml.as_bytes(), &AnyFile, &ctx()).unwrap_or_else(|e| panic!("valid: {e:?}"))
}

fn codes(yaml: &str) -> Vec<(String, String)> {
    validate(yaml.as_bytes(), &AnyFile, &ctx())
        .expect_err("manifest should be refused")
        .into_iter()
        .map(|i: ManifestIssue| (i.at, i.code.unwrap_or_default()))
        .collect()
}

fn has(issues: &[(String, String)], at: &str, code: &str) -> bool {
    issues.iter().any(|(a, c)| a.starts_with(at) && c == code)
}

const BASE: &str = r#"
version: 1
profiles:
  debian: {}
  docker:
    includes: [debian]
    jobs: [docker-install]
  web:
    includes: [docker]
    jobs: [web-deploy]
inventories:
  fleet: { path: inventory/hosts.yml }
groups:
  webservers: [web]
jobs:
  baseline: { playbook: playbooks/b.yaml, profiles: [debian], inventory: fleet }
  docker-install: { playbook: playbooks/d.yaml, profiles: [docker], inventory: fleet }
  web-deploy:
    playbook: playbooks/w.yaml
    profiles: [web]
    inventory: fleet
    covers: [docker-install]
    env: { CLOUDFLARE_API_TOKEN: cf-token }
"#;

#[test]
fn recipes_groups_covers_and_env_validate() {
    let m = ok(BASE);
    assert!(m.profiles["web"].has_recipe());
    assert!(!m.profiles["debian"].has_recipe());
    assert_eq!(m.groups["webservers"], vec!["web".to_owned()]);
    let def = &m.jobs["web-deploy"].definition;
    assert_eq!(def.covers, vec!["docker-install".to_owned()]);
    assert_eq!(def.env["CLOUDFLARE_API_TOKEN"], "cf-token");
    // debian has no recipe, so it falls back to the jobs naming it.
    assert_eq!(
        converge::expand(&["web".to_owned()], &m).unwrap(),
        vec!["baseline", "docker-install", "web-deploy"]
    );
    let cov = converge::coverage(&m).unwrap();
    let plan = converge::host_plan(&["web".to_owned()], &m, &cov).unwrap();
    let jobs: Vec<_> = plan.iter().map(|s| s.job.as_str()).collect();
    assert_eq!(jobs, vec!["baseline", "web-deploy"]);
}

#[test]
fn definitions_without_new_keys_serialise_as_before() {
    let m = ok(BASE);
    let json = serde_json::to_value(&m.jobs["baseline"].definition).unwrap();
    assert!(json.get("covers").is_none(), "{json}");
    assert!(json.get("env").is_none(), "{json}");
    let json = serde_json::to_value(&m.profiles["debian"]).unwrap();
    assert_eq!(json, serde_json::json!({ "description": null }));
}

#[test]
fn recipe_names_must_exist_and_jobs_appear_once() {
    let y = BASE.replace("includes: [debian]", "includes: [debain]");
    assert!(has(
        &codes(&y),
        "profiles.docker.includes",
        "unknown_include"
    ));
    let y = BASE.replace("jobs: [docker-install]", "jobs: [docker-instal]");
    assert!(has(
        &codes(&y),
        "profiles.docker.jobs",
        "unknown_recipe_job"
    ));
    let y = BASE.replace(
        "jobs: [docker-install]",
        "jobs: [docker-install, docker-install]",
    );
    assert!(has(
        &codes(&y),
        "profiles.docker.jobs",
        "duplicate_recipe_job"
    ));
}

#[test]
fn include_cycles_are_refused_once_with_their_path() {
    let y = BASE.replace("  debian: {}", "  debian: { includes: [web] }");
    let issues = validate(y.as_bytes(), &AnyFile, &ctx()).unwrap_err();
    let cycles: Vec<_> = issues
        .iter()
        .filter(|i| i.code.as_deref() == Some("profile_cycle"))
        .collect();
    assert_eq!(cycles.len(), 1, "{issues:?}");
    for p in ["debian", "docker", "web"] {
        assert!(cycles[0].message.contains(p), "{}", cycles[0].message);
    }
}

#[test]
fn recipes_longer_than_the_limit_are_refused() {
    use std::fmt::Write as _;
    let mut y = String::from(
        "version: 1\ninventories:\n  fleet: { path: h.yml }\nprofiles:\n  big:\n    jobs: [",
    );
    let n = MAX_PLAN_STEPS + 1;
    let names: Vec<String> = (0..n).map(|i| format!("j{i}")).collect();
    y.push_str(&names.join(", "));
    y.push_str("]\njobs:\n");
    for j in &names {
        writeln!(
            y,
            "  {j}: {{ playbook: p.yaml, profiles: [big], inventory: fleet }}"
        )
        .unwrap();
    }
    assert!(has(&codes(&y), "profiles.big", "recipe_too_long"));
}

#[test]
fn groups_must_name_profiles() {
    let y = BASE.replace("webservers: [web]", "webservers: [wbe]");
    assert!(has(
        &codes(&y),
        "groups.webservers",
        "unknown_group_profile"
    ));
}

#[test]
fn covers_refusals() {
    let y = BASE.replace("covers: [docker-install]", "covers: [web-deploy]");
    assert!(has(&codes(&y), "jobs.web-deploy.covers", "self_cover"));
    let y = BASE.replace("covers: [docker-install]", "covers: [nope]");
    assert!(has(&codes(&y), "jobs.web-deploy.covers", "unknown_cover"));
    let y = BASE.replace(
        "inventory: fleet }\n  web-deploy:",
        "inventory: fleet, covers: [web-deploy] }\n  web-deploy:",
    );
    assert!(has(&codes(&y), "jobs.", "cover_cycle"));
}

#[test]
fn env_refusals() {
    for (name, code) in [
        ("lower_case", "env_name_invalid"),
        ("1LEADING", "env_name_invalid"),
        ("PATH", "env_name_reserved"),
        ("LD_PRELOAD", "env_name_reserved"),
        ("ANSIBLE_CONFIG", "env_name_reserved"),
        ("PYTHONPATH", "env_name_reserved"),
        ("KEEL_TOKEN", "env_name_reserved"),
        ("BASH_ENV", "env_name_reserved"),
    ] {
        let y = BASE.replace("CLOUDFLARE_API_TOKEN", name);
        assert!(
            has(&codes(&y), "jobs.web-deploy.env", code),
            "{name}: {:?}",
            codes(&y)
        );
    }
    let long = "A".repeat(65);
    let y = BASE.replace("CLOUDFLARE_API_TOKEN", &long);
    assert!(has(&codes(&y), "jobs.web-deploy.env", "env_name_invalid"));
    let y = BASE.replace("cf-token", "Not A Name");
    assert!(has(&codes(&y), "jobs.web-deploy.env", "env_name_invalid"));
    let many: Vec<String> = (0..17).map(|i| format!("V{i}: cred")).collect();
    let y = BASE.replace(
        "{ CLOUDFLARE_API_TOKEN: cf-token }",
        &format!("{{ {} }}", many.join(", ")),
    );
    assert!(has(&codes(&y), "jobs.web-deploy.env", "env_too_many"));
}

#[test]
fn unknown_keys_inside_new_blocks_are_still_refused() {
    let y = BASE.replace(
        "    jobs: [web-deploy]",
        "    jobs: [web-deploy]\n    order: 1",
    );
    assert!(!codes(&y).is_empty());
}

/// SC-006: the owner's real manifest, which predates spec 068, validates unchanged and
/// every profile without a recipe expands to the jobs naming it.
#[test]
fn the_real_ansible_manifest_validates_unchanged() {
    let yaml = include_str!("fixtures/real/ansible-jobs.yaml");
    let m = ok(yaml);
    assert!(m.groups.is_empty());
    for (name, p) in &m.profiles {
        assert!(!p.has_recipe(), "{name}");
        let expected: Vec<&str> = m
            .jobs
            .iter()
            .filter(|(_, j)| j.definition.profiles.iter().any(|q| q == name))
            .map(|(n, _)| n.as_str())
            .collect();
        let got = converge::expand(std::slice::from_ref(name), &m).unwrap();
        assert_eq!(got, expected, "{name}");
    }
    for job in m.jobs.values() {
        let json = serde_json::to_value(&job.definition).unwrap();
        assert!(json.get("covers").is_none() && json.get("env").is_none());
    }
}

/// The same for the synthetic repository the catalog tests use.
#[test]
fn the_fixture_repo_manifest_validates_unchanged() {
    let yaml = include_str!("fixtures/ansible-repo/.keel/jobs.yaml");
    let m = ok(yaml);
    assert!(m.profiles.values().all(|p| !p.has_recipe()));
}

/// Spec 068 T074: a `choice` param with no `default` is required, which makes its job
/// manual-only (contracts/ansible.md §7 `provision`).
const MANUAL: &str = r#"
version: 1
profiles:
  prox-vm: {}
  gameserver: {}
inventories:
  fleet: { path: inventory/hosts.yml }
jobs:
  prox-base: { playbook: playbooks/b.yaml, profiles: [prox-vm], inventory: fleet }
  provision:
    playbook: playbooks/provision.yaml
    profiles: [prox-vm, gameserver]
    inventory: fleet
    params:
      stack: { kind: choice, values: [web, db] }
      size: { kind: choice, values: [s, m], default: s }
"#;

#[test]
fn a_choice_without_a_default_is_required() {
    let m = ok(MANUAL);
    let def = &m.jobs["provision"].definition;
    assert!(def.params["stack"].required());
    assert!(!def.params["size"].required());
    assert!(def.manual_only());
    assert_eq!(def.required_params().collect::<Vec<_>>(), ["stack"]);
    assert_eq!(def.missing_required(&BTreeMap::new()), Some("stack"));
    let sent = BTreeMap::from([("stack".to_owned(), "web".to_owned())]);
    assert_eq!(def.missing_required(&sent), None);
    assert!(!m.jobs["prox-base"].definition.manual_only());
}

#[test]
fn a_digest_param_stays_optional() {
    let m = ok(&MANUAL.replace(
        "stack: { kind: choice, values: [web, db] }",
        "stack: { kind: oci_digest }",
    ));
    let def = &m.jobs["provision"].definition;
    assert!(!def.params["stack"].required());
    assert!(!def.manual_only());
}

#[test]
fn a_required_param_with_paths_is_refused() {
    let yaml = MANUAL.replace(
        "    inventory: fleet\n    params:",
        "    inventory: fleet\n    paths: ['composes/**']\n    params:",
    );
    let issues = codes(&yaml);
    assert!(
        has(&issues, "jobs.provision.paths", "required_param_paths"),
        "{issues:?}"
    );
    // A default makes the param optional, so paths are fine again.
    ok(&yaml.replace(
        "stack: { kind: choice, values: [web, db] }",
        "stack: { kind: choice, values: [web, db], default: web }",
    ));
}

#[test]
fn a_recipe_listing_a_manual_only_job_is_refused() {
    let yaml = MANUAL.replace(
        "  gameserver: {}",
        "  gameserver:\n    jobs: [prox-base, provision]",
    );
    let issues = codes(&yaml);
    assert!(
        has(
            &issues,
            "profiles.gameserver.jobs",
            "manual_only_recipe_job"
        ),
        "{issues:?}"
    );
}

#[test]
fn an_implicit_converge_skips_a_manual_only_job() {
    let m = ok(MANUAL);
    let cov = converge::coverage(&m).unwrap();
    let assigned = vec!["prox-vm".to_owned()];
    assert_eq!(converge::expand(&assigned, &m).unwrap(), ["prox-base"]);
    let plan = converge::host_steps(&assigned, &m, &cov).unwrap();
    assert_eq!(
        plan.steps
            .iter()
            .map(|s| s.job.as_str())
            .collect::<Vec<_>>(),
        ["prox-base"]
    );
    assert_eq!(plan.skipped.len(), 1);
    assert_eq!(plan.skipped[0].job, "provision");
    assert_eq!(plan.skipped[0].param, "stack");
    assert_eq!(plan.skipped[0].via, ["prox-vm"]);
    // Reached through two profiles, it is reported once.
    let both = vec!["gameserver".to_owned(), "prox-vm".to_owned()];
    let plan = converge::host_steps(&both, &m, &cov).unwrap();
    assert_eq!(plan.skipped.len(), 1);
    // A profile whose only job is manual-only plans no steps, and that is not an error.
    let only = vec!["gameserver".to_owned()];
    assert!(converge::host_plan(&only, &m, &cov).unwrap().is_empty());
}
