//! 058 T004: the reference systemd units the base profile vendors.
//!
//! - `keel-executor@.service` runs as its own account, `keel-exec`, never as the human
//!   `automation` group (058 research R3).
//! - `keel-runner.service` does not restart after exit 78 (`EX_CONFIG`), so a host Keel rejected
//!   stops instead of looping (055 contracts/join.md).

use std::path::PathBuf;

/// `(section, key, value)` for every assignment in a unit file, in order. Comments and blank
/// lines are skipped; the unit files use no line continuations.
fn parse(name: &str) -> Vec<(String, String, String)> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("systemd")
        .join(name);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut section = String::new();
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s.to_owned();
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .unwrap_or_else(|| panic!("{name}: not an assignment: {line}"));
        out.push((
            section.clone(),
            key.trim().to_owned(),
            value.trim().to_owned(),
        ));
    }
    out
}

/// Every value `key` takes in the `[Service]` section.
fn service_values(name: &str, key: &str) -> Vec<String> {
    parse(name)
        .into_iter()
        .filter(|(s, k, _)| s == "Service" && k == key)
        .map(|(_, _, v)| v)
        .collect()
}

#[test]
fn executor_runs_as_keel_exec() {
    assert_eq!(
        service_values("keel-executor@.service", "User"),
        ["keel-exec"]
    );
    assert_eq!(
        service_values("keel-executor@.service", "Group"),
        ["keel-exec"]
    );
}

#[test]
fn runner_does_not_restart_on_config_error() {
    assert_eq!(
        service_values("keel-runner.service", "RestartPreventExitStatus"),
        ["78"]
    );
}
