use crate::protocol::{
    BackendEvent, BackendEventEnvelope, ContainerMetrics, ContainerMetricsHistory,
    ContainerResourceSpec, Correlation, MetricsSample, PodMetricsHistory, PodMetricsSummary,
    ResourceReadEvent, Route, section_keys,
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
use super::state::{BackendState, MetricsAvailability, client_or_err};

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

    // Skip metrics polling if metrics API is not available
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
