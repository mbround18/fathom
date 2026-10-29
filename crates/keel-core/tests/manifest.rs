//! `.keel/jobs.yaml` validation, one case per rule in contracts/repo-manifest.md (055 T041).

use std::collections::{BTreeMap, BTreeSet};

use keel_core::manifest::{
    Context, InventorySource, JobDefinition, ManifestIssue, ParamKind, PathKind, RepoTree, validate,
};

/// An in-memory tree: files, directories (implied by their children) and symlinks.
#[derive(Default)]
struct Tree(BTreeMap<String, PathKind>);

impl Tree {
    fn standard() -> Self {
        let mut t = Self::default();
        for f in [
            "playbooks/web.yaml",
            "playbooks/web-rollback.yaml",
            "inventories/prod/hosts.yaml",
            "inventories/schema.json",
        ] {
            t.file(f);
        }
        t
    }
    fn file(&mut self, p: &str) -> &mut Self {
        let parts: Vec<&str> = p.split('/').collect();
        for i in 1..parts.len() {
            self.0.insert(parts[..i].join("/"), PathKind::Dir);
        }
        self.0.insert(p.to_owned(), PathKind::File);
        self
    }
    fn link(&mut self, p: &str, target: &str) -> &mut Self {
        self.0
            .insert(p.to_owned(), PathKind::Symlink(target.to_owned()));
        self
    }
}

impl RepoTree for Tree {
    fn kind(&self, path: &str) -> Option<PathKind> {
        self.0.get(path).cloned()
    }
}

const GOOD: &str = r#"
version: 1
profiles:
  debian: { description: "Every Debian host" }
  gameserver: { description: "Game servers" }
inventories:
  prod:
    path: inventories/prod/hosts.yaml
    schema: inventories/schema.json
jobs:
  web-deploy:
    playbook: playbooks/web.yaml
    profiles: [debian]
    inventory: prod
    limit: "web:&prod"
    vars: { release_channel: stable }
    credentials: [prod-vault, web-secrets]
    paths: ["playbooks/web.yaml", "roles/web/**", "inventories/prod/**"]
    timeout: 30m
    rollback:
      playbook: playbooks/web-rollback.yaml
      reapply: last_success
"#;

fn granted(names: &[&str]) -> Context {
    Context {
        signed_mode: false,
        granted_credentials: names
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<BTreeSet<_>>(),
    }
}

fn errors(yaml: &str, tree: &Tree, ctx: &Context) -> Vec<ManifestIssue> {
    validate(yaml.as_bytes(), tree, ctx).expect_err("manifest should be refused")
}

fn mentions(issues: &[ManifestIssue], at: &str, needle: &str) -> bool {
    issues
        .iter()
        .any(|i| i.at.starts_with(at) && i.message.contains(needle))
}

#[test]
fn the_contract_example_validates() {
    let m = validate(
        GOOD.as_bytes(),
        &Tree::standard(),
        &granted(&["prod-vault", "web-secrets"]),
    )
    .expect("valid");
    let job = &m.jobs["web-deploy"];
    assert_eq!(job.definition.playbook, "playbooks/web.yaml");
    assert_eq!(job.definition.inventory_source, InventorySource::Repo);
    assert_eq!(job.definition.timeout_secs, Some(1800));
    assert_eq!(job.blocked_reason(), None);
    assert_eq!(m.profiles.len(), 2);
    // The definition is what lands in jobs.definition (jsonb).
    let json = serde_json::to_value(&job.definition).unwrap();
    assert_eq!(json["vars"]["release_channel"], "stable");
    assert_eq!(json["rollback"]["reapply"], "last_success");
}

#[test]
fn version_must_be_one() {
    let y = GOOD.replace("version: 1", "version: 2");
    assert!(mentions(
        &errors(&y, &Tree::standard(), &granted(&[])),
        "version",
        "version"
    ));
    let y = GOOD.replace("version: 1\n", "");
    assert!(!errors(&y, &Tree::standard(), &granted(&[])).is_empty());
}

#[test]
fn unknown_keys_are_refused_at_every_level() {
    for (from, to) in [
        ("version: 1", "version: 1\napproval: none"),
        (
            "    playbook: playbooks/web.yaml",
            "    playbook: playbooks/web.yaml\n    approve: false",
        ),
        (
            "    schema: inventories/schema.json",
            "    schema: inventories/schema.json\n    signed: false",
        ),
        (
            "      reapply: last_success",
            "      reapply: last_success\n      quorum: 0",
        ),
    ] {
        let y = GOOD.replace(from, to);
        let issues = errors(&y, &Tree::standard(), &granted(&[]));
        assert!(
            issues.iter().any(|i| i.message.contains("unknown field")),
            "{to}: {issues:?}"
        );
    }
}

#[test]
fn paths_stay_inside_the_repository() {
    let tree = Tree::standard();
    for (bad, needle) in [
        ("../outside.yaml", ".."),
        ("playbooks/../../x.yaml", ".."),
        ("/etc/passwd", "absolute"),
    ] {
        let y = GOOD.replace(
            "    playbook: playbooks/web.yaml",
            &format!("    playbook: {bad}"),
        );
        assert!(
            mentions(
                &errors(&y, &tree, &granted(&[])),
                "jobs.web-deploy.playbook",
                needle
            ),
            "{bad}"
        );
    }
    let y = GOOD.replace("\"roles/web/**\"", "\"../roles/**\"");
    assert!(mentions(
        &errors(&y, &tree, &granted(&[])),
        "jobs.web-deploy.paths",
        ".."
    ));
    let y = GOOD.replace("path: inventories/prod/hosts.yaml", "path: /srv/hosts");
    assert!(mentions(
        &errors(&y, &tree, &granted(&[])),
        "inventories.prod.path",
        "absolute"
    ));
}

#[test]
fn symlinks_may_not_leave_the_tree() {
    let mut tree = Tree::standard();
    tree.link("playbooks/escape.yaml", "../../etc/passwd")
        .link("playbooks/abs.yaml", "/etc/passwd")
        .link("outdir", "..")
        .link("playbooks/inside.yaml", "web.yaml")
        .link("pb", "playbooks")
        .link("loop", "loop");
    let check = |pb: &str| {
        let y = GOOD.replace(
            "    playbook: playbooks/web.yaml",
            &format!("    playbook: {pb}"),
        );
        validate(
            y.as_bytes(),
            &tree,
            &granted(&["prod-vault", "web-secrets"]),
        )
    };
    for bad in [
        "playbooks/escape.yaml",
        "playbooks/abs.yaml",
        "outdir/x.yaml",
        "loop",
    ] {
        let issues = check(bad).expect_err(bad);
        assert!(
            mentions(&issues, "jobs.web-deploy.playbook", "symbolic link"),
            "{bad}: {issues:?}"
        );
    }
    // Links that stay inside resolve like the file they point at.
    check("playbooks/inside.yaml").expect("inside link");
    check("pb/web.yaml").expect("linked directory");
}

#[test]
fn the_playbook_must_exist_at_the_commit() {
    let y = GOOD.replace(
        "    playbook: playbooks/web.yaml",
        "    playbook: playbooks/missing.yaml",
    );
    assert!(mentions(
        &errors(&y, &Tree::standard(), &granted(&[])),
        "jobs.web-deploy.playbook",
        "does not exist"
    ));
    let y = GOOD.replace(
        "    playbook: playbooks/web.yaml",
        "    playbook: playbooks",
    );
    assert!(mentions(
        &errors(&y, &Tree::standard(), &granted(&[])),
        "jobs.web-deploy.playbook",
        "not a file"
    ));
    let y = GOOD.replace(
        "playbook: playbooks/web-rollback.yaml",
        "playbook: playbooks/nope.yaml",
    );
    assert!(mentions(
        &errors(&y, &Tree::standard(), &granted(&[])),
        "jobs.web-deploy.rollback.playbook",
        "does not exist"
    ));
}

#[test]
fn profiles_are_required_and_declared() {
    let y = GOOD.replace("profiles: [debian]", "profiles: []");
    assert!(mentions(
        &errors(&y, &Tree::standard(), &granted(&[])),
        "jobs.web-deploy.profiles",
        "at least one"
    ));
    let y = GOOD.replace("profiles: [debian]", "profiles: [debian, windows]");
    assert!(mentions(
        &errors(&y, &Tree::standard(), &granted(&[])),
        "jobs.web-deploy.profiles",
        "'windows'"
    ));
}

#[test]
fn the_limit_must_parse() {
    for bad in ["web prod", "-e@/etc/x", "web[0:", "a::b"] {
        let y = GOOD.replace("limit: \"web:&prod\"", &format!("limit: \"{bad}\""));
        assert!(
            mentions(
                &errors(&y, &Tree::standard(), &granted(&[])),
                "jobs.web-deploy.limit",
                ""
            ),
            "{bad}"
        );
    }
}

#[test]
fn signed_mode_needs_a_repository_inventory() {
    let y = GOOD.replace("    inventory: prod", "    inventory: managed-prod");
    let ok = validate(
        y.as_bytes(),
        &Tree::standard(),
        &granted(&["prod-vault", "web-secrets"]),
    )
    .unwrap();
    assert_eq!(
        ok.jobs["web-deploy"].definition.inventory_source,
        InventorySource::Managed
    );

    let mut signed = granted(&["prod-vault", "web-secrets"]);
    signed.signed_mode = true;
    assert!(mentions(
        &errors(&y, &Tree::standard(), &signed),
        "jobs.web-deploy.inventory",
        "signed mode"
    ));
    validate(GOOD.as_bytes(), &Tree::standard(), &signed)
        .expect("repo inventory is fine in signed mode");
}

#[test]
fn ungranted_credentials_block_the_job_instead_of_dropping_it() {
    let m = validate(
        GOOD.as_bytes(),
        &Tree::standard(),
        &granted(&["prod-vault"]),
    )
    .unwrap();
    let job = &m.jobs["web-deploy"];
    assert_eq!(job.blocked_reason(), Some("credential_unavailable"));
    assert_eq!(job.missing_credentials, vec!["web-secrets".to_owned()]);
    // The credential stays in the definition.
    assert_eq!(
        job.definition.credentials,
        vec!["prod-vault", "web-secrets"]
    );
}

#[test]
fn secret_like_vars_are_refused() {
    for key in [
        "ansible_password",
        "ansible_ssh_password",
        "ANSIBLE_Become_Password",
        "ansible_become_pass",
        "ansible_ssh_private_key_file",
        "ansible_winrm_password",
    ] {
        let y = GOOD.replace(
            "vars: { release_channel: stable }",
            &format!("vars: {{ {key}: hunter2 }}"),
        );
        let issues = errors(&y, &Tree::standard(), &granted(&[]));
        assert!(
            mentions(&issues, "jobs.web-deploy.vars", "credentials"),
            "{key}"
        );
        // The value is never echoed back.
        assert!(
            issues
                .iter()
                .all(|i| !i.message.contains("hunter2") && !i.at.contains("hunter2"))
        );
    }
    // Ordinary variables, even ones mentioning passwords outside ansible_*, are fine.
    let y = GOOD.replace(
        "vars: { release_channel: stable }",
        "vars: { password_policy: strict }",
    );
    validate(
        y.as_bytes(),
        &Tree::standard(),
        &granted(&["prod-vault", "web-secrets"]),
    )
    .unwrap();
}

#[test]
fn every_problem_is_reported_at_once() {
    let y = GOOD
        .replace("profiles: [debian]", "profiles: []")
        .replace(
            "    playbook: playbooks/web.yaml",
            "    playbook: ../x.yaml",
        )
        .replace("timeout: 30m", "timeout: forever");
    let issues = errors(&y, &Tree::standard(), &granted(&[]));
    assert!(issues.len() >= 3, "{issues:?}");
}

#[test]
fn oversized_and_malformed_manifests_are_refused() {
    let big = format!("{GOOD}\n# {}", "x".repeat(300 * 1024));
    assert!(!errors(&big, &Tree::standard(), &granted(&[])).is_empty());
    assert!(!errors("version: [", &Tree::standard(), &granted(&[])).is_empty());
}

// ---- job params (spec 061 T011, data-model.md "Job parameters") ------------------------------

const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// GOOD with `block` (already indented under the job) as the job's `params`.
fn with_params(block: &str) -> String {
    GOOD.replace(
        "    timeout: 30m\n",
        &format!("    timeout: 30m\n    params:\n{block}"),
    )
}

fn ok_ctx() -> Context {
    granted(&["prod-vault", "web-secrets"])
}

#[test]
fn a_job_without_params_parses_as_before() {
    let m = validate(GOOD.as_bytes(), &Tree::standard(), &ok_ctx()).unwrap();
    let def = &m.jobs["web-deploy"].definition;
    assert!(def.params.is_empty());
    // The stored definition is unchanged for jobs without params, and one stored before this
    // spec still reads back.
    let json = serde_json::to_value(def).unwrap();
    assert!(json.get("params").is_none(), "{json}");
    let back: JobDefinition = serde_json::from_value(json).unwrap();
    assert_eq!(&back, def);
}

#[test]
fn a_valid_params_block_parses() {
    let y = with_params("      keel_server_image_digest: { kind: oci_digest }\n");
    let m = validate(y.as_bytes(), &Tree::standard(), &ok_ctx()).unwrap();
    let def = &m.jobs["web-deploy"].definition;
    assert_eq!(
        def.params["keel_server_image_digest"].kind,
        ParamKind::OciDigest
    );
    let json = serde_json::to_value(def).unwrap();
    assert_eq!(
        json["params"]["keel_server_image_digest"]["kind"],
        "oci_digest"
    );
    let back: JobDefinition = serde_json::from_value(json).unwrap();
    assert_eq!(&back, def);
}

#[test]
fn param_names_follow_the_rules() {
    let long = "a".repeat(65);
    for bad in [
        "Upper",
        "9lives",
        "has-dash",
        "ansible_host",
        "keel_internal_x",
        long.as_str(),
    ] {
        let y = with_params(&format!("      {bad}: {{ kind: oci_digest }}\n"));
        let issues = errors(&y, &Tree::standard(), &ok_ctx());
        assert!(
            mentions(&issues, "jobs.web-deploy.params", "parameter name"),
            "{bad}: {issues:?}"
        );
    }
    // 64 characters is the longest allowed, and a leading '_' is fine.
    let longest = "a".repeat(64);
    for good in [longest.as_str(), "_digest"] {
        let y = with_params(&format!("      {good}: {{ kind: oci_digest }}\n"));
        validate(y.as_bytes(), &Tree::standard(), &ok_ctx()).unwrap();
    }
}

#[test]
fn at_most_eight_params() {
    let block = |n: usize| -> String {
        (0..n)
            .map(|i| format!("      p{i}: {{ kind: oci_digest }}\n"))
            .collect()
    };
    let issues = errors(&with_params(&block(9)), &Tree::standard(), &ok_ctx());
    assert!(
        mentions(&issues, "jobs.web-deploy.params", "at most 8"),
        "{issues:?}"
    );
    validate(
        with_params(&block(8)).as_bytes(),
        &Tree::standard(),
        &ok_ctx(),
    )
    .unwrap();
}

#[test]
fn param_kinds_are_a_closed_set() {
    let y = with_params("      image: { kind: string }\n");
    assert!(!errors(&y, &Tree::standard(), &ok_ctx()).is_empty());
    // A param spec takes no other keys.
    let y = with_params("      image: { kind: oci_digest, default: x }\n");
    assert!(!errors(&y, &Tree::standard(), &ok_ctx()).is_empty());
}

#[test]
fn a_param_may_not_also_be_a_var() {
    let y = with_params("      release_channel: { kind: oci_digest }\n");
    let issues = errors(&y, &Tree::standard(), &ok_ctx());
    assert!(
        mentions(&issues, "jobs.web-deploy.params.release_channel", "vars"),
        "{issues:?}"
    );
}

#[test]
fn oci_digest_values_are_checked() {
    assert!(ParamKind::OciDigest.validate(DIGEST).is_ok());
    let upper = DIGEST.replace("abcdef", "ABCDEF");
    let short = &DIGEST[..DIGEST.len() - 1];
    let long = format!("{DIGEST}0");
    let spaced = format!(" {DIGEST}");
    let other = DIGEST.replace("sha256", "sha512");
    for bad in [
        "",
        "sha256:",
        upper.as_str(),
        short,
        long.as_str(),
        spaced.as_str(),
        other.as_str(),
    ] {
        assert!(ParamKind::OciDigest.validate(bad).is_err(), "{bad:?}");
    }
}
