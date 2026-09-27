//! External integrations: Prometheus, Grafana, ArgoCD, FluxCD
//!
//! This module manages configuration and connections to external services.

use super::longhorn;
use super::persistence;
use crate::backend::events;
use crate::backend::metrics_provider::PromEndpoint;
use crate::backend::oidc::store::Secret;
use crate::backend::portforward;
use crate::backend::prom_discovery;
use crate::backend::secret_store::{GRAFANA_SERVICE, SecretStore, grafana_key_set, set_verified};
use crate::backend::state::BackendState;
use crate::protocol::BackendEventEnvelope;
use crate::protocol::{
    BackendEvent, Correlation, DetectedMetricsComponent, DiskUsage, MetricsComponentKind,
    MetricsStackScanResult, MetricsStackTier, PrometheusStatusKind, Route, SystemEvent,
    section_keys,
};
use crate::protocol::{GrafanaSettings, PrometheusSettings};
use anyhow::Result;
use crossbeam_channel::Sender;
use k8s_openapi::api::core::v1::Service;
use kube::Client;
use kube::api::Api;
use kube::api::ListParams;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::AtomicU64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrometheusConfig {
    pub enabled: bool,
    pub namespace: String,
    pub service: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrafanaConfig {
    pub enabled: bool,
    pub base_url: String,
    /// Legacy plaintext key from files written before 039. Read-only: it is migrated into the
    /// credential store and is never serialized (`skip_serializing`).
    #[serde(default, skip_serializing)]
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArgoConfig {
    pub enabled: bool,
    pub namespace: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FluxConfig {
    pub enabled: bool,
    pub namespace: String,
}

/// 039: one context's observability settings, stored under `by_context`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextIntegrations {
    #[serde(default)]
    pub prometheus: Option<PrometheusConfig>,
    #[serde(default)]
    pub grafana: Option<GrafanaConfig>,
}

/// The file format is additive: the legacy top-level `context`/`prometheus`/`grafana`/`argo`/
/// `flux` record stays readable (and is still refreshed for the context last saved, so an
/// older build keeps working), and `by_context` holds per-context settings.
///
/// A value returned by [`IntegrationConfig::load`] is already resolved for one context:
/// `context`, `prometheus` and `grafana` describe that context only.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntegrationConfig {
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub prometheus: Option<PrometheusConfig>,
    #[serde(default)]
    pub grafana: Option<GrafanaConfig>,
    #[serde(default)]
    pub argo: Option<ArgoConfig>,
    #[serde(default)]
    pub flux: Option<FluxConfig>,
    #[serde(default)]
    pub by_context: std::collections::BTreeMap<String, ContextIntegrations>,
}

impl IntegrationConfig {
    fn empty(context: &str) -> Self {
        Self {
            context: context.to_owned(),
            ..Self::default()
        }
    }

    /// Load the settings for `context` from ~/.fathom/integrations.json.
    pub fn load(context: &str) -> Result<Self> {
        match persistence::integrations_config_path() {
            Ok(path) => Self::load_from(&path, context),
            Err(_) => Ok(Self::empty(context)),
        }
    }

    /// Resolve `context` from the file at `path`. A missing or unreadable file yields empty
    /// settings and is never modified (Constitution VI).
    pub fn load_from(path: &Path, context: &str) -> Result<Self> {
        let file = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<IntegrationConfig>(&text).ok());
        let Some(file) = file else {
            return Ok(Self::empty(context));
        };
        let legacy_applies = file.context == context;
        let entry = file.by_context.get(context).cloned().unwrap_or_default();
        let mut grafana = entry
            .grafana
            .or_else(|| legacy_applies.then(|| file.grafana.clone()).flatten());
        if let Some(g) = grafana.as_mut()
            && g.api_key.is_empty()
            && legacy_applies
            && let Some(legacy) = file.grafana.as_ref()
        {
            g.api_key = legacy.api_key.clone();
        }
        let prometheus = entry
            .prometheus
            .or_else(|| legacy_applies.then(|| file.prometheus.clone()).flatten());
        Ok(Self {
            context: context.to_owned(),
            prometheus,
            grafana,
            argo: file.argo,
            flux: file.flux,
            by_context: file.by_context,
        })
    }

    /// Save to ~/.fathom/integrations.json.
    pub fn save(&self) -> Result<()> {
        self.save_to(&persistence::integrations_config_path()?)
    }

    /// Merge this context's settings into the file at `path`, keeping every other context.
    /// Refuses (and leaves the file untouched) if the existing file is not valid JSON.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        let mut raw = match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str::<serde_json::Value>(&text).map_err(|e| {
                anyhow::anyhow!("{} is unreadable, not overwriting: {e}", path.display())
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
            Err(e) => return Err(e.into()),
        };
        let Some(obj) = raw.as_object_mut() else {
            anyhow::bail!("{} is not a JSON object, not overwriting", path.display());
        };
        let entry = ContextIntegrations {
            prometheus: self.prometheus.clone(),
            grafana: self.grafana.clone(),
        };
        let by_context = obj
            .entry("by_context")
            .or_insert_with(|| serde_json::json!({}));
        if !by_context.is_object() {
            *by_context = serde_json::json!({});
        }
        by_context
            .as_object_mut()
            .expect("object")
            .insert(self.context.clone(), serde_json::to_value(entry)?);
        // Keep the legacy record current for the context last saved, unless it still holds
        // another context's un-migrated record.
        let legacy_ctx = obj.get("context").and_then(|v| v.as_str()).unwrap_or("");
        if legacy_ctx.is_empty() || legacy_ctx == self.context {
            obj.insert("context".into(), self.context.clone().into());
            obj.insert("prometheus".into(), serde_json::to_value(&self.prometheus)?);
            obj.insert("grafana".into(), serde_json::to_value(&self.grafana)?);
            obj.insert("argo".into(), serde_json::to_value(&self.argo)?);
            obj.insert("flux".into(), serde_json::to_value(&self.flux)?);
        }
        persistence::save_json_file_atomic(path, &raw)?;
        Ok(())
    }
}

/// 039: the non-secret settings event payload for `cfg` (already resolved for its context).
pub fn observability_settings_event(
    cfg: &IntegrationConfig,
    store: &dyn SecretStore,
) -> SystemEvent {
    SystemEvent::ObservabilitySettings {
        context: cfg.context.clone(),
        prometheus: cfg.prometheus.as_ref().map(|p| PrometheusSettings {
            namespace: p.namespace.clone(),
            service: p.service.clone(),
            port: p.port,
        }),
        grafana: cfg.grafana.as_ref().map(|g| GrafanaSettings {
            base_url: g.base_url.clone(),
            key_set: grafana_key_set(store, &cfg.context),
        }),
    }
}

/// 039 T005: move a legacy plaintext Grafana key into the secret store, then rewrite the file
/// without it. On any failure the file is left as it was so the key is not lost.
pub fn migrate_legacy_grafana_key(
    path: &Path,
    cfg: &mut IntegrationConfig,
    store: &dyn SecretStore,
) {
    let Some(g) = cfg.grafana.as_mut() else {
        return;
    };
    if g.api_key.is_empty() {
        return;
    }
    if set_verified(
        store,
        GRAFANA_SERVICE,
        &cfg.context,
        &Secret::new(g.api_key.clone()),
    )
    .is_err()
    {
        return;
    }
    g.api_key.clear();
    if cfg.save_to(path).is_err() {
        tracing::warn!("could not rewrite integrations.json after migrating the Grafana key");
    }
}

/// Result of a settings command: the event to emit and an error to notify, if any.
pub struct SettingsOutcome {
    pub event: SystemEvent,
    pub error: Option<String>,
}

/// `GetObservabilitySettings` core.
pub fn read_settings(path: &Path, context: &str, store: &dyn SecretStore) -> SettingsOutcome {
    let mut cfg = IntegrationConfig::load_from(path, context)
        .unwrap_or_else(|_| IntegrationConfig::empty(context));
    migrate_legacy_grafana_key(path, &mut cfg, store);
    SettingsOutcome {
        event: observability_settings_event(&cfg, store),
        error: None,
    }
}

/// `ConfigureGrafana` core (FR-002, FR-010). A non-empty key replaces the stored one, a blank
/// key keeps it. If the credential store fails the URL is still saved, `key_set` reports the
/// truth and an error is returned for notification. The error text never contains the key.
pub fn configure_grafana(
    path: &Path,
    context: &str,
    store: &dyn SecretStore,
    base_url: &str,
    api_key: &str,
) -> SettingsOutcome {
    let mut cfg = IntegrationConfig::load_from(path, context)
        .unwrap_or_else(|_| IntegrationConfig::empty(context));
    cfg.context = context.to_owned();
    // Move any legacy plaintext key first so saving cannot drop it.
    migrate_legacy_grafana_key(path, &mut cfg, store);
    cfg.grafana = Some(GrafanaConfig {
        enabled: true,
        base_url: base_url.trim().to_owned(),
        api_key: String::new(),
    });
    let mut error = None;
    if let Err(e) = cfg.save_to(path) {
        error = Some(format!("Failed to save Grafana config: {e}"));
    } else if !api_key.is_empty()
        && let Err(e) = set_verified(store, GRAFANA_SERVICE, context, &Secret::new(api_key))
    {
        error = Some(format!(
            "Grafana URL saved, but the API key could not be stored in the credential store: {e}"
        ));
    }
    SettingsOutcome {
        event: observability_settings_event(&cfg, store),
        error,
    }
}

/// `ClearGrafanaKey` core.
pub fn clear_grafana_key(path: &Path, context: &str, store: &dyn SecretStore) -> SettingsOutcome {
    let cfg = IntegrationConfig::load_from(path, context)
        .unwrap_or_else(|_| IntegrationConfig::empty(context));
    let error = store
        .delete(GRAFANA_SERVICE, context)
        .err()
        .map(|e| format!("Failed to clear the Grafana API key: {e}"));
    SettingsOutcome {
        event: observability_settings_event(&cfg, store),
        error,
    }
}

/// Query Prometheus for time-series metrics
/// Returns JSON metric data
pub async fn query_prometheus(endpoint: &PromEndpoint, query: &str) -> Result<serde_json::Value> {
    endpoint
        .get_json(&format!(
            "/api/v1/query?query={}",
            urlencoding::encode(query)
        ))
        .await
}

/// Pick a Prometheus base URL from available evidence, in priority order:
/// an explicit client URL, a port-forward mapped to 9090, then a configured port.
pub fn prometheus_base_url_from_parts(
    prom_client_url: Option<&str>,
    port_forwards: &[(u16, u16)], // (local_port, remote_port)
    configured_prometheus_port: Option<u16>,
) -> Option<String> {
    if let Some(url) = prom_client_url {
        return Some(url.to_string());
    }

    for (local, remote) in port_forwards {
        if *remote == 9090 || *local == 9090 {
            return Some(format!("http://127.0.0.1:{}", local));
        }
    }

    if let Some(p) = configured_prometheus_port {
        return Some(format!("http://127.0.0.1:{}", p));
    }

    None
}

/// The Prometheus endpoint for the active context: the resolved (saved or detected) Service via
/// the API server proxy, else a Fathom-managed port-forward to 9090 (see
/// [`prometheus_base_url_from_parts`]).
pub(super) async fn prometheus_endpoint(state: &BackendState) -> Option<PromEndpoint> {
    if let Ok((endpoint, _)) = resolve_prometheus(
        state.cluster.current_context.as_deref(),
        state.cluster.client.as_ref(),
    )
    .await
    {
        return Some(endpoint);
    }
    forwarded_prometheus(state)
}

/// A Fathom-managed port-forward to Prometheus (9090), from state alone: no cluster round trip,
/// so it is safe to read under a brief lock and use after releasing it.
pub(super) fn forwarded_prometheus(state: &BackendState) -> Option<PromEndpoint> {
    let port_forwards: Vec<(u16, u16)> = state
        .port_forward_manager
        .list()
        .into_iter()
        .map(|pf| (pf.local_port, pf.remote_port))
        .collect();
    prometheus_base_url_from_parts(None, &port_forwards, None).map(PromEndpoint::Url)
}

/// Get Grafana dashboard URL with variable substitution
pub fn grafana_dashboard_url(
    base_url: &str,
    dashboard_uid: &str,
    namespace: &str,
    pod: &str,
) -> String {
    // TODO: Generate URL with variable interpolation
    format!(
        "{}/d/{}?var-namespace={}&var-pod={}",
        base_url, dashboard_uid, namespace, pod
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskUsageProvider {
    Longhorn,
    Prometheus,
    None,
}

/// Detect disk usage provider: prefer Longhorn CRDs when present, otherwise prefer configured Prometheus.
pub async fn detect_disk_provider(client: &Client, context: Option<&str>) -> DiskUsageProvider {
    // Check CRDs for Longhorn volumes
    use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
    let crd_api: Api<CustomResourceDefinition> = Api::all(client.clone());
    if let Ok(list) = crd_api.list(&ListParams::default()).await {
        for crd in list.items {
            if let Some(name) = crd.metadata.name
                && name == "volumes.longhorn.io"
            {
                return DiskUsageProvider::Longhorn;
            }
        }
    }

    // A reachable Prometheus (saved or auto-detected) for this context.
    if resolve_prometheus(context, Some(client)).await.is_ok() {
        return DiskUsageProvider::Prometheus;
    }

    DiskUsageProvider::None
}

/// Fetch disk usage using the detected provider and emit a System event with the results
/// `context` is captured at request start so a late result keeps its origin (FR-016);
/// `forwarded` is [`forwarded_prometheus`] at that moment, used when the API server proxy can't
/// reach Prometheus. Needs no state lock.
pub(super) async fn fetch_disk_usage(
    client: &Client,
    seq: &AtomicU64,
    context: Option<String>,
    forwarded: Option<PromEndpoint>,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let provider = detect_disk_provider(client, context.as_deref()).await;
    let route = Route::cluster(section_keys::integrations_metrics(), context.clone());

    match provider {
        DiskUsageProvider::Longhorn => {
            let vols = longhorn::list_longhorn_volumes(client)
                .await
                .unwrap_or_default();
            events::emit(
                seq,
                sender,
                route,
                Correlation::None,
                BackendEvent::System(SystemEvent::DiskUsageUpdated(vols)),
            );
        }
        DiskUsageProvider::Prometheus => {
            // Basic query for persistent volume claim usage available from kubelet metrics
            // This will return time series with labels `persistentvolumeclaim` and `namespace`
            let resolved = resolve_prometheus(context.as_deref(), Some(client)).await;
            let Some(endpoint) = resolved.ok().map(|(e, _)| e).or(forwarded) else {
                return Ok(());
            };
            let query = "kubelet_volume_stats_used_bytes";
            if let Ok(json) = query_prometheus(&endpoint, query).await
                && let Some(results) = json
                    .get("data")
                    .and_then(|d| d.get("result"))
                    .and_then(|r| r.as_array())
            {
                let mut disk_items = Vec::new();
                for item in results {
                    let metric = item
                        .get("metric")
                        .and_then(|m| m.as_object())
                        .cloned()
                        .unwrap_or_default();
                    let pvc = metric
                        .get("persistentvolumeclaim")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let ns = metric
                        .get("namespace")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let value_str = item
                        .get("value")
                        .and_then(|v| v.as_array())
                        .and_then(|a| a.get(1))
                        .and_then(|v| v.as_str())
                        .unwrap_or("0");
                    let used = value_str.parse::<f64>().ok().map(|f| f as u64);
                    disk_items.push(DiskUsage {
                        namespace: ns,
                        name: pvc,
                        provisioned_bytes: 0,
                        used_bytes: used,
                        state: None,
                    });
                }
                events::emit(
                    seq,
                    sender,
                    route,
                    Correlation::None,
                    BackendEvent::System(SystemEvent::DiskUsageUpdated(disk_items)),
                );
            }
        }
        DiskUsageProvider::None => {
            // Nothing available; emit info to user
            events::notify_info(
                seq,
                sender,
                "No disk usage provider detected (no Longhorn CRDs and no Prometheus).".to_owned(),
            );
        }
    }

    Ok(())
}
// --- Prometheus auto-discovery and port-forward helpers extracted from backend.rs ---

/// Auto-start a Prometheus port forward for the active context, best-effort.
///
/// Called after a connect without the command lock held: every cluster lookup runs unlocked and
/// the lock is taken only to read config and to register a forward, so pane refreshes are never
/// queued behind this discovery. Stops quietly once `still_current` says the connection that
/// `client` belongs to has been replaced.
pub(super) async fn ensure_metrics_port_forward(
    state: &tokio::sync::Mutex<BackendState>,
    sender: &Sender<BackendEventEnvelope>,
    client: &Client,
    context: Option<&str>,
    still_current: impl Fn(&BackendState) -> bool,
) {
    let candidates = {
        let s = state.lock().await;
        if !still_current(&s) {
            return;
        }
        prometheus_candidates(&s)
    };

    // 041: a saved or detected Prometheus reachable through the API server proxy needs no
    // port-forward at all. Only fall back to forwarding when that fails.
    let resolved = resolve_prometheus(context, Some(client)).await;
    let connected = resolved.is_ok();
    let seq = {
        let s = state.lock().await;
        if !still_current(&s) {
            return;
        }
        emit_prometheus_status(&s, sender, resolved.map(|(_, t)| t));
        s.cluster.seq.clone()
    };
    if connected {
        let _ = fetch_disk_usage(client, &seq, context.map(str::to_owned), None, sender).await;
        return;
    }

    for (namespace, service, port) in candidates {
        let Some(pod_name) = service_pod(client, &namespace, &service).await else {
            continue;
        };
        let target = ForwardTarget {
            namespace: &namespace,
            pod_name: &pod_name,
            port,
            verb: "Auto-started",
        };
        if forward_while_current(state, sender, client, target, &still_current).await {
            return;
        }
    }

    // Fallback: Check if port 9090 is available locally and try to find any Prometheus instance
    if portforward::is_local_port_available(9090) {
        // Try to find any Prometheus pod in common namespaces
        for namespace in &["monitoring", "prometheus", "kube-monitoring", "default"] {
            let Ok(Some(pod_name)) = find_any_prometheus_pod(client, namespace).await else {
                continue;
            };
            let target = ForwardTarget {
                namespace,
                pod_name: &pod_name,
                port: 9090,
                verb: "Auto-discovered",
            };
            if forward_while_current(state, sender, client, target, &still_current).await {
                return;
            }
        }
    }

    // Attempt to fetch disk usage after auto-discovery/port-forward attempts.
    let (seq, forwarded) = {
        let s = state.lock().await;
        if !still_current(&s) {
            return;
        }
        (s.cluster.seq.clone(), forwarded_prometheus(&s))
    };
    let _ = fetch_disk_usage(client, &seq, context.map(str::to_owned), forwarded, sender).await;

    events::notify_info(
        &seq,
        sender,
        "Prometheus auto port-forward not started (no matching service/pod found). Configure a port forward under Network > Port Forwards or adjust Prometheus settings under Integrations.".to_owned(),
    );
}

/// A pod backing `namespace/service`, if one exists (lookup failures are only logged).
async fn service_pod(client: &Client, namespace: &str, service: &str) -> Option<String> {
    match find_pod_for_service(client, namespace, service).await {
        Ok(Some(pod_name)) => Some(pod_name),
        Ok(None) => {
            tracing::debug!("No pod found for service {}/{}", namespace, service);
            None
        }
        Err(e) => {
            tracing::debug!("Error finding pod for {}/{}: {}", namespace, service, e);
            None
        }
    }
}

/// Take the lock briefly and start `target`'s forward, unless the connection was replaced.
/// Returns `true` when discovery should stop (stale connection, or a forward exists/started).
async fn forward_while_current(
    state: &tokio::sync::Mutex<BackendState>,
    sender: &Sender<BackendEventEnvelope>,
    client: &Client,
    target: ForwardTarget<'_>,
    still_current: &impl Fn(&BackendState) -> bool,
) -> bool {
    let mut s = state.lock().await;
    if !still_current(&s) {
        return true;
    }
    start_prometheus_forward(&mut s, sender, client, target).await
}

/// `(namespace, service, port)` candidates: the user-configured Prometheus for the
/// current context first (if enabled), then common default locations.
fn prometheus_candidates(state: &BackendState) -> Vec<(String, String, u16)> {
    // Load user-configured Prometheus first (per context), then fall back to common defaults
    let context = state
        .cluster
        .current_context
        .clone()
        .unwrap_or_else(|| "default".to_owned());

    let mut prometheus_configs: Vec<(String, String, u16)> = Vec::new();

    if let Ok(cfg) = IntegrationConfig::load(&context)
        && let Some(prom) = cfg.prometheus.as_ref().filter(|p| p.enabled)
    {
        prometheus_configs.push((prom.namespace.clone(), prom.service.clone(), prom.port));
    }

    // Try common Prometheus locations
    prometheus_configs.extend_from_slice(&[
        ("monitoring".to_owned(), "prometheus".to_owned(), 9090u16),
        ("prometheus".to_owned(), "prometheus".to_owned(), 9090),
        ("kube-monitoring".to_owned(), "prometheus".to_owned(), 9090),
    ]);
    prometheus_configs
}

/// A Prometheus pod to forward `localhost:port -> namespace/pod_name:port` to.
struct ForwardTarget<'a> {
    namespace: &'a str,
    pod_name: &'a str,
    port: u16,
    /// Prefix for the success notification ("Auto-started" / "Auto-discovered").
    verb: &'a str,
}

/// Start a Prometheus port forward unless one already exists.
///
/// Returns `true` when discovery is done (a forward already existed or was started),
/// `false` when starting failed and the next candidate should be tried.
async fn start_prometheus_forward(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    client: &Client,
    target: ForwardTarget<'_>,
) -> bool {
    let ForwardTarget {
        namespace,
        pod_name,
        port,
        verb,
    } = target;

    // Check if a port forward for this pod already exists
    let existing = state
        .port_forward_manager
        .list()
        .iter()
        .any(|pf| pf.namespace == namespace && pf.pod == pod_name && pf.local_port == port);

    if existing {
        tracing::debug!(
            "Port forward for {}/{} on port {} already exists, skipping",
            namespace,
            pod_name,
            port
        );
        return true;
    }

    match state
        .port_forward_manager
        .start(client, namespace, pod_name, port, port, None)
        .await
    {
        Ok(_) => {
            events::notify_info(
                state.cluster.seq.as_ref(),
                sender,
                format!(
                    "{} Prometheus port forward: localhost:{} -> {}/{}:{}",
                    verb, port, namespace, pod_name, port
                ),
            );
            // Emit updated list and save to disk
            portforward::emit_port_forward_list(state, sender);
            portforward::save_port_forwards_to_disk(state);
            true
        }
        Err(e) => {
            tracing::debug!(
                "Failed to start port forward for {}/{}: {}",
                namespace,
                pod_name,
                e
            );
            false
        }
    }
}

pub(super) async fn find_pod_for_service(
    client: &Client,
    namespace: &str,
    service: &str,
) -> anyhow::Result<Option<String>> {
    use ::kube::Api;
    use k8s_openapi::api::core::v1::Service;

    let svc_api = Api::<Service>::namespaced(client.clone(), namespace);

    if let Ok(svc) = svc_api.get(service).await {
        // First, try to resolve via Endpoints to honor any non-standard selectors/endpoints
        if let Some(pod_name) = find_pod_via_endpoints(client, namespace, service).await? {
            return Ok(Some(pod_name));
        }

        // Get the selector labels from the service
        if let Some(selector) = svc.spec.as_ref().and_then(|s| s.selector.as_ref()) {
            // Build a label selector string
            let mut label_query = String::new();
            for (i, (key, val)) in selector.iter().enumerate() {
                if i > 0 {
                    label_query.push(',');
                }
                label_query.push_str(&format!("{}={}", key, val));
            }

            // Find a pod matching these labels
            use k8s_openapi::api::core::v1::Pod;
            let pod_api = Api::<Pod>::namespaced(client.clone(), namespace);
            let list_params = ::kube::api::ListParams::default().labels(&label_query);

            if let Ok(pods) = pod_api.list(&list_params).await
                && let Some(pod) = pods.items.first()
                && let Some(name) = pod.metadata.name.as_ref()
            {
                return Ok(Some(name.clone()));
            }
        }
    }

    Ok(None)
}

pub(super) async fn find_pod_via_endpoints(
    client: &Client,
    namespace: &str,
    service: &str,
) -> anyhow::Result<Option<String>> {
    use ::kube::Api;
    use k8s_openapi::api::core::v1::Endpoints;

    let ep_api = Api::<Endpoints>::namespaced(client.clone(), namespace);
    if let Ok(eps) = ep_api.get(service).await
        && let Some(subsets) = eps.subsets
    {
        for subset in subsets {
            // Prefer ready addresses, fallback to not_ready_addresses
            let candidates = subset
                .addresses
                .or(subset.not_ready_addresses)
                .unwrap_or_default();
            for addr in candidates {
                if let Some(target_ref) = addr.target_ref
                    && target_ref.kind.as_deref() == Some("Pod")
                    && let Some(name) = target_ref.name
                {
                    return Ok(Some(name));
                }
            }
        }
    }

    Ok(None)
}

pub(super) async fn find_any_prometheus_pod(
    client: &Client,
    namespace: &str,
) -> anyhow::Result<Option<String>> {
    use ::kube::Api;
    use k8s_openapi::api::core::v1::Pod;

    let pod_api = Api::<Pod>::namespaced(client.clone(), namespace);
    match pod_api.list(&Default::default()).await {
        Ok(pods) => {
            // Find first pod with "prometheus" in the name
            for pod in pods.items {
                if let Some(name) = pod.metadata.name.as_ref()
                    && name.to_lowercase().contains("prometheus")
                {
                    return Ok(Some(name.clone()));
                }
            }
            Ok(None)
        }
        Err(_) => Ok(None),
    }
}

/// Classify a Service name as a known metrics-stack component, if recognized.
/// Order matters: more specific names (node-exporter, kube-state-metrics,
/// openobserve) are checked before the generic "prometheus" substring so
/// e.g. "prometheus-node-exporter" is classified as NodeExporter, not Prometheus.
fn classify_metrics_service_name(name: &str) -> Option<MetricsComponentKind> {
    let n = name.to_lowercase();

    if n.contains("openobserve") || n.contains("zincobserve") {
        Some(MetricsComponentKind::OpenObserve)
    } else if n.contains("alertmanager") {
        Some(MetricsComponentKind::Alertmanager)
    } else if n.contains("kube-state-metrics") {
        Some(MetricsComponentKind::KubeStateMetrics)
    } else if n.contains("node-exporter") {
        Some(MetricsComponentKind::NodeExporter)
    } else if n.contains("grafana") {
        Some(MetricsComponentKind::Grafana)
    } else if n.contains("metrics-server") {
        Some(MetricsComponentKind::MetricsServer)
    } else if n.contains("prometheus")
        && !n.contains("operator")
        && !n.contains("adapter")
        && !n.contains("webhook")
    {
        Some(MetricsComponentKind::Prometheus)
    } else {
        None
    }
}

/// Default query port for a component when the Service doesn't expose one
/// under a name we recognize (falls back to the component's first port).
fn default_port_for(kind: MetricsComponentKind) -> u16 {
    match kind {
        MetricsComponentKind::Prometheus => 9090,
        MetricsComponentKind::Grafana => 3000,
        MetricsComponentKind::Alertmanager => 9093,
        MetricsComponentKind::KubeStateMetrics => 8080,
        MetricsComponentKind::NodeExporter => 9100,
        MetricsComponentKind::OpenObserve => 5080,
        MetricsComponentKind::MetricsServer => 443,
    }
}

/// Classify how complete a Prometheus-family stack is from the components found.
/// OpenObserve takes priority since it replaces the whole stack on its own.
fn classify_stack_tier(components: &[DetectedMetricsComponent]) -> MetricsStackTier {
    let has = |kind: MetricsComponentKind| components.iter().any(|c| c.kind == kind);

    if has(MetricsComponentKind::OpenObserve) {
        MetricsStackTier::OpenObserve
    } else if has(MetricsComponentKind::Prometheus)
        && has(MetricsComponentKind::KubeStateMetrics)
        && has(MetricsComponentKind::NodeExporter)
    {
        MetricsStackTier::Full
    } else if !components.is_empty() {
        MetricsStackTier::Partial
    } else {
        MetricsStackTier::None
    }
}

/// Pick the query-endpoint candidate to prefill into Prometheus-compatible config:
/// prefer a real Prometheus, otherwise fall back to OpenObserve's compatible API.
fn pick_suggested_endpoint(
    components: &[DetectedMetricsComponent],
) -> Option<DetectedMetricsComponent> {
    components
        .iter()
        .find(|c| c.kind == MetricsComponentKind::Prometheus)
        .or_else(|| {
            components
                .iter()
                .find(|c| c.kind == MetricsComponentKind::OpenObserve)
        })
        .cloned()
}

/// A metrics-stack scan: the protocol result plus the ranked Prometheus query targets (which
/// carry the scheme and route prefix the protocol type does not).
pub struct MetricsScan {
    pub result: MetricsStackScanResult,
    /// Best first (see [`prom_discovery::rank_candidates`]).
    pub prometheus: Vec<PromTarget>,
}

fn service_facts(svc: &Service) -> Option<prom_discovery::ServiceFacts> {
    let spec = svc.spec.as_ref();
    Some(prom_discovery::ServiceFacts {
        namespace: svc.metadata.namespace.clone()?,
        name: svc.metadata.name.clone()?,
        labels: svc.metadata.labels.clone().unwrap_or_default(),
        selector: spec.and_then(|s| s.selector.clone()).unwrap_or_default(),
        ports: spec
            .and_then(|s| s.ports.as_ref())
            .map(|ports| {
                ports
                    .iter()
                    .map(|p| prom_discovery::PortFacts {
                        name: p.name.clone(),
                        port: p.port as u16,
                        app_protocol: p.app_protocol.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        headless: spec.and_then(|s| s.cluster_ip.as_deref()) == Some("None"),
        ready: None,
    })
}

/// Every Service in the cluster; when a cluster-wide list is denied (namespace-scoped access),
/// the Services of the namespaces Prometheus usually lives in.
async fn list_services(client: &Client) -> anyhow::Result<Vec<Service>> {
    let all: Api<Service> = Api::all(client.clone());
    match all.list(&ListParams::default()).await {
        Ok(list) => Ok(list.items),
        Err(e) => {
            tracing::debug!("cluster-wide Service list failed ({e}); scanning common namespaces");
            let namespaces = prom_discovery::COMMON_NAMESPACES
                .iter()
                .chain(["default", "kube-system"].iter());
            let lists = futures::future::join_all(namespaces.map(|ns| {
                let api: Api<Service> = Api::namespaced(client.clone(), ns);
                async move { api.list(&ListParams::default()).await }
            }))
            .await;
            let mut items = Vec::new();
            let mut any_ok = false;
            for list in lists.into_iter().flatten() {
                any_ok = true;
                items.extend(list.items);
            }
            if any_ok { Ok(items) } else { Err(e.into()) }
        }
    }
}

/// prometheus-operator `Prometheus` objects (route prefix, port name). A missing CRD or an RBAC
/// denial is not an error: discovery then relies on Services alone.
async fn list_operator_prometheuses(client: &Client) -> Vec<prom_discovery::OperatorPrometheus> {
    use kube::core::{ApiResource, DynamicObject, GroupVersionKind};
    let gvk = GroupVersionKind::gvk("monitoring.coreos.com", "v1", "Prometheus");
    let resource = ApiResource::from_gvk_with_plural(&gvk, "prometheuses");
    let api: Api<DynamicObject> = Api::all_with(client.clone(), &resource);
    match api.list(&ListParams::default()).await {
        Ok(list) => list
            .items
            .iter()
            .filter_map(|o| {
                let text = |ptr: &str| {
                    o.data
                        .pointer(ptr)
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                };
                Some(prom_discovery::OperatorPrometheus {
                    namespace: o.metadata.namespace.clone()?,
                    name: o.metadata.name.clone()?,
                    route_prefix: text("/spec/routePrefix"),
                    port_name: text("/spec/portName"),
                })
            })
            .collect(),
        Err(e) => {
            tracing::debug!("Prometheus CR list unavailable: {e}");
            Vec::new()
        }
    }
}

/// Whether `namespace/service` has a ready endpoint (`None` when that cannot be read).
async fn service_ready(client: &Client, namespace: &str, service: &str) -> Option<bool> {
    use k8s_openapi::api::discovery::v1::EndpointSlice;
    let api: Api<EndpointSlice> = Api::namespaced(client.clone(), namespace);
    let params = ListParams::default().labels(&format!("kubernetes.io/service-name={service}"));
    let slices = api.list(&params).await.ok()?;
    Some(
        slices
            .items
            .iter()
            .flat_map(|s| s.endpoints.iter())
            .any(|e| e.conditions.as_ref().and_then(|c| c.ready).unwrap_or(true)),
    )
}

/// Scan the cluster for known metrics-stack Services (Prometheus, Grafana, Alertmanager,
/// kube-state-metrics, node-exporter, OpenObserve, metrics-server) and classify how complete the
/// stack is. Prometheus servers are scored and ranked (labels, selector, operator objects, query
/// port, ready endpoints) rather than taken by first name match (feedback #31).
pub(super) async fn scan_metrics_stack(client: &Client) -> anyhow::Result<MetricsScan> {
    let (services, operators) =
        tokio::join!(list_services(client), list_operator_prometheuses(client));
    let services = services?;

    let mut others: Vec<DetectedMetricsComponent> = Vec::new();
    let mut facts: Vec<prom_discovery::ServiceFacts> = Vec::new();
    for svc in &services {
        let Some(f) = service_facts(svc) else {
            continue;
        };
        match classify_metrics_service_name(&f.name) {
            Some(MetricsComponentKind::Prometheus) | None => {
                if prom_discovery::prometheus_signal(&f).is_some() {
                    facts.push(f);
                }
            }
            Some(kind) => others.push(DetectedMetricsComponent {
                kind,
                port: f
                    .ports
                    .first()
                    .map_or_else(|| default_port_for(kind), |p| p.port),
                namespace: f.namespace,
                service: f.name,
            }),
        }
    }

    // Endpoint readiness for the best few candidates, then the final ranking.
    let top: Vec<(String, String)> = prom_discovery::rank_candidates(&facts, &operators)
        .into_iter()
        .take(prom_discovery::READINESS_LOOKUPS)
        .map(|c| (c.namespace, c.service))
        .collect();
    let readiness =
        futures::future::join_all(top.iter().map(|(ns, name)| service_ready(client, ns, name)))
            .await;
    for ((ns, name), ready) in top.iter().zip(readiness) {
        if let Some(f) = facts
            .iter_mut()
            .find(|f| &f.namespace == ns && &f.name == name)
        {
            f.ready = ready;
        }
    }
    let ranked = prom_discovery::rank_candidates(&facts, &operators);

    let mut components: Vec<DetectedMetricsComponent> = ranked
        .iter()
        .map(|c| DetectedMetricsComponent {
            kind: MetricsComponentKind::Prometheus,
            namespace: c.namespace.clone(),
            service: c.service.clone(),
            port: c.port,
        })
        .collect();
    components.extend(others);
    let tier = classify_stack_tier(&components);
    let suggested_endpoint = pick_suggested_endpoint(&components);

    Ok(MetricsScan {
        result: MetricsStackScanResult {
            tier,
            components,
            suggested_endpoint,
        },
        prometheus: ranked.iter().map(PromTarget::from_candidate).collect(),
    })
}

// --- 041-observability-autoconnect: Prometheus endpoint resolution ---
//
// A detected Prometheus is queried through the API server's service proxy with the active
// context's credentials, so metrics work with no port-forward and no user input. A saved
// (manual) Prometheus config for the context still wins when it answers.

/// Where a context's Prometheus lives, and how Fathom found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromTarget {
    pub namespace: String,
    pub service: String,
    pub port: u16,
    /// Proxy with `https:` (the Service port is TLS).
    pub https: bool,
    /// Prometheus route prefix ("" or "/prefix").
    pub path_prefix: String,
    pub source: PromSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromSource {
    /// The user's saved Prometheus settings for this context (or a typed Test target).
    Saved,
    /// Found by the metrics-stack scan.
    Detected,
}

impl PromTarget {
    /// A user-entered target (plain http, no prefix).
    pub fn saved(namespace: String, service: String, port: u16) -> Self {
        Self {
            namespace,
            service,
            port,
            https: false,
            path_prefix: String::new(),
            source: PromSource::Saved,
        }
    }

    fn from_candidate(c: &prom_discovery::Candidate) -> Self {
        Self {
            namespace: c.namespace.clone(),
            service: c.service.clone(),
            port: c.port,
            https: c.https,
            path_prefix: c.path_prefix.clone(),
            source: PromSource::Detected,
        }
    }

    fn same_place(&self, other: &Self) -> bool {
        self.namespace == other.namespace
            && self.service == other.service
            && self.port == other.port
    }

    /// `ns/svc:port[/prefix]`.
    pub fn label(&self) -> String {
        format!(
            "{}/{}:{}{}",
            self.namespace, self.service, self.port, self.path_prefix
        )
    }

    pub fn endpoint(&self, client: &Client) -> PromEndpoint {
        PromEndpoint::ServiceProxy {
            client: client.clone(),
            namespace: self.namespace.clone(),
            service: self.service.clone(),
            port: self.port,
            https: self.https,
            path_prefix: self.path_prefix.clone(),
        }
    }

    fn probe_path(&self) -> String {
        prom_discovery::service_proxy_path(
            &self.namespace,
            &self.service,
            self.https,
            self.port,
            &self.path_prefix,
            "/api/v1/query?query=up",
        )
    }

    /// Status line for the Metrics pane.
    pub fn status_message(&self) -> String {
        let how = match self.source {
            PromSource::Saved => "Using saved",
            PromSource::Detected => "Auto-detected",
        };
        format!("{how} Prometheus {} via the API server proxy", self.label())
    }

    /// A short, classified status for a failed probe of this target (feedback #32): never the
    /// raw body inline; that goes to `detail`.
    pub fn failure(&self, err: &anyhow::Error) -> PromFailure {
        let (kind, why, raw) = match err.downcast_ref::<prom_discovery::PromQueryError>() {
            Some(q) => (
                match q.kind {
                    prom_discovery::FailureKind::Unreachable => PrometheusStatusKind::Unreachable,
                    prom_discovery::FailureKind::QueryFailed => PrometheusStatusKind::QueryFailed,
                },
                q.to_string(),
                q.raw.clone(),
            ),
            None => (
                PrometheusStatusKind::QueryFailed,
                prom_discovery::summarize_body(&err.to_string()),
                err.to_string(),
            ),
        };
        let whose = match self.source {
            PromSource::Saved => format!("Prometheus {}", self.label()),
            PromSource::Detected => format!("Found Prometheus at {}", self.label()),
        };
        let message = match kind {
            PrometheusStatusKind::Unreachable => format!("{whose}, but it is not reachable: {why}"),
            _ => format!("{whose}, but the query failed: {why}"),
        };
        PromFailure {
            kind,
            message,
            detail: Some(format!("GET {}\n{why}\n\n{raw}", self.probe_path())),
        }
    }
}

/// Why no Prometheus could be used, classified for the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromFailure {
    pub kind: PrometheusStatusKind,
    /// One short line.
    pub message: String,
    /// Raw responses / per-candidate failures.
    pub detail: Option<String>,
}

impl PromFailure {
    fn not_found(message: &str, detail: Option<String>) -> Self {
        Self {
            kind: PrometheusStatusKind::NotFound,
            message: message.to_owned(),
            detail,
        }
    }

    /// Combine failures from trying several targets in order: the first leads.
    fn combine(mut failures: Vec<PromFailure>) -> Option<Self> {
        if failures.is_empty() {
            return None;
        }
        let details: Vec<String> = failures.iter().filter_map(|f| f.detail.clone()).collect();
        let others = failures.len() - 1;
        let mut first = failures.remove(0);
        if others > 0 {
            first.message = format!(
                "{} ({others} other candidate{} also failed)",
                first.message,
                if others == 1 { "" } else { "s" }
            );
        }
        first.detail = (!details.is_empty()).then(|| details.join("\n\n---\n\n"));
        Some(first)
    }
}

impl std::fmt::Display for PromFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// The `PrometheusConnectionTest` event for a resolution / probe outcome.
pub fn prometheus_status_event(resolved: Result<PromTarget, PromFailure>) -> SystemEvent {
    match resolved {
        Ok(t) => SystemEvent::PrometheusConnectionTest {
            success: true,
            message: t.status_message(),
            kind: Some(PrometheusStatusKind::Connected),
            detail: None,
        },
        Err(f) => SystemEvent::PrometheusConnectionTest {
            success: false,
            message: f.message,
            kind: Some(f.kind),
            detail: f.detail,
        },
    }
}

/// How long a failed resolution is remembered before the next caller re-probes.
const PROM_NEGATIVE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// How many detected candidates are probed (best first) before giving up.
const MAX_DETECTED_PROBES: usize = 3;

#[derive(Clone)]
enum Resolution {
    Found(PromTarget),
    Missing {
        at: std::time::Instant,
        failure: PromFailure,
    },
}

type ContextMap<T> = std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, T>>>;

/// Ranked scan results per context (empty: scanned and nothing found).
static DETECTED_PROMETHEUS: ContextMap<Vec<PromTarget>> =
    std::sync::LazyLock::new(Default::default);
/// Probed resolution per context.
static RESOLVED_PROMETHEUS: ContextMap<Resolution> = std::sync::LazyLock::new(Default::default);

/// Remember a scan for `context` and drop its cached resolution so the next query re-probes.
pub fn remember_metrics_scan(context: &str, scan: &MetricsScan) {
    if let Ok(mut m) = DETECTED_PROMETHEUS.lock() {
        m.insert(context.to_owned(), scan.prometheus.clone());
    }
    forget_prometheus_resolution(context);
}

/// Drop the cached resolution for `context` (settings changed, rescan, reconnect).
pub fn forget_prometheus_resolution(context: &str) {
    if let Ok(mut m) = RESOLVED_PROMETHEUS.lock() {
        m.remove(context);
    }
}

/// A user-entered target, with the scheme and route prefix discovery learned for the same
/// Service port (so testing the prefilled detected endpoint probes it the same way).
pub fn user_target(
    context: Option<&str>,
    namespace: String,
    service: String,
    port: u16,
) -> PromTarget {
    let mut t = PromTarget::saved(namespace, service, port);
    let known = context.and_then(|c| {
        DETECTED_PROMETHEUS.lock().ok().and_then(|m| {
            m.get(c)
                .and_then(|v| v.iter().find(|d| d.same_place(&t)).cloned())
        })
    });
    if let Some(d) = known {
        t.https = d.https;
        t.path_prefix = d.path_prefix;
    }
    t
}

fn saved_target(context: &str) -> Option<PromTarget> {
    IntegrationConfig::load(context)
        .ok()
        .and_then(|cfg| cfg.prometheus)
        .filter(|p| p.enabled)
        .map(|p| user_target(Some(context), p.namespace, p.service, p.port))
}

/// Candidates in priority order: the saved target first, then the best detected ones (skipping
/// one equal to the saved target).
fn prometheus_candidates_for(
    saved: Option<PromTarget>,
    detected: Vec<PromTarget>,
) -> Vec<PromTarget> {
    let mut out: Vec<PromTarget> = saved.into_iter().collect();
    let fresh: Vec<PromTarget> = detected
        .into_iter()
        .filter(|d| !out.iter().any(|s| s.same_place(d)))
        .take(MAX_DETECTED_PROBES)
        .collect();
    out.extend(fresh);
    out
}

/// Resolve the Prometheus to query for `context`: probe the saved target, then the best detected
/// ones (scanning the cluster once if it has not been scanned yet). The answer is cached per
/// context; a miss is cached for [`PROM_NEGATIVE_TTL`].
pub async fn resolve_prometheus(
    context: Option<&str>,
    client: Option<&Client>,
) -> Result<(PromEndpoint, PromTarget), PromFailure> {
    let Some(context) = context else {
        return Err(PromFailure::not_found("No active context", None));
    };
    let Some(client) = client else {
        return Err(PromFailure::not_found("Not connected to a cluster", None));
    };
    let cached = RESOLVED_PROMETHEUS
        .lock()
        .ok()
        .and_then(|m| m.get(context).cloned());
    match cached {
        Some(Resolution::Found(t)) => return Ok((t.endpoint(client), t)),
        Some(Resolution::Missing { at, failure }) if at.elapsed() < PROM_NEGATIVE_TTL => {
            return Err(failure);
        }
        _ => {}
    }

    let scanned = DETECTED_PROMETHEUS
        .lock()
        .ok()
        .and_then(|m| m.get(context).cloned());
    let mut scan_error = None;
    let detected = match scanned {
        Some(d) => d,
        None => match scan_metrics_stack(client).await {
            Ok(scan) => {
                if let Ok(mut m) = DETECTED_PROMETHEUS.lock() {
                    m.insert(context.to_owned(), scan.prometheus.clone());
                }
                scan.prometheus
            }
            Err(e) => {
                tracing::debug!("metrics stack scan failed: {e}");
                scan_error = Some(format!("Cluster scan failed: {e}"));
                Vec::new()
            }
        },
    };

    let mut failures = Vec::new();
    for target in prometheus_candidates_for(saved_target(context), detected) {
        let endpoint = target.endpoint(client);
        match endpoint.probe().await {
            Ok(()) => {
                if let Ok(mut m) = RESOLVED_PROMETHEUS.lock() {
                    m.insert(context.to_owned(), Resolution::Found(target.clone()));
                }
                return Ok((endpoint, target));
            }
            Err(e) => failures.push(target.failure(&e)),
        }
    }
    let failure = PromFailure::combine(failures).unwrap_or_else(|| {
        PromFailure::not_found(
            "No Prometheus found in this cluster. Enter its namespace, service and port below.",
            scan_error,
        )
    });
    if let Ok(mut m) = RESOLVED_PROMETHEUS.lock() {
        m.insert(
            context.to_owned(),
            Resolution::Missing {
                at: std::time::Instant::now(),
                failure: failure.clone(),
            },
        );
    }
    Err(failure)
}

/// Report the resolved Prometheus (or why there is none) to the Metrics pane status line.
pub(super) fn emit_prometheus_status(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    resolved: Result<PromTarget, PromFailure>,
) {
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::global(section_keys::integrations_metrics()),
        Correlation::None,
        BackendEvent::System(prometheus_status_event(resolved)),
    );
}

/// On connect / context switch: scan for a metrics stack and remember what was found, so the
/// first query auto-connects. Never writes settings (a scan used to persist the first
/// "prometheus"-named Service in `monitoring`, which saved Alertmanager as Prometheus).
pub(super) async fn auto_discover_prometheus(
    client: &Client,
    context: &str,
    _store: &dyn SecretStore,
) -> anyhow::Result<()> {
    let scan = scan_metrics_stack(client).await?;
    if let Some(t) = scan.prometheus.first() {
        tracing::info!("Auto-detected Prometheus service: {}", t.label());
    }
    remember_metrics_scan(context, &scan);
    Ok(())
}

#[cfg(test)]
mod metrics_stack_tests {
    use super::*;

    fn target(ns: &str, svc: &str, source: PromSource) -> PromTarget {
        PromTarget {
            source,
            ..PromTarget::saved(ns.to_owned(), svc.to_owned(), 9090)
        }
    }

    #[test]
    fn saved_prometheus_is_tried_before_detected() {
        let saved = target("monitoring", "prom", PromSource::Saved);
        let detected = target(
            "fathom-metrics-prometheus",
            "prometheus",
            PromSource::Detected,
        );
        let order = prometheus_candidates_for(Some(saved.clone()), vec![detected.clone()]);
        assert_eq!(order, vec![saved, detected]);
    }

    #[test]
    fn detected_equal_to_saved_is_not_probed_twice() {
        let saved = target("m", "prometheus", PromSource::Saved);
        let detected = target("m", "prometheus", PromSource::Detected);
        assert_eq!(
            prometheus_candidates_for(Some(saved.clone()), vec![detected]),
            vec![saved]
        );
    }

    #[test]
    fn only_the_best_few_detected_candidates_are_probed_in_rank_order() {
        let detected: Vec<PromTarget> = (0..6)
            .map(|i| target("m", &format!("p{i}"), PromSource::Detected))
            .collect();
        let order = prometheus_candidates_for(None, detected.clone());
        assert_eq!(order, detected[..MAX_DETECTED_PROBES].to_vec());
    }

    #[test]
    fn scan_candidates_carry_scheme_and_prefix() {
        let c = prom_discovery::Candidate {
            namespace: "monitoring".into(),
            service: "prometheus".into(),
            port: 9091,
            https: true,
            path_prefix: "/prom".into(),
            ready: Some(true),
            score: 1,
        };
        let t = PromTarget::from_candidate(&c);
        assert_eq!(t.source, PromSource::Detected);
        assert_eq!(
            t.probe_path(),
            "/api/v1/namespaces/monitoring/services/https:prometheus:9091/proxy/prom/api/v1/query?query=up"
        );
        assert_eq!(
            t.status_message(),
            "Auto-detected Prometheus monitoring/prometheus:9091/prom via the API server proxy"
        );
    }

    #[test]
    fn failures_are_short_and_classified_with_raw_in_detail() {
        let t = target("monitoring", "prom", PromSource::Detected);
        let not_found_page: anyhow::Error =
            prom_discovery::classify_status(404, "404 page not found\n", true).into();
        let f = t.failure(&not_found_page);
        assert_eq!(f.kind, PrometheusStatusKind::QueryFailed);
        assert_eq!(
            f.message,
            "Found Prometheus at monitoring/prom:9090, but the query failed: 404 page not found"
        );
        let detail = f.detail.unwrap();
        assert!(detail.contains("/services/prom:9090/proxy/api/v1/query"));
        assert!(detail.contains("404 page not found"));

        let no_endpoints: anyhow::Error = prom_discovery::classify_status(
            503,
            "no endpoints available for service \"prom:9090\"",
            false,
        )
        .into();
        let f = target("monitoring", "prom", PromSource::Saved).failure(&no_endpoints);
        assert_eq!(f.kind, PrometheusStatusKind::Unreachable);
        assert_eq!(
            f.message,
            "Prometheus monitoring/prom:9090, but it is not reachable: 503 no ready endpoints"
        );
    }

    #[test]
    fn combined_failures_lead_with_the_first_and_keep_every_detail() {
        let t = target("m", "a", PromSource::Detected);
        let e: anyhow::Error = prom_discovery::unreachable("request timed out").into();
        let f = PromFailure::combine(vec![t.failure(&e), t.failure(&e)]).unwrap();
        assert!(f.message.ends_with("(1 other candidate also failed)"));
        assert_eq!(f.detail.unwrap().matches("GET ").count(), 2);
        assert_eq!(PromFailure::combine(vec![]), None);
    }

    #[test]
    fn status_event_carries_kind_and_detail() {
        let SystemEvent::PrometheusConnectionTest {
            success,
            kind,
            detail,
            ..
        } = prometheus_status_event(Err(PromFailure::not_found("none", None)))
        else {
            panic!("wrong event")
        };
        assert!(!success);
        assert_eq!(kind, Some(PrometheusStatusKind::NotFound));
        assert_eq!(detail, None);
    }

    #[test]
    fn classifies_kube_prometheus_stack_names() {
        assert_eq!(
            classify_metrics_service_name("prometheus-kube-prometheus-prometheus"),
            Some(MetricsComponentKind::Prometheus)
        );
        assert_eq!(
            classify_metrics_service_name("prometheus-kube-prometheus-alertmanager"),
            Some(MetricsComponentKind::Alertmanager)
        );
        assert_eq!(
            classify_metrics_service_name("prometheus-kube-state-metrics"),
            Some(MetricsComponentKind::KubeStateMetrics)
        );
        assert_eq!(
            classify_metrics_service_name("prometheus-prometheus-node-exporter"),
            Some(MetricsComponentKind::NodeExporter)
        );
        assert_eq!(
            classify_metrics_service_name("prometheus-grafana"),
            Some(MetricsComponentKind::Grafana)
        );
    }

    #[test]
    fn excludes_operator_and_adapter_services_from_prometheus() {
        assert_eq!(
            classify_metrics_service_name("prometheus-kube-prometheus-operator"),
            None
        );
        assert_eq!(classify_metrics_service_name("prometheus-adapter"), None);
    }

    #[test]
    fn classifies_openobserve() {
        assert_eq!(
            classify_metrics_service_name("openobserve"),
            Some(MetricsComponentKind::OpenObserve)
        );
    }

    #[test]
    fn classifies_metrics_server_and_ignores_unrelated_names() {
        assert_eq!(
            classify_metrics_service_name("metrics-server"),
            Some(MetricsComponentKind::MetricsServer)
        );
        assert_eq!(classify_metrics_service_name("kube-dns"), None);
    }

    fn component(kind: MetricsComponentKind) -> DetectedMetricsComponent {
        DetectedMetricsComponent {
            kind,
            namespace: "monitoring".to_string(),
            service: "svc".to_string(),
            port: default_port_for(kind),
        }
    }

    #[test]
    fn tier_full_requires_prometheus_and_exporters() {
        let full = vec![
            component(MetricsComponentKind::Prometheus),
            component(MetricsComponentKind::KubeStateMetrics),
            component(MetricsComponentKind::NodeExporter),
        ];
        assert_eq!(classify_stack_tier(&full), MetricsStackTier::Full);
    }

    #[test]
    fn tier_partial_when_stack_incomplete() {
        let partial = vec![component(MetricsComponentKind::Prometheus)];
        assert_eq!(classify_stack_tier(&partial), MetricsStackTier::Partial);
    }

    #[test]
    fn tier_openobserve_wins_even_with_other_components_present() {
        let mixed = vec![
            component(MetricsComponentKind::Prometheus),
            component(MetricsComponentKind::OpenObserve),
        ];
        assert_eq!(classify_stack_tier(&mixed), MetricsStackTier::OpenObserve);
    }

    #[test]
    fn tier_none_when_nothing_found() {
        assert_eq!(classify_stack_tier(&[]), MetricsStackTier::None);
    }

    #[test]
    fn suggested_endpoint_prefers_prometheus_over_openobserve() {
        let mixed = vec![
            component(MetricsComponentKind::OpenObserve),
            component(MetricsComponentKind::Prometheus),
        ];
        assert_eq!(
            pick_suggested_endpoint(&mixed).map(|c| c.kind),
            Some(MetricsComponentKind::Prometheus)
        );
    }

    #[test]
    fn suggested_endpoint_falls_back_to_openobserve() {
        let only_oo = vec![component(MetricsComponentKind::OpenObserve)];
        assert_eq!(
            pick_suggested_endpoint(&only_oo).map(|c| c.kind),
            Some(MetricsComponentKind::OpenObserve)
        );
    }

    #[test]
    fn suggested_endpoint_none_when_neither_present() {
        let neither = vec![component(MetricsComponentKind::Grafana)];
        assert!(pick_suggested_endpoint(&neither).is_none());
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;
    use crate::backend::oidc::store::BackendError;
    use crate::backend::secret_store::memory_store;

    const KEY: &str = "glsa_SUPERSECRET_1234";

    fn tmp() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("integrations.json");
        (dir, path)
    }

    fn grafana_of(event: &SystemEvent) -> Option<GrafanaSettings> {
        match event {
            SystemEvent::ObservabilitySettings { grafana, .. } => grafana.clone(),
            other => panic!("unexpected {other:?}"),
        }
    }

    /// A store whose every operation fails, like a locked or absent keyring.
    struct Broken;
    impl SecretStore for Broken {
        fn get(&self, _: &str, _: &str) -> Result<Option<Secret>, BackendError> {
            Err(BackendError("locked".into()))
        }
        fn set(&self, _: &str, _: &str, _: &Secret) -> Result<(), BackendError> {
            Err(BackendError("locked".into()))
        }
        fn delete(&self, _: &str, _: &str) -> Result<(), BackendError> {
            Err(BackendError("locked".into()))
        }
    }

    #[test]
    fn settings_are_isolated_per_context() {
        let (_d, path) = tmp();
        let store = memory_store();
        configure_grafana(&path, "a", &store, "http://a", KEY);
        configure_grafana(&path, "b", &store, "http://b", "");
        let a = grafana_of(&read_settings(&path, "a", &store).event).unwrap();
        let b = grafana_of(&read_settings(&path, "b", &store).event).unwrap();
        assert_eq!((a.base_url.as_str(), a.key_set), ("http://a", true));
        assert_eq!((b.base_url.as_str(), b.key_set), ("http://b", false));
        assert!(grafana_of(&read_settings(&path, "c", &store).event).is_none());

        let mut cfg = IntegrationConfig::load_from(&path, "a").unwrap();
        cfg.prometheus = Some(PrometheusConfig {
            enabled: true,
            namespace: "m".into(),
            service: "p".into(),
            port: 9090,
        });
        cfg.save_to(&path).unwrap();
        let b_cfg = IntegrationConfig::load_from(&path, "b").unwrap();
        assert!(b_cfg.prometheus.is_none());
        let a_cfg = IntegrationConfig::load_from(&path, "a").unwrap();
        assert!(a_cfg.prometheus.is_some());
    }

    #[test]
    fn legacy_file_loads_for_its_context_only() {
        let (_d, path) = tmp();
        std::fs::write(
            &path,
            r#"{"context":"old","prometheus":{"enabled":true,"namespace":"m","service":"p","port":9091},"grafana":null,"argo":null,"flux":null}"#,
        )
        .unwrap();
        let old = IntegrationConfig::load_from(&path, "old").unwrap();
        assert_eq!(old.prometheus.unwrap().port, 9091);
        let other = IntegrationConfig::load_from(&path, "other").unwrap();
        assert!(other.prometheus.is_none());
        // Saving another context keeps the legacy record readable.
        let mut other = other;
        other.grafana = Some(GrafanaConfig {
            enabled: true,
            base_url: "http://g".into(),
            api_key: String::new(),
        });
        other.save_to(&path).unwrap();
        let old = IntegrationConfig::load_from(&path, "old").unwrap();
        assert_eq!(old.prometheus.unwrap().port, 9091);
    }

    #[test]
    fn corrupt_file_falls_back_without_deletion() {
        let (_d, path) = tmp();
        std::fs::write(&path, "{ not json").unwrap();
        let cfg = IntegrationConfig::load_from(&path, "a").unwrap();
        assert!(cfg.prometheus.is_none() && cfg.grafana.is_none());
        assert!(cfg.save_to(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
        let out = configure_grafana(&path, "a", &memory_store(), "http://g", KEY);
        assert!(out.error.is_some());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    }

    #[test]
    fn legacy_plaintext_key_migrates_and_is_stripped() {
        let (_d, path) = tmp();
        let text = format!(
            r#"{{"context":"a","prometheus":null,"grafana":{{"enabled":true,"base_url":"http://g","api_key":"{KEY}"}},"argo":null,"flux":null}}"#
        );
        std::fs::write(&path, text).unwrap();
        let store = memory_store();
        let out = read_settings(&path, "a", &store);
        assert!(grafana_of(&out.event).unwrap().key_set);
        let stored = store.get(GRAFANA_SERVICE, "a").unwrap().unwrap();
        assert_eq!(stored.expose(), KEY);
        assert!(!std::fs::read_to_string(&path).unwrap().contains(KEY));
    }

    #[test]
    fn legacy_key_stays_in_file_if_store_fails() {
        let (_d, path) = tmp();
        let text = format!(
            r#"{{"context":"a","grafana":{{"enabled":true,"base_url":"http://g","api_key":"{KEY}"}}}}"#
        );
        std::fs::write(&path, &text).unwrap();
        read_settings(&path, "a", &Broken);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }

    #[test]
    fn no_secret_in_serialized_output() {
        let (_d, path) = tmp();
        let store = memory_store();
        let out = configure_grafana(&path, "a", &store, "http://g", KEY);
        assert!(!std::fs::read_to_string(&path).unwrap().contains(KEY));
        let cfg = IntegrationConfig::load_from(&path, "a").unwrap();
        assert!(!serde_json::to_string(&cfg).unwrap().contains(KEY));
        assert!(!format!("{:?}", out.event).contains(KEY));
    }

    #[test]
    fn blank_key_keeps_and_clear_removes() {
        let (_d, path) = tmp();
        let store = memory_store();
        configure_grafana(&path, "a", &store, "http://g", KEY);
        let out = configure_grafana(&path, "a", &store, "http://g2", "");
        let g = grafana_of(&out.event).unwrap();
        assert_eq!((g.base_url.as_str(), g.key_set), ("http://g2", true));
        let out = clear_grafana_key(&path, "a", &store);
        assert!(out.error.is_none());
        let g = grafana_of(&out.event).unwrap();
        assert_eq!((g.base_url.as_str(), g.key_set), ("http://g2", false));
    }

    /// A store that accepts writes and returns nothing, like the keyring crate's mock.
    struct Dropping;
    impl SecretStore for Dropping {
        fn get(&self, _: &str, _: &str) -> Result<Option<Secret>, BackendError> {
            Ok(None)
        }
        fn set(&self, _: &str, _: &str, _: &Secret) -> Result<(), BackendError> {
            Ok(())
        }
        fn delete(&self, _: &str, _: &str) -> Result<(), BackendError> {
            Ok(())
        }
    }

    #[test]
    fn silently_dropping_store_is_a_failure_without_the_key() {
        let (_d, path) = tmp();
        let out = configure_grafana(&path, "a", &Dropping, "http://g", KEY);
        let g = grafana_of(&out.event).unwrap();
        assert_eq!((g.base_url.as_str(), g.key_set), ("http://g", false));
        let err = out.error.expect("error notification");
        assert!(!err.contains(KEY));
    }

    #[test]
    fn keyring_failure_keeps_url_reports_unset_and_errors_without_the_key() {
        let (_d, path) = tmp();
        let out = configure_grafana(&path, "a", &Broken, "http://g", KEY);
        let g = grafana_of(&out.event).unwrap();
        assert_eq!((g.base_url.as_str(), g.key_set), ("http://g", false));
        let err = out.error.expect("error notification");
        assert!(!err.contains(KEY));
        let reread = read_settings(&path, "a", &Broken);
        assert_eq!(grafana_of(&reread.event).unwrap().base_url, "http://g");
    }

    /// Captures everything `tracing` emits while Grafana saves run.
    #[derive(Clone, Default)]
    struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn logs_never_contain_the_key() {
        let (_d, path) = tmp();
        let cap = Capture::default();
        let sub = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(cap.clone())
            .finish();
        tracing::subscriber::with_default(sub, || {
            configure_grafana(&path, "a", &memory_store(), "http://g", KEY);
            configure_grafana(&path, "a", &Broken, "http://g", KEY);
            read_settings(&path, "a", &Broken);
        });
        let logged = String::from_utf8(cap.0.lock().unwrap().clone()).unwrap();
        assert!(!logged.contains(KEY), "{logged}");
    }
}
