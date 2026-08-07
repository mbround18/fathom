//! External integrations: Prometheus, Grafana, ArgoCD, FluxCD
//!
//! This module manages configuration and connections to external services.

use super::longhorn;
use super::persistence;
use crate::backend::events;
use crate::backend::portforward;
use crate::backend::state::BackendState;
use crate::protocol::BackendEventEnvelope;
use crate::protocol::{
    BackendEvent, Correlation, DetectedMetricsComponent, DiskUsage, MetricsComponentKind,
    MetricsStackScanResult, MetricsStackTier, Route, SystemEvent, section_keys,
};
use anyhow::Result;
use crossbeam_channel::Sender;
use k8s_openapi::api::core::v1::Service;
use kube::Client;
use kube::api::Api;
use kube::api::ListParams;
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
    let context = state.cluster.current_context.as_deref();
    let provider = detect_disk_provider(client, context).await;

    match provider {
        DiskUsageProvider::Longhorn => {
            let vols = longhorn::list_longhorn_volumes(client)
                .await
                .unwrap_or_default();
            events::emit(
                state.cluster.seq.as_ref(),
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
                    Route::global(section_keys::integrations_metrics()),
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
                            state.cluster.seq.as_ref(),
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
                            state.cluster.seq.as_ref(),
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
        state.cluster.seq.as_ref(),
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
