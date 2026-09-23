//! Process-level setup shared by every Fathom binary (egui `fathom`, Tauri `fathom-desktop`).

use tracing_subscriber::EnvFilter;

/// Install the rustls crypto provider (required for rustls 0.23+).
///
/// Safe to call more than once; later calls are no-ops.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Initialize `tracing` output on **stderr**.
///
/// Prefers `FATHOM_LOG` but falls back to the conventional `RUST_LOG`.
/// Example: `FATHOM_LOG=fathom=debug,kube=info`
///
/// Stderr, not stdout, because `fathom --only-mcp` (015-mcp-agent-exposure) hands this
/// process's stdout to an external MCP client as a JSON-RPC channel: a single log line written
/// there is a framing error the client cannot recover from. Every other Fathom binary is
/// indifferent to which stream its logs land on, so one unconditional choice avoids a
/// "logs corrupt the protocol" footgun that depends on argument parsing order.
pub fn init_tracing() {
    let filter = std::env::var("FATHOM_LOG")
        .ok()
        .or_else(|| std::env::var("RUST_LOG").ok())
        .unwrap_or_else(|| "info".to_owned());

    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(EnvFilter::new(filter))
        .with_target(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .compact()
        .try_init();
}
