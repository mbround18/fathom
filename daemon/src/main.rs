//! fathom-daemon: optional in-cluster read-only daemon (spec 036).

use anyhow::{Context, Result};
use fathom::protocol::v1::{DaemonRetention, DaemonScope, DaemonScopeMode};
use fathom_daemon::authz::{KubeReviewer, ScopeResolver};
use fathom_daemon::config::DaemonConfig;
use fathom_daemon::derive::Coalescer;
use fathom_daemon::http::{AppState, HttpConfig, api_router, health_router};
use fathom_daemon::hub::{Hub, SyncTracker};
use fathom_daemon::metrics_ring::{
    DEFAULT_MAX_BYTES, DEFAULT_SAMPLES_PER_SERIES, MetricsAvailability, MetricsRing, run_sampler,
};
use fathom_daemon::tls;
use fathom_daemon::watchers::{Ctx, Snapshots, WATCHED_KINDS, spawn_all, sync_keys};
use k8s_openapi::api::core::v1::ConfigMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;

async fn requestheader_ca(client: &kube::Client, cfg: &DaemonConfig) -> Result<Vec<u8>> {
    if let Some(p) = &cfg.requestheader_ca_file {
        return std::fs::read(p).with_context(|| format!("reading {p}"));
    }
    let api: kube::Api<ConfigMap> = kube::Api::namespaced(client.clone(), "kube-system");
    let cm = api
        .get("extension-apiserver-authentication")
        .await
        .context("reading kube-system/extension-apiserver-authentication")?;
    let ca = cm
        .data
        .and_then(|d| d.get("requestheader-client-ca-file").cloned())
        .context("extension-apiserver-authentication has no requestheader-client-ca-file")?;
    Ok(ca.into_bytes())
}

async fn shutdown_signal() {
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = term => {},
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    // Secret-free logging: only kinds, counts and errors are ever logged.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let cfg = DaemonConfig::from_env()?;
    let client = kube::Client::try_default().await.context("kube client")?;

    let cert = std::fs::read(format!("{}/tls.crt", cfg.tls_dir)).context("reading tls.crt")?;
    let key = std::fs::read(format!("{}/tls.key", cfg.tls_dir)).context("reading tls.key")?;
    let ca = requestheader_ca(&client, &cfg).await?;
    let tls_cfg = Arc::new(tls::server_config(&cert, &key, &ca)?);

    let epoch = format!("{:032x}", rand_epoch());
    let hub = Arc::new(Hub::new(epoch, cfg.max_events, cfg.max_age, 1024));
    let keys = sync_keys(&cfg);
    let sync = Arc::new(SyncTracker::new(
        &keys.iter().map(String::as_str).collect::<Vec<_>>(),
    ));
    let ring = Arc::new(MetricsRing::new(
        DEFAULT_SAMPLES_PER_SERIES,
        DEFAULT_MAX_BYTES,
    ));
    let avail = Arc::new(MetricsAvailability::default());
    let resolver = Arc::new(ScopeResolver::new(
        Arc::new(KubeReviewer::new(client.clone())),
        Duration::from_secs(60),
        1024,
    ));
    let http_cfg = HttpConfig {
        kinds: WATCHED_KINDS.iter().map(|k| k.to_string()).collect(),
        scope: DaemonScope {
            mode: if cfg.namespaces.is_empty() {
                DaemonScopeMode::Cluster
            } else {
                DaemonScopeMode::Namespaces
            },
            namespaces: cfg.namespaces.clone(),
        },
        retention: DaemonRetention {
            max_events: cfg.max_events as u32,
            max_age_secs: cfg.max_age.num_seconds().max(0) as u64,
        },
        ..HttpConfig::default()
    };
    let snaps = Snapshots::new(&cfg);
    let state = AppState::new(
        hub.clone(),
        resolver,
        ring.clone(),
        avail.clone(),
        sync.clone(),
        http_cfg,
    )
    .with_snapshots(snaps.sources());

    let ctx = Arc::new(Ctx {
        hub: hub.clone(),
        sync,
        coalescer: Mutex::new(Coalescer::new(cfg.coalesce)),
    });
    spawn_all(client.clone(), &cfg, ctx, &snaps);
    tokio::spawn(run_sampler(client, hub, ring, avail));

    let health = TcpListener::bind(&cfg.health_listen)
        .await
        .context("health listener")?;
    let health_app = health_router(state.clone());
    tokio::spawn(async move {
        let _ = axum::serve(health, health_app).await;
    });
    let api = TcpListener::bind(&cfg.listen)
        .await
        .context("api listener")?;
    tracing::info!(listen = %cfg.listen, health = %cfg.health_listen, "fathom-daemon started");
    tls::serve(api, tls_cfg, api_router(state), shutdown_signal()).await;
    tracing::info!("shutting down");
    Ok(())
}

/// Random per-process epoch id (128 bits from the OS via a hashed clock+pid fallback-free
/// source: `RandomState` is seeded from the OS RNG).
fn rand_epoch() -> u128 {
    use std::hash::{BuildHasher, Hasher};
    let mut a = std::collections::hash_map::RandomState::new().build_hasher();
    a.write_u32(std::process::id());
    let mut b = std::collections::hash_map::RandomState::new().build_hasher();
    b.write_u128(chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u128);
    ((a.finish() as u128) << 64) | b.finish() as u128
}
