//! The `keel-relay` CLI contract: `key` prints exactly two stdout lines, and an invalid config
//! exits 78 (`EX_CONFIG`) for both `check-config` and `serve`.

use std::path::Path;
use std::process::{Command, Output};

fn relay(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_keel-relay"))
        .args(args)
        .env_remove("KEEL_RELAY_CONFIG")
        .env_remove("KEEL_RELAY_KEY")
        .output()
        .unwrap()
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn key_prints_exactly_the_public_line_and_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("host_ed25519");
    let first = relay(&["key", "--key", s(&key)]);
    assert!(first.status.success(), "{first:?}");
    let stdout = String::from_utf8(first.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "{stdout:?}");
    assert!(stdout.ends_with('\n'));
    assert!(lines[0].starts_with("ssh-ed25519 "), "{stdout}");
    assert!(lines[1].starts_with("SHA256:"), "{stdout}");

    let again = relay(&["key", "--key", s(&key)]);
    assert_eq!(String::from_utf8(again.stdout).unwrap(), stdout, "same key");
}

#[test]
fn an_invalid_config_exits_78() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("relay.toml");
    let key = dir.path().join("host_ed25519");

    // Missing file, unknown key, then a hub_keys file that is empty.
    let hub_keys = dir.path().join("hub_keys");
    std::fs::write(&hub_keys, "").unwrap();
    for text in [
        None,
        Some("hub_keys = \"/x\"\nbogus = 1\n".to_owned()),
        Some(format!("hub_keys = \"{}\"\n", s(&hub_keys))),
    ] {
        if let Some(text) = &text {
            std::fs::write(&config, text).unwrap();
        }
        for args in [
            vec!["check-config", "--config", s(&config)],
            vec!["serve", "--config", s(&config), "--key", s(&key)],
        ] {
            let out = relay(&args);
            assert_eq!(out.status.code(), Some(78), "{args:?} {text:?} {out:?}");
            assert!(out.stdout.is_empty(), "logs go to stderr");
        }
    }
}

#[test]
fn a_valid_config_checks_ok() {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("host_ed25519");
    let out = relay(&["key", "--key", s(&key)]);
    let public = String::from_utf8(out.stdout).unwrap();
    let hub_keys = dir.path().join("hub_keys");
    std::fs::write(&hub_keys, public.lines().next().unwrap()).unwrap();
    let config = dir.path().join("relay.toml");
    std::fs::write(&config, format!("hub_keys = \"{}\"\n", s(&hub_keys))).unwrap();
    let out = relay(&["check-config", "--config", s(&config)]);
    assert!(out.status.success(), "{out:?}");
    assert!(out.stdout.is_empty());
}
