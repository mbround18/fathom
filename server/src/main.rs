//! `fathom-server`: the hosted shell binary (spec 031, T027).
//! Fail closed: invalid configuration exits 2, failed OIDC discovery exits 3, and in both cases
//! no listener is ever bound.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use fathom::startup::{init_tracing, install_crypto_provider};
use fathom_server::config::{EXIT_DISCOVERY_FAILED, HostedConfig};
use fathom_server::host::HostedTenantHost;
use fathom_server::http::{AppState, router};
use fathom_server::oidc::OidcClient;
use fathom_server::session::SessionStore;

#[tokio::main]
async fn main() -> ExitCode {
    install_crypto_provider();
    init_tracing();

    let cfg = match HostedConfig::from_env() {
        Ok(c) => Arc::new(c),
        Err(e) => {
            tracing::error!("invalid configuration: {e}");
            return ExitCode::from(e.exit_code() as u8);
        }
    };
    let oidc = match OidcClient::discover(&cfg).await {
        Ok(c) => Arc::new(c),
        Err(e) => {
            tracing::error!("{e}");
            return ExitCode::from(EXIT_DISCOVERY_FAILED as u8);
        }
    };
    tracing::info!("OIDC discovery loaded for {}", oidc.issuer());

    let sessions = SessionStore::new(cfg.session_config());
    let host = HostedTenantHost::new(cfg.clone(), oidc.clone(), sessions.clone());
    let app = AppState::with_sessions(cfg.clone(), oidc, host, sessions);
    app.sessions.spawn_sweeper(Duration::from_secs(30));

    let listener = match tokio::net::TcpListener::bind(&cfg.listen).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("cannot bind {}: {e}", cfg.listen);
            return ExitCode::from(1);
        }
    };
    tracing::info!("fathom-server listening on {}", cfg.listen);

    let shutdown = app.shutdown.clone();
    let served = axum::serve(listener, router(app))
        .with_graceful_shutdown(async move {
            wait_for_signal().await;
            tracing::info!("shutting down");
            let _ = shutdown.send(true);
        })
        .await;
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("server error: {e}");
            ExitCode::from(1)
        }
    }
}

async fn wait_for_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = term.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
