use crate::protocol::{
    BackendEvent, BackendEventEnvelope, ContainerMetrics, Correlation, PodMetricsSummary,
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
