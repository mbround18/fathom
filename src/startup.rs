//! Process-level setup shared by every Fathom binary (egui `fathom`, Tauri `fathom-desktop`).

use tracing_subscriber::EnvFilter;

/// Install the rustls crypto provider (required for rustls 0.23+).
///
/// Safe to call more than once; later calls are no-ops.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Log every panic through `tracing` (location and payload) before the previous hook runs.
///
/// A panic inside a backend command handler is caught and reported to the UI
/// (`backend::run_cmd`), but the only record of *where* it happened is this log line: the
/// default hook writes to stderr, which a desktop shell started from a launcher has nowhere to
/// show. Installed once per process; later calls are no-ops.
pub fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let payload = info.payload();
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "non-string panic payload".to_owned());
            let location = info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                .unwrap_or_else(|| "unknown location".to_owned());
            tracing::error!(%location, "panic: {message}");
            previous(info);
        }));
    });
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
    install_panic_hook();
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
