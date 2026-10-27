//! Inventories (spec 055 T057, research R9): Ansible YAML and INI inventories, with the
//! `group_vars/` and `host_vars/` beside a repository inventory, parsed into one normalized
//! content form and validated.
//!
//! Validation collects every problem: Ansible's name rules for groups, hosts and vars, no group
//! cycles, no two hosts with the same identity (names are case-insensitive), and an optional
//! JSON Schema over each host's effective vars. A key that looks like a secret (`*password*`,
//! `*token*`, `*secret*`) holding a plain string is a warning; vault values are fine. An invalid
//! inventory still yields content and a hash, so it can be stored as an `invalid` revision with
//! its errors.
//!
//! Nothing here reads a repository: the caller passes [`InventoryFiles`] for the fetched head.
//! Issues name where the problem is and never quote var values.
//!
//! The content form, as stored in `inventory_revisions.content`:
//!
//! ```json
//! { "hosts":  { "<host>":  { "vars": {..} } },
//!   "groups": { "<group>": { "hosts": [..], "children": [..], "vars": {..} } },
//!   "source": "<the text, for managed inventories>",
//!   "unparsed": "<sha256 of text that did not parse>" }
//! ```
//!
//! `all` is always present, with every top-level group as a child. A vault value is
//! `{"__ansible_vault": "<ciphertext>"}`. The hash covers everything but `source`, so a change
//! to comments or layout alone is not a new revision.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// The largest inventory (or vars file) Keel parses.
pub const MAX_INVENTORY_BYTES: usize = 1024 * 1024;

/// The most hosts one inventory may hold after ranges expand.
pub const MAX_HOSTS: usize = 10_000;

/// The key a vault value is kept under.
pub const VAULT_KEY: &str = "__ansible_vault";

type Vars = Map<String, Value>;

/// One problem: where it is and what is wrong.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct InventoryIssue {
    /// A dotted location (`hosts.web1.vars.http_port`), a file (`group_vars/web.yml`) or an
    /// INI line (`line 4`).
    pub at: String,
    pub message: String,
}

impl InventoryIssue {
    fn new(at: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            at: at.into(),
            message: message.into(),
        }
    }
}

/// How an inventory file is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// Ansible's YAML inventory (JSON is accepted as YAML).
    Yaml,
    /// Ansible's INI inventory.
    Ini,
}

impl Format {
    /// YAML for `.yml`, `.yaml` and `.json`; INI otherwise, as Ansible does.
    #[must_use]
    pub fn from_path(path: &str) -> Self {
        let lower = path.to_ascii_lowercase();
        if [".yml", ".yaml", ".json"]
            .iter()
            .any(|e| lower.ends_with(e))
        {
            Self::Yaml
        } else {
            Self::Ini
        }
    }
}

/// A read-only view of the repository at one commit.
pub trait InventoryFiles {
    /// The UTF-8 text of the file at `path` (relative, `/`-separated), or `None`.
    fn read(&self, path: &str) -> Option<String>;
    /// The names directly inside directory `dir`, or `None` when it is not a directory.
    fn list(&self, dir: &str) -> Option<Vec<String>>;
}

/// A host and the groups it is in (transitively, without `all`; `ungrouped` when none), as
/// `inventory_hosts` stores them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostGroups {
    pub host: String,
    pub groups: Vec<String>,
}

/// A parsed, validated inventory, ready to store as a revision.
#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    pub content: Value,
    /// [`content_hash`] of `content`: 64 hex digits.
    pub hash: String,
    /// Sorted by host.
    pub hosts: Vec<HostGroups>,
    pub errors: Vec<InventoryIssue>,
    pub warnings: Vec<InventoryIssue>,
}

impl Checked {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Parses and validates inventory `text` (a managed inventory, or one pasted in to check). The
/// text is kept in the content as `source`.
#[must_use]
pub fn check_text(text: &str, format: Format, schema: Option<&Value>) -> Checked {
    let mut issues = Issues::default();
    let inv = parse(text, format, &mut issues);
    finish(inv, text, Some(text), schema, issues)
}

/// Reads and validates the repository inventory at `path`, with the `group_vars/` and
/// `host_vars/` beside it.
#[must_use]
pub fn check_files(files: &dyn InventoryFiles, path: &str, schema: Option<&Value>) -> Checked {
    let mut issues = Issues::default();
    let Some(text) = files.read(path) else {
        issues.error(path, "inventory file not found");
        return finish(None, "", None, schema, issues);
    };
    let mut inv = parse(&text, Format::from_path(path), &mut issues);
    if let Some(inv) = inv.as_mut() {
        let base = path.rsplit_once('/').map_or("", |(dir, _)| dir);
        load_vars(files, base, VarsKind::Group, inv, &mut issues);
        load_vars(files, base, VarsKind::Host, inv, &mut issues);
    }
    finish(inv, &text, None, schema, issues)
}

/// The managed source text kept in `content`, if any.
#[must_use]
pub fn source_of(content: &Value) -> Option<&str> {
    content.get("source").and_then(Value::as_str)
}

/// SHA-256 over the canonical JSON of `content` without `source`: 64 hex digits.
#[must_use]
pub fn content_hash(content: &Value) -> String {
    let mut canonical = String::new();
    match content {
        Value::Object(m) => {
            let mut stripped = m.clone();
            stripped.remove("source");
            write_canonical(&Value::Object(stripped), &mut canonical);
        }
        other => write_canonical(other, &mut canonical),
    }
    sha256_hex(canonical.as_bytes())
}

// --- Diff ---------------------------------------------------------------------------------------

/// How one path differs between two revisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Removed,
    Changed,
}

/// One difference between two contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    /// A dotted path, such as `hosts.web1.vars.http_port`.
    pub path: String,
    pub kind: ChangeKind,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

/// The structural differences from `from` to `to`, by path. Objects are compared key by key;
/// anything else (lists included) as a whole. `source` is ignored.
#[must_use]
pub fn diff(from: &Value, to: &Value) -> Vec<Change> {
    let mut out = Vec::new();
    let strip = |v: &Value| match v {
        Value::Object(m) => {
            let mut m = m.clone();
            m.remove("source");
            Value::Object(m)
        }
        other => other.clone(),
    };
    diff_at("", &strip(from), &strip(to), &mut out);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn diff_at(path: &str, a: &Value, b: &Value, out: &mut Vec<Change>) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let keys: BTreeSet<&String> = x.keys().chain(y.keys()).collect();
            for k in keys {
                let p = join_dot(path, k);
                match (x.get(k), y.get(k)) {
                    (Some(va), Some(vb)) => diff_at(&p, va, vb, out),
                    (Some(va), None) => out.push(Change {
                        path: p,
                        kind: ChangeKind::Removed,
                        before: Some(va.clone()),
                        after: None,
                    }),
                    (None, Some(vb)) => out.push(Change {
                        path: p,
                        kind: ChangeKind::Added,
                        before: None,
                        after: Some(vb.clone()),
                    }),
                    (None, None) => {}
                }
            }
        }
        _ if a == b => {}
        _ => out.push(Change {
            path: path.to_owned(),
            kind: ChangeKind::Changed,
            before: Some(a.clone()),
            after: Some(b.clone()),
        }),
    }
}

fn join_dot(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

// --- Model --------------------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Group {
    hosts: BTreeSet<String>,
    children: BTreeSet<String>,
    vars: Vars,
}

#[derive(Debug, Default)]
struct Inventory {
    hosts: BTreeMap<String, Vars>,
    groups: BTreeMap<String, Group>,
}

impl Inventory {
    fn group(&mut self, name: &str) -> &mut Group {
        self.groups.entry(name.to_owned()).or_default()
    }

    fn add_host(&mut self, group: &str, host: &str, vars: Vars, issues: &mut Issues) {
        if !self.hosts.contains_key(host) && self.hosts.len() >= MAX_HOSTS {
            issues.error(
                format!("hosts.{host}"),
                format!("more than {MAX_HOSTS} hosts"),
            );
            return;
        }
        self.hosts.entry(host.to_owned()).or_default().extend(vars);
        self.group(group).hosts.insert(host.to_owned());
    }

    fn add_child(&mut self, parent: &str, child: &str, issues: &mut Issues) {
        if child == "all" {
            issues.error(
                format!("groups.{parent}.children.all"),
                "`all` contains every group and cannot be a child",
            );
            return;
        }
        self.group(child);
        self.group(parent).children.insert(child.to_owned());
    }
}

#[derive(Default)]
struct Issues {
    errors: Vec<InventoryIssue>,
    warnings: Vec<InventoryIssue>,
}

impl Issues {
    fn error(&mut self, at: impl Into<String>, message: impl Into<String>) {
        self.errors.push(InventoryIssue::new(at, message));
    }
    fn warn(&mut self, at: impl Into<String>, message: impl Into<String>) {
        self.warnings.push(InventoryIssue::new(at, message));
    }
}

fn parse(text: &str, format: Format, issues: &mut Issues) -> Option<Inventory> {
    if text.len() > MAX_INVENTORY_BYTES {
        issues.error(
            "inventory",
            format!("larger than {MAX_INVENTORY_BYTES} bytes"),
        );
        return None;
    }
    let mut inv = match format {
        Format::Yaml => parse_yaml(text, issues),
        Format::Ini => Some(parse_ini(text, issues)),
    }?;
    inv.group("all");
    Some(inv)
}

fn finish(
    inv: Option<Inventory>,
    text: &str,
    source: Option<&str>,
    schema: Option<&Value>,
    mut issues: Issues,
) -> Checked {
    let (mut content, hosts) = match inv {
        Some(mut inv) => {
            validate(&mut inv, schema, &mut issues);
            let hosts = host_groups(&inv);
            (content_of(&inv), hosts)
        }
        None => {
            let mut m = Map::new();
            m.insert("hosts".into(), Value::Object(Map::new()));
            m.insert("groups".into(), Value::Object(Map::new()));
            m.insert(
                "unparsed".into(),
                Value::String(sha256_hex(text.as_bytes())),
            );
            (Value::Object(m), Vec::new())
        }
    };
    if let (Some(src), Value::Object(m)) = (source, &mut content) {
        m.insert("source".into(), Value::String(src.to_owned()));
    }
    let hash = content_hash(&content);
    let tidy = |mut v: Vec<InventoryIssue>| {
        v.sort();
        v.dedup();
        v
    };
    Checked {
        content,
        hash,
        hosts,
        errors: tidy(issues.errors),
        warnings: tidy(issues.warnings),
    }
}

fn content_of(inv: &Inventory) -> Value {
    let hosts: Map<String, Value> = inv
        .hosts
        .iter()
        .map(|(h, vars)| {
            let mut o = Map::new();
            o.insert("vars".into(), Value::Object(vars.clone()));
            (h.clone(), Value::Object(o))
        })
        .collect();
    let list = |s: &BTreeSet<String>| Value::Array(s.iter().cloned().map(Value::String).collect());
    let groups: Map<String, Value> = inv
        .groups
        .iter()
        .map(|(g, grp)| {
            let mut o = Map::new();
            o.insert("hosts".into(), list(&grp.hosts));
            o.insert("children".into(), list(&grp.children));
            o.insert("vars".into(), Value::Object(grp.vars.clone()));
            (g.clone(), Value::Object(o))
        })
        .collect();
    let mut m = Map::new();
    m.insert("hosts".into(), Value::Object(hosts));
    m.insert("groups".into(), Value::Object(groups));
    Value::Object(m)
}

// --- YAML ---------------------------------------------------------------------------------------

fn parse_yaml(text: &str, issues: &mut Issues) -> Option<Inventory> {
    let doc: serde_yaml::Value = match serde_yaml::from_str(text) {
        Ok(d) => d,
        Err(e) => {
            issues.error("inventory", format!("not valid YAML: {e}"));
            return None;
        }
    };
    let mut inv = Inventory::default();
    match doc {
        serde_yaml::Value::Null => {}
        serde_yaml::Value::Mapping(m) => {
            for (k, v) in m {
                let Some(name) = key_str(&k) else {
                    issues.error("inventory", "group names must be strings");
                    continue;
                };
                yaml_group(&name, v, &mut inv, issues, 0);
            }
        }
        _ => {
            issues.error("inventory", "expected a mapping of groups");
            return None;
        }
    }
    Some(inv)
}

fn yaml_group(
    name: &str,
    v: serde_yaml::Value,
    inv: &mut Inventory,
    issues: &mut Issues,
    depth: usize,
) {
    inv.group(name);
    if depth > 64 {
        issues.error(format!("groups.{name}"), "groups nested too deeply");
        return;
    }
    let m = match v {
        serde_yaml::Value::Null => return,
        serde_yaml::Value::Mapping(m) => m,
        _ => {
            issues.error(format!("groups.{name}"), "expected hosts, vars or children");
            return;
        }
    };
    for (k, val) in m {
        let key = key_str(&k).unwrap_or_default();
        match key.as_str() {
            "hosts" => match val {
                serde_yaml::Value::Null => {}
                serde_yaml::Value::Mapping(hosts) => {
                    for (hk, hv) in hosts {
                        let Some(pattern) = key_str(&hk) else {
                            issues.error(format!("{name}.hosts"), "host names must be strings");
                            continue;
                        };
                        let vars = match vars_of(hv) {
                            Ok(v) => v,
                            Err(e) => {
                                issues.error(format!("hosts.{pattern}"), e);
                                Vars::new()
                            }
                        };
                        match expand(&pattern) {
                            Ok(names) => {
                                for h in names {
                                    inv.add_host(name, &h, vars.clone(), issues);
                                }
                            }
                            Err(e) => issues.error(format!("hosts.{pattern}"), e),
                        }
                    }
                }
                _ => issues.error(format!("{name}.hosts"), "expected a mapping of hosts"),
            },
            "vars" => match vars_of(val) {
                Ok(vars) => inv.group(name).vars.extend(vars),
                Err(e) => issues.error(format!("groups.{name}.vars"), e),
            },
            "children" => match val {
                serde_yaml::Value::Null => {}
                serde_yaml::Value::Mapping(children) => {
                    for (ck, cv) in children {
                        let Some(child) = key_str(&ck) else {
                            issues.error(format!("{name}.children"), "group names must be strings");
                            continue;
                        };
                        inv.add_child(name, &child, issues);
                        if child != "all" {
                            yaml_group(&child, cv, inv, issues, depth + 1);
                        }
                    }
                }
                _ => issues.error(format!("{name}.children"), "expected a mapping of groups"),
            },
            _ => issues.error(
                format!("{name}.{key}"),
                "unknown key: a group has only hosts, vars and children",
            ),
        }
    }
}

fn key_str(k: &serde_yaml::Value) -> Option<String> {
    match k {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// A vars mapping (or nothing) as JSON.
fn vars_of(v: serde_yaml::Value) -> Result<Vars, String> {
    match v {
        serde_yaml::Value::Null => Ok(Vars::new()),
        serde_yaml::Value::Mapping(_) => match to_json(v)? {
            Value::Object(m) => Ok(m),
            _ => Err("expected a mapping of vars".into()),
        },
        _ => Err("expected a mapping of vars".into()),
    }
}

fn to_json(v: serde_yaml::Value) -> Result<Value, String> {
    Ok(match v {
        serde_yaml::Value::Null => Value::Null,
        serde_yaml::Value::Bool(b) => Value::Bool(b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::from(i)
            } else if let Some(u) = n.as_u64() {
                Value::from(u)
            } else {
                n.as_f64()
                    .and_then(serde_json::Number::from_f64)
                    .map_or(Value::Null, Value::Number)
            }
        }
        serde_yaml::Value::String(s) => Value::String(s),
        serde_yaml::Value::Sequence(items) => {
            Value::Array(items.into_iter().map(to_json).collect::<Result<_, _>>()?)
        }
        serde_yaml::Value::Mapping(m) => {
            let mut out = Map::new();
            for (k, v) in m {
                let key = key_str(&k).ok_or("mapping keys must be strings")?;
                out.insert(key, to_json(v)?);
            }
            Value::Object(out)
        }
        serde_yaml::Value::Tagged(t) => {
            if t.tag == "vault" {
                match t.value {
                    serde_yaml::Value::String(s) => vault(s.trim().to_owned()),
                    _ => return Err("a !vault value must be a string".into()),
                }
            } else if t.tag == "unsafe" {
                to_json(t.value)?
            } else {
                return Err(format!("unsupported tag {}", t.tag));
            }
        }
    })
}

fn vault(ciphertext: String) -> Value {
    let mut m = Map::new();
    m.insert(VAULT_KEY.into(), Value::String(ciphertext));
    Value::Object(m)
}

fn is_vault(v: &Value) -> bool {
    matches!(v, Value::Object(m) if m.len() == 1 && m.get(VAULT_KEY).is_some_and(Value::is_string))
}

// --- INI ----------------------------------------------------------------------------------------

enum Section {
    Hosts(String),
    Vars(String),
    Children(String),
    /// After a malformed header: lines are skipped until the next good one.
    Skip,
}

fn parse_ini(text: &str, issues: &mut Issues) -> Inventory {
    let mut inv = Inventory::default();
    let mut section = Section::Hosts("ungrouped".into());
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (i, raw) in text.lines().enumerate() {
        let at = format!("line {}", i + 1);
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            seen.clear();
            section = match ini_header(line) {
                Ok(s) => {
                    match &s {
                        Section::Hosts(g) | Section::Vars(g) | Section::Children(g) => {
                            inv.group(g);
                        }
                        Section::Skip => {}
                    }
                    s
                }
                Err(e) => {
                    issues.error(at, e);
                    Section::Skip
                }
            };
            continue;
        }
        match &section {
            Section::Skip => {}
            Section::Hosts(g) => {
                let g = g.clone();
                ini_host_line(line, &at, &g, &mut seen, &mut inv, issues);
            }
            Section::Vars(g) => match line.split_once('=') {
                Some((k, v)) => {
                    let v = unquote(v.trim());
                    inv.group(g)
                        .vars
                        .insert(k.trim().to_owned(), Value::String(v));
                }
                None => issues.error(at, "expected key=value"),
            },
            Section::Children(g) => {
                let g = g.clone();
                match tokens(line) {
                    Ok(t) if t.len() == 1 => inv.add_child(&g, &t[0].0, issues),
                    _ => issues.error(at, "expected one group name"),
                }
            }
        }
    }
    inv
}

fn ini_header(line: &str) -> Result<Section, String> {
    let inner = line
        .strip_prefix('[')
        .and_then(|l| l.strip_suffix(']'))
        .ok_or("section header is not closed")?;
    let (name, kind) = match inner.split_once(':') {
        Some((n, k)) => (n, Some(k)),
        None => (inner, None),
    };
    let name = name.to_owned();
    match kind {
        None => Ok(Section::Hosts(name)),
        Some("vars") => Ok(Section::Vars(name)),
        Some("children") => Ok(Section::Children(name)),
        Some(other) => Err(format!(
            "unknown section kind `{other}`: expected vars or children"
        )),
    }
}

fn ini_host_line(
    line: &str,
    at: &str,
    group: &str,
    seen: &mut BTreeSet<String>,
    inv: &mut Inventory,
    issues: &mut Issues,
) {
    let toks = match tokens(line) {
        Ok(t) if !t.is_empty() => t,
        Ok(_) => return,
        Err(e) => {
            issues.error(at, e);
            return;
        }
    };
    let pattern = &toks[0].0;
    let mut vars = Vars::new();
    for (tok, quoted) in &toks[1..] {
        match tok.split_once('=') {
            Some((k, v)) => {
                vars.insert(k.to_owned(), literal(v, *quoted));
            }
            None => {
                issues.error(at, "host vars must be key=value");
                return;
            }
        }
    }
    let names = match expand(pattern) {
        Ok(n) => n,
        Err(e) => {
            issues.error(at, e);
            return;
        }
    };
    for h in names {
        if !seen.insert(h.clone()) {
            issues.error(at, format!("host `{h}` is listed twice in this section"));
            continue;
        }
        inv.add_host(group, &h, vars.clone(), issues);
    }
}

/// Splits a line shell-style, stopping at a `#` that starts a token. Each token comes with
/// whether any of it was quoted.
fn tokens(line: &str) -> Result<Vec<(String, bool)>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut in_token = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_token {
                    out.push((std::mem::take(&mut cur), quoted));
                    quoted = false;
                    in_token = false;
                }
            }
            '#' if !in_token => break,
            '\'' | '"' => {
                in_token = true;
                quoted = true;
                let mut closed = false;
                while let Some(q) = chars.next() {
                    if q == c {
                        closed = true;
                        break;
                    }
                    if q == '\\' && c == '"' {
                        if let Some(e) = chars.next() {
                            cur.push(e);
                        }
                        continue;
                    }
                    cur.push(q);
                }
                if !closed {
                    return Err("unclosed quote".into());
                }
            }
            '\\' => {
                in_token = true;
                if let Some(e) = chars.next() {
                    cur.push(e);
                }
            }
            c => {
                in_token = true;
                cur.push(c);
            }
        }
    }
    if in_token {
        out.push((cur, quoted));
    }
    Ok(out)
}

fn unquote(v: &str) -> String {
    for q in ['"', '\''] {
        if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
            return v[1..v.len() - 1].to_owned();
        }
    }
    v.to_owned()
}

/// A host-line value, typed as Ansible's literal evaluation would: numbers and booleans when
/// unquoted, strings otherwise.
fn literal(v: &str, quoted: bool) -> Value {
    if quoted {
        return Value::String(v.to_owned());
    }
    if let Ok(i) = v.parse::<i64>() {
        return Value::from(i);
    }
    if v.contains('.')
        && let Ok(f) = v.parse::<f64>()
        && let Some(n) = serde_json::Number::from_f64(f)
    {
        return Value::Number(n);
    }
    match v {
        "True" | "true" => Value::Bool(true),
        "False" | "false" => Value::Bool(false),
        _ => Value::String(v.to_owned()),
    }
}

// --- Host ranges --------------------------------------------------------------------------------

/// Expands `www[01:10].example.com` and `db-[a:f]` (with an optional `:stride`).
fn expand(pattern: &str) -> Result<Vec<String>, String> {
    let Some(open) = pattern.find('[') else {
        return Ok(vec![pattern.to_owned()]);
    };
    let close = pattern[open..]
        .find(']')
        .map(|i| open + i)
        .ok_or("unclosed `[` in host range")?;
    let (head, inner, tail) = (
        &pattern[..open],
        &pattern[open + 1..close],
        &pattern[close + 1..],
    );
    let parts: Vec<&str> = inner.split(':').collect();
    let (start, end, stride) = match parts.as_slice() {
        [s, e] => (*s, *e, 1usize),
        [s, e, st] => (*s, *e, st.parse().map_err(|_| "bad range stride")?),
        _ => return Err("a host range is [start:end] or [start:end:stride]".into()),
    };
    if stride == 0 {
        return Err("a range stride must be positive".into());
    }
    let items: Vec<String> = if let (Ok(a), Ok(b)) = (start.parse::<u64>(), end.parse::<u64>()) {
        if a > b {
            return Err("a host range must not run backwards".into());
        }
        if (b - a) / stride as u64 >= MAX_HOSTS as u64 {
            return Err(format!(
                "a host range may expand to at most {MAX_HOSTS} hosts"
            ));
        }
        let width = if start.len() > 1 && start.starts_with('0') {
            start.len()
        } else {
            0
        };
        (a..=b)
            .step_by(stride)
            .map(|n| format!("{n:0width$}"))
            .collect()
    } else {
        let (mut a, mut b) = (start.chars(), end.chars());
        match (a.next(), a.next(), b.next(), b.next()) {
            (Some(x), None, Some(y), None)
                if x.is_ascii_alphabetic() && y.is_ascii_alphabetic() && x <= y =>
            {
                (x..=y).step_by(stride).map(String::from).collect()
            }
            _ => return Err("a host range is numbers or single letters".into()),
        }
    };
    let rest = expand(tail)?;
    if items.len().saturating_mul(rest.len()) > MAX_HOSTS {
        return Err(format!(
            "a host range may expand to at most {MAX_HOSTS} hosts"
        ));
    }
    Ok(items
        .iter()
        .flat_map(|i| rest.iter().map(move |r| format!("{head}{i}{r}")))
        .collect())
}

// --- group_vars and host_vars -------------------------------------------------------------------

#[derive(Clone, Copy)]
enum VarsKind {
    Group,
    Host,
}

impl VarsKind {
    fn dir(self) -> &'static str {
        match self {
            Self::Group => "group_vars",
            Self::Host => "host_vars",
        }
    }
}

fn join_path(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_owned()
    } else {
        format!("{base}/{name}")
    }
}

/// The name a vars file is for: no extension, or `.yml`, `.yaml` or `.json`.
fn vars_stem(file: &str) -> Option<&str> {
    for ext in [".yml", ".yaml", ".json"] {
        if let Some(s) = file.strip_suffix(ext) {
            return Some(s);
        }
    }
    (!file.contains('.')).then_some(file)
}

fn load_vars(
    files: &dyn InventoryFiles,
    base: &str,
    kind: VarsKind,
    inv: &mut Inventory,
    issues: &mut Issues,
) {
    let dir = join_path(base, kind.dir());
    let Some(mut names) = files.list(&dir) else {
        return;
    };
    names.sort();
    for name in names.iter().filter(|n| !n.starts_with('.')) {
        let entry = join_path(&dir, name);
        let rel = format!("{}/{name}", kind.dir());
        // A directory holds several files for one target; a file is for its stem.
        let (target, parts): (&str, Vec<(String, String)>) = match files.list(&entry) {
            Some(mut sub) => {
                sub.sort();
                let parts = sub
                    .iter()
                    .filter(|f| !f.starts_with('.') && vars_stem(f).is_some())
                    .map(|f| (join_path(&entry, f), format!("{rel}/{f}")))
                    .collect();
                (name.as_str(), parts)
            }
            None => match vars_stem(name) {
                Some(stem) => (stem, vec![(entry.clone(), rel.clone())]),
                None => continue,
            },
        };
        let known = match kind {
            VarsKind::Group => inv.groups.contains_key(target),
            VarsKind::Host => inv.hosts.contains_key(target),
        };
        if !known {
            let what = match kind {
                VarsKind::Group => "group",
                VarsKind::Host => "host",
            };
            issues.warn(
                rel,
                format!("vars for `{target}`, which is not a {what} in this inventory"),
            );
            continue;
        }
        for (path, shown) in parts {
            let Some(text) = files.read(&path) else {
                continue;
            };
            if text.trim_start().starts_with("$ANSIBLE_VAULT") {
                // A vault-encrypted file: Keel cannot see inside it, which is the point.
                continue;
            }
            if text.len() > MAX_INVENTORY_BYTES {
                issues.error(shown, format!("larger than {MAX_INVENTORY_BYTES} bytes"));
                continue;
            }
            let vars = serde_yaml::from_str::<serde_yaml::Value>(&text)
                .map_err(|e| format!("not valid YAML: {e}"))
                .and_then(vars_of);
            match vars {
                Ok(v) => match kind {
                    VarsKind::Group => inv.group(target).vars.extend(v),
                    VarsKind::Host => inv.hosts.entry(target.to_owned()).or_default().extend(v),
                },
                Err(e) => issues.error(shown, e),
            }
        }
    }
}

// --- Validation ---------------------------------------------------------------------------------

fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    c.next()
        .is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
        && c.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn is_host_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && !s.starts_with('-')
        && !s.starts_with('.')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':'))
}

const SECRET_WORDS: [&str; 3] = ["password", "token", "secret"];

fn validate(inv: &mut Inventory, schema: Option<&Value>, issues: &mut Issues) {
    // `all` holds every group that has no other parent.
    let parented: BTreeSet<String> = inv
        .groups
        .values()
        .flat_map(|g| g.children.iter().cloned())
        .collect();
    let top: Vec<String> = inv
        .groups
        .keys()
        .filter(|g| *g != "all" && !parented.contains(*g))
        .cloned()
        .collect();
    inv.group("all").children.extend(top);

    for (name, g) in &inv.groups {
        if !is_ident(name) {
            issues.error(
                format!("groups.{name}"),
                "group names are letters, digits and underscores, not starting with a digit",
            );
        }
        check_var_names(&g.vars, &format!("groups.{name}.vars"), issues);
        secrets(&g.vars, &format!("groups.{name}.vars"), issues);
    }
    let mut lower: BTreeMap<String, &String> = BTreeMap::new();
    for (name, vars) in &inv.hosts {
        if !is_host_name(name) {
            issues.error(
                format!("hosts.{name}"),
                "host names are letters, digits, `.`, `_`, `-` and `:`, not starting with `-` or `.`",
            );
        }
        if let Some(first) = lower.insert(name.to_ascii_lowercase(), name) {
            issues.error(
                format!("hosts.{name}"),
                format!("the same host as `{first}`: host names are not case-sensitive"),
            );
        }
        if inv.groups.contains_key(name) {
            issues.warn(
                format!("hosts.{name}"),
                "a group has the same name as this host",
            );
        }
        check_var_names(vars, &format!("hosts.{name}.vars"), issues);
        secrets(vars, &format!("hosts.{name}.vars"), issues);
    }
    let cyclic = cycles(inv, issues);

    if let Some(schema) = schema {
        let before = issues.errors.len();
        check_schema_keywords(schema, "schema", issues);
        if issues.errors.len() == before && !cyclic {
            for host in inv.hosts.keys() {
                let vars = Value::Object(effective_vars(inv, host));
                check_value(&vars, schema, &format!("hosts.{host}.vars"), issues);
            }
        }
    }
}

fn check_var_names(vars: &Vars, at: &str, issues: &mut Issues) {
    for k in vars.keys() {
        if !is_ident(k) {
            issues.error(
                format!("{at}.{k}"),
                "var names are letters, digits and underscores, not starting with a digit",
            );
        }
    }
}

fn secrets(vars: &Vars, at: &str, issues: &mut Issues) {
    for (k, v) in vars {
        let path = format!("{at}.{k}");
        match v {
            Value::String(s) => {
                let lk = k.to_ascii_lowercase();
                if SECRET_WORDS.iter().any(|w| lk.contains(w)) && !s.is_empty() && !s.contains("{{")
                {
                    issues.warn(
                        path,
                        "looks like a secret in plain text: encrypt it with ansible-vault or use a credential",
                    );
                }
            }
            Value::Object(m) if !is_vault(v) => secrets(m, &path, issues),
            _ => {}
        }
    }
}

/// Reports each group cycle once. True when there is any.
fn cycles(inv: &Inventory, issues: &mut Issues) -> bool {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Open,
        Done,
    }
    fn visit(
        g: &str,
        inv: &Inventory,
        marks: &mut BTreeMap<String, Mark>,
        stack: &mut Vec<String>,
        issues: &mut Issues,
        found: &mut bool,
    ) {
        marks.insert(g.to_owned(), Mark::Open);
        stack.push(g.to_owned());
        if let Some(group) = inv.groups.get(g) {
            for child in &group.children {
                match marks.get(child.as_str()) {
                    Some(Mark::Open) => {
                        let from = stack.iter().position(|s| s == child).unwrap_or(0);
                        let mut path: Vec<&str> =
                            stack[from..].iter().map(String::as_str).collect();
                        path.push(child);
                        issues.error(
                            format!("groups.{child}"),
                            format!("group cycle: {}", path.join(" -> ")),
                        );
                        *found = true;
                    }
                    Some(Mark::Done) => {}
                    None => visit(child, inv, marks, stack, issues, found),
                }
            }
        }
        stack.pop();
        marks.insert(g.to_owned(), Mark::Done);
    }
    let mut marks = BTreeMap::new();
    let mut found = false;
    // Groups under `all` first is not needed: every group is a root of its own walk.
    for g in inv.groups.keys().filter(|g| *g != "all") {
        if !marks.contains_key(g.as_str()) {
            visit(g, inv, &mut marks, &mut Vec::new(), issues, &mut found);
        }
    }
    found
}

fn parents(inv: &Inventory) -> BTreeMap<&str, Vec<&str>> {
    let mut p: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (name, g) in &inv.groups {
        for c in &g.children {
            p.entry(c.as_str()).or_default().push(name.as_str());
        }
    }
    p
}

/// Every group `host` is in, with ancestors, `all` included.
fn groups_of<'a>(
    inv: &'a Inventory,
    parents: &BTreeMap<&str, Vec<&'a str>>,
    host: &str,
) -> BTreeSet<&'a str> {
    let mut out: BTreeSet<&str> = BTreeSet::new();
    let mut todo: Vec<&str> = inv
        .groups
        .iter()
        .filter(|(_, g)| g.hosts.contains(host))
        .map(|(n, _)| n.as_str())
        .collect();
    while let Some(g) = todo.pop() {
        if out.insert(g)
            && let Some(ps) = parents.get(g)
        {
            todo.extend(ps.iter().copied());
        }
    }
    out.insert("all");
    out
}

fn host_groups(inv: &Inventory) -> Vec<HostGroups> {
    let parents = parents(inv);
    inv.hosts
        .keys()
        .map(|h| {
            let mut groups: Vec<String> = groups_of(inv, &parents, h)
                .into_iter()
                .filter(|g| *g != "all")
                .map(str::to_owned)
                .collect();
            if groups.is_empty() {
                groups.push("ungrouped".into());
            }
            HostGroups {
                host: h.clone(),
                groups,
            }
        })
        .collect()
}

/// A host's vars as Ansible resolves them: `all`, then groups by depth (then name), then the
/// host's own.
fn effective_vars(inv: &Inventory, host: &str) -> Vars {
    let parents = parents(inv);
    let mut depth: BTreeMap<&str, usize> = BTreeMap::new();
    fn depth_of<'a>(
        g: &'a str,
        parents: &BTreeMap<&str, Vec<&'a str>>,
        memo: &mut BTreeMap<&'a str, usize>,
        guard: usize,
    ) -> usize {
        if g == "all" || guard > 64 {
            return 0;
        }
        if let Some(d) = memo.get(g) {
            return *d;
        }
        let d = 1 + parents.get(g).map_or(0, |ps| {
            ps.iter()
                .map(|p| depth_of(p, parents, memo, guard + 1))
                .max()
                .unwrap_or(0)
        });
        memo.insert(g, d);
        d
    }
    let mut groups: Vec<(usize, &str)> = groups_of(inv, &parents, host)
        .into_iter()
        .map(|g| (depth_of(g, &parents, &mut depth, 0), g))
        .collect();
    groups.sort();
    let mut out = Vars::new();
    for (_, g) in groups {
        if let Some(group) = inv.groups.get(g) {
            out.extend(group.vars.clone());
        }
    }
    if let Some(v) = inv.hosts.get(host) {
        out.extend(v.clone());
    }
    out
}

// --- JSON Schema (a subset) ---------------------------------------------------------------------

/// Keywords Keel checks. Any other keyword is refused rather than silently ignored.
const SCHEMA_KEYWORDS: &[&str] = &[
    "type",
    "properties",
    "required",
    "additionalProperties",
    "enum",
    "const",
    "minimum",
    "maximum",
    "minLength",
    "maxLength",
    "items",
    "minItems",
    "maxItems",
];

/// Keywords that only annotate.
const SCHEMA_ANNOTATIONS: &[&str] = &[
    "$schema",
    "$id",
    "$comment",
    "title",
    "description",
    "default",
    "examples",
];

fn check_schema_keywords(s: &Value, at: &str, issues: &mut Issues) {
    let m = match s {
        Value::Bool(_) => return,
        Value::Object(m) => m,
        _ => {
            issues.error(at, "a schema is an object or a boolean");
            return;
        }
    };
    for (k, v) in m {
        let here = format!("{at}.{k}");
        if SCHEMA_ANNOTATIONS.contains(&k.as_str()) {
            continue;
        }
        if !SCHEMA_KEYWORDS.contains(&k.as_str()) {
            issues.error(here, format!("unsupported schema keyword `{k}`"));
            continue;
        }
        match k.as_str() {
            "properties" => match v {
                Value::Object(props) => {
                    for (p, sub) in props {
                        check_schema_keywords(sub, &format!("{here}.{p}"), issues);
                    }
                }
                _ => issues.error(here, "expected an object of schemas"),
            },
            "items" | "additionalProperties" => check_schema_keywords(v, &here, issues),
            _ => {}
        }
    }
}

fn type_matches(v: &Value, t: &str) -> bool {
    match t {
        "object" => v.is_object() && !is_vault(v),
        "array" => v.is_array(),
        "string" => v.is_string() || is_vault(v),
        "integer" => {
            v.as_i64().is_some()
                || v.as_u64().is_some()
                || v.as_f64().is_some_and(|f| f.fract() == 0.0)
        }
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        _ => false,
    }
}

fn check_value(v: &Value, s: &Value, at: &str, issues: &mut Issues) {
    let m = match s {
        Value::Bool(true) => return,
        Value::Bool(false) => {
            issues.error(at, "not allowed by the schema");
            return;
        }
        Value::Object(m) => m,
        _ => return,
    };
    if let Some(t) = m.get("type") {
        let types: Vec<&str> = match t {
            Value::String(s) => vec![s.as_str()],
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        if !types.iter().any(|t| type_matches(v, t)) {
            issues.error(at, format!("expected {}", types.join(" or ")));
            return;
        }
    }
    let vaulted = is_vault(v);
    if !vaulted {
        if let Some(Value::Array(allowed)) = m.get("enum")
            && !allowed.contains(v)
        {
            let shown: Vec<String> = allowed.iter().map(Value::to_string).collect();
            issues.error(at, format!("must be one of {}", shown.join(", ")));
        }
        if let Some(c) = m.get("const")
            && c != v
        {
            issues.error(at, format!("must be {c}"));
        }
    }
    if let Some(n) = v.as_f64() {
        if let Some(min) = m.get("minimum").and_then(Value::as_f64)
            && n < min
        {
            issues.error(at, format!("must be at least {min}"));
        }
        if let Some(max) = m.get("maximum").and_then(Value::as_f64)
            && n > max
        {
            issues.error(at, format!("must be at most {max}"));
        }
    }
    if let Some(s) = v.as_str() {
        let len = s.chars().count() as u64;
        if let Some(min) = m.get("minLength").and_then(Value::as_u64)
            && len < min
        {
            issues.error(at, format!("must be at least {min} characters"));
        }
        if let Some(max) = m.get("maxLength").and_then(Value::as_u64)
            && len > max
        {
            issues.error(at, format!("must be at most {max} characters"));
        }
    }
    if let Value::Object(obj) = v
        && !vaulted
    {
        if let Some(Value::Array(req)) = m.get("required") {
            for r in req.iter().filter_map(Value::as_str) {
                if !obj.contains_key(r) {
                    issues.error(at, format!("missing required `{r}`"));
                }
            }
        }
        let props = m.get("properties").and_then(Value::as_object);
        for (k, val) in obj {
            let here = format!("{at}.{k}");
            match props.and_then(|p| p.get(k)) {
                Some(sub) => check_value(val, sub, &here, issues),
                None => {
                    if let Some(extra) = m.get("additionalProperties") {
                        check_value(val, extra, &here, issues);
                    }
                }
            }
        }
    }
    if let Value::Array(items) = v {
        let n = items.len() as u64;
        if let Some(min) = m.get("minItems").and_then(Value::as_u64)
            && n < min
        {
            issues.error(at, format!("must have at least {min} items"));
        }
        if let Some(max) = m.get("maxItems").and_then(Value::as_u64)
            && n > max
        {
            issues.error(at, format!("must have at most {max} items"));
        }
        if let Some(sub) = m.get("items") {
            for (i, item) in items.iter().enumerate() {
                check_value(item, sub, &format!("{at}.{i}"), issues);
            }
        }
    }
}

// --- Hashing ------------------------------------------------------------------------------------

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest.iter() {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// JSON with object keys sorted, whatever order the map keeps.
fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String((*k).clone()).to_string());
                out.push(':');
                write_canonical(&m[*k], out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, item) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}
