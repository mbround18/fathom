//! `config.json -> oidc` persistence (033 T007): additive key, defaults on invalid, the
//! original never clobbered. `$HOME` is process-global, so tests serialize on `HOME_LOCK`.

use fathom::backend::persistence::{load_oidc_config, save_oidc_config};
use fathom::protocol::{ContextAuthConfig, ContextAuthMode, OidcConfig, OidcIssuerConfig};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

static HOME_LOCK: Mutex<()> = Mutex::new(());

struct TempHome {
    dir: tempfile::TempDir,
    _guard: MutexGuard<'static, ()>,
}

impl TempHome {
    fn new() -> Self {
        let guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        // SAFETY: every test that touches `$HOME` holds `HOME_LOCK`.
        unsafe { std::env::set_var("HOME", dir.path()) };
        Self { dir, _guard: guard }
    }
    fn config_path(&self) -> PathBuf {
        self.dir.path().join(".fathom").join("config.json")
    }
    fn write_raw(&self, text: &str) {
        std::fs::create_dir_all(self.config_path().parent().unwrap()).unwrap();
        std::fs::write(self.config_path(), text).unwrap();
    }
    fn write_config(&self, value: &Value) {
        self.write_raw(&serde_json::to_string_pretty(value).unwrap());
    }
    fn read_config(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(self.config_path()).unwrap()).unwrap()
    }
}

fn issuer(id: &str) -> OidcIssuerConfig {
    serde_json::from_value(json!({
        "id": id, "issuer_url": "https://auth.localtest.me/auth/v1/", "client_id": "fathom-desktop"
    }))
    .unwrap()
}

fn sample() -> OidcConfig {
    OidcConfig {
        issuers: vec![issuer("rauthy")],
        contexts: vec![ContextAuthConfig {
            context_name: "kind".into(),
            mode: ContextAuthMode::Oidc,
            issuer_id: Some("rauthy".into()),
        }],
        ..OidcConfig::default()
    }
}

#[test]
fn missing_config_yields_defaults_and_writes_nothing() {
    let home = TempHome::new();
    assert_eq!(load_oidc_config(), OidcConfig::default());
    assert!(!home.config_path().exists());
}

#[test]
fn save_then_load_round_trips_and_preserves_other_keys() {
    let home = TempHome::new();
    home.write_config(&json!({"auto_start": {"mcp_stdio": true}, "ui": {"schema": 1}}));
    save_oidc_config(&sample()).unwrap();
    assert_eq!(load_oidc_config(), sample());
    let on_disk = home.read_config();
    assert_eq!(on_disk["auto_start"], json!({"mcp_stdio": true}));
    assert_eq!(on_disk["ui"], json!({"schema": 1}));
    assert!(on_disk.get("oidc").is_some());
}

#[test]
fn pre_033_config_without_oidc_key_is_untouched_by_load() {
    let home = TempHome::new();
    let original = json!({"navigation": {"expanded_groups": ["a"]}});
    home.write_config(&original);
    assert_eq!(load_oidc_config(), OidcConfig::default());
    assert_eq!(home.read_config(), original);
}

#[test]
fn invalid_oidc_is_moved_aside_and_defaults_used() {
    let home = TempHome::new();
    let bad = json!({"issuers": [{"id": "Bad Id", "issuer_url": "https://x", "client_id": "c"}]});
    home.write_config(&json!({"oidc": bad.clone(), "other": 1}));
    assert_eq!(load_oidc_config(), OidcConfig::default());
    let on_disk = home.read_config();
    assert!(on_disk.get("oidc").is_none());
    assert_eq!(on_disk["oidc_invalid"], bad, "original preserved verbatim");
    assert_eq!(on_disk["other"], json!(1));
}

#[test]
fn wrong_shape_is_moved_aside() {
    let home = TempHome::new();
    home.write_config(&json!({"oidc": "nonsense"}));
    assert_eq!(load_oidc_config(), OidcConfig::default());
    assert_eq!(home.read_config()["oidc_invalid"], json!("nonsense"));
}

#[test]
fn unparseable_config_is_never_clobbered() {
    let home = TempHome::new();
    home.write_raw("{ not json");
    assert_eq!(load_oidc_config(), OidcConfig::default());
    assert!(save_oidc_config(&sample()).is_err());
    assert_eq!(
        std::fs::read_to_string(home.config_path()).unwrap(),
        "{ not json"
    );
}

#[test]
fn invalid_config_is_refused_on_save() {
    let home = TempHome::new();
    home.write_config(&json!({"keep": true}));
    let mut bad = sample();
    bad.issuers.push(issuer("rauthy"));
    assert!(save_oidc_config(&bad).is_err());
    assert_eq!(home.read_config(), json!({"keep": true}));
}

/// SC-006: a pre-033 `~/.fathom/` (config, ports, integrations) loads unchanged through the
/// 033 code paths, and nothing is written, so the files stay byte-identical.
#[test]
fn pre_033_fathom_dir_loads_unchanged_and_is_not_rewritten() {
    let home = TempHome::new();
    let dir = home.config_path().parent().unwrap().to_path_buf();
    let config = json!({
        "auto_start": {"mcp_stdio": true},
        "ui": {"schema": 1},
        "navigation": {"expanded_groups": ["workloads"]}
    });
    home.write_config(&config);
    std::fs::write(dir.join("ports.json"), "[]").unwrap();
    std::fs::write(dir.join("integrations.json"), r#"{"prometheus":{}}"#).unwrap();
    let files = ["config.json", "ports.json", "integrations.json"];
    let snapshot = || -> Vec<Vec<u8>> {
        files
            .iter()
            .map(|f| std::fs::read(dir.join(f)).unwrap())
            .collect()
    };
    let before = snapshot();

    // Kubeconfig-only: no OIDC issuers, no context opted in.
    let oidc = load_oidc_config();
    assert_eq!(oidc, OidcConfig::default());
    assert!(oidc.issuers.is_empty() && oidc.contexts.is_empty());
    assert_eq!(fathom::backend::persistence::load_config().unwrap(), config);
    assert!(fathom::backend::persistence::load_ports().is_ok());
    assert!(fathom::backend::persistence::load_integrations().is_ok());

    assert_eq!(before, snapshot());
}
