//! Turns a run's `Secrets` into files in its private directory (contracts/runner.md
//! "Guarantees"): each written `0600`, removed with the directory, never logged.
//!
//! - `vault_password`: `vault-<n>`, passed as `--vault-id <name>@<file>`.
//! - `env`: `NAME=value` lines, set in `ansible-playbook`'s environment. Names that would steer
//!   Ansible, Python or the dynamic linker (`ANSIBLE_*`, `PYTHON*`, `LD_*`, `PATH`, `HOME`,
//!   `KEEL_*`) are refused.
//! - `env_var` (spec 068): one value, set as `env_name` in `ansible-playbook`'s environment.
//!   The name is checked again with `keel_core::env_names` ([`check_env_items`] also checks it
//!   against the job's `env:` map, and refuses `env_refused` before anything runs).
//! - `secret_vars`: a JSON object whose keys become playbook variables. They are written to
//!   `secret_vars.yaml` with every string tagged `!unsafe`, so Ansible never templates them: a
//!   value of `{{ lookup('pipe','id') }}` reaches the playbook as that literal text.
//!
//! Any other kind (including the server-only Git credentials) fails the run. Error messages
//! name credentials, never values.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::Path;

use keel_exec::frames::SecretItem;
use serde_json::Value;

pub const SECRET_VARS_FILE: &str = "secret_vars.yaml";

/// What the secrets became.
#[derive(Default)]
pub struct Materialized {
    /// `--vault-id <name>@<file>` pairs, in order.
    pub vault_args: Vec<OsString>,
    /// Environment for `ansible-playbook`. Values are secret.
    pub env: Vec<(String, String)>,
}

impl std::fmt::Debug for Materialized {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.env.iter().map(|(k, _)| k.as_str()).collect();
        f.debug_struct("Materialized")
            .field("vault_args", &self.vault_args)
            .field("env", &names)
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("credential {0:?}: {1}")]
    Bad(String, &'static str),
    #[error("credential {0:?} has kind {1:?}, which a host does not accept")]
    Kind(String, String),
    #[error("credential {0:?} is sent twice")]
    Duplicate(String),
    #[error("secret variable {0:?} is set by two credentials")]
    DuplicateVar(String),
    #[error("writing secrets: {0}")]
    Io(String),
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

// One rule with the manifest's `jobs.<j>.env` (spec 068 T011).
use keel_core::env_names::{is_reserved as reserved_env_name, is_shell_name as valid_env_name};

/// The kind of a single-value environment credential (spec 068 `jobs.<j>.env`).
pub const ENV_VAR_KIND: &str = "env_var";

/// The credentials the job's `env:` map names that were not sent as `env_var` items for that
/// variable: the run cannot set the variable, so it fails like any credential not sent.
pub fn missing_env(items: &[SecretItem], job_env: &BTreeMap<String, String>) -> Vec<String> {
    job_env
        .iter()
        .filter(|(var, cred)| {
            !items.iter().any(|i| {
                i.kind == ENV_VAR_KIND && &i.name == *cred && i.env_name.as_ref() == Some(*var)
            })
        })
        .map(|(_, cred)| cred.clone())
        .collect()
}

/// Checks every `env_var` item against `keel_core::env_names` and the job's `env:` map at the
/// target (environment name → credential name): the item must carry an `env_name`, the name must
/// be a valid, unreserved shell name, and the job must map that name to this credential. Keel
/// cannot set a variable the job does not ask for. The error names the credential and the
/// variable, never the value.
pub fn check_env_items(
    items: &[SecretItem],
    job_env: &BTreeMap<String, String>,
) -> Result<(), String> {
    for item in items.iter().filter(|i| i.kind == ENV_VAR_KIND) {
        let Some(env_name) = item.env_name.as_deref() else {
            return Err(format!("credential {:?} has no env_name", item.name));
        };
        if let Err(e) = keel_core::env_names::check(env_name) {
            return Err(format!(
                "credential {:?}: environment name {env_name:?} is refused ({})",
                item.name,
                e.code()
            ));
        }
        if job_env.get(env_name) != Some(&item.name) {
            return Err(format!(
                "credential {:?}: the job's env does not set {env_name:?} from it",
                item.name
            ));
        }
    }
    Ok(())
}

fn valid_var_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b) if b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !name.starts_with("ansible_")
        && !name.starts_with("keel_")
}

/// Writes the secrets into `dir` and returns how to pass them. `secret_vars.yaml` is always
/// written (an empty mapping when there are none).
pub fn materialize(dir: &Path, items: &[SecretItem]) -> Result<Materialized, SecretError> {
    let io = |e: std::io::Error| SecretError::Io(e.kind().to_string());
    let mut out = Materialized::default();
    let mut names = BTreeSet::new();
    let mut env_names = BTreeSet::new();
    let mut vars: BTreeMap<String, Value> = BTreeMap::new();
    for (n, item) in items.iter().enumerate() {
        let name = &item.name;
        if !valid_name(name) {
            return Err(SecretError::Bad(
                name.clone(),
                "the name has characters outside [A-Za-z0-9_.-]",
            ));
        }
        if !names.insert(name.clone()) {
            return Err(SecretError::Duplicate(name.clone()));
        }
        if item.env_name.is_some() && item.kind != ENV_VAR_KIND {
            return Err(SecretError::Bad(
                name.clone(),
                "env_name is only for kind env_var",
            ));
        }
        let value = item.value.expose();
        match item.kind.as_str() {
            "vault_password" => {
                let file = dir.join(format!("vault-{n}"));
                crate::write_private_new(&file, value.as_bytes()).map_err(io)?;
                let mut arg = OsString::from(format!("{name}@"));
                arg.push(file.as_os_str());
                out.vault_args.push("--vault-id".into());
                out.vault_args.push(arg);
            }
            "env" => {
                for line in value.lines() {
                    if line.trim().is_empty() {
                        continue;
                    }
                    let Some((k, v)) = line.split_once('=') else {
                        return Err(SecretError::Bad(name.clone(), "a line is not NAME=value"));
                    };
                    if !valid_env_name(k) {
                        return Err(SecretError::Bad(name.clone(), "an invalid variable name"));
                    }
                    if reserved_env_name(k) {
                        return Err(SecretError::Bad(name.clone(), "a reserved variable name"));
                    }
                    if v.contains('\0') {
                        return Err(SecretError::Bad(name.clone(), "a value contains NUL"));
                    }
                    if !env_names.insert(k.to_owned()) {
                        return Err(SecretError::Bad(name.clone(), "a variable is set twice"));
                    }
                    out.env.push((k.to_owned(), v.to_owned()));
                }
            }
            ENV_VAR_KIND => {
                let Some(k) = item.env_name.as_deref() else {
                    return Err(SecretError::Bad(
                        name.clone(),
                        "an env_var without env_name",
                    ));
                };
                if keel_core::env_names::check(k).is_err() {
                    return Err(SecretError::Bad(
                        name.clone(),
                        "an invalid or reserved variable name",
                    ));
                }
                if value.contains('\0') {
                    return Err(SecretError::Bad(name.clone(), "the value contains NUL"));
                }
                if !env_names.insert(k.to_owned()) {
                    return Err(SecretError::Bad(name.clone(), "a variable is set twice"));
                }
                out.env.push((k.to_owned(), value.to_owned()));
            }
            "secret_vars" => {
                let Ok(Value::Object(map)) = serde_json::from_str::<Value>(value) else {
                    return Err(SecretError::Bad(name.clone(), "not a JSON object"));
                };
                for (k, v) in map {
                    if !valid_var_name(&k) {
                        return Err(SecretError::Bad(
                            name.clone(),
                            "a variable name is invalid or reserved (ansible_*, keel_*)",
                        ));
                    }
                    if vars.contains_key(&k) {
                        return Err(SecretError::DuplicateVar(k));
                    }
                    vars.insert(k, v);
                }
            }
            other => return Err(SecretError::Kind(name.clone(), other.to_owned())),
        }
    }
    let mut yaml = String::new();
    write_mapping(&mut yaml, vars.iter());
    yaml.push('\n');
    crate::write_private_new(&dir.join(SECRET_VARS_FILE), yaml.as_bytes()).map_err(io)?;
    Ok(out)
}

/// A YAML flow mapping whose keys and strings are all `!unsafe` double-quoted scalars.
fn write_mapping<'a>(out: &mut String, entries: impl Iterator<Item = (&'a String, &'a Value)>) {
    out.push('{');
    for (i, (k, v)) in entries.enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        write_unsafe_str(out, k);
        out.push_str(": ");
        write_value(out, v);
    }
    out.push('}');
}

fn write_value(out: &mut String, v: &Value) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => write_unsafe_str(out, s),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_value(out, item);
            }
            out.push(']');
        }
        Value::Object(map) => write_mapping(out, map.iter()),
    }
}

/// `!unsafe "<escaped>"`: printable ASCII as is (except `"` and `\`), everything else escaped.
fn write_unsafe_str(out: &mut String, s: &str) {
    out.push_str("!unsafe \"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            ' '..='~' => out.push(c),
            c if (c as u32) <= 0xFFFF => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push_str(&format!("\\U{:08X}", c as u32)),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use keel_exec::frames::SecretString;

    fn item(name: &str, kind: &str, value: &str) -> SecretItem {
        SecretItem {
            name: name.to_owned(),
            kind: kind.to_owned(),
            value: SecretString::new(value),
            env_name: None,
        }
    }

    fn env_item(name: &str, env_name: &str, value: &str) -> SecretItem {
        SecretItem {
            env_name: Some(env_name.to_owned()),
            ..item(name, ENV_VAR_KIND, value)
        }
    }

    #[test]
    fn an_env_var_item_is_set_under_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let m = materialize(
            dir.path(),
            &[env_item("cf-token", "CLOUDFLARE_API_TOKEN", "s3cret value")],
        )
        .unwrap();
        assert_eq!(
            m.env,
            vec![("CLOUDFLARE_API_TOKEN".to_owned(), "s3cret value".to_owned())]
        );
        // Debug names the variable, never the value.
        assert!(!format!("{m:?}").contains("s3cret"));
    }

    #[test]
    fn env_var_items_are_refused_without_a_good_name() {
        for bad in [
            env_item("a", "LD_PRELOAD", "x"),
            env_item("a", "PATH", "x"),
            env_item("a", "lower", "x"),
            env_item("a", "A", "nul\0"),
            item("a", ENV_VAR_KIND, "x"),
            SecretItem {
                env_name: Some("TOKEN".to_owned()),
                ..item("a", "vault_password", "x")
            },
        ] {
            let dir = tempfile::tempdir().unwrap();
            let e = materialize(dir.path(), &[bad]).unwrap_err().to_string();
            assert!(!e.contains("nul"), "{e}");
        }
        // Two items for one variable.
        let dir = tempfile::tempdir().unwrap();
        assert!(
            materialize(
                dir.path(),
                &[env_item("a", "TOKEN", "1"), env_item("b", "TOKEN", "2")]
            )
            .is_err()
        );
    }

    #[test]
    fn env_var_items_must_be_what_the_job_asks_for() {
        let env = BTreeMap::from([("CF_TOKEN".to_owned(), "cf-token".to_owned())]);
        assert!(check_env_items(&[env_item("cf-token", "CF_TOKEN", "v")], &env).is_ok());
        // Other kinds are not looked at.
        assert!(check_env_items(&[item("x", "vault_password", "v")], &env).is_ok());
        for bad in [
            env_item("cf-token", "OTHER", "v"),
            env_item("other", "CF_TOKEN", "v"),
            env_item("cf-token", "ANSIBLE_CONFIG", "v"),
            item("cf-token", ENV_VAR_KIND, "v"),
        ] {
            let e = check_env_items(&[bad], &env).unwrap_err();
            assert!(!e.contains("\"v\""), "{e}");
        }
    }

    #[test]
    fn missing_env_names_what_was_not_sent() {
        let env = BTreeMap::from([
            ("A_TOKEN".to_owned(), "a".to_owned()),
            ("B_TOKEN".to_owned(), "b".to_owned()),
        ]);
        assert_eq!(
            missing_env(&[env_item("a", "A_TOKEN", "v")], &env),
            vec!["b".to_owned()]
        );
        assert!(
            missing_env(
                &[env_item("a", "A_TOKEN", "v"), env_item("b", "B_TOKEN", "v")],
                &env
            )
            .is_empty()
        );
    }

    #[test]
    fn secret_vars_are_tagged_unsafe_and_escaped() {
        let dir = tempfile::tempdir().unwrap();
        let v = r#"{"token":"{{ lookup('pipe','id') }}","n":3,"nested":{"k":["a\"b","é\n"]}}"#;
        materialize(dir.path(), &[item("app", "secret_vars", v)]).unwrap();
        let yaml = std::fs::read_to_string(dir.path().join(SECRET_VARS_FILE)).unwrap();
        assert_eq!(
            yaml,
            "{!unsafe \"n\": 3, !unsafe \"nested\": {!unsafe \"k\": [!unsafe \"a\\\"b\", \
             !unsafe \"\\u00E9\\u000A\"]}, !unsafe \"token\": !unsafe \"{{ lookup('pipe','id') }}\"}\n"
        );
    }

    #[test]
    fn reserved_names_and_unknown_kinds_fail() {
        let dir = tempfile::tempdir().unwrap();
        for bad in [
            item("e", "env", "LD_PRELOAD=/tmp/x.so"),
            item("e", "env", "ANSIBLE_CONFIG=/tmp/a"),
            item("e", "env", "PATH=/tmp"),
            item("e", "env", "lower=1"),
            item("v", "secret_vars", r#"{"ansible_connection":"ssh"}"#),
            item("v", "secret_vars", "[1]"),
            item("g", "git_ssh_key", "k"),
            item("bad name", "env", "A=1"),
        ] {
            let sub = tempfile::tempdir_in(dir.path()).unwrap();
            assert!(materialize(sub.path(), &[bad]).is_err());
        }
        let sub = tempfile::tempdir_in(dir.path()).unwrap();
        let twice = [
            item("a", "secret_vars", r#"{"x":1}"#),
            item("b", "secret_vars", r#"{"x":2}"#),
        ];
        assert!(matches!(
            materialize(sub.path(), &twice),
            Err(SecretError::DuplicateVar(_))
        ));
    }

    #[test]
    fn errors_and_debug_never_show_values() {
        let dir = tempfile::tempdir().unwrap();
        let m = materialize(dir.path(), &[item("e", "env", "API_TOKEN=hunter2")]).unwrap();
        assert!(!format!("{m:?}").contains("hunter2"));
        let sub = tempfile::tempdir_in(dir.path()).unwrap();
        let err = materialize(sub.path(), &[item("e", "env", "hunter2")]).unwrap_err();
        assert!(!err.to_string().contains("hunter2"));
    }
}
