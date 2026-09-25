//! SC-009: an egui-era `~/.fathom/` directory keeps loading under the Tauri-era backend.
//!
//! The fixture is written into a temp `$HOME` (ports.json, metrics history, integrations,
//! credentials, config.json without any `ui` key), then every load path the new UI relies on is
//! exercised. `$HOME` is process-global, so the test holds `HOME_LOCK`.

use fathom::backend::persistence::{
    load_all_ports, load_credentials, load_integrations, load_metrics_history, load_ports,
    load_ui_config,
};
use fathom::sections::network::PortForwardStatus;
use serde_json::json;
use std::sync::Mutex;

static HOME_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn egui_era_fathom_dir_loads_under_the_new_backend() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: guarded by HOME_LOCK; no other test in this binary touches $HOME.
    unsafe { std::env::set_var("HOME", home.path()) };
    let dir = home.path().join(".fathom");
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: &str, v: serde_json::Value| {
        std::fs::write(dir.join(name), serde_json::to_string_pretty(&v).unwrap()).unwrap();
    };

    // egui-era ports.json predates `auto_start`, so the field must default.
    write(
        "ports.json",
        json!([{
            "id": "pf-1", "namespace": "default", "pod": "web-0",
            "local_port": 8080, "remote_port": 80, "container": null, "status": "Closed"
        }]),
    );
    write("metrics.json", json!({ "default/web-0": [{ "cpu": 1.5 }] }));
    write(
        "integrations.json",
        json!({ "prometheus": { "namespace": "monitoring", "service": "prometheus", "port": 9090 } }),
    );
    write("credentials.json", json!({ "encrypted": "opaque-blob" }));
    // No `ui` key: the egui build never wrote one. `mcp_http` is the pre-015 spelling.
    write(
        "config.json",
        json!({
            "auto_start": { "mcp_http": true },
            "navigation": { "expanded_groups": ["Network", "Storage"] }
        }),
    );

    let ports = load_all_ports().expect("ports load");
    assert_eq!(ports.len(), 1);
    assert_eq!(ports[0].local_port, 8080);
    assert!(!ports[0].auto_start, "missing auto_start defaults to false");
    assert_eq!(ports[0].status, PortForwardStatus::Closed);
    assert_eq!(load_ports().unwrap()[0]["pod"], json!("web-0"));

    assert_eq!(
        load_integrations().unwrap()["prometheus"]["port"],
        json!(9090)
    );
    assert_eq!(
        load_credentials().unwrap()["encrypted"],
        json!("opaque-blob")
    );
    assert!(load_metrics_history().is_ok());

    let ui = load_ui_config();
    assert_eq!(ui.ui, None, "no ui key means the UI applies its defaults");
    assert_eq!(ui.expanded_groups, vec!["Network", "Storage"]);
    assert!(ui.mcp_stdio_enabled, "mcp_http is still honoured");
}
