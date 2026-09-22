use crate::protocol::{
    BackendEvent, BackendEventEnvelope, ContainerMetrics, ContainerMetricsAggregate,
    ContainerMetricsHistory, ContainerResourceSpec, Correlation, MetricsSample, PodMetricsHistory,
    PodMetricsSummary, ResourceReadEvent, Route, WorkloadMetricsAggregate, section_keys,
};
use crossbeam_channel::Sender;
use kube::api::ListParams;
use kube::core::DynamicObject;
use kube::{
    Client,
    api::Api,
    core::{ApiResource, GroupVersionKind},
};
use std::time::Duration;

use super::events;
use super::state::{BackendState, MetricsAvailability};
use crate::backend::integrations::IntegrationConfig;
use crate::backend::persistence;
use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::Deserialize;

pub(super) async fn probe_metrics_api(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    if state.metrics.availability != MetricsAvailability::Unknown {
        return Ok(());
    }

    let client = state.cluster.client_or_err()?;
    let availability = detect_metrics_api(&client).await;
    state.metrics.availability = availability;

    match availability {
        MetricsAvailability::Available => {
            // No UI noise needed.
        }
        MetricsAvailability::Unavailable => {
            if !state.metrics.unavailable_notified {
                state.metrics.unavailable_notified = true;
                events::notify_info(
                    state.cluster.seq.as_ref(),
                    sender,
                    "Metrics API not detected in this cluster (metrics-server missing?)."
                        .to_owned(),
                );
            }
        }
        MetricsAvailability::Forbidden => {
            if !state.metrics.forbidden_notified {
                state.metrics.forbidden_notified = true;
                events::notify_info(
                    state.cluster.seq.as_ref(),
                    sender,
                    "Metrics API detected but access is forbidden (RBAC).".to_owned(),
                );
            }
        }
        MetricsAvailability::Unknown => {}
    }

    Ok(())
}

async fn detect_metrics_api(client: &Client) -> MetricsAvailability {
    // Discovery-by-use: attempt a cheap list against the PodMetrics resource.
    // - 404 => metrics API group not served
    // - 403 => served but not permitted
    let gvk = GroupVersionKind::gvk("metrics.k8s.io", "v1beta1", "PodMetrics");
    let resource = ApiResource::from_gvk(&gvk);
    let api: Api<DynamicObject> = Api::all_with(client.clone(), &resource);

    match api.list(&ListParams::default().limit(1)).await {
        Ok(_) => MetricsAvailability::Available,
        Err(kube::Error::Api(ae)) if ae.code == 404 => MetricsAvailability::Unavailable,
        Err(kube::Error::Api(ae)) if ae.code == 403 => MetricsAvailability::Forbidden,
        Err(_) => MetricsAvailability::Unknown,
    }
}

pub(super) async fn fetch_metrics(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    // Try Prometheus first if configured
    if let Some(prom_client) = &state.metrics.prometheus_client {
        if let Ok(metrics) =
            super::metrics_provider::query_pod_metrics(prom_client, namespace, name).await
        {
            let route = Route::namespaced(
                section_keys::workloads_pods(),
                state.cluster.current_context.clone(),
                Some(namespace.to_owned()),
            );
            events::emit(
                state.cluster.seq.as_ref(),
                sender,
                route,
                Correlation::None,
                BackendEvent::ResourceRead(ResourceReadEvent::PrometheusMetricsUpdated(metrics)),
            );
            return Ok(());
        } else {
            tracing::debug!(
                "Prometheus metrics fetch failed, falling back to Kubernetes metrics API"
            );
        }
    }

    // Skip metrics fetch if API is unavailable
    if state.metrics.availability == MetricsAvailability::Unavailable
        || state.metrics.availability == MetricsAvailability::Forbidden
    {
        tracing::debug!("Skipping metrics fetch: metrics API unavailable");
        return Ok(());
    }

    // Ensure we do at least a best-effort discovery per context.
    probe_metrics_api(state, sender).await?;

    match state.metrics.availability {
        MetricsAvailability::Available => {}
        MetricsAvailability::Unavailable => return Ok(()),
        MetricsAvailability::Forbidden => return Ok(()),
        MetricsAvailability::Unknown => {
            // If discovery failed in an unexpected way, still attempt and let errors bubble once.
        }
    }

    let client = state.cluster.client_or_err()?;

    let gvk = GroupVersionKind::gvk("metrics.k8s.io", "v1beta1", "PodMetrics");
    let resource = ApiResource::from_gvk(&gvk);
    let api: Api<DynamicObject> = Api::namespaced_with(client, namespace, &resource);

    let obj = api.get(name).await?;
    let data = obj.data;

    let mut containers = Vec::new();
    if let Some(serde_json::Value::Array(items)) = data.get("containers") {
        for item in items {
            let cname = item
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();

            let cpu = item
                .get("usage")
                .and_then(|u| u.get("cpu"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();

            let memory = item
                .get("usage")
                .and_then(|u| u.get("memory"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();

            containers.push(ContainerMetrics {
                name: cname,
                cpu,
                memory,
            });
        }
    }

    let metrics = PodMetricsSummary {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        containers,
    };

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::workloads_pods(),
            state.cluster.current_context.clone(),
            Some(namespace.to_owned()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::PodMetricsUpdated(metrics)),
    );
    Ok(())
}

/// Parse Kubernetes CPU quantity string to millicores.
/// Examples: "15m" => 15.0, "1500n" => 0.0015, "2" => 2000.0
pub(super) fn parse_cpu_quantity(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Parse suffix
    if let Some(stripped) = s.strip_suffix('n') {
        // nanocores: divide by 1,000,000
        stripped.parse::<f64>().ok().map(|v| v / 1_000_000.0)
    } else if let Some(stripped) = s.strip_suffix('u') {
        // microcores: divide by 1,000
        stripped.parse::<f64>().ok().map(|v| v / 1_000.0)
    } else if let Some(stripped) = s.strip_suffix('m') {
        // millicores: direct
        stripped.parse::<f64>().ok()
    } else {
        // cores: multiply by 1000
        s.parse::<f64>().ok().map(|v| v * 1000.0)
    }
}

/// Parse Kubernetes memory quantity string to bytes.
/// Examples: "128Mi" => 134217728.0, "1Gi" => 1073741824.0, "1000" => 1000.0
pub(super) fn parse_memory_quantity(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Binary suffixes (Ki, Mi, Gi, Ti, Pi, Ei)
    if let Some(stripped) = s.strip_suffix("Ei") {
        stripped.parse::<f64>().ok().map(|v| v * 1024.0_f64.powi(6))
    } else if let Some(stripped) = s.strip_suffix("Pi") {
        stripped.parse::<f64>().ok().map(|v| v * 1024.0_f64.powi(5))
    } else if let Some(stripped) = s.strip_suffix("Ti") {
        stripped.parse::<f64>().ok().map(|v| v * 1024.0_f64.powi(4))
    } else if let Some(stripped) = s.strip_suffix("Gi") {
        stripped.parse::<f64>().ok().map(|v| v * 1024.0_f64.powi(3))
    } else if let Some(stripped) = s.strip_suffix("Mi") {
        stripped.parse::<f64>().ok().map(|v| v * 1024.0_f64.powi(2))
    } else if let Some(stripped) = s.strip_suffix("Ki") {
        stripped.parse::<f64>().ok().map(|v| v * 1024.0)
    }
    // Decimal suffixes (k, M, G, T, P, E)
    else if let Some(stripped) = s.strip_suffix('E') {
        stripped.parse::<f64>().ok().map(|v| v * 1e18)
    } else if let Some(stripped) = s.strip_suffix('P') {
        stripped.parse::<f64>().ok().map(|v| v * 1e15)
    } else if let Some(stripped) = s.strip_suffix('T') {
        stripped.parse::<f64>().ok().map(|v| v * 1e12)
    } else if let Some(stripped) = s.strip_suffix('G') {
        stripped.parse::<f64>().ok().map(|v| v * 1e9)
    } else if let Some(stripped) = s.strip_suffix('M') {
        stripped.parse::<f64>().ok().map(|v| v * 1e6)
    } else if let Some(stripped) = s.strip_suffix('k') {
        stripped.parse::<f64>().ok().map(|v| v * 1e3)
    }
    // Millis, micros, nanos (uncommon for memory but handle anyway)
    else if let Some(stripped) = s.strip_suffix('m') {
        stripped.parse::<f64>().ok().map(|v| v / 1e3)
    } else if let Some(stripped) = s.strip_suffix('u') {
        stripped.parse::<f64>().ok().map(|v| v / 1e6)
    } else if let Some(stripped) = s.strip_suffix('n') {
        stripped.parse::<f64>().ok().map(|v| v / 1e9)
    }
    // Plain number (bytes)
    else {
        s.parse::<f64>().ok()
    }
}

pub(super) async fn start_metrics_polling(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: String,
    name: String,
    resource_specs: Vec<ContainerResourceSpec>,
    interval_secs: u64,
) -> anyhow::Result<()> {
    // Stop existing polling task if any
    stop_metrics_polling(state);

    // Try Prometheus first if configured for this context
    let prom_config = enabled_prometheus(state.cluster.current_context.as_deref());

    if let Some(prom) = prom_config {
        let target = PodPollTarget {
            namespace: namespace.clone(),
            name: name.clone(),
            resource_specs,
            interval_secs,
        };
        let emitter = PollEmitter::from_state(state, sender);
        state.metrics.poll_task = Some(tokio::spawn(poll_prometheus_pod_metrics(
            prom, target, emitter,
        )));
        // Only notify once per polling session to avoid spam
        if !state.metrics.polling_notified.unwrap_or(false) {
            events::notify_info(
                state.cluster.seq.as_ref(),
                sender,
                format!(
                    "Watching POD {}/{} for metrics via Prometheus",
                    namespace, name
                ),
            );
            state.metrics.polling_notified = Some(true);
        }
        return Ok(());
    }

    // Fallback to metrics API polling
    if state.metrics.availability == MetricsAvailability::Unavailable
        || state.metrics.availability == MetricsAvailability::Forbidden
    {
        tracing::debug!("Skipping metrics polling: metrics API unavailable");
        return Ok(());
    }

    events::notify_info(
        state.cluster.seq.as_ref(),
        sender,
        format!("Watching POD {}/{} for metrics", namespace, name),
    );

    let client = state.cluster.client_or_err()?;
    let target = PodPollTarget {
        namespace,
        name,
        resource_specs,
        interval_secs,
    };
    let emitter = PollEmitter::from_state(state, sender);
    state.metrics.poll_task = Some(tokio::spawn(poll_metrics_api_pod_metrics(
        client, target, emitter,
    )));
    Ok(())
}

/// The Prometheus integration for `context`, if configured and enabled.
fn enabled_prometheus(
    context: Option<&str>,
) -> Option<crate::backend::integrations::PrometheusConfig> {
    context
        .and_then(|ctx| IntegrationConfig::load(ctx).ok())
        .and_then(|cfg| cfg.prometheus)
        .filter(|p| p.enabled)
}

/// Owned handles a background metrics task needs to emit events.
struct PollEmitter {
    seq: std::sync::Arc<std::sync::atomic::AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
    context: Option<String>,
}

impl PollEmitter {
    fn from_state(state: &BackendState, sender: &Sender<BackendEventEnvelope>) -> Self {
        Self {
            seq: state.cluster.seq.clone(),
            sender: sender.clone(),
            context: state.cluster.current_context.clone(),
        }
    }

    /// Emit a `PodMetricsHistoryUpdated` event routed to the pods section.
    fn emit_pod_history(&self, namespace: &str, history: PodMetricsHistory) {
        events::emit(
            self.seq.as_ref(),
            &self.sender,
            Route::namespaced(
                section_keys::workloads_pods(),
                self.context.clone(),
                Some(namespace.to_owned()),
            ),
            Correlation::None,
            BackendEvent::ResourceRead(ResourceReadEvent::PodMetricsHistoryUpdated(history)),
        );
    }
}

/// The pod whose metrics a polling task watches.
struct PodPollTarget {
    namespace: String,
    name: String,
    resource_specs: Vec<ContainerResourceSpec>,
    interval_secs: u64,
}

/// Background task: every interval, fetch the pod's recent history (20 intervals)
/// from Prometheus, persist it, and emit it.
async fn poll_prometheus_pod_metrics(
    prom: crate::backend::integrations::PrometheusConfig,
    target: PodPollTarget,
    emitter: PollEmitter,
) {
    let interval_secs = target.interval_secs;
    let interval = Duration::from_secs(interval_secs);
    let mut has_failed = false;

    loop {
        let end = Utc::now();
        let start = end - chrono::Duration::seconds((interval_secs * 20) as i64);
        let step = format!("{}s", interval_secs);

        let result = fetch_prom_pod_history(
            &prom,
            &target.namespace,
            &target.name,
            &step,
            start,
            end,
            &target.resource_specs,
        )
        .await;

        match result {
            Ok(history) => {
                has_failed = false;
                save_metrics_history(&history);
                emitter.emit_pod_history(&target.namespace, history);
            }
            Err(e) => log_prometheus_poll_failure(&mut has_failed, &e),
        }

        tokio::time::sleep(interval).await;
    }
}

/// Save metrics to ~/.fathom/metrics.json for historical tracking (best effort).
fn save_metrics_history(history: &PodMetricsHistory) {
    if let Err(e) = save_metrics_to_file(history) {
        tracing::debug!("Failed to save metrics history: {}", e);
    }
}

/// Warn on the first consecutive Prometheus polling failure; debug-log retries.
fn log_prometheus_poll_failure(has_failed: &mut bool, e: &anyhow::Error) {
    if !*has_failed {
        *has_failed = true;
        tracing::warn!("Prometheus metrics polling failed: {}", e);
    } else {
        tracing::debug!("Prometheus metrics polling retry failed: {}", e);
    }
}

/// Background task: every interval, sample the metrics API for the pod, keep a
/// bounded per-container history, and emit it.
async fn poll_metrics_api_pod_metrics(client: Client, target: PodPollTarget, emitter: PollEmitter) {
    let interval = Duration::from_secs(target.interval_secs);
    let mut history_map: std::collections::HashMap<String, Vec<MetricsSample>> =
        std::collections::HashMap::new();
    let mut has_failed = false;

    loop {
        tokio::time::sleep(interval).await;

        // Fetch current metrics
        let result = fetch_metrics_snapshot(&client, &target.namespace, &target.name).await;

        match result {
            Ok(metrics) => {
                has_failed = false;
                record_metrics_samples(&mut history_map, &metrics);

                let history = PodMetricsHistory {
                    namespace: target.namespace.clone(),
                    name: target.name.clone(),
                    containers: container_histories(&history_map, &target.resource_specs),
                };

                // Emit update event
                emitter.emit_pod_history(&target.namespace, history);
            }
            Err(e) => {
                // Only log the first failure to avoid spam
                if !has_failed {
                    has_failed = true;
                    tracing::debug!("Metrics polling encountered error: {}", e);
                }
                // Subsequent failures logged at trace level (effectively silent)
                else {
                    tracing::trace!("Metrics polling retry failed: {}", e);
                }
            }
        }
    }
}

/// Append one sample per container (when both CPU and memory parse) to its
/// history, keeping each history bounded to the most recent samples.
fn record_metrics_samples(
    history_map: &mut std::collections::HashMap<String, Vec<MetricsSample>>,
    metrics: &PodMetricsSummary,
) {
    const MAX_SAMPLES: usize = 100;
    let timestamp = k8s_openapi::chrono::Utc::now().to_rfc3339();

    // Update history for each container
    for container_metrics in &metrics.containers {
        let cpu_millicores = parse_cpu_quantity(&container_metrics.cpu);
        let memory_bytes = parse_memory_quantity(&container_metrics.memory);

        if let (Some(cpu), Some(mem)) = (cpu_millicores, memory_bytes) {
            let sample = MetricsSample {
                timestamp: timestamp.clone(),
                cpu_millicores: cpu,
                memory_bytes: mem,
            };

            let history = history_map
                .entry(container_metrics.name.clone())
                .or_default();
            history.push(sample);

            // Keep ring buffer bounded
            if history.len() > MAX_SAMPLES {
                history.remove(0);
            }
        }
    }
}

/// Build `ContainerMetricsHistory` entries, attaching each container's resource specs.
fn container_histories(
    history_map: &std::collections::HashMap<String, Vec<MetricsSample>>,
    resource_specs: &[ContainerResourceSpec],
) -> Vec<ContainerMetricsHistory> {
    history_map
        .iter()
        .map(|(container_name, samples)| {
            let spec = resource_specs
                .iter()
                .find(|s| &s.container == container_name);

            ContainerMetricsHistory {
                name: container_name.clone(),
                samples: samples.clone(),
                cpu_request_millicores: spec.and_then(|s| s.cpu_request_millicores),
                cpu_limit_millicores: spec.and_then(|s| s.cpu_limit_millicores),
                memory_request_bytes: spec.and_then(|s| s.memory_request_bytes),
                memory_limit_bytes: spec.and_then(|s| s.memory_limit_bytes),
            }
        })
        .collect()
}

#[derive(Debug, Deserialize)]
struct PromResult {
    result: Vec<PromSeries>,
}

#[derive(Debug, Deserialize)]
struct PromSeries {
    metric: std::collections::HashMap<String, String>,
    values: Vec<(f64, String)>,
}

#[derive(Debug, Deserialize)]
struct PromResponse {
    status: String,
    data: PromResult,
}

async fn prom_query_range(
    base_url: &str,
    query: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    step: &str,
) -> anyhow::Result<Vec<PromSeries>> {
    let url = format!(
        "{}/api/v1/query_range?query={}&start={}&end={}&step={}",
        base_url,
        urlencoding::encode(query),
        start.timestamp(),
        end.timestamp(),
        step
    );

    let resp = reqwest::get(&url)
        .await
        .with_context(|| format!("Prometheus GET failed for {}", url))?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        anyhow::bail!("Prometheus returned {}: {}", status, body);
    }

    let parsed: PromResponse = serde_json::from_str(&body)
        .with_context(|| format!("Failed to parse Prometheus response: {}", body))?;
    if parsed.status != "success" {
        anyhow::bail!("Prometheus returned non-success status: {}", parsed.status);
    }
    Ok(parsed.data.result)
}

/// Pair CPU and memory samples by their (RFC3339) timestamp rather than by
/// array position. `prom_query_range` returns one sample per `step` interval
/// only where the underlying metric actually has data - if either series has
/// a gap (a container that just started, a missed scrape, differing
/// retention) its array is shorter than the other's from that point on, and
/// zipping by index silently pairs a CPU sample from one moment with a
/// memory sample from a different one under a single shared timestamp for
/// the rest of the series. Pairing by timestamp instead drops points where
/// either metric is missing, rather than mislabeling them.
fn zip_samples_by_timestamp(
    cpu_samples: &[(String, f64)],
    mem_samples: &[(String, f64)],
) -> Vec<MetricsSample> {
    let mem_by_ts: std::collections::HashMap<&str, f64> = mem_samples
        .iter()
        .map(|(ts, v)| (ts.as_str(), *v))
        .collect();

    cpu_samples
        .iter()
        .filter_map(|(ts, cpu)| {
            mem_by_ts.get(ts.as_str()).map(|mem| MetricsSample {
                timestamp: ts.clone(),
                cpu_millicores: *cpu,
                memory_bytes: *mem,
            })
        })
        .collect()
}

async fn fetch_prom_pod_history(
    prom: &crate::backend::integrations::PrometheusConfig,
    namespace: &str,
    pod: &str,
    step: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    resource_specs: &[ContainerResourceSpec],
) -> anyhow::Result<PodMetricsHistory> {
    // Try localhost first (port-forwarded), then fall back to service DNS
    let base_url = format!("http://localhost:{}", prom.port);

    // Verify we can reach localhost; if not, use service DNS
    let base_url = match reqwest::Client::new()
        .get(format!("{}/api/v1/query", base_url))
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
    {
        Ok(_) => {
            // localhost is reachable
            format!("http://localhost:{}", prom.port)
        }
        Err(_) => {
            // Fall back to service DNS
            format!(
                "http://{}.{}.svc:{}",
                prom.service, prom.namespace, prom.port
            )
        }
    };

    // CPU: millicores
    let cpu_query = format!(
        "rate(container_cpu_usage_seconds_total{{namespace=\"{}\",pod=\"{}\",container!=\"POD\"}}[5m])*1000",
        namespace, pod
    );

    // Memory: bytes
    let mem_query = format!(
        "container_memory_working_set_bytes{{namespace=\"{}\",pod=\"{}\",container!=\"POD\"}}",
        namespace, pod
    );

    let cpu_series = prom_query_range(&base_url, &cpu_query, start, end, step).await?;
    let mem_series = prom_query_range(&base_url, &mem_query, start, end, step).await?;

    let mut cpu_map: std::collections::HashMap<String, Vec<(String, f64)>> =
        std::collections::HashMap::new();
    for series in cpu_series {
        let cname = series
            .metric
            .get("container")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        let mut samples = Vec::new();
        for (ts, val) in series.values {
            if let Ok(v) = val.parse::<f64>() {
                let ts = DateTime::<Utc>::from_timestamp(ts as i64, 0)
                    .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).unwrap())
                    .to_rfc3339();
                samples.push((ts, v));
            }
        }
        cpu_map.insert(cname, samples);
    }

    let mut mem_map: std::collections::HashMap<String, Vec<(String, f64)>> =
        std::collections::HashMap::new();
    for series in mem_series {
        let cname = series
            .metric
            .get("container")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        let mut samples = Vec::new();
        for (ts, val) in series.values {
            if let Ok(v) = val.parse::<f64>() {
                let ts = DateTime::<Utc>::from_timestamp(ts as i64, 0)
                    .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).unwrap())
                    .to_rfc3339();
                samples.push((ts, v));
            }
        }
        mem_map.insert(cname, samples);
    }

    let mut containers = Vec::new();
    for (cname, cpu_samples) in cpu_map.iter() {
        if let Some(mem_samples) = mem_map.get(cname) {
            let samples = zip_samples_by_timestamp(cpu_samples, mem_samples);

            let spec = resource_specs.iter().find(|s| &s.container == cname);
            containers.push(ContainerMetricsHistory {
                name: cname.clone(),
                samples,
                cpu_request_millicores: spec.and_then(|s| s.cpu_request_millicores),
                cpu_limit_millicores: spec.and_then(|s| s.cpu_limit_millicores),
                memory_request_bytes: spec.and_then(|s| s.memory_request_bytes),
                memory_limit_bytes: spec.and_then(|s| s.memory_limit_bytes),
            });
        }
    }

    Ok(PodMetricsHistory {
        namespace: namespace.to_string(),
        name: pod.to_string(),
        containers,
    })
}

pub(super) fn stop_metrics_polling(state: &mut BackendState) {
    if let Some(task) = state.metrics.poll_task.take() {
        task.abort();
        tracing::info!("Stopped watching pod for metrics");
    }
}

async fn fetch_metrics_snapshot(
    client: &Client,
    namespace: &str,
    name: &str,
) -> anyhow::Result<PodMetricsSummary> {
    let gvk = GroupVersionKind::gvk("metrics.k8s.io", "v1beta1", "PodMetrics");
    let resource = ApiResource::from_gvk(&gvk);
    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), namespace, &resource);

    let obj = api.get(name).await?;
    let data = obj.data;

    let mut containers = Vec::new();
    if let Some(serde_json::Value::Array(items)) = data.get("containers") {
        for item in items {
            let cname = item
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();

            let cpu = item
                .get("usage")
                .and_then(|u| u.get("cpu"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();

            let memory = item
                .get("usage")
                .and_then(|u| u.get("memory"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();

            containers.push(ContainerMetrics {
                name: cname,
                cpu,
                memory,
            });
        }
    }

    Ok(PodMetricsSummary {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        containers,
    })
}
/// A workload metrics aggregate with only replica_count populated, used when
/// no metrics provider (Prometheus/OpenObserve) is configured or reachable.
/// Still gives the UI a live replica count instead of nothing at all.
fn metrics_unavailable_aggregate(
    namespace: &str,
    workload_name: &str,
    workload_kind: &str,
    replica_count: u32,
) -> WorkloadMetricsAggregate {
    WorkloadMetricsAggregate {
        namespace: namespace.to_owned(),
        name: workload_name.to_owned(),
        workload_kind: workload_kind.to_owned(),
        replica_count,
        total_cpu_usage_percent: None,
        total_memory_usage_bytes: None,
        total_memory_limit_bytes: None,
        total_memory_request_bytes: None,
        total_cpu_request_millicores: None,
        total_cpu_limit_millicores: None,
        containers: Vec::new(),
        timestamp: Utc::now().timestamp_millis() as u64,
    }
}

/// Total CPU usage across a workload's replicas as a percentage of the
/// workload's total CPU *limit* - matching how `total_memory_usage_bytes`
/// is turned into a percent a few lines below this function's call site in
/// `workloads.rs` (`(mem_bytes / limit_bytes) * 100.0`), and how the
/// red/yellow/green thresholds in that same view are meant to be read: a
/// "danger zone" only makes sense relative to an allocated budget.
///
/// Previously this divided by a hardcoded 10.0 (i.e. always treated 1000m as
/// 100%, regardless of how many replicas or containers were being summed)
/// and capped the result at 100%. For any workload actually using more than
/// one core's worth of CPU total - a handful of busy replicas is enough -
/// that silently pinned the displayed value at "100%" even if the workload
/// was nowhere near its real limit, or well past it.
///
/// Returns `None` when there's no limit to be relative to (mirrors memory's
/// behavior: no limit means no percent, not a fabricated one).
fn workload_cpu_percent(total_millicores: f64, total_limit_millicores: u64) -> Option<f64> {
    if total_limit_millicores == 0 {
        return None;
    }
    Some(total_millicores / total_limit_millicores as f64 * 100.0)
}

async fn aggregate_from_prometheus(
    prom: &crate::backend::integrations::PrometheusConfig,
    namespace: &str,
    workload_name: &str,
    workload_kind: &str,
    pods: &[k8s_openapi::api::core::v1::Pod],
    replica_count: u32,
) -> anyhow::Result<WorkloadMetricsAggregate> {
    let base_url = format!(
        "http://{}.{}.svc:{}",
        prom.service, prom.namespace, prom.port
    );

    // Aggregate CPU across all pods and containers
    let mut total_cpu_millicores = 0.0;
    let mut total_memory_bytes = 0.0;
    let mut total_memory_limit_bytes: u64 = 0;
    let mut total_memory_request_bytes: u64 = 0;
    let mut total_cpu_request_millicores: u64 = 0;
    let mut total_cpu_limit_millicores: u64 = 0;

    // Collect per-container aggregates
    let mut container_map: std::collections::HashMap<String, ContainerMetricsAggregate> =
        std::collections::HashMap::new();

    for pod in pods {
        let pod_name = pod.metadata.name.as_deref().unwrap_or("unknown");

        // Get CPU usage from Prometheus
        let cpu_query = format!(
            "sum(rate(container_cpu_usage_seconds_total{{namespace=\"{}\",pod=\"{}\",container!=\"POD\"}}[1m]))*1000",
            namespace, pod_name
        );

        if let Ok(cpu_val) = prom_query_instant(&base_url, &cpu_query).await {
            total_cpu_millicores += cpu_val;

            // Distribute CPU proportionally across containers
            if let Some(spec) = &pod.spec {
                for container in &spec.containers {
                    let cname = container.name.clone();
                    let container_cpu_query = format!(
                        "rate(container_cpu_usage_seconds_total{{namespace=\"{}\",pod=\"{}\",container=\"{}\"}}[1m])*100",
                        namespace, pod_name, cname
                    );

                    let cpu_pct = prom_query_instant(&base_url, &container_cpu_query)
                        .await
                        .ok();

                    // Aggregate requests/limits from pod spec
                    let ContainerResourceTotals {
                        cpu_req,
                        cpu_lim,
                        mem_req,
                        mem_lim,
                    } = container_resource_totals(container);

                    total_cpu_request_millicores =
                        total_cpu_request_millicores.saturating_add(cpu_req);
                    total_cpu_limit_millicores = total_cpu_limit_millicores.saturating_add(cpu_lim);
                    total_memory_request_bytes = total_memory_request_bytes.saturating_add(mem_req);
                    total_memory_limit_bytes = total_memory_limit_bytes.saturating_add(mem_lim);

                    container_map.entry(cname.clone()).or_insert_with(|| {
                        ContainerMetricsAggregate {
                            name: cname.clone(),
                            cpu_usage_percent: cpu_pct,
                            memory_usage_bytes: None,
                            memory_limit_bytes: nonzero(mem_lim),
                            memory_request_bytes: nonzero(mem_req),
                            cpu_request_millicores: nonzero(cpu_req),
                            cpu_limit_millicores: nonzero(cpu_lim),
                        }
                    });
                }
            }
        }

        // Get memory usage from Prometheus
        let mem_query = format!(
            "sum(container_memory_working_set_bytes{{namespace=\"{}\",pod=\"{}\",container!=\"POD\"}})",
            namespace, pod_name
        );

        if let Ok(mem_val) = prom_query_instant(&base_url, &mem_query).await {
            total_memory_bytes += mem_val;

            // Get per-container memory
            add_container_memory(&base_url, namespace, pod_name, pod, &mut container_map).await;
        }
    }

    let total_cpu_percent = workload_cpu_percent(total_cpu_millicores, total_cpu_limit_millicores);

    let containers: Vec<ContainerMetricsAggregate> = container_map.into_values().collect();

    Ok(WorkloadMetricsAggregate {
        namespace: namespace.to_owned(),
        name: workload_name.to_owned(),
        workload_kind: workload_kind.to_owned(),
        replica_count,
        total_cpu_usage_percent: total_cpu_percent,
        total_memory_usage_bytes: if total_memory_bytes > 0.0 {
            Some(total_memory_bytes as u64)
        } else {
            None
        },
        total_memory_limit_bytes: nonzero(total_memory_limit_bytes),
        total_memory_request_bytes: nonzero(total_memory_request_bytes),
        total_cpu_request_millicores: nonzero(total_cpu_request_millicores),
        total_cpu_limit_millicores: nonzero(total_cpu_limit_millicores),
        containers,
        timestamp: Utc::now().timestamp_millis() as u64,
    })
}

/// Add each container's working-set memory (from Prometheus) to its existing
/// aggregate entry; containers without an entry or without data are skipped.
async fn add_container_memory(
    base_url: &str,
    namespace: &str,
    pod_name: &str,
    pod: &k8s_openapi::api::core::v1::Pod,
    container_map: &mut std::collections::HashMap<String, ContainerMetricsAggregate>,
) {
    let Some(spec) = &pod.spec else {
        return;
    };
    for container in &spec.containers {
        let cname = container.name.clone();
        let container_mem_query = format!(
            "container_memory_working_set_bytes{{namespace=\"{}\",pod=\"{}\",container=\"{}\"}}",
            namespace, pod_name, cname
        );

        if let Ok(mem_bytes) = prom_query_instant(base_url, &container_mem_query).await {
            container_map.entry(cname.clone()).and_modify(|c| {
                c.memory_usage_bytes = Some(c.memory_usage_bytes.unwrap_or(0) + mem_bytes as u64);
            });
        }
    }
}

/// `Some(v)` when `v > 0`, else `None` (zero means "not set" for requests/limits).
fn nonzero(v: u64) -> Option<u64> {
    if v > 0 { Some(v) } else { None }
}

/// A container's CPU (millicores) and memory (bytes) requests/limits; 0 when unset
/// or unparseable.
struct ContainerResourceTotals {
    cpu_req: u64,
    cpu_lim: u64,
    mem_req: u64,
    mem_lim: u64,
}

fn container_resource_totals(
    container: &k8s_openapi::api::core::v1::Container,
) -> ContainerResourceTotals {
    let mut totals = ContainerResourceTotals {
        cpu_req: 0,
        cpu_lim: 0,
        mem_req: 0,
        mem_lim: 0,
    };
    let Some(resources) = &container.resources else {
        return totals;
    };
    if let Some(requests) = &resources.requests {
        if let Some(cpu_qty) = requests.get("cpu")
            && let Some(millis) = parse_cpu_to_millis(&cpu_qty.0)
        {
            totals.cpu_req = millis;
        }
        if let Some(mem_qty) = requests.get("memory")
            && let Some(bytes) = parse_memory_to_bytes(&mem_qty.0)
        {
            totals.mem_req = bytes as u64;
        }
    }
    if let Some(limits) = &resources.limits {
        if let Some(cpu_qty) = limits.get("cpu")
            && let Some(millis) = parse_cpu_to_millis(&cpu_qty.0)
        {
            totals.cpu_lim = millis;
        }
        if let Some(mem_qty) = limits.get("memory")
            && let Some(bytes) = parse_memory_to_bytes(&mem_qty.0)
        {
            totals.mem_lim = bytes as u64;
        }
    }
    totals
}

/// Helper: parse CPU strings like "100m", "1", "2500m" to millicores
fn parse_cpu_to_millis(s: &str) -> Option<u64> {
    if let Some(stripped) = s.strip_suffix('m') {
        stripped.parse::<u64>().ok()
    } else if let Some(stripped) = s.strip_suffix('n') {
        stripped.parse::<u64>().ok().map(|v| v / 1_000_000)
    } else if let Some(stripped) = s.strip_suffix('u') {
        stripped.parse::<u64>().ok().map(|v| v / 1_000)
    } else {
        // Plain number = cores
        s.parse::<f64>().ok().map(|v| (v * 1000.0) as u64)
    }
}

/// Helper: parse memory strings like "128Mi", "1Gi" to bytes
fn parse_memory_to_bytes(s: &str) -> Option<f64> {
    if let Some(stripped) = s.strip_suffix("Ki") {
        stripped.parse::<f64>().ok().map(|v| v * 1024.0)
    } else if let Some(stripped) = s.strip_suffix("Mi") {
        stripped.parse::<f64>().ok().map(|v| v * 1024.0 * 1024.0)
    } else if let Some(stripped) = s.strip_suffix("Gi") {
        stripped
            .parse::<f64>()
            .ok()
            .map(|v| v * 1024.0 * 1024.0 * 1024.0)
    } else if let Some(stripped) = s.strip_suffix("Ti") {
        stripped
            .parse::<f64>()
            .ok()
            .map(|v| v * 1024.0 * 1024.0 * 1024.0 * 1024.0)
    } else if let Some(stripped) = s.strip_suffix('k') {
        stripped.parse::<f64>().ok().map(|v| v * 1000.0)
    } else if let Some(stripped) = s.strip_suffix('M') {
        stripped.parse::<f64>().ok().map(|v| v * 1_000_000.0)
    } else if let Some(stripped) = s.strip_suffix('G') {
        stripped.parse::<f64>().ok().map(|v| v * 1_000_000_000.0)
    } else {
        s.parse::<f64>().ok()
    }
}

/// Query Prometheus for an instant value (latest data point)
async fn prom_query_instant(base_url: &str, query: &str) -> anyhow::Result<f64> {
    let url = format!(
        "{}/api/v1/query?query={}",
        base_url,
        urlencoding::encode(query)
    );
    let response = reqwest::get(&url)
        .await?
        .json::<serde_json::Value>()
        .await?;

    let result = response
        .get("data")
        .and_then(|d| d.get("result"))
        .and_then(|r| r.as_array())
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("value"))
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.get(1))
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<f64>().ok());

    result.ok_or_else(|| anyhow::anyhow!("No data from Prometheus"))
}

/// Start polling metrics for a workload (deployment, statefulset, etc.)
pub(super) async fn start_workload_metrics_polling(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: String,
    workload_name: String,
    workload_kind: String,
    pod_selector: String,
    interval_secs: u64,
) -> anyhow::Result<()> {
    // Stop existing polling task if any
    stop_workload_metrics_polling(state);

    let client = state.cluster.client.clone();
    let sender_clone = sender.clone();
    let seq_clone = state.cluster.seq.clone();
    let context = state.cluster.current_context.clone();

    // Clone for logging after spawn
    let namespace_for_log = namespace.clone();
    let workload_name_for_log = workload_name.clone();
    let workload_kind_for_log = workload_kind.clone();

    // Spawn polling task
    let handle = tokio::spawn(async move {
        use k8s_openapi::api::core::v1::Pod;

        let interval = Duration::from_secs(interval_secs);
        let mut has_failed = false;

        loop {
            // Prefer the workload's real label selector (`.spec.selector.matchLabels`,
            // resolved by the details fetch for Deployment/StatefulSet/DaemonSet) over
            // guessing by name prefix — the prefix guess breaks for custom naming and
            // can also match unrelated pods that happen to share it.
            let pods: Vec<Pod> =
                list_workload_pods(client.as_ref(), &namespace, &workload_name, &pod_selector)
                    .await;

            let replica_count = pods.len() as u32;

            // Re-check config every tick so enabling Prometheus later (or it
            // going away) is picked up without restarting the poll.
            let prom_config = enabled_prometheus(context.as_deref());

            let result = match &prom_config {
                Some(prom) => {
                    aggregate_from_prometheus(
                        prom,
                        &namespace,
                        &workload_name,
                        &workload_kind,
                        &pods,
                        replica_count,
                    )
                    .await
                }
                // No metrics provider configured: still report replica_count so
                // the UI isn't blank while metrics remain unavailable.
                None => Ok(metrics_unavailable_aggregate(
                    &namespace,
                    &workload_name,
                    &workload_kind,
                    replica_count,
                )),
            };

            match result {
                Ok(metrics) => {
                    has_failed = false;

                    let Some(event) = workload_metrics_event(&workload_kind, metrics) else {
                        return;
                    };

                    events::emit(
                        seq_clone.as_ref(),
                        &sender_clone,
                        Route::namespaced(
                            section_keys::workloads_deployments(),
                            context.clone(),
                            Some(namespace.clone()),
                        ),
                        Correlation::None,
                        event,
                    );
                }
                Err(e) => {
                    if !has_failed {
                        has_failed = true;
                        let prom_base_url = prom_config.as_ref().map(|prom| {
                            format!(
                                "http://{}.{}.svc:{}",
                                prom.service, prom.namespace, prom.port
                            )
                        });
                        tracing::warn!(
                            namespace = %namespace,
                            workload = %workload_name,
                            "Workload metrics polling failed: {}. Prom URL: {:?}",
                            e, prom_base_url
                        );
                    } else {
                        tracing::debug!(
                            namespace = %namespace,
                            workload = %workload_name,
                            "Workload metrics polling retry failed: {}", e
                        );
                    }
                }
            }

            tokio::time::sleep(interval).await;
        }
    });

    state.metrics.workload_poll_task = Some(handle);
    tracing::info!(
        namespace = %namespace_for_log,
        workload = %workload_name_for_log,
        kind = %workload_kind_for_log,
        interval = interval_secs,
        "Started workload metrics polling"
    );

    Ok(())
}

/// Pods belonging to a workload.
///
/// Prefer the workload's real label selector (`.spec.selector.matchLabels`,
/// resolved by the details fetch for Deployment/StatefulSet/DaemonSet) over
/// guessing by name prefix — the prefix guess breaks for custom naming and
/// can also match unrelated pods that happen to share it.
async fn list_workload_pods(
    client: Option<&Client>,
    namespace: &str,
    workload_name: &str,
    pod_selector: &str,
) -> Vec<k8s_openapi::api::core::v1::Pod> {
    use k8s_openapi::api::core::v1::Pod;

    let Some(kube_client) = client else {
        return Vec::new();
    };
    let pod_api: ::kube::Api<Pod> = ::kube::Api::namespaced(kube_client.clone(), namespace);
    let list_params = if pod_selector.is_empty() {
        ::kube::api::ListParams::default()
    } else {
        ::kube::api::ListParams::default().labels(pod_selector)
    };
    let list = pod_api.list(&list_params).await;

    if pod_selector.is_empty() {
        // No selector available: fall back to the name-prefix heuristic.
        list.map(|list| {
            list.items
                .into_iter()
                .filter(|p| {
                    p.metadata
                        .name
                        .as_deref()
                        .map(|n| n.starts_with(&format!("{}-", workload_name)))
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default()
    } else {
        list.map(|list| list.items).unwrap_or_default()
    }
}

/// One-shot workload metrics aggregate for a fixed set of pods, without starting a
/// background poll loop. Used by `SelectApplication` (research.md R4: applications compute
/// metrics on-demand, not via a new poll loop) with the exact same Prometheus-or-unavailable
/// fallback the workload poll loop above uses each tick.
pub(super) async fn aggregate_workload_metrics_once(
    context: Option<&str>,
    namespace: &str,
    workload_name: &str,
    workload_kind: &str,
    pods: &[k8s_openapi::api::core::v1::Pod],
) -> WorkloadMetricsAggregate {
    let replica_count = pods.len() as u32;
    let prom_config = enabled_prometheus(context);
    let result = match &prom_config {
        Some(prom) => {
            aggregate_from_prometheus(
                prom,
                namespace,
                workload_name,
                workload_kind,
                pods,
                replica_count,
            )
            .await
        }
        None => Ok(metrics_unavailable_aggregate(
            namespace,
            workload_name,
            workload_kind,
            replica_count,
        )),
    };
    result.unwrap_or_else(|_| {
        metrics_unavailable_aggregate(namespace, workload_name, workload_kind, replica_count)
    })
}

/// Wrap workload metrics in the event for its kind; `None` (with a warning) for
/// unknown kinds.
fn workload_metrics_event(
    workload_kind: &str,
    metrics: WorkloadMetricsAggregate,
) -> Option<BackendEvent> {
    let event = match workload_kind {
        "Deployment" => ResourceReadEvent::DeploymentMetricsUpdated(metrics),
        "StatefulSet" => ResourceReadEvent::StatefulSetMetricsUpdated(metrics),
        "DaemonSet" => ResourceReadEvent::DaemonSetMetricsUpdated(metrics),
        _ => {
            tracing::warn!("Unknown workload kind: {}", workload_kind);
            return None;
        }
    };
    Some(BackendEvent::ResourceRead(event))
}

/// Stop polling metrics for a workload
pub(super) fn stop_workload_metrics_polling(state: &mut BackendState) {
    if let Some(handle) = state.metrics.workload_poll_task.take() {
        handle.abort();
        tracing::info!("Stopped workload metrics polling");
    }
}

/// Save metrics history to JSON file for persistence across sessions
fn save_metrics_to_file(history: &PodMetricsHistory) -> anyhow::Result<()> {
    // Load existing metrics or start with empty object
    let mut all_metrics = persistence::load_metrics_history()?;

    // Create a key for this pod (namespace/name)
    let pod_key = format!("{}/{}", history.namespace, history.name);

    // Convert containers to JSON-serializable format
    let containers_json: Vec<serde_json::Value> = history
        .containers
        .iter()
        .map(|c| {
            serde_json::json!({
                "name": c.name,
                "samples": c.samples.iter().map(|s| serde_json::json!({
                    "timestamp": s.timestamp,
                    "cpu_millicores": s.cpu_millicores,
                    "memory_bytes": s.memory_bytes,
                })).collect::<Vec<_>>(),
                "cpu_request_millicores": c.cpu_request_millicores,
                "cpu_limit_millicores": c.cpu_limit_millicores,
                "memory_request_bytes": c.memory_request_bytes,
                "memory_limit_bytes": c.memory_limit_bytes,
            })
        })
        .collect();

    // Update or create the pod entry
    all_metrics[&pod_key] = serde_json::json!({
        "namespace": history.namespace,
        "name": history.name,
        "containers": containers_json,
        "last_updated": chrono::Utc::now().to_rfc3339(),
    });

    persistence::save_metrics_history(&all_metrics)?;
    Ok(())
}

/// Load historical metrics for a pod from the persistence file
#[allow(dead_code)] // planned API: counterpart to the history save path, for restoring graphs
pub(super) fn load_pod_metrics_history(
    namespace: &str,
    name: &str,
) -> anyhow::Result<Option<PodMetricsHistory>> {
    let all_metrics = persistence::load_metrics_history()?;
    let pod_key = format!("{}/{}", namespace, name);

    if let Some(pod_data) = all_metrics.get(&pod_key) {
        // Parse containers from JSON
        let containers: Vec<ContainerMetricsHistory> = pod_data
            .get("containers")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|container_obj| {
                        let name = container_obj.get("name")?.as_str()?.to_string();
                        let samples: Vec<MetricsSample> = container_obj
                            .get("samples")
                            .and_then(|s| s.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|sample| {
                                        serde_json::from_value(sample.clone()).ok()
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();

                        Some(ContainerMetricsHistory {
                            name,
                            samples,
                            cpu_request_millicores: container_obj
                                .get("cpu_request_millicores")
                                .and_then(|v| v.as_f64()),
                            cpu_limit_millicores: container_obj
                                .get("cpu_limit_millicores")
                                .and_then(|v| v.as_f64()),
                            memory_request_bytes: container_obj
                                .get("memory_request_bytes")
                                .and_then(|v| v.as_f64()),
                            memory_limit_bytes: container_obj
                                .get("memory_limit_bytes")
                                .and_then(|v| v.as_f64()),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(Some(PodMetricsHistory {
            namespace: namespace.to_string(),
            name: name.to_string(),
            containers,
        }))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for a bug found by inspection: `fetch_prom_pod_history`
    /// used to zip CPU and memory sample arrays purely by index position,
    /// assuming both series had identical timestamps at every index. When
    /// one series has a gap the other doesn't (e.g. a container that just
    /// started reporting memory but not CPU yet, or a missed scrape), the
    /// arrays diverge and every sample after the gap gets silently paired
    /// with a value from a *different point in time*, all reported under
    /// one shared (wrong) timestamp.
    #[test]
    fn zip_samples_by_timestamp_drops_misaligned_points_instead_of_mispairing_them() {
        let cpu = vec![
            ("t1".to_owned(), 100.0),
            ("t2".to_owned(), 200.0),
            // gap at t3: no CPU sample (e.g. a missed scrape)
            ("t4".to_owned(), 400.0),
        ];
        let mem = vec![
            ("t1".to_owned(), 1000.0),
            ("t2".to_owned(), 2000.0),
            ("t3".to_owned(), 3000.0),
            ("t4".to_owned(), 4000.0),
        ];

        let samples = zip_samples_by_timestamp(&cpu, &mem);

        // The old positional-zip logic would pair cpu[2]=(t4,400) with
        // mem[2]=(t3,3000) - a CPU reading from t4 mislabeled as t3's memory
        // reading. Pairing by timestamp should instead just skip t3 (no CPU
        // data for it) and t4 should keep its own memory value (4000), not
        // whatever sat at the same array index in the old, shorter zip.
        let by_ts: std::collections::HashMap<&str, (f64, f64)> = samples
            .iter()
            .map(|s| (s.timestamp.as_str(), (s.cpu_millicores, s.memory_bytes)))
            .collect();

        assert_eq!(
            samples.len(),
            3,
            "t3 has no CPU sample and should be dropped, not mispaired"
        );
        assert_eq!(by_ts.get("t1"), Some(&(100.0, 1000.0)));
        assert_eq!(by_ts.get("t2"), Some(&(200.0, 2000.0)));
        assert!(!by_ts.contains_key("t3"));
        assert_eq!(
            by_ts.get("t4"),
            Some(&(400.0, 4000.0)),
            "t4's CPU sample must be paired with t4's own memory sample (4000), \
             not mem[2] (3000, which is actually t3's)"
        );
    }

    /// Regression test for a bug found by inspection: the workload CPU
    /// percent used to be `(total_millicores / 10.0).min(100.0)` - a fixed
    /// "1000m = 100%" single-core baseline that ignored the workload's own
    /// `total_cpu_limit_millicores` (computed right there in the same
    /// function) and silently capped at 100%, hiding real overage.
    #[test]
    fn workload_cpu_percent_is_relative_to_the_limit_not_a_fixed_single_core_cap() {
        // 3 replicas each using ~1700m against a 5000m total limit - well
        // under the limit (~102%... let's use a case that's unambiguous).
        // Total usage 5100m against a 10000m limit (5 replicas x 2000m limit
        // each) should read as 51%, not a capped 100%.
        assert_eq!(workload_cpu_percent(5100.0, 10_000), Some(51.0));

        // A workload genuinely over its limit should show that, not be
        // clamped down to hide it.
        assert_eq!(workload_cpu_percent(12_000.0, 10_000), Some(120.0));

        // No limit configured - no baseline to be relative to, so no
        // fabricated percent (mirrors how memory_usage_percent behaves).
        assert_eq!(workload_cpu_percent(5000.0, 0), None);
    }
}
