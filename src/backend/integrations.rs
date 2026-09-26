//! External integrations: Prometheus, Grafana, ArgoCD, FluxCD
//!
//! This module manages configuration and connections to external services.

use super::longhorn;
use super::persistence;
use crate::backend::events;
use crate::backend::oidc::store::Secret;
use crate::backend::portforward;
use crate::backend::secret_store::{GRAFANA_SERVICE, SecretStore, grafana_key_set};
use crate::backend::state::BackendState;
use crate::protocol::BackendEventEnvelope;
use crate::protocol::{
    BackendEvent, Correlation, DetectedMetricsComponent, DiskUsage, MetricsComponentKind,
    MetricsStackScanResult, MetricsStackTier, Route, SystemEvent, section_keys,
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
    if store
        .set(
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
        && let Err(e) = store.set(GRAFANA_SERVICE, context, &Secret::new(api_key))
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
pub async fn query_prometheus(
    base_url: &str,
    query: &str,
    _step: &str,
) -> Result<serde_json::Value> {
    // Simple instant query against Prometheus HTTP API
    let client = reqwest::Client::new();
    let url = format!(
        "{}/api/v1/query?query={}",
        base_url.trim_end_matches('/'),
        urlencoding::encode(query)
    );
    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await?
        .error_for_status()?;
    let json: serde_json::Value = resp.json().await?;
    Ok(json)
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

/// Compose a Prometheus base URL from live backend state, delegating the
/// selection logic to the testable [`prometheus_base_url_from_parts`].
pub(super) fn prometheus_base_url(state: &BackendState) -> Option<String> {
    let prom_client_url = state
        .metrics
        .prometheus_client
        .as_ref()
        .map(|p| p.url.as_str());

    let port_forwards: Vec<(u16, u16)> = state
        .port_forward_manager
        .list()
        .into_iter()
        .map(|pf| (pf.local_port, pf.remote_port))
        .collect();

    let configured_prom_port = state
        .cluster
        .current_context
        .as_deref()
        .and_then(|ctx| IntegrationConfig::load(ctx).ok())
        .and_then(|cfg| cfg.prometheus.map(|p| p.port));

    prometheus_base_url_from_parts(prom_client_url, &port_forwards, configured_prom_port)
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

    // Check prometheus configuration (context-aware)
    if let Some(ctx) = context
        && let Ok(cfg) = IntegrationConfig::load(ctx)
        && cfg.prometheus.as_ref().filter(|p| p.enabled).is_some()
    {
        return DiskUsageProvider::Prometheus;
    }

    // Fallback: try localhost:9090
    if portforward::is_local_port_available(9090) {
        return DiskUsageProvider::Prometheus;
    }

    DiskUsageProvider::None
}

/// Fetch disk usage using the detected provider and emit a System event with the results
pub(super) async fn fetch_disk_usage(
    client: &Client,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    // Capture the context at request start so a late result keeps its origin (FR-016).
    let context = state.cluster.current_context.clone();
    let provider = detect_disk_provider(client, context.as_deref()).await;
    let route = Route::cluster(section_keys::integrations_metrics(), context);

    match provider {
        DiskUsageProvider::Longhorn => {
            let vols = longhorn::list_longhorn_volumes(client)
                .await
                .unwrap_or_default();
            events::emit(
                state.cluster.seq.as_ref(),
                sender,
                route,
                Correlation::None,
                BackendEvent::System(SystemEvent::DiskUsageUpdated(vols)),
            );
        }
        DiskUsageProvider::Prometheus => {
            // Basic query for persistent volume claim usage available from kubelet metrics
            // This will return time series with labels `persistentvolumeclaim` and `namespace`
            let base =
                prometheus_base_url(state).unwrap_or_else(|| "http://localhost:9090".to_string());
            let query = "kubelet_volume_stats_used_bytes";
            if let Ok(json) = query_prometheus(&base, query, "").await
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
                    state.cluster.seq.as_ref(),
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
                state.cluster.seq.as_ref(),
                sender,
                "No disk usage provider detected (no Longhorn CRDs and no Prometheus).".to_owned(),
            );
        }
    }

    Ok(())
}
// --- Prometheus auto-discovery and port-forward helpers extracted from backend.rs ---

pub(super) async fn ensure_metrics_port_forward(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state
        .cluster
        .client
        .clone()
        .ok_or_else(|| anyhow::anyhow!("No Kubernetes client"))?;

    for (namespace, service, port) in prometheus_candidates(state) {
        // Check if a pod matching the service exists
        match find_pod_for_service(&client, &namespace, &service).await {
            Ok(Some(pod_name)) => {
                let target = ForwardTarget {
                    namespace: &namespace,
                    pod_name: &pod_name,
                    port,
                    verb: "Auto-started",
                };
                if start_prometheus_forward(state, sender, &client, target).await {
                    return Ok(());
                }
            }
            Ok(None) => {
                tracing::debug!("No pod found for service {}/{}", namespace, service);
            }
            Err(e) => {
                tracing::debug!("Error finding pod for {}/{}: {}", namespace, service, e);
            }
        }
    }

    // Fallback: Check if port 9090 is available locally and try to find any Prometheus instance
    if portforward::is_local_port_available(9090) {
        // Try to find any Prometheus pod in common namespaces
        for namespace in &["monitoring", "prometheus", "kube-monitoring", "default"] {
            if let Ok(Some(pod_name)) = find_any_prometheus_pod(&client, namespace).await {
                let target = ForwardTarget {
                    namespace,
                    pod_name: &pod_name,
                    port: 9090,
                    verb: "Auto-discovered",
                };
                if start_prometheus_forward(state, sender, &client, target).await {
                    return Ok(());
                }
            }
        }
    }

    // Attempt to fetch disk usage after auto-discovery/port-forward attempts.
    let _ = fetch_disk_usage(&client, state, sender).await;

    events::notify_info(
        state.cluster.seq.as_ref(),
        sender,
        "Prometheus auto port-forward not started (no matching service/pod found). Configure a port forward under Network > Port Forwards or adjust Prometheus settings under Integrations.".to_owned(),
    );

    Ok(())
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

/// Scan every namespace for known metrics-stack Services (Prometheus, Grafana,
/// Alertmanager, kube-state-metrics, node-exporter, OpenObserve, metrics-server)
/// and classify how complete the stack is.
pub(super) async fn scan_metrics_stack(client: &Client) -> anyhow::Result<MetricsStackScanResult> {
    let svc_api: Api<Service> = Api::all(client.clone());
    let services = svc_api.list(&ListParams::default()).await?;

    let mut components: Vec<DetectedMetricsComponent> = Vec::new();

    for svc in services.items {
        let Some(name) = svc.metadata.name.clone() else {
            continue;
        };
        let Some(kind) = classify_metrics_service_name(&name) else {
            continue;
        };
        let Some(namespace) = svc.metadata.namespace.clone() else {
            continue;
        };

        let port = svc
            .spec
            .as_ref()
            .and_then(|s| s.ports.as_ref())
            .and_then(|ports| ports.first())
            .map(|p| p.port as u16)
            .unwrap_or_else(|| default_port_for(kind));

        components.push(DetectedMetricsComponent {
            kind,
            namespace,
            service: name,
            port,
        });
    }

    let tier = classify_stack_tier(&components);
    let suggested_endpoint = pick_suggested_endpoint(&components);

    Ok(MetricsStackScanResult {
        tier,
        components,
        suggested_endpoint,
    })
}

pub(super) async fn auto_discover_prometheus(client: &Client, context: &str) -> anyhow::Result<()> {
    use ::kube::Api;
    use k8s_openapi::api::core::v1::Service;

    let svc_api = Api::<Service>::namespaced(client.clone(), "monitoring");
    match svc_api.list(&Default::default()).await {
        Ok(services) => {
            // Look for prometheus-k8s or prometheus-stack-kube-prom-prometheus
            for svc in services.items {
                if let Some(name) = svc.metadata.name.as_ref()
                    && name.contains("prometheus")
                    && !name.contains("operator")
                    && !name.contains("adapter")
                {
                    // Found a viable Prometheus service
                    tracing::info!("Auto-discovered Prometheus service: monitoring/{}", name);

                    // Update or create integration config
                    if let Ok(mut cfg) = IntegrationConfig::load(context) {
                        cfg.prometheus = Some(PrometheusConfig {
                            enabled: true,
                            namespace: "monitoring".to_string(),
                            service: name.clone(),
                            port: 9090,
                        });
                        let _ = cfg.save();
                        return Ok(());
                    }
                }
            }
        }
        Err(e) => {
            tracing::debug!("Failed to list services in monitoring namespace: {}", e);
        }
    }
    Ok(())
}

#[cfg(test)]
mod metrics_stack_tests {
    use super::*;

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
