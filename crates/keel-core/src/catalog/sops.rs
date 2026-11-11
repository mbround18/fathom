//! Sops awareness without values (R5).
//!
//! `.sops.yaml` creation rules are read for their recipients. Encrypted files are read for their
//! `sops` metadata and the *names* of their keys. Every value — ciphertext, the MAC, the
//! encrypted data keys — is dropped as it is reached, so nothing later can leak one.

use std::collections::BTreeSet;

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::{Repo, get, parse_yaml, scalar};

const RULES_FILES: [&str; 2] = [".sops.yaml", ".sops.yml"];
const SOPS_SUFFIXES: [&str; 4] = [".sops.yaml", ".sops.yml", ".sops.json", ".sops.env"];
/// Longest `path_regex` compiled.
const MAX_PATTERN: usize = 4096;
/// Compiled program size per `path_regex`.
const REGEX_SIZE: usize = 1 << 20;
/// Key names kept per file.
const MAX_KEYS: usize = 10_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Secrets {
    #[serde(default)]
    pub rules_file: Option<String>,
    pub rules: Vec<Rule>,
    pub files: Vec<SecretFile>,
}

/// One `.sops.yaml` creation rule: who a matching file is encrypted to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub index: usize,
    #[serde(default)]
    pub path_regex: Option<String>,
    /// Age recipients, from `age` and every `key_groups[].age`.
    pub age: Vec<String>,
    /// PGP fingerprints, likewise.
    pub pgp: Vec<String>,
}

/// One sops file: its metadata and key names. No value of any kind.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretFile {
    pub path: String,
    /// The first creation rule that matches it.
    #[serde(default)]
    pub rule: Option<usize>,
    pub age: Vec<String>,
    pub pgp: Vec<String>,
    #[serde(default)]
    pub lastmodified: Option<String>,
    #[serde(default)]
    pub encrypted_regex: Option<String>,
    #[serde(default)]
    pub encrypted_suffix: Option<String>,
    #[serde(default)]
    pub unencrypted_suffix: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    /// Dotted key paths.
    pub keys: Vec<String>,
    /// How its age recipients differ from its rule's; `None` when they match or there is no rule.
    #[serde(default)]
    pub drift: Option<Drift>,
    #[serde(default)]
    pub problem: Option<String>,
    /// Manifest jobs that read it.
    pub read_by: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Drift {
    /// Named by the rule, absent from the file: `sops updatekeys` would add them.
    pub missing: Vec<String>,
    /// In the file, no longer named by the rule.
    pub extra: Vec<String>,
}

enum Matcher {
    All,
    Re(Regex),
    Never,
}

/// Recipients written as a comma-separated string or a list (of such strings).
fn recipients(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Sequence(items)) => items.iter().flat_map(|i| recipients(Some(i))).collect(),
        Some(other) => scalar(other)
            .map(|s| {
                s.split(',')
                    .map(str::trim)
                    .filter(|r| !r.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        None => Vec::new(),
    }
}

fn sorted(v: impl IntoIterator<Item = String>) -> Vec<String> {
    v.into_iter().collect::<BTreeSet<_>>().into_iter().collect()
}

fn rules(repo: &Repo<'_>, secrets: &mut Secrets) -> Vec<Matcher> {
    let Some(file) = repo.first(&RULES_FILES) else {
        return Vec::new();
    };
    secrets.rules_file = Some(file.to_owned());
    let Some(doc) = repo.yaml(file) else {
        return Vec::new();
    };
    let mut matchers = Vec::new();
    for (index, r) in get(&doc, "creation_rules")
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let groups = get(r, "key_groups").and_then(Value::as_sequence);
        let mut age = recipients(get(r, "age"));
        let mut pgp = recipients(get(r, "pgp"));
        for g in groups.into_iter().flatten() {
            age.extend(recipients(get(g, "age")));
            pgp.extend(recipients(get(g, "pgp")));
        }
        let path_regex = get(r, "path_regex").and_then(scalar);
        let matcher = match &path_regex {
            None => Matcher::All,
            Some(p) if p.len() > MAX_PATTERN => {
                repo.problem(
                    file,
                    None,
                    format!("creation_rules[{index}].path_regex is too long"),
                );
                Matcher::Never
            }
            Some(p) => match RegexBuilder::new(p)
                .size_limit(REGEX_SIZE)
                .dfa_size_limit(REGEX_SIZE)
                .build()
            {
                Ok(re) => Matcher::Re(re),
                Err(_) => {
                    repo.problem(
                        file,
                        None,
                        format!("creation_rules[{index}].path_regex does not compile"),
                    );
                    Matcher::Never
                }
            },
        };
        matchers.push(matcher);
        secrets.rules.push(Rule {
            index,
            path_regex,
            age: sorted(age),
            pgp: sorted(pgp),
        });
    }
    matchers
}

/// Rules, then every sops file: named `*.sops.*`, or matched by a rule's `path_regex`.
pub(crate) fn scan(repo: &Repo<'_>) -> Secrets {
    let mut secrets = Secrets::default();
    let matchers = rules(repo, &mut secrets);
    for path in &repo.paths {
        if RULES_FILES.contains(&path.as_str()) {
            continue;
        }
        let rule = matchers.iter().position(|m| match m {
            Matcher::All => true,
            Matcher::Re(re) => re.is_match(path),
            Matcher::Never => false,
        });
        let by_regex = rule.is_some_and(|i| matches!(matchers[i], Matcher::Re(_)));
        let named = SOPS_SUFFIXES.iter().any(|s| path.ends_with(s));
        if !(by_regex || named) {
            continue;
        }
        let mut f = read_file(repo, path);
        f.rule = rule;
        if let Some(i) = rule
            && f.problem.is_none()
        {
            let want: BTreeSet<&String> = secrets.rules[i].age.iter().collect();
            let have: BTreeSet<&String> = f.age.iter().collect();
            let missing: Vec<String> = want.difference(&have).map(|s| (*s).clone()).collect();
            let extra: Vec<String> = have.difference(&want).map(|s| (*s).clone()).collect();
            if !missing.is_empty() || !extra.is_empty() {
                f.drift = Some(Drift { missing, extra });
            }
        }
        secrets.files.push(f);
    }
    secrets
}

fn read_file(repo: &Repo<'_>, path: &str) -> SecretFile {
    let mut f = SecretFile {
        path: path.to_owned(),
        ..SecretFile::default()
    };
    let Some(text) = repo.text(path) else {
        f.problem = Some("not readable within the catalog limits".into());
        return f;
    };
    let mut keys = BTreeSet::new();
    let has_meta = if path.ends_with(".env") {
        dotenv(&text, &mut f, &mut keys)
    } else {
        match parse_yaml(&text) {
            Ok(doc) => structured(&doc, &mut f, &mut keys),
            Err(line) => {
                f.problem = Some(match line {
                    Some(l) => format!("not valid YAML or JSON (line {l})"),
                    None => "not valid YAML or JSON".into(),
                });
                return f;
            }
        }
    };
    if has_meta {
        f.keys = keys.into_iter().collect();
    } else {
        f.problem = Some("no sops metadata: the file is not encrypted".into());
    }
    f
}

/// A YAML or JSON document. Returns whether it has a `sops` mapping.
fn structured(doc: &Value, f: &mut SecretFile, keys: &mut BTreeSet<String>) -> bool {
    let mut has_meta = false;
    for (k, v) in doc.as_mapping().into_iter().flatten() {
        let Some(k) = scalar(k) else { continue };
        if k == "sops" {
            has_meta = v.is_mapping();
            metadata(v, f);
        } else {
            key_paths(&k, v, keys);
        }
    }
    has_meta
}

fn key_paths(prefix: &str, v: &Value, out: &mut BTreeSet<String>) {
    if out.len() >= MAX_KEYS {
        return;
    }
    match v {
        Value::Mapping(m) if !m.is_empty() => {
            for (k, child) in m {
                if let Some(k) = scalar(k) {
                    key_paths(&format!("{prefix}.{k}"), child, out);
                }
            }
        }
        // The value itself is dropped here, whatever it is.
        _ => {
            out.insert(prefix.to_owned());
        }
    }
}

/// Keeps recipients, fingerprints, and the non-secret settings. Drops `mac` and every `enc`.
fn metadata(sops: &Value, f: &mut SecretFile) {
    let mut age = Vec::new();
    let mut pgp = Vec::new();
    let mut collect = |m: &Value| {
        for a in get(m, "age")
            .and_then(Value::as_sequence)
            .into_iter()
            .flatten()
        {
            age.extend(get(a, "recipient").and_then(scalar));
        }
        for p in get(m, "pgp")
            .and_then(Value::as_sequence)
            .into_iter()
            .flatten()
        {
            pgp.extend(get(p, "fp").and_then(scalar));
        }
    };
    collect(sops);
    for g in get(sops, "key_groups")
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
    {
        collect(g);
    }
    f.age = sorted(age);
    f.pgp = sorted(pgp);
    let s = |k: &str| get(sops, k).and_then(scalar);
    f.lastmodified = s("lastmodified");
    f.encrypted_regex = s("encrypted_regex");
    f.encrypted_suffix = s("encrypted_suffix");
    f.unencrypted_suffix = s("unencrypted_suffix");
    f.version = s("version");
}

/// A dotenv file: key names, and metadata from the `sops_*` keys. Returns whether it has any.
fn dotenv(text: &str, f: &mut SecretFile, keys: &mut BTreeSet<String>) -> bool {
    let mut has_meta = false;
    let mut age = Vec::new();
    let mut pgp = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let key = key.strip_prefix("export ").unwrap_or(key).trim();
        let Some(meta) = key.strip_prefix("sops_") else {
            if keys.len() < MAX_KEYS {
                keys.insert(key.to_owned());
            }
            continue;
        };
        has_meta = true;
        let value = value.trim().trim_matches(['"', '\'']).to_owned();
        match meta {
            "lastmodified" => f.lastmodified = Some(value),
            "version" => f.version = Some(value),
            "encrypted_regex" => f.encrypted_regex = Some(value),
            "encrypted_suffix" => f.encrypted_suffix = Some(value),
            "unencrypted_suffix" => f.unencrypted_suffix = Some(value),
            m if m.starts_with("age__list_") && m.ends_with("__map_recipient") => age.push(value),
            m if m.starts_with("pgp__list_") && m.ends_with("__map_fp") => pgp.push(value),
            // sops_mac, sops_age__list_N__map_enc, …: dropped.
            _ => {}
        }
    }
    f.age = sorted(age);
    f.pgp = sorted(pgp);
    has_meta
}

#[cfg(test)]
mod tests {
    use super::super::{Limits, MemFiles, build};
    use super::*;

    const K1: &str = "age1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq";
    const K2: &str = "age1zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz";

    fn secrets(files: MemFiles) -> (Secrets, Vec<super::super::Problem>) {
        let c = build(&files, None, Limits::default());
        (c.secrets, c.problems)
    }

    #[test]
    fn rules_accept_strings_lists_and_key_groups() {
        let (s, problems) = secrets(MemFiles::new().with(
            ".sops.yaml",
            format!(
                "creation_rules:\n  - path_regex: a/.*\n    age: '{K1}, {K2}'\n    pgp: ABC\n  \
                 - path_regex: '('\n    age: [{K1}]\n  \
                 - key_groups:\n      - age: [{K2}]\n        pgp: [DEF]\n"
            ),
        ));
        assert_eq!(s.rules.len(), 3);
        assert_eq!(s.rules[0].age, [K1, K2]);
        assert_eq!(s.rules[0].pgp, ["ABC"]);
        assert_eq!(s.rules[2].age, [K2]);
        assert_eq!(s.rules[2].path_regex, None);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].message.contains("creation_rules[1]"));
    }

    #[test]
    fn first_matching_rule_and_drift() {
        let enc = format!(
            "a: ENC[AES256_GCM,data:QUFB,iv:x,tag:y,type:str]\nsops:\n  age:\n    - recipient: {K2}\n      \
             enc: SECRETDATAKEY\n  mac: ENC[AES256_GCM,data:TUFD,type:str]\n"
        );
        let (s, _) = secrets(
            MemFiles::new()
                .with(
                    ".sops.yaml",
                    format!("creation_rules:\n  - path_regex: ^x/\n    age: {K1}\n  - age: {K2}\n"),
                )
                .with("x/secret.yaml", &enc)
                .with("y/other.sops.yaml", &enc)
                .with("y/plain.yaml", "a: 1\n"),
        );
        let paths: Vec<&str> = s.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            ["x/secret.yaml", "y/other.sops.yaml"],
            "the catch-all rule makes no candidates"
        );
        let x = &s.files[0];
        assert_eq!(x.rule, Some(0));
        let d = x.drift.as_ref().unwrap();
        assert_eq!(
            (d.missing.as_slice(), d.extra.as_slice()),
            ([K1.to_owned()].as_slice(), [K2.to_owned()].as_slice())
        );
        assert_eq!(s.files[1].rule, Some(1));
        assert_eq!(s.files[1].drift, None);
        let json = serde_json::to_string(&s).unwrap();
        for leak in ["ENC[", "QUFB", "TUFD", "SECRETDATAKEY"] {
            assert!(!json.contains(leak), "{leak}");
        }
    }

    #[test]
    fn json_and_dotenv_keep_names_and_metadata_only() {
        let (s, _) = secrets(
            MemFiles::new()
                .with(
                    "a.sops.json",
                    format!(
                        r#"{{"db": {{"password": "ENC[AES256_GCM,data:SkpTT04=]"}}, "list": ["ENC[x]"],
                           "sops": {{"age": [{{"recipient": "{K1}", "enc": "DATAKEY"}}],
                                     "mac": "ENC[MACVALUE]", "version": "3.9.0"}}}}"#
                    ),
                )
                .with(
                    "b.sops.env",
                    format!(
                        "TOKEN=ENC[AES256_GCM,data:RU5WVkFM]\nexport OTHER=ENC[y]\n\
                         sops_age__list_0__map_recipient={K2}\nsops_age__list_0__map_enc=DATAKEY2\n\
                         sops_mac=ENC[MAC2]\nsops_lastmodified=2026-01-01T00:00:00Z\nsops_version=3.9.0\n"
                    ),
                ),
        );
        assert_eq!(s.files[0].keys, ["db.password", "list"]);
        assert_eq!(s.files[0].age, [K1]);
        assert_eq!(s.files[0].version.as_deref(), Some("3.9.0"));
        assert_eq!(s.files[1].keys, ["OTHER", "TOKEN"]);
        assert_eq!(s.files[1].age, [K2]);
        assert_eq!(
            s.files[1].lastmodified.as_deref(),
            Some("2026-01-01T00:00:00Z")
        );
        let json = serde_json::to_string(&s).unwrap();
        for leak in ["ENC[", "SkpTT04", "RU5WVkFM", "DATAKEY", "MAC"] {
            assert!(!json.contains(leak), "{leak}");
        }
    }

    #[test]
    fn unencrypted_and_unparsable_files_are_problems_without_content() {
        let (s, _) = secrets(
            MemFiles::new()
                .with("a.sops.yaml", "password: hunter2\n")
                .with("b.sops.yaml", "password: [hunter3\n"),
        );
        assert!(
            s.files
                .iter()
                .all(|f| f.problem.is_some() && f.keys.is_empty())
        );
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("hunter"));
    }
}
