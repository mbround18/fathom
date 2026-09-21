fn main() {
    // Generates `allow-send-cmd` / `allow-subscribe-events` (and `deny-*`) permissions,
    // referenced from `capabilities/default.json`.
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&["send_cmd", "subscribe_events"]),
        ),
    )
    .expect("failed to run tauri-build");
}
