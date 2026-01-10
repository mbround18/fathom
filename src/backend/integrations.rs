//! External integrations: Prometheus, Grafana, ArgoCD, FluxCD
//!
//! This module manages configuration and connections to external services.

use super::longhorn;
use super::persistence;
use crate::backend::events;
use crate::backend::portforward;
use crate::backend::state::BackendState;
use crate::protocol::BackendEventEnvelope;
use crate::protocol::{BackendEvent, Correlation, DiskUsage, Route, SystemEvent, section_keys};
use anyhow::Result;
use crossbeam_channel::Sender;
use kube::Client;
use kube::api::Api;
use kube::api::ListParams;
use kube::core::{ApiResource, GroupVersionKind};
use serde::{Deserialize, Serialize};

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
    pub api_key: String, // Stored encrypted
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationConfig {
    pub context: String,
    pub prometheus: Option<PrometheusConfig>,
    pub grafana: Option<GrafanaConfig>,
    pub argo: Option<ArgoConfig>,
    pub flux: Option<FluxConfig>,
}

impl IntegrationConfig {
    /// Load integrations config from ~/.fathom/integrations.json
    pub fn load(context: &str) -> Result<Self> {
        match persistence::load_integrations() {
            Ok(json) => {
                if let Ok(config) = serde_json::from_value::<IntegrationConfig>(json) {
                    Ok(config)
                } else {
                    // If parsing fails, return new default config
                    Ok(IntegrationConfig {
                        context: context.to_string(),
                        prometheus: None,
                        grafana: None,
                        argo: None,
                        flux: None,
                    })
                }
            }
            Err(_) => {
                // File doesn't exist, return new default config
                Ok(IntegrationConfig {
                    context: context.to_string(),
                    prometheus: None,
                    grafana: None,
                    argo: None,
                    flux: None,
                })
            }
        }
    }

    /// Save integrations config to ~/.fathom/integrations.json
    pub fn save(&self) -> Result<()> {
        let json = serde_json::to_value(self)?;
        persistence::save_integrations(&json)?;
        Ok(())
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
            if let Some(name) = crd.metadata.name {
                if name == "volumes.longhorn.io" {
                    return DiskUsageProvider::Longhorn;
                }
            }
        }
    }

    // Check prometheus configuration (context-aware)
    if let Some(ctx) = context {
        if let Ok(cfg) = IntegrationConfig::load(ctx) {
            if cfg.prometheus.as_ref().filter(|p| p.enabled).is_some() {
                return DiskUsageProvider::Prometheus;
            }
        }
    }

    // Fallback: try localhost:9090
    if portforward::is_local_port_available(9090) {
        return DiskUsageProvider::Prometheus;
    }

    DiskUsageProvider::None
}

/// Resolve a best-effort Prometheus base URL for local queries.
/// Priority:
///  1. Use configured `state.prometheus_client.url` if present
///  2. Use any active port-forward mapping to port 9090 (or local port mapped to it)
///  3. Use configured IntegrationConfig prometheus port (assume local port)
/// Returns None if no reasonable base URL could be determined.
/// Public helper that resolves a Prometheus base URL from explicit parts. This is used by unit tests to avoid needing live state.
/// Precedence: prom_client_url > any port-forward mapping to remote 9090 (returns local port) > prometheus config port
pub fn prometheus_base_url_from_parts(
    prom_client_url: Option<&str>,
    port_forwards: &[(u16, u16)], // (local_port, remote_port)
    configured_prometheus_port: Option<u16>,
) -> Option<String> {
    // 1) explicit client URL
    if let Some(url) = prom_client_url {
        return Some(url.to_string());
    }

    // 2) port forward mapping
    for (local, remote) in port_forwards {
        if *remote == 9090 || *local == 9090 {
            return Some(format!("http://127.0.0.1:{}", local));
        }
    }

    // 3) configured prometheus port
    if let Some(p) = configured_prometheus_port {
        return Some(format!("http://127.0.0.1:{}", p));
    }

    None
}

pub(super) fn prometheus_base_url(state: &BackendState) -> Option<String> {
    // Compose parts from state and delegate to the testable helper
    let prom_client_url = state.prometheus_client.as_ref().map(|p| p.url.as_str());

    let port_forwards: Vec<(u16, u16)> = state
        .port_forward_manager
        .list()
        .into_iter()
        .map(|pf| (pf.local_port, pf.remote_port))
        .collect();

    let configured_prom_port = state
        .current_context
        .as_deref()
        .and_then(|ctx| IntegrationConfig::load(ctx).ok())
        .and_then(|cfg| cfg.prometheus.map(|p| p.port));

    prometheus_base_url_from_parts(prom_client_url, &port_forwards, configured_prom_port)
}

/// Fetch disk usage using the detected provider and emit a System event with the results
pub async fn fetch_disk_usage(
    client: &Client,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let context = state.current_context.as_deref();
    let provider = detect_disk_provider(client, context).await;

    match provider {
        DiskUsageProvider::Longhorn => {
            let vols = longhorn::list_longhorn_volumes(client)
                .await
                .unwrap_or_default();
            events::emit(
                state.seq.as_ref(),
                sender,
                Route::global(section_keys::integrations_metrics()),
                Correlation::None,
                BackendEvent::System(SystemEvent::DiskUsageUpdated(vols)),
            );
        }
        DiskUsageProvider::Prometheus => {
            // Basic query for persistent volume claim usage available from kubelet metrics
            // This will return time series with labels `persistentvolumeclaim` and `namespace`
            let base =
                prometheus_base_url(state).unwrap_or_else(|| "http://127.0.0.1:9090".to_string());
            let query = "kubelet_volume_stats_used_bytes";
            if let Ok(json) = query_prometheus(&base, query, "").await {
                if let Some(results) = json
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
                        state.seq.as_ref(),
                        sender,
                        Route::global(section_keys::integrations_metrics()),
                        Correlation::None,
                        BackendEvent::System(SystemEvent::DiskUsageUpdated(disk_items)),
                    );
                }
            }
        }
        DiskUsageProvider::None => {
            // Nothing available; emit info to user
            events::notify_info(
                state.seq.as_ref(),
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
        .client
        .clone()
        .ok_or_else(|| anyhow::anyhow!("No Kubernetes client"))?;

    // Load user-configured Prometheus first (per context), then fall back to common defaults
    let context = state
        .current_context
        .clone()
        .unwrap_or_else(|| "default".to_owned());

    let mut prometheus_configs: Vec<(String, String, u16)> = Vec::new();

    if let Ok(cfg) = IntegrationConfig::load(&context) {
        if let Some(prom) = cfg.prometheus.as_ref().filter(|p| p.enabled) {
            prometheus_configs.push((prom.namespace.clone(), prom.service.clone(), prom.port));
        }
    }

    // Try common Prometheus locations
    prometheus_configs.extend_from_slice(&[
        ("monitoring".to_owned(), "prometheus".to_owned(), 9090u16),
        ("prometheus".to_owned(), "prometheus".to_owned(), 9090),
        ("kube-monitoring".to_owned(), "prometheus".to_owned(), 9090),
    ]);

    for (namespace, service, port) in prometheus_configs {
        // Check if a pod matching the service exists
        match find_pod_for_service(&client, &namespace, &service).await {
            Ok(Some(pod_name)) => {
                // Check if a port forward for this pod already exists
                let existing = state.port_forward_manager.list().iter().any(|pf| {
                    pf.namespace == namespace && pf.pod == pod_name && pf.local_port == port
                });

                if existing {
                    tracing::debug!(
                        "Port forward for {}/{} on port {} already exists, skipping",
                        namespace,
                        pod_name,
                        port
                    );
                    return Ok(());
                }

                match state
                    .port_forward_manager
                    .start(&client, &namespace, &pod_name, port, port, None)
                    .await
                {
                    Ok(_) => {
                        events::notify_info(
                            state.seq.as_ref(),
                            sender,
                            format!(
                                "Auto-started Prometheus port forward: localhost:{} -> {}/{}:{}",
                                port, namespace, pod_name, port
                            ),
                        );
                        // Emit updated list and save to disk
                        portforward::emit_port_forward_list(state, sender);
                        portforward::save_port_forwards_to_disk(state);
                        return Ok(());
                    }
                    Err(e) => {
                        tracing::debug!(
                            "Failed to start port forward for {}/{}: {}",
                            namespace,
                            pod_name,
                            e
                        );
                    }
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
                // Check if a port forward for this pod already exists
                let existing = state.port_forward_manager.list().iter().any(|pf| {
                    pf.namespace == *namespace && pf.pod == pod_name && pf.local_port == 9090
                });

                if existing {
                    tracing::debug!(
                        "Port forward for {}/{} on port 9090 already exists, skipping",
                        namespace,
                        pod_name
                    );
                    return Ok(());
                }

                match state
                    .port_forward_manager
                    .start(&client, namespace, &pod_name, 9090, 9090, None)
                    .await
                {
                    Ok(_) => {
                        events::notify_info(
                            state.seq.as_ref(),
                            sender,
                            format!(
                                "Auto-discovered Prometheus port forward: localhost:9090 -> {}/{}:9090",
                                namespace, pod_name
                            ),
                        );
                        // Emit updated list and save to disk
                        portforward::emit_port_forward_list(state, sender);
                        portforward::save_port_forwards_to_disk(state);
                        return Ok(());
                    }
                    Err(e) => {
                        tracing::debug!(
                            "Failed to start port forward for {}/{}: {}",
                            namespace,
                            pod_name,
                            e
                        );
                    }
                }
            }
        }
    }

    // Attempt to fetch disk usage after auto-discovery/port-forward attempts.
    let _ = fetch_disk_usage(&client, state, sender).await;

    events::notify_info(
        state.seq.as_ref(),
        sender,
        "Prometheus auto port-forward not started (no matching service/pod found). Configure a port forward under Network > Port Forwards or adjust Prometheus settings under Integrations.".to_owned(),
    );

    Ok(())
}

pub(super) async fn find_pod_for_service(
    client: &Client,
    namespace: &str,
    service: &str,
) -> anyhow::Result<Option<String>> {
    use ::kube::Api;
    use k8s_openapi::api::core::v1::Service;

    let svc_api = Api::<Service>::namespaced(client.clone(), namespace);

    match svc_api.get(service).await {
        Ok(svc) => {
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

                match pod_api.list(&list_params).await {
                    Ok(pods) => {
                        if let Some(pod) = pods.items.first() {
                            if let Some(name) = pod.metadata.name.as_ref() {
                                return Ok(Some(name.clone()));
                            }
                        }
                    }
                    Err(_) => {}
                }
            }
        }
        Err(_) => {}
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
    if let Ok(eps) = ep_api.get(service).await {
        if let Some(subsets) = eps.subsets {
            for subset in subsets {
                // Prefer ready addresses, fallback to not_ready_addresses
                let candidates = subset
                    .addresses
                    .or(subset.not_ready_addresses)
                    .unwrap_or_default();
                for addr in candidates {
                    if let Some(target_ref) = addr.target_ref {
                        if target_ref.kind.as_deref() == Some("Pod") {
                            if let Some(name) = target_ref.name {
                                return Ok(Some(name));
                            }
                        }
                    }
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
                if let Some(name) = pod.metadata.name.as_ref() {
                    if name.to_lowercase().contains("prometheus") {
                        return Ok(Some(name.clone()));
                    }
                }
            }
            Ok(None)
        }
        Err(_) => Ok(None),
    }
}

pub(super) async fn auto_discover_prometheus(client: &Client, context: &str) -> anyhow::Result<()> {
    use ::kube::Api;
    use k8s_openapi::api::core::v1::Service;

    let svc_api = Api::<Service>::namespaced(client.clone(), "monitoring");
    match svc_api.list(&Default::default()).await {
        Ok(services) => {
            // Look for prometheus-k8s or prometheus-stack-kube-prom-prometheus
            for svc in services.items {
                if let Some(name) = svc.metadata.name.as_ref() {
                    if name.contains("prometheus")
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
        }
        Err(e) => {
            tracing::debug!("Failed to list services in monitoring namespace: {}", e);
        }
    }
    Ok(())
}
