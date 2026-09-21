//! Process-level setup shared by every Fathom binary (egui `fathom`, Tauri `fathom-desktop`).

use tracing_subscriber::EnvFilter;

/// Install the rustls crypto provider (required for rustls 0.23+).
///
/// Safe to call more than once; later calls are no-ops.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Initialize `tracing` output.
///
/// Prefers `FATHOM_LOG` but falls back to the conventional `RUST_LOG`.
/// Example: `FATHOM_LOG=fathom=debug,kube=info`
pub fn init_tracing() {
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
