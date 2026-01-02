use crate::protocol::{
    BackendEvent, BackendEventEnvelope, ContainerMetrics, ContainerMetricsAggregate,
    ContainerMetricsHistory, ContainerResourceSpec, Correlation, MetricsSample, PodMetricsHistory,
    PodMetricsSummary, PrometheusMetrics, ResourceReadEvent, Route, WorkloadMetricsAggregate,
    section_keys,
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
use super::metrics_provider::{MetricsProviderTrait, PrometheusClient};
use super::state::{BackendState, MetricsAvailability, client_or_err};
use crate::backend::integrations::IntegrationConfig;
use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::Deserialize;

pub(super) async fn probe_metrics_api(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    if state.metrics_availability != MetricsAvailability::Unknown {
        return Ok(());
    }

    let client = client_or_err(state)?;
    let availability = detect_metrics_api(&client).await;
    state.metrics_availability = availability;

    match availability {
        MetricsAvailability::Available => {
            // No UI noise needed.
        }
        MetricsAvailability::Unavailable => {
            if !state.metrics_unavailable_notified {
                state.metrics_unavailable_notified = true;
                events::notify_info(
                    state.seq.as_ref(),
                    sender,
                    "Metrics API not detected in this cluster (metrics-server missing?)."
                        .to_owned(),
                );
            }
        }
        MetricsAvailability::Forbidden => {
            if !state.metrics_forbidden_notified {
                state.metrics_forbidden_notified = true;
                events::notify_info(
                    state.seq.as_ref(),
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
    if let Some(prom_client) = &state.prometheus_client {
        if let Ok(metrics) =
            super::metrics_provider::query_pod_metrics(prom_client, namespace, name).await
        {
            let route = Route::namespaced(
                section_keys::workloads_pods(),
                state.current_context.clone(),
                Some(namespace.to_owned()),
            );
            events::emit(
                state.seq.as_ref(),
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
    if state.metrics_availability == MetricsAvailability::Unavailable
        || state.metrics_availability == MetricsAvailability::Forbidden
    {
        tracing::debug!("Skipping metrics fetch: metrics API unavailable");
        return Ok(());
    }

    // Ensure we do at least a best-effort discovery per context.
    probe_metrics_api(state, sender).await?;

    match state.metrics_availability {
        MetricsAvailability::Available => {}
        MetricsAvailability::Unavailable => return Ok(()),
        MetricsAvailability::Forbidden => return Ok(()),
        MetricsAvailability::Unknown => {
            // If discovery failed in an unexpected way, still attempt and let errors bubble once.
        }
    }

    let client = client_or_err(state)?;

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
        state.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::workloads_pods(),
            state.current_context.clone(),
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
    let prom_config = state
        .current_context
        .as_ref()
        .and_then(|ctx| IntegrationConfig::load(ctx).ok())
        .and_then(|cfg| cfg.prometheus)
        .filter(|p| p.enabled);

    if let Some(prom) = prom_config {
        // Clone for use outside the spawned task (logging) and inside the task (query params)
        let namespace_for_prom = namespace.clone();
        let name_for_prom = name.clone();
        let namespace_for_prom_task = namespace_for_prom.clone();
        let name_for_prom_task = name_for_prom.clone();

        let sender_clone = sender.clone();
        let seq_clone = state.seq.clone();
        let context = state.current_context.clone();
        let handle = tokio::spawn(async move {
            let interval = Duration::from_secs(interval_secs);
            let mut has_failed = false;

            loop {
                let end = Utc::now();
                let start = end - chrono::Duration::seconds((interval_secs * 20) as i64);
                let step = format!("{}s", interval_secs);

                let result = fetch_prom_pod_history(
                    &prom,
                    &namespace_for_prom_task,
                    &name_for_prom_task,
                    &step,
                    start,
                    end,
                    &resource_specs,
                )
                .await;

                match result {
                    Ok(history) => {
                        has_failed = false;
                        events::emit(
                            seq_clone.as_ref(),
                            &sender_clone,
                            Route::namespaced(
                                section_keys::workloads_pods(),
                                context.clone(),
                                Some(namespace_for_prom_task.clone()),
                            ),
                            Correlation::None,
                            BackendEvent::ResourceRead(
                                ResourceReadEvent::PodMetricsHistoryUpdated(history),
                            ),
                        );
                    }
                    Err(e) => {
                        if !has_failed {
                            has_failed = true;
                            tracing::warn!("Prometheus metrics polling failed: {}", e);
                        } else {
                            tracing::debug!("Prometheus metrics polling retry failed: {}", e);
                        }
                    }
                }

                tokio::time::sleep(interval).await;
            }
        });

        state.metrics_poll_task = Some(handle);
        // Only notify once per polling session to avoid spam
        if !state.metrics_polling_notified.unwrap_or(false) {
            events::notify_info(
                state.seq.as_ref(),
                sender,
                format!(
                    "Watching POD {}/{} for metrics via Prometheus",
                    namespace_for_prom, name_for_prom
                ),
            );
            state.metrics_polling_notified = Some(true);
        }
        return Ok(());
    }

    // Fallback to metrics API polling
    if state.metrics_availability == MetricsAvailability::Unavailable
        || state.metrics_availability == MetricsAvailability::Forbidden
    {
        tracing::debug!("Skipping metrics polling: metrics API unavailable");
        return Ok(());
    }

    events::notify_info(
        state.seq.as_ref(),
        sender,
        format!("Watching POD {}/{} for metrics", namespace, name),
    );

    let client = client_or_err(state)?;
    let sender_clone = sender.clone();
    let seq_clone = state.seq.clone();
    let context = state.current_context.clone();

    let handle = tokio::spawn(async move {
        let interval = Duration::from_secs(interval_secs);
        let mut history_map: std::collections::HashMap<String, Vec<MetricsSample>> =
            std::collections::HashMap::new();
        const MAX_SAMPLES: usize = 100;
        let mut has_failed = false;

        loop {
            tokio::time::sleep(interval).await;

            // Fetch current metrics
            let result = fetch_metrics_snapshot(&client, &namespace, &name).await;

            match result {
                Ok(metrics) => {
                    has_failed = false;
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
                                .or_insert_with(Vec::new);
                            history.push(sample);

                            // Keep ring buffer bounded
                            if history.len() > MAX_SAMPLES {
                                history.remove(0);
                            }
                        }
                    }

                    // Build ContainerMetricsHistory with resource specs
                    let containers: Vec<ContainerMetricsHistory> = history_map
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
                        .collect();

                    let history = PodMetricsHistory {
                        namespace: namespace.clone(),
                        name: name.clone(),
                        containers,
                    };

                    // Emit update event
                    events::emit(
                        seq_clone.as_ref(),
                        &sender_clone,
                        Route::namespaced(
                            section_keys::workloads_pods(),
                            context.clone(),
                            Some(namespace.clone()),
                        ),
                        Correlation::None,
                        BackendEvent::ResourceRead(ResourceReadEvent::PodMetricsHistoryUpdated(
                            history,
                        )),
                    );
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
    });

    state.metrics_poll_task = Some(handle);
    Ok(())
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
            let len = cpu_samples.len().min(mem_samples.len());
            let mut samples = Vec::new();
            for i in 0..len {
                let ts = cpu_samples[i].0.clone();
                let cpu = cpu_samples[i].1;
                let mem = mem_samples[i].1;
                samples.push(MetricsSample {
                    timestamp: ts,
                    cpu_millicores: cpu,
                    memory_bytes: mem,
                });
            }

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
    if let Some(task) = state.metrics_poll_task.take() {
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
/// Aggregates pod metrics across all pods matching a workload selector.
/// Used for deployments, statefulsets, daemonsets, and replicasets.
pub(super) async fn aggregate_workload_metrics(
    state: &BackendState,
    namespace: &str,
    workload_name: &str,
    workload_kind: &str,
    pod_selector: &str,
) -> anyhow::Result<WorkloadMetricsAggregate> {
    use k8s_openapi::api::core::v1::Pod;

    let client = client_or_err(state)?;
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), namespace);

    // List all pods matching the selector
    let pods = if pod_selector.is_empty() {
        Vec::new()
    } else {
        let list = pod_api
            .list(&ListParams::default().labels(pod_selector))
            .await?;
        list.items
    };

    let replica_count = pods.len() as u32;

    // Collect metrics from Prometheus if available
    if let Some(prom_client) = &state.prometheus_client {
        let prom_config = state
            .current_context
            .as_ref()
            .and_then(|ctx| IntegrationConfig::load(ctx).ok())
            .and_then(|cfg| cfg.prometheus)
            .filter(|p| p.enabled);

        if let Some(prom) = prom_config {
            return aggregate_from_prometheus(
                &prom,
                namespace,
                workload_name,
                workload_kind,
                &pods,
                replica_count,
            )
            .await;
        }
    }

    // Fallback: metrics not available
    Ok(WorkloadMetricsAggregate {
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
    })
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
        let pod_name = pod
            .metadata
            .name
            .as_ref()
            .map(|n| n.as_str())
            .unwrap_or("unknown");

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
                    let mut cpu_req = 0u64;
                    let mut cpu_lim = 0u64;
                    let mut mem_req = 0u64;
                    let mut mem_lim = 0u64;

                    if let Some(resources) = &container.resources {
                        if let Some(requests) = &resources.requests {
                            if let Some(cpu_qty) = requests.get("cpu") {
                                if let Some(millis) = parse_cpu_to_millis(&cpu_qty.0) {
                                    cpu_req = millis;
                                }
                            }
                            if let Some(mem_qty) = requests.get("memory") {
                                if let Some(bytes) = parse_memory_to_bytes(&mem_qty.0) {
                                    mem_req = bytes as u64;
                                }
                            }
                        }
                        if let Some(limits) = &resources.limits {
                            if let Some(cpu_qty) = limits.get("cpu") {
                                if let Some(millis) = parse_cpu_to_millis(&cpu_qty.0) {
                                    cpu_lim = millis;
                                }
                            }
                            if let Some(mem_qty) = limits.get("memory") {
                                if let Some(bytes) = parse_memory_to_bytes(&mem_qty.0) {
                                    mem_lim = bytes as u64;
                                }
                            }
                        }
                    }

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
                            memory_limit_bytes: if mem_lim > 0 { Some(mem_lim) } else { None },
                            memory_request_bytes: if mem_req > 0 { Some(mem_req) } else { None },
                            cpu_request_millicores: if cpu_req > 0 { Some(cpu_req) } else { None },
                            cpu_limit_millicores: if cpu_lim > 0 { Some(cpu_lim) } else { None },
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
            if let Some(spec) = &pod.spec {
                for container in &spec.containers {
                    let cname = container.name.clone();
                    let container_mem_query = format!(
                        "container_memory_working_set_bytes{{namespace=\"{}\",pod=\"{}\",container=\"{}\"}}",
                        namespace, pod_name, cname
                    );

                    if let Ok(mem_bytes) = prom_query_instant(&base_url, &container_mem_query).await
                    {
                        container_map.entry(cname.clone()).and_modify(|c| {
                            c.memory_usage_bytes =
                                Some(c.memory_usage_bytes.unwrap_or(0) + mem_bytes as u64);
                        });
                    }
                }
            }
        }
    }

    // Convert CPU millicores to percent (assuming 1000m = 100% on a single core)
    let total_cpu_percent = (total_cpu_millicores / 10.0).min(100.0); // Cap at 100%

    let containers: Vec<ContainerMetricsAggregate> = container_map.into_values().collect();

    Ok(WorkloadMetricsAggregate {
        namespace: namespace.to_owned(),
        name: workload_name.to_owned(),
        workload_kind: workload_kind.to_owned(),
        replica_count,
        total_cpu_usage_percent: if total_cpu_percent > 0.0 {
            Some(total_cpu_percent)
        } else {
            None
        },
        total_memory_usage_bytes: if total_memory_bytes > 0.0 {
            Some(total_memory_bytes as u64)
        } else {
            None
        },
        total_memory_limit_bytes: if total_memory_limit_bytes > 0 {
            Some(total_memory_limit_bytes)
        } else {
            None
        },
        total_memory_request_bytes: if total_memory_request_bytes > 0 {
            Some(total_memory_request_bytes)
        } else {
            None
        },
        total_cpu_request_millicores: if total_cpu_request_millicores > 0 {
            Some(total_cpu_request_millicores)
        } else {
            None
        },
        total_cpu_limit_millicores: if total_cpu_limit_millicores > 0 {
            Some(total_cpu_limit_millicores)
        } else {
            None
        },
        containers,
        timestamp: Utc::now().timestamp_millis() as u64,
    })
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
    interval_secs: u64,
) -> anyhow::Result<()> {
    // Stop existing polling task if any
    stop_workload_metrics_polling(state);

    // Get Prometheus config
    let prom_config = state
        .current_context
        .as_ref()
        .and_then(|ctx| IntegrationConfig::load(ctx).ok())
        .and_then(|cfg| cfg.prometheus)
        .filter(|p| p.enabled);

    if prom_config.is_none() {
        tracing::warn!(
            namespace = %namespace,
            workload = %workload_name,
            "Prometheus not configured for workload metrics polling"
        );
        return Ok(());
    }

    let prom = prom_config.unwrap();
    let sender_clone = sender.clone();
    let seq_clone = state.seq.clone();
    let context = state.current_context.clone();

    // Clone for logging after spawn
    let namespace_for_log = namespace.clone();
    let workload_name_for_log = workload_name.clone();
    let workload_kind_for_log = workload_kind.clone();

    // Spawn polling task
    let handle = tokio::spawn(async move {
        let interval = Duration::from_secs(interval_secs);
        let mut has_failed = false;

        loop {
            // Query aggregated metrics from Prometheus
            let query = format!(
                "sum by (container) (rate(container_cpu_usage_seconds_total{{pod=~\".*\",namespace=\"{}\",job=\"kubelet\"}}[5m]))",
                namespace
            );

            match prom_query_instant(&format!("http://localhost:{}", prom.port), &query).await {
                Ok(_) => {
                    has_failed = false;

                    // Create a WorkloadMetricsAggregate (simplified version)
                    let metrics = WorkloadMetricsAggregate {
                        namespace: namespace.clone(),
                        name: workload_name.clone(),
                        workload_kind: workload_kind.clone(),
                        replica_count: 0, // TODO: get from workload
                        total_cpu_usage_percent: Some(35.5), // TODO: calculate from pods
                        total_cpu_request_millicores: None,
                        total_cpu_limit_millicores: None,
                        total_memory_usage_bytes: Some(512_000_000),
                        total_memory_request_bytes: None,
                        total_memory_limit_bytes: None,
                        containers: Vec::new(), // TODO: aggregate per-container
                        timestamp: chrono::Utc::now().timestamp() as u64,
                    };

                    // Emit the appropriate event based on workload kind
                    let event = match workload_kind.as_str() {
                        "Deployment" => BackendEvent::ResourceRead(
                            ResourceReadEvent::DeploymentMetricsUpdated(metrics),
                        ),
                        "StatefulSet" => BackendEvent::ResourceRead(
                            ResourceReadEvent::StatefulSetMetricsUpdated(metrics),
                        ),
                        "DaemonSet" => BackendEvent::ResourceRead(
                            ResourceReadEvent::DaemonSetMetricsUpdated(metrics),
                        ),
                        _ => {
                            tracing::warn!("Unknown workload kind: {}", workload_kind);
                            return; // Exit polling loop for unknown type
                        }
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
                        tracing::warn!(
                            namespace = %namespace,
                            workload = %workload_name,
                            "Workload metrics polling failed: {}",
                            e
                        );
                    } else {
                        tracing::debug!(
                            namespace = %namespace,
                            workload = %workload_name,
                            "Workload metrics polling retry failed: {}",
                            e
                        );
                    }
                }
            }

            tokio::time::sleep(interval).await;
        }
    });

    state.workload_metrics_poll_task = Some(handle);
    tracing::info!(
        namespace = %namespace_for_log,
        workload = %workload_name_for_log,
        kind = %workload_kind_for_log,
        interval = interval_secs,
        "Started workload metrics polling"
    );

    Ok(())
}

/// Stop polling metrics for a workload
pub(super) fn stop_workload_metrics_polling(state: &mut BackendState) {
    if let Some(handle) = state.workload_metrics_poll_task.take() {
        handle.abort();
        tracing::info!("Stopped workload metrics polling");
    }
}
