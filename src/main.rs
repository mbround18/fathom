use app::FathomApp;
use crossbeam_channel::unbounded;
use std::thread;
use tracing_subscriber::EnvFilter;

mod app;
mod backend;
mod protocol;
use protocol::{BackendCmd, BackendEventEnvelope};

fn main() -> eframe::Result<()> {
    init_tracing();

    // Channels (UI thread never awaits; backend owns tokio runtime)
    let (gui_tx, gui_rx) = unbounded::<BackendEventEnvelope>();
    let (backend_tx, backend_rx) = unbounded::<BackendCmd>();

    thread::spawn(move || {
        backend::init(backend_rx, gui_tx);
    });

    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "Fathom K8s Dashboard",
        native_options,
        Box::new(|_cc| Ok(Box::new(FathomApp::new(gui_rx, backend_tx)))),
    )
}

fn init_tracing() {
    // Prefer `FATHOM_LOG` but fall back to the conventional `RUST_LOG`.
    // Example: `FATHOM_LOG=fathom=debug,kube=info`
    let filter = std::env::var("FATHOM_LOG")
        .ok()
        .or_else(|| std::env::var("RUST_LOG").ok())
        .unwrap_or_else(|| "info".to_owned());

    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .with_target(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .compact()
        .try_init();
}
