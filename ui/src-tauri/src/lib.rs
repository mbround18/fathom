//! Fathom desktop shell: hosts the React UI in a Tauri webview and bridges it to the
//! existing backend thread (contracts/ipc.md). The backend still owns its tokio runtime
//! and talks over crossbeam channels; the shell-independent pump lives in
//! [`fathom::bridge`].

pub mod bridge;

/// Re-exported for existing users of `fathom_desktop::pump`.
pub use fathom::bridge::pump;

use bridge::{BridgeState, EventHub};
use clap::Parser;
use crossbeam_channel::{Sender, unbounded};
use fathom::bridge::{BackendLink, spawn_backend, spawn_pump};
use fathom::config::Args;
use fathom::protocol::{BackendCmd, BackendEventEnvelope, CmdRequest};
use fathom::startup::{init_tracing, install_crypto_provider};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How long shutdown waits for the backend thread before exiting anyway. `shutdown_backend`
/// returns as soon as the thread finishes, so the long bound only matters when a 037 node-shell
/// debug pod is being deleted (the backend bounds that delete at 4s, `APP_EXIT_BOUND`).
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(6);

/// The Ctrl+C handlers cannot observe the backend thread, so they wait a fixed short time.
const CTRL_C_GRACE: Duration = Duration::from_millis(500);

/// Entry point for desktop (via `main.rs`) and mobile builds.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    run_with_args(Args::parse());
}

/// Start the backend thread and event pump, then run the Tauri app until it exits.
pub fn run_with_args(args: Args) {
    install_crypto_provider();
    init_tracing();

    let (gui_tx, gui_rx) = unbounded::<BackendEventEnvelope>();
    let (cmd_tx, cmd_rx) = unbounded::<CmdRequest>();
    let backend_thread = spawn_backend(cmd_rx, gui_tx, args);

    let ctrlc_tx = cmd_tx.clone();
    if let Err(e) = ctrlc::set_handler(move || {
        tracing::info!("Received Ctrl+C, shutting down...");
        let _ = ctrlc_tx.send(BackendCmd::Shutdown.into());
        std::thread::sleep(CTRL_C_GRACE);
        std::process::exit(0);
    }) {
        tracing::warn!("Failed to install Ctrl+C handler: {}", e);
    }

    let hub: Arc<EventHub> = Arc::new(EventHub::new(pump::BUFFER_CAP));
    let link = BackendLink::new(cmd_tx.clone());
    spawn_pump(gui_rx, Arc::clone(&hub), &link, |_| {});

    let state = BridgeState { link, hub };
    let backend_thread = Mutex::new(Some(backend_thread));

    let app = tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            bridge::send_cmd,
            bridge::subscribe_events
        ])
        .build(tauri::generate_context!())
        .expect("error while building the Fathom desktop app");

    app.run(move |_app, event| {
        // Window close (last window) and app.exit() both end here.
        if let tauri::RunEvent::Exit = event {
            let handle = backend_thread
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            shutdown_backend(&cmd_tx, handle);
        }
    });
}

/// Headless `--only-mcp` mode: backend (and its MCP server) only, no webview.
pub fn run_only_mcp(args: Args) {
    install_crypto_provider();
    init_tracing();
    tracing::info!("Running in headless MCP-only mode");

    let (gui_tx, gui_rx) = unbounded::<BackendEventEnvelope>();
    let (cmd_tx, cmd_rx) = unbounded::<CmdRequest>();

    ctrlc::set_handler(move || {
        tracing::info!("Received Ctrl+C, shutting down...");
        let _ = cmd_tx.send(BackendCmd::Shutdown.into());
        std::thread::sleep(CTRL_C_GRACE);
        std::process::exit(0);
    })
    .expect("Error setting Ctrl+C handler");

    let backend_thread = spawn_backend(cmd_rx, gui_tx, args);
    // Nobody renders events; drain them so the unbounded channel doesn't grow.
    while gui_rx.recv().is_ok() {}
    let _ = backend_thread.join();
}

/// Send `Shutdown` and give the backend up to [`SHUTDOWN_GRACE`] to finish.
fn shutdown_backend(cmd_tx: &Sender<CmdRequest>, backend_thread: Option<JoinHandle<()>>) {
    let _ = cmd_tx.send(BackendCmd::Shutdown.into());
    let Some(handle) = backend_thread else {
        return;
    };
    let deadline = Instant::now() + SHUTDOWN_GRACE;
    while !handle.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if handle.is_finished() {
        let _ = handle.join();
        tracing::info!("Backend thread exited cleanly");
    } else {
        tracing::warn!(
            "Backend thread still running after {:?}; exiting anyway",
            SHUTDOWN_GRACE
        );
    }
}
