use crate::protocol::{
    BackendEvent, BackendEventEnvelope, ContainerPort, ContainerResourceSpec, Correlation,
    PodDetails, PodOwner, PodSummary, ResourceListEvent, ResourceReadEvent, Route, StreamEvent,
    section_keys,
};
use crossbeam_channel::Sender;
use futures_util::{StreamExt, io::AsyncBufReadExt};
use k8s_openapi::api::apps::v1::ReplicaSet;
use k8s_openapi::api::core::v1::Pod;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams, ListParams, LogParams},
};
use std::collections::{BTreeMap, HashMap};

use super::interactions;
use super::state::{client_or_err, stop_logs};
use super::{events, state::BackendState};

pub(super) async fn refresh_pods(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = client_or_err(state)?;
    let api: Api<Pod> = Api::all(client);

    let pods = api.list(&ListParams::default()).await?;
    let summaries = pods_to_summaries(pods.items, &state.namespace_filter);
    events::emit(
        state.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::workloads_pods(),
            state.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::PodsUpdated(summaries)),
    );
    Ok(())
}

pub(super) async fn delete_pod(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = client_or_err(state)?;
    let api: Api<Pod> = Api::namespaced(client, namespace);

    let ts = k8s_openapi::chrono::Utc::now().to_rfc3339();
    let _ = interactions::annotate_action(
        &api,
        name,
        state.current_context.as_deref(),
        "delete",
        &ts,
        serde_json::json!({}),
    )
    .await;

    api.delete(name, &DeleteParams::default()).await?;
    events::notify_info(
        state.seq.as_ref(),
        sender,
        format!("Deleted pod {}/{}", namespace, name),
    );
    Ok(())
}

pub(super) fn pods_to_summaries(pods: Vec<Pod>, filter: &Vec<String>) -> Vec<PodSummary> {
    // Deduplicate by ns/name (watchers can be noisy; list shouldn't duplicate but keep it safe).
    let mut map: HashMap<(String, String), PodSummary> = HashMap::new();

    for pod in pods {
        let namespace = pod.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = pod.name_any();
        let phase = pod
            .status
            .as_ref()
            .and_then(|s| s.phase.clone())
            .unwrap_or_else(|| "Unknown".to_owned());

        let node = pod
            .spec
            .as_ref()
            .and_then(|s| s.node_name.clone())
            .unwrap_or_default();

        let (ready, total, restarts) = pod
            .status
            .as_ref()
            .and_then(|s| s.container_statuses.as_ref())
            .map(|statuses| {
                let total = statuses.len() as u32;
                let ready = statuses.iter().filter(|cs| cs.ready).count() as u32;
                let restarts = statuses.iter().map(|cs| cs.restart_count).sum::<i32>();
                (ready, total, restarts)
            })
            .unwrap_or((0, 0, 0));

        map.insert(
            (namespace.clone(), name.clone()),
            PodSummary {
                name,
                namespace,
                phase,
                node,
                ready,
                total,
                restarts,
            },
        );
    }

    let mut summaries: Vec<PodSummary> = map.into_values().collect();
    summaries.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    summaries
}

pub(super) async fn fetch_pod_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = client_or_err(state)?;
    let api: Api<Pod> = Api::namespaced(client, namespace);
    let pod = api.get(name).await?;

    let (owner, resolved_owner) = resolve_pod_owner(state, namespace, &pod).await;

    let labels: BTreeMap<String, String> = pod
        .metadata
        .labels
        .clone()
        .unwrap_or_default()
        .into_iter()
        .collect();
    let annotations: BTreeMap<String, String> = pod
        .metadata
        .annotations
        .clone()
        .unwrap_or_default()
        .into_iter()
        .collect();

    let creation_timestamp = pod
        .metadata
        .creation_timestamp
        .as_ref()
        .map(|t| t.0.to_rfc3339())
        .unwrap_or_default();

    let containers = pod
        .spec
        .as_ref()
        .map(|s| {
            s.containers
                .iter()
                .map(|c| format!("{} ({})", c.name, c.image.clone().unwrap_or_default()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let ports: Vec<ContainerPort> = pod
        .spec
        .as_ref()
        .map(|s| {
            s.containers
                .iter()
                .flat_map(|c| {
                    c.ports.iter().flatten().map(|p| ContainerPort {
                        container: c.name.clone(),
                        name: p.name.clone(),
                        port: p.container_port as u16,
                        protocol: p.protocol.clone().unwrap_or_else(|| "TCP".to_owned()),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let resource_specs: Vec<ContainerResourceSpec> = extract_resource_specs(&pod);

    let raw_json = serde_json::to_string_pretty(&pod)?;

    let details = PodDetails {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        creation_timestamp,
        labels,
        annotations,
        containers,
        ports,
        resource_specs,
        owner,
        resolved_owner,
        raw_json,
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
        BackendEvent::ResourceRead(ResourceReadEvent::PodDetailsUpdated(details)),
    );
    Ok(())
}

async fn resolve_pod_owner(
    state: &BackendState,
    namespace: &str,
    pod: &Pod,
) -> (Option<PodOwner>, Option<PodOwner>) {
    let refs = pod.metadata.owner_references.clone().unwrap_or_default();
    if refs.is_empty() {
        return (None, None);
    }

    // Prefer the controller owner if present.
    let controller = refs
        .iter()
        .find(|r| r.controller.unwrap_or(false))
        .or_else(|| refs.first());

    let Some(r) = controller else {
        return (None, None);
    };

    let owner = Some(PodOwner {
        kind: r.kind.clone(),
        name: r.name.clone(),
    });

    // Best-effort resolve ReplicaSet -> Deployment.
    if r.kind == "ReplicaSet" {
        if let Ok(client) = client_or_err(state) {
            let rs_api: Api<ReplicaSet> = Api::namespaced(client, namespace);
            if let Ok(rs) = rs_api.get(&r.name).await {
                let rs_refs = rs.metadata.owner_references.unwrap_or_default();
                let rs_controller = rs_refs
                    .iter()
                    .find(|rr| rr.controller.unwrap_or(false))
                    .or_else(|| rs_refs.first());
                if let Some(rr) = rs_controller {
                    if rr.kind == "Deployment" {
                        return (
                            owner,
                            Some(PodOwner {
                                kind: rr.kind.clone(),
                                name: rr.name.clone(),
                            }),
                        );
                    }
                }
            }
        }
    }

    (owner, None)
}

pub(super) async fn start_logs(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: String,
    name: String,
    container: Option<String>,
) -> anyhow::Result<()> {
    stop_logs(state);

    let stream_id = state.next_stream_id();

    let client = client_or_err(state)?;
    let api: Api<Pod> = Api::namespaced(client.clone(), &namespace);

    let log_params = LogParams {
        follow: true,
        tail_lines: Some(200),
        container,
        ..Default::default()
    };

    let sender_cloned = sender.clone();
    let seq = state.seq.clone();
    let context = state.current_context.clone();
    let route = Route::namespaced(
        section_keys::workloads_pods(),
        context,
        Some(namespace.clone()),
    );
    let handle = tokio::spawn(async move {
        events::notify_info(
            seq.as_ref(),
            &sender_cloned,
            format!("Streaming logs: {}/{}", namespace, name),
        );

        let stream = match api.log_stream(&name, &log_params).await {
            Ok(s) => s,
            Err(e) => {
                events::notify_error(
                    seq.as_ref(),
                    &sender_cloned,
                    format!("Failed to start logs: {}", e),
                );
                return;
            }
        };

        let mut lines_stream = stream.lines();
        while let Some(line) = lines_stream.next().await {
            match line {
                Ok(line) => {
                    if !line.is_empty() {
                        events::emit(
                            seq.as_ref(),
                            &sender_cloned,
                            route.clone(),
                            Correlation::Stream(stream_id),
                            BackendEvent::Stream(StreamEvent::PodLogsChunk {
                                stream_id,
                                lines: vec![line],
                            }),
                        );
                    }
                }
                Err(e) => {
                    events::notify_error(
                        seq.as_ref(),
                        &sender_cloned,
                        format!("Log stream error: {}", e),
                    );
                    break;
                }
            }
        }

        events::emit(
            seq.as_ref(),
            &sender_cloned,
            route,
            Correlation::Stream(stream_id),
            BackendEvent::Stream(StreamEvent::StreamClosed {
                stream_id,
                reason: "ended".to_owned(),
            }),
        );
    });

    state.log_task = Some(handle);
    Ok(())
}

pub(super) fn extract_resource_specs(pod: &Pod) -> Vec<ContainerResourceSpec> {
    use super::metrics::{parse_cpu_quantity, parse_memory_quantity};

    pod.spec
        .as_ref()
        .map(|spec| {
            spec.containers
                .iter()
                .map(|container| {
                    let resources = container.resources.as_ref();

                    let cpu_request_millicores = resources
                        .and_then(|r| r.requests.as_ref())
                        .and_then(|req| req.get("cpu"))
                        .map(|q| q.0.as_str())
                        .and_then(parse_cpu_quantity);

                    let cpu_limit_millicores = resources
                        .and_then(|r| r.limits.as_ref())
                        .and_then(|lim| lim.get("cpu"))
                        .map(|q| q.0.as_str())
                        .and_then(parse_cpu_quantity);

                    let memory_request_bytes = resources
                        .and_then(|r| r.requests.as_ref())
                        .and_then(|req| req.get("memory"))
                        .map(|q| q.0.as_str())
                        .and_then(parse_memory_quantity);

                    let memory_limit_bytes = resources
                        .and_then(|r| r.limits.as_ref())
                        .and_then(|lim| lim.get("memory"))
                        .map(|q| q.0.as_str())
                        .and_then(parse_memory_quantity);

                    ContainerResourceSpec {
                        container: container.name.clone(),
                        cpu_request_millicores,
                        cpu_limit_millicores,
                        memory_request_bytes,
                        memory_limit_bytes,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}
