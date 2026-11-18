//! Spec 068 T061: `choice` params (contracts/manifest.md "Param kind `choice`") and
//! `RepoTree::list_dirs`.

use std::collections::{BTreeMap, BTreeSet};

use keel_core::manifest::{
    ChoiceError, Context, MAX_CHOICE_FROM, ManifestIssue, ParamKind, ParamSpec, PathKind, RepoTree,
    is_choice_value, validate,
};

/// A tree of files and directories by path. `composes/` holds the given entries.
struct Tree {
    entries: BTreeMap<String, PathKind>,
}

impl Tree {
    fn new(composes: &[(&str, PathKind)]) -> Self {
        let mut entries = BTreeMap::new();
        for p in ["playbooks/p.yaml", "inv/hosts.yaml"] {
            entries.insert(p.to_owned(), PathKind::File);
        }
        for d in ["playbooks", "inv", "composes", "empty"] {
            entries.insert(d.to_owned(), PathKind::Dir);
        }
        for (name, kind) in composes {
            entries.insert(format!("composes/{name}"), kind.clone());
        }
        Self { entries }
    }

    fn many(n: usize) -> Self {
        let names: Vec<String> = (0..n).map(|i| format!("stack-{i:03}")).collect();
        let composes: Vec<(&str, PathKind)> =
            names.iter().map(|n| (n.as_str(), PathKind::Dir)).collect();
        Self::new(&composes)
    }
}

impl RepoTree for Tree {
    fn kind(&self, path: &str) -> Option<PathKind> {
        if path.is_empty() {
            return Some(PathKind::Dir);
        }
        self.entries.get(path).cloned()
    }

    fn list_dirs(&self, path: &str) -> Option<Vec<String>> {
        if self.kind(path) != Some(PathKind::Dir) {
            return None;
        }
        let prefix = format!("{path}/");
        Some(
            self.entries
                .iter()
                .filter_map(|(p, k)| {
                    let rest = p.strip_prefix(&prefix)?;
                    (!rest.contains('/') && *k == PathKind::Dir).then(|| rest.to_owned())
                })
                .collect(),
        )
    }
}

/// A tree that cannot list (the trait's default).
struct NoList(Tree);

impl RepoTree for NoList {
    fn kind(&self, path: &str) -> Option<PathKind> {
        self.0.kind(path)
    }
}

fn stacks() -> Tree {
    Tree::new(&[
        ("web", PathKind::Dir),
        ("db", PathKind::Dir),
        (".git-crypt", PathKind::Dir),
        ("bad name", PathKind::Dir),
        ("README.md", PathKind::File),
        ("link", PathKind::Symlink("web".into())),
    ])
}

fn manifest(param: &str) -> String {
    format!(
        r#"
version: 1
profiles:
  debian: {{}}
inventories:
  prod: {{ path: inv/hosts.yaml }}
jobs:
  provision:
    playbook: playbooks/p.yaml
    profiles: [debian]
    inventory: prod
    params:
      stack: {param}
"#
    )
}

fn ctx() -> Context {
    Context {
        signed_mode: false,
        granted_credentials: BTreeSet::new(),
    }
}

fn spec(tree: &dyn RepoTree, param: &str) -> ParamSpec {
    let m = validate(manifest(param).as_bytes(), tree, &ctx())
        .unwrap_or_else(|e| panic!("valid: {e:?}"));
    m.jobs["provision"].definition.params["stack"].clone()
}

fn codes(tree: &dyn RepoTree, param: &str) -> Vec<(String, String)> {
    validate(manifest(param).as_bytes(), tree, &ctx())
        .expect_err("should be refused")
        .into_iter()
        .map(|i: ManifestIssue| (i.at, i.code.unwrap_or_default()))
        .collect()
}

fn has(issues: &[(String, String)], at: &str, code: &str) -> bool {
    issues.iter().any(|(a, c)| a == at && c == code)
}

const AT: &str = "jobs.provision.params.stack";

#[test]
fn values_shape_validates_and_lists_its_values() {
    let tree = stacks();
    let s = spec(
        &tree,
        "{ kind: choice, values: [small, medium, large], default: medium }",
    );
    assert_eq!(s.kind, ParamKind::Choice);
    assert_eq!(s.default.as_deref(), Some("medium"));
    assert_eq!(
        s.choices(&tree).unwrap().unwrap(),
        vec!["small", "medium", "large"]
    );
    assert!(s.check_value("large", &tree).is_ok());
    assert!(s.check_value("huge", &tree).is_err());
    // The error never echoes the value.
    assert!(!s.check_value("huge", &tree).unwrap_err().contains("huge"));
    assert!(s.check_value("../etc", &tree).is_err());
}

#[test]
fn from_lists_matching_directories_only() {
    let tree = stacks();
    let s = spec(&tree, "{ kind: choice, from: composes, default: web }");
    // Hidden, ill-formed names, files and symbolic links are not choices.
    assert_eq!(s.choices(&tree).unwrap().unwrap(), vec!["db", "web"]);
    assert!(s.check_value("db", &tree).is_ok());
    assert!(s.check_value(".git-crypt", &tree).is_err());
    assert!(s.check_value("link", &tree).is_err());
    // An empty directory is valid, with no choices.
    let s = spec(&tree, "{ kind: choice, from: empty }");
    assert!(s.choices(&tree).unwrap().unwrap().is_empty());
}

#[test]
fn shape_refusals() {
    let tree = stacks();
    for bad in [
        "{ kind: choice }",
        "{ kind: choice, values: [a], from: composes }",
        "{ kind: choice, values: [] }",
        "{ kind: choice, values: [a, a] }",
        "{ kind: choice, values: ['-a'] }",
        "{ kind: choice, values: ['a b'] }",
        "{ kind: oci_digest, values: [a] }",
        "{ kind: oci_digest, default: a }",
    ] {
        let issues = codes(&tree, bad);
        assert!(
            issues
                .iter()
                .any(|(a, c)| a.starts_with(AT) && c == "choice_shape"),
            "{bad}: {issues:?}"
        );
    }
    let many: Vec<String> = (0..65).map(|i| format!("v{i}")).collect();
    let issues = codes(
        &tree,
        &format!("{{ kind: choice, values: [{}] }}", many.join(", ")),
    );
    assert!(
        has(&issues, &format!("{AT}.values"), "choice_shape"),
        "{issues:?}"
    );
    // 64 is fine.
    let ok: Vec<String> = (0..64).map(|i| format!("v{i}")).collect();
    spec(
        &tree,
        &format!("{{ kind: choice, values: [{}] }}", ok.join(", ")),
    );
    // An unknown key is refused at parse.
    assert!(
        validate(
            manifest("{ kind: choice, values: [a], extra: 1 }").as_bytes(),
            &tree,
            &ctx()
        )
        .is_err()
    );
}

#[test]
fn from_refusals() {
    let tree = stacks();
    for missing in [
        "{ kind: choice, from: nope }",
        "{ kind: choice, from: playbooks/p.yaml }",
        "{ kind: choice, from: ../composes }",
        "{ kind: choice, from: /etc }",
        "{ kind: choice, from: composes/link }",
    ] {
        let issues = codes(&tree, missing);
        assert!(
            has(&issues, &format!("{AT}.from"), "choice_from_missing"),
            "{missing}: {issues:?}"
        );
    }
    // A tree that cannot list fails closed.
    let issues = codes(&NoList(stacks()), "{ kind: choice, from: composes }");
    assert!(has(&issues, &format!("{AT}.from"), "choice_from_missing"));

    let issues = codes(
        &Tree::many(MAX_CHOICE_FROM + 1),
        "{ kind: choice, from: composes }",
    );
    assert!(
        has(&issues, &format!("{AT}.from"), "choice_from_too_many"),
        "{issues:?}"
    );
    spec(
        &Tree::many(MAX_CHOICE_FROM),
        "{ kind: choice, from: composes }",
    );
}

#[test]
fn a_default_must_be_a_choice() {
    let tree = stacks();
    let issues = codes(&tree, "{ kind: choice, values: [a, b], default: c }");
    assert!(
        has(&issues, &format!("{AT}.default"), "choice_default"),
        "{issues:?}"
    );
    let issues = codes(
        &tree,
        "{ kind: choice, from: composes, default: .git-crypt }",
    );
    assert!(
        has(&issues, &format!("{AT}.default"), "choice_default"),
        "{issues:?}"
    );
}

#[test]
fn choice_values_and_kind_validation() {
    for good in ["a", "A9", "web-1.prod_x", &"a".repeat(63)] {
        assert!(is_choice_value(good), "{good}");
        assert!(ParamKind::Choice.validate(good).is_ok());
    }
    for bad in [
        "",
        ".hidden",
        "-a",
        "_a",
        "a b",
        "a/b",
        "é",
        &"a".repeat(64),
    ] {
        assert!(!is_choice_value(bad), "{bad}");
        assert!(ParamKind::Choice.validate(bad).is_err());
    }
    assert_eq!(ChoiceError::FromTooMany.code(), "choice_from_too_many");
}

#[test]
fn an_oci_digest_param_serializes_as_before() {
    let s = ParamSpec::of(ParamKind::OciDigest);
    assert_eq!(
        serde_json::to_value(&s).unwrap(),
        serde_json::json!({ "kind": "oci_digest" })
    );
    // Not a choice: no choices, any digest-shaped value passes check_value.
    assert!(s.choices(&stacks()).is_none());
}
