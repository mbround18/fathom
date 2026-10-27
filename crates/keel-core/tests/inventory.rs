//! Inventory parsing and validation (spec 055 T053): YAML and INI inventories with
//! `group_vars/` and `host_vars/`, Ansible name rules, group cycles, duplicate host identities,
//! a JSON Schema over host vars, and the plaintext-secret warning.

use std::collections::BTreeMap;

use keel_core::inventory::{
    Change, ChangeKind, Checked, Format, InventoryFiles, check_files, check_text, content_hash,
    diff, source_of,
};
use serde_json::{Value, json};

/// An in-memory repository: files by path; directories are implied by their children.
#[derive(Default)]
struct Files(BTreeMap<String, String>);

impl Files {
    fn with(mut self, path: &str, text: &str) -> Self {
        self.0.insert(path.to_owned(), text.to_owned());
        self
    }
}

impl InventoryFiles for Files {
    fn read(&self, path: &str) -> Option<String> {
        self.0.get(path).cloned()
    }
    fn list(&self, dir: &str) -> Option<Vec<String>> {
        let prefix = format!("{dir}/");
        let mut names: Vec<String> = self
            .0
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix))
            .map(|rest| rest.split('/').next().unwrap_or(rest).to_owned())
            .collect();
        names.dedup();
        (!names.is_empty()).then_some(names)
    }
}

fn yaml(text: &str) -> Checked {
    check_text(text, Format::Yaml, None)
}

fn ini(text: &str) -> Checked {
    check_text(text, Format::Ini, None)
}

fn host_groups(c: &Checked) -> Vec<(String, Vec<String>)> {
    c.hosts
        .iter()
        .map(|h| (h.host.clone(), h.groups.clone()))
        .collect()
}

fn at(issues: &[keel_core::inventory::InventoryIssue]) -> Vec<String> {
    issues.iter().map(|i| i.at.clone()).collect()
}

const WEB: &str = "\
all:
  vars:
    ntp: pool.example.org
  children:
    web:
      hosts:
        web1.example.com:
          http_port: 8080
        web2.example.com:
    db:
      hosts:
        db1.example.com:
      vars:
        pg_version: 17
";

// --- YAML ---------------------------------------------------------------------------------------

#[test]
fn yaml_inventory_lists_hosts_with_their_groups() {
    let c = yaml(WEB);
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    assert_eq!(
        host_groups(&c),
        vec![
            ("db1.example.com".into(), vec!["db".into()]),
            ("web1.example.com".into(), vec!["web".into()]),
            ("web2.example.com".into(), vec!["web".into()]),
        ]
    );
    assert_eq!(
        c.content["hosts"]["web1.example.com"]["vars"]["http_port"],
        json!(8080)
    );
    assert_eq!(c.content["groups"]["db"]["vars"]["pg_version"], json!(17));
    assert_eq!(c.content["groups"]["all"]["children"], json!(["db", "web"]));
}

#[test]
fn nested_groups_give_hosts_every_ancestor() {
    let c = yaml(
        "all:\n  children:\n    prod:\n      children:\n        web:\n          hosts:\n            w1:\n",
    );
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    assert_eq!(
        host_groups(&c),
        vec![("w1".into(), vec!["prod".into(), "web".into()])]
    );
}

#[test]
fn hosts_in_no_group_are_ungrouped() {
    let c = yaml("all:\n  hosts:\n    lonely:\n");
    assert_eq!(
        host_groups(&c),
        vec![("lonely".into(), vec!["ungrouped".into()])]
    );
}

#[test]
fn host_ranges_expand() {
    let c = yaml("web:\n  hosts:\n    www[01:03].example.com:\n    db-[a:b]:\n");
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    let names: Vec<String> = c.hosts.iter().map(|h| h.host.clone()).collect();
    assert_eq!(
        names,
        [
            "db-a",
            "db-b",
            "www01.example.com",
            "www02.example.com",
            "www03.example.com"
        ]
    );
}

#[test]
fn unknown_group_keys_are_errors() {
    let c = yaml("web:\n  host:\n    w1:\n");
    assert_eq!(at(&c.errors), ["web.host"]);
}

#[test]
fn unparseable_yaml_is_an_error_not_a_panic() {
    let c = yaml("all: [unclosed\n");
    assert!(!c.errors.is_empty());
    assert!(c.hosts.is_empty());
}

#[test]
fn vault_values_are_kept_as_vault() {
    let c = yaml(
        "all:\n  hosts:\n    h1:\n      db_password: !vault |\n        $ANSIBLE_VAULT;1.1;AES256\n        6162\n",
    );
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    assert!(
        c.content["hosts"]["h1"]["vars"]["db_password"]["__ansible_vault"]
            .as_str()
            .is_some_and(|s| s.starts_with("$ANSIBLE_VAULT"))
    );
}

// --- INI ----------------------------------------------------------------------------------------

#[test]
fn ini_inventory_with_vars_and_children() {
    let c = ini("\
bastion ansible_port=2222

[web]
web1 http_port=8080 role='front end'
web2

[web:vars]
ntp=pool.example.org

[prod:children]
web
");
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    assert_eq!(
        host_groups(&c),
        vec![
            ("bastion".into(), vec!["ungrouped".into()]),
            ("web1".into(), vec!["prod".into(), "web".into()]),
            ("web2".into(), vec!["prod".into(), "web".into()]),
        ]
    );
    // Host-line values are typed; `:vars` values stay strings.
    assert_eq!(
        c.content["hosts"]["bastion"]["vars"]["ansible_port"],
        json!(2222)
    );
    assert_eq!(
        c.content["hosts"]["web1"]["vars"]["role"],
        json!("front end")
    );
    assert_eq!(
        c.content["groups"]["web"]["vars"]["ntp"],
        json!("pool.example.org")
    );
}

#[test]
fn ini_comments_and_blank_lines_are_ignored() {
    let c = ini("# a comment\n; another\n\n[web]\nweb1 # trailing\n");
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    assert_eq!(c.hosts.len(), 1);
}

#[test]
fn ini_malformed_lines_are_errors_with_line_numbers() {
    let c = ini("[web\nweb1\n[db]\ndb1 novalue\n");
    assert_eq!(at(&c.errors), ["line 1", "line 4"]);
}

#[test]
fn ini_host_twice_in_one_section_is_a_duplicate() {
    let c = ini("[web]\nweb1\nweb1 a=1\n");
    assert_eq!(at(&c.errors), ["line 3"]);
}

// --- Name rules, cycles, identities ------------------------------------------------------------

#[test]
fn group_names_follow_ansible_rules() {
    let c = yaml("web-servers:\n  hosts:\n    w1:\n");
    assert_eq!(at(&c.errors), ["groups.web-servers"]);
    let c = ini("[1web]\nw1\n");
    assert_eq!(at(&c.errors), ["groups.1web"]);
}

#[test]
fn host_names_are_checked() {
    let c = yaml("web:\n  hosts:\n    'bad host':\n    -leading:\n");
    assert_eq!(at(&c.errors), ["hosts.-leading", "hosts.bad host"]);
}

#[test]
fn var_names_are_identifiers() {
    let c = yaml("web:\n  vars:\n    bad-name: 1\n  hosts:\n    w1:\n      ok_name: 1\n");
    assert_eq!(at(&c.errors), ["groups.web.vars.bad-name"]);
}

#[test]
fn group_cycles_are_refused() {
    let c = ini("[a:children]\nb\n[b:children]\nc\n[c:children]\na\n");
    assert_eq!(c.errors.len(), 1, "{:?}", c.errors);
    assert!(
        c.errors[0].message.contains("a -> b -> c -> a"),
        "{}",
        c.errors[0].message
    );
}

#[test]
fn a_group_cannot_contain_itself() {
    let c = ini("[a:children]\na\n");
    assert_eq!(c.errors.len(), 1, "{:?}", c.errors);
}

#[test]
fn hosts_differing_only_in_case_are_duplicates() {
    let c = yaml("web:\n  hosts:\n    Web1:\ndb:\n  hosts:\n    web1:\n");
    assert_eq!(at(&c.errors), ["hosts.web1"]);
}

#[test]
fn reserved_groups_cannot_have_children_cycles_or_be_children() {
    let c = ini("[web:children]\nall\n");
    assert_eq!(at(&c.errors), ["groups.web.children.all"]);
}

// --- group_vars and host_vars ------------------------------------------------------------------

#[test]
fn repo_inventory_reads_group_and_host_vars() {
    let files = Files::default()
        .with("inventories/prod/hosts.ini", "[web]\nweb1\nweb2\n")
        .with(
            "inventories/prod/group_vars/all.yml",
            "ntp: pool.example.org\n",
        )
        .with(
            "inventories/prod/group_vars/web/main.yml",
            "http_port: 80\n",
        )
        .with("inventories/prod/group_vars/web/tls.yaml", "tls: true\n")
        .with("inventories/prod/host_vars/web1.yaml", "http_port: 8080\n");
    let c = check_files(&files, "inventories/prod/hosts.ini", None);
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    assert_eq!(
        c.content["groups"]["all"]["vars"]["ntp"],
        json!("pool.example.org")
    );
    assert_eq!(
        c.content["groups"]["web"]["vars"],
        json!({"http_port": 80, "tls": true})
    );
    assert_eq!(c.content["hosts"]["web1"]["vars"]["http_port"], json!(8080));
}

#[test]
fn vars_for_unknown_groups_or_hosts_warn() {
    let files = Files::default()
        .with("inv/hosts.yaml", "web:\n  hosts:\n    web1:\n")
        .with("inv/group_vars/dbs.yml", "a: 1\n")
        .with("inv/host_vars/ghost.yml", "a: 1\n");
    let c = check_files(&files, "inv/hosts.yaml", None);
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    assert_eq!(
        at(&c.warnings),
        ["group_vars/dbs.yml", "host_vars/ghost.yml"]
    );
}

#[test]
fn a_missing_inventory_file_is_an_error() {
    let c = check_files(&Files::default(), "inv/hosts.yaml", None);
    assert_eq!(at(&c.errors), ["inv/hosts.yaml"]);
}

#[test]
fn bad_vars_files_are_errors() {
    let files = Files::default()
        .with("hosts.yml", "web:\n  hosts:\n    web1:\n")
        .with("group_vars/web.yml", "- not\n- a mapping\n");
    let c = check_files(&files, "hosts.yml", None);
    assert_eq!(at(&c.errors), ["group_vars/web.yml"]);
}

// --- Schema ------------------------------------------------------------------------------------

fn schema() -> Value {
    json!({
        "type": "object",
        "required": ["http_port"],
        "properties": {
            "http_port": {"type": "integer", "minimum": 1, "maximum": 65535},
            "env": {"enum": ["prod", "staging"]},
            "db_password": {"type": "string"}
        }
    })
}

#[test]
fn schema_is_checked_against_each_hosts_effective_vars() {
    let text = "all:\n  vars:\n    http_port: 80\n  children:\n    web:\n      hosts:\n        w1:\n          http_port: 70000\n        w2:\n          env: dev\n";
    let c = check_text(text, Format::Yaml, Some(&schema()));
    assert_eq!(
        at(&c.errors),
        ["hosts.w1.vars.http_port", "hosts.w2.vars.env"]
    );
}

#[test]
fn schema_required_and_types() {
    let c = check_text(
        "web:\n  hosts:\n    w1:\n      http_port: '80'\n    w2:\n",
        Format::Yaml,
        Some(&schema()),
    );
    assert_eq!(at(&c.errors), ["hosts.w1.vars.http_port", "hosts.w2.vars"]);
}

#[test]
fn vault_values_satisfy_string_types() {
    let text = "web:\n  hosts:\n    w1:\n      http_port: 80\n      db_password: !vault |\n        $ANSIBLE_VAULT;1.1;AES256\n        6162\n";
    let c = check_text(text, Format::Yaml, Some(&schema()));
    assert!(c.errors.is_empty(), "{:?}", c.errors);
}

#[test]
fn unsupported_schema_keywords_fail_closed() {
    let s = json!({"type": "object", "patternProperties": {"^x": {}}});
    let c = check_text("web:\n  hosts:\n    w1:\n", Format::Yaml, Some(&s));
    assert_eq!(at(&c.errors), ["schema.patternProperties"]);
}

// --- Plaintext secrets -------------------------------------------------------------------------

#[test]
fn plaintext_secret_like_keys_warn_without_the_value() {
    let text = "all:\n  vars:\n    api_token: hunter2\n    db_password: '{{ lookup(\"env\", \"PW\") }}'\n  hosts:\n    h1:\n      Root_Secret: s3cr3t\n      password_policy: 12\n";
    let c = yaml(text);
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    assert_eq!(
        at(&c.warnings),
        ["groups.all.vars.api_token", "hosts.h1.vars.Root_Secret"]
    );
    let all = format!("{:?}", c.warnings);
    assert!(!all.contains("hunter2") && !all.contains("s3cr3t"));
}

// --- Content, hash, diff -----------------------------------------------------------------------

#[test]
fn hash_ignores_formatting_and_key_order() {
    let a = yaml("web:\n  hosts:\n    w1: {a: 1, b: 2}\n");
    let b = yaml("# comment\nweb:\n  hosts:\n    w1:\n      b: 2\n      a: 1\n");
    assert_eq!(a.hash, b.hash);
    assert_eq!(a.hash, content_hash(&b.content));
    let c = yaml("web:\n  hosts:\n    w1: {a: 1, b: 3}\n");
    assert_ne!(a.hash, c.hash);
    assert_eq!(a.hash.len(), 64);
}

#[test]
fn managed_content_keeps_its_source_text() {
    let text = "# keep me\nweb:\n  hosts:\n    w1:\n";
    let c = yaml(text);
    assert_eq!(source_of(&c.content), Some(text));
}

#[test]
fn invalid_input_still_has_content_and_a_hash() {
    let a = yaml("web: [\n");
    let b = yaml("web: {\n");
    assert!(!a.errors.is_empty());
    assert_eq!(a.hash.len(), 64);
    assert_ne!(a.hash, b.hash);
    assert_eq!(source_of(&a.content), Some("web: [\n"));
}

#[test]
fn diff_lists_structural_changes() {
    let a = yaml("web:\n  hosts:\n    w1: {port: 80}\n    w2:\n");
    let b = yaml("web:\n  hosts:\n    w1: {port: 8080}\n    w3:\n");
    let changes = diff(&a.content, &b.content);
    let summary: Vec<(String, ChangeKind)> = changes
        .iter()
        .map(|c: &Change| (c.path.clone(), c.kind))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("groups.web.hosts".into(), ChangeKind::Changed),
            ("hosts.w1.vars.port".into(), ChangeKind::Changed),
            ("hosts.w2".into(), ChangeKind::Removed),
            ("hosts.w3".into(), ChangeKind::Added),
        ]
    );
    assert_eq!(changes[1].before, Some(json!(80)));
    assert_eq!(changes[1].after, Some(json!(8080)));
    assert!(diff(&a.content, &a.content).is_empty());
}
