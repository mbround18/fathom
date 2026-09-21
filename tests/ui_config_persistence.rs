//! `config.json → ui` persistence (FR-022, data-model.md §2).
//!
//! Each test points `$HOME` at a fresh temp dir, so `~/.fathom/config.json` is isolated.
//! `$HOME` is process-global, so the tests serialize on `HOME_LOCK`.

use fathom::backend::persistence::{
    DEFAULT_EXPANDED_GROUPS, LoadedUiConfig, load_ui_config, save_mcp_auto_start, save_ui_config,
};
use fathom::protocol::{Density, ThemeAccent, UiConfig};
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

    fn write_config(&self, value: &Value) {
        std::fs::create_dir_all(self.config_path().parent().unwrap()).unwrap();
        std::fs::write(
            self.config_path(),
            serde_json::to_string_pretty(value).unwrap(),
        )
        .unwrap();
    }

    fn read_config(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(self.config_path()).unwrap()).unwrap()
    }
}

fn custom_ui() -> UiConfig {
    let mut ui = UiConfig::default();
    ui.theme.accent = ThemeAccent::Magenta;
    ui.theme.glitch = true;
    ui.density.override_density = Some(Density::Compact);
    ui
}

fn groups(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn missing_config_yields_defaults() {
    let home = TempHome::new();
    let loaded = load_ui_config();
    assert_eq!(loaded, LoadedUiConfig::default());
    assert_eq!(loaded.expanded_groups, groups(&DEFAULT_EXPANDED_GROUPS));
    assert!(
        !home.config_path().exists(),
        "load must not create config.json"
    );
}

#[test]
fn save_then_load_round_trips() {
    let _home = TempHome::new();
    let ui = custom_ui();
    save_ui_config(&ui, &groups(&["Network", "Storage"])).unwrap();

    let loaded = load_ui_config();
    assert_eq!(loaded.ui, Some(ui));
    assert_eq!(loaded.expanded_groups, groups(&["Network", "Storage"]));
    assert!(!loaded.mcp_auto_start);
}

#[test]
fn save_preserves_unrelated_keys() {
    let home = TempHome::new();
    let original = json!({
        "auto_start": { "mcp_http": true, "something_else": [1, 2, 3] },
        "navigation": { "expanded_groups": ["Overview"], "pinned": ["pods"] },
        "custom_top_level": { "nested": { "keep": "me" } },
        "ui_invalid": { "schema": 99 }
    });
    home.write_config(&original);

    save_ui_config(&custom_ui(), &groups(&["Workloads"])).unwrap();
    let after = home.read_config();

    assert_eq!(after["auto_start"], original["auto_start"]);
    assert_eq!(after["custom_top_level"], original["custom_top_level"]);
    assert_eq!(
        after["ui_invalid"], original["ui_invalid"],
        "ui_invalid is left alone"
    );
    assert_eq!(after["navigation"]["pinned"], json!(["pods"]));
    assert_eq!(after["navigation"]["expanded_groups"], json!(["Workloads"]));
    assert_eq!(after["ui"], serde_json::to_value(custom_ui()).unwrap());

    let loaded = load_ui_config();
    assert!(
        loaded.mcp_auto_start,
        "auto_start.mcp_http is read from its existing key"
    );
}

#[test]
fn mcp_auto_start_keeps_ui() {
    let _home = TempHome::new();
    save_ui_config(&custom_ui(), &groups(&["Overview"])).unwrap();
    save_mcp_auto_start(true).unwrap();

    let loaded = load_ui_config();
    assert!(loaded.mcp_auto_start);
    assert_eq!(loaded.ui, Some(custom_ui()));
}

#[test]
fn invalid_ui_is_moved_to_ui_invalid() {
    let home = TempHome::new();
    let bad = json!({ "schema": 1, "theme": { "mode": "neon" } });
    home.write_config(&json!({
        "ui": bad,
        "ui_invalid": { "old": true },
        "navigation": { "expanded_groups": ["Config"] },
        "auto_start": { "mcp_http": true }
    }));

    let loaded = load_ui_config();
    assert_eq!(loaded.ui, None);
    assert_eq!(loaded.expanded_groups, groups(&["Config"]));
    assert!(loaded.mcp_auto_start);

    let after = home.read_config();
    assert!(after.get("ui").is_none());
    assert_eq!(
        after["ui_invalid"], bad,
        "newest invalid blob replaces the older one"
    );
    assert_eq!(after["navigation"]["expanded_groups"], json!(["Config"]));
    assert_eq!(after["auto_start"]["mcp_http"], json!(true));
}

#[test]
fn unsupported_schema_is_invalid() {
    let home = TempHome::new();
    let mut v = serde_json::to_value(custom_ui()).unwrap();
    v["schema"] = json!(2);
    home.write_config(&json!({ "ui": v.clone() }));

    assert_eq!(load_ui_config().ui, None);
    assert_eq!(home.read_config()["ui_invalid"], v);
}

#[test]
fn unparseable_config_is_never_clobbered() {
    let home = TempHome::new();
    std::fs::create_dir_all(home.config_path().parent().unwrap()).unwrap();
    std::fs::write(home.config_path(), "{ not json").unwrap();

    assert_eq!(load_ui_config(), LoadedUiConfig::default());
    assert!(save_ui_config(&custom_ui(), &[]).is_err());
    assert_eq!(
        std::fs::read_to_string(home.config_path()).unwrap(),
        "{ not json"
    );
}

#[test]
fn invalid_ui_is_refused_on_save() {
    let home = TempHome::new();
    let mut ui = UiConfig::default();
    ui.schema = 7;
    assert!(save_ui_config(&ui, &[]).is_err());
    assert!(!home.config_path().exists());
}

#[test]
fn null_navigation_is_replaced() {
    let home = TempHome::new();
    home.write_config(&json!({ "navigation": null }));
    save_ui_config(&UiConfig::default(), &groups(&["Overview"])).unwrap();
    assert_eq!(
        home.read_config()["navigation"],
        json!({ "expanded_groups": ["Overview"] })
    );
}
