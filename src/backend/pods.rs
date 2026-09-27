use crate::protocol::{
    BackendEvent, BackendEventEnvelope, CLIENT_STREAM_ID_MIN, ContainerLifecycle, ContainerPort,
    ContainerResourceSpec, ContainerState, Correlation, LifecycleHookSpec, PodDetails, PodOwner,
    PodSummary, ProbeSpec, ResourceListEvent, ResourceReadEvent, Route, StreamEvent, StreamId,
    section_keys,
};
use crossbeam_channel::Sender;
use futures_util::{StreamExt, io::AsyncBufReadExt};
use k8s_openapi::api::apps::v1::ReplicaSet;
use k8s_openapi::api::core::v1::{
    Container, ContainerState as K8sContainerState, ContainerStatus, LifecycleHandler, Pod, Probe,
};
use kube::{
    ResourceExt,
    api::{Api, DeleteParams, LogParams, Patch, PatchParams},
};
use std::collections::{BTreeMap, HashMap};

use super::interactions;
use super::log_streams::{CloseGuard, StreamSink};
use super::{events, state::BackendState};

pub(super) async fn refresh_pods(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let filter = &state.cluster.namespace_filter;
    // 036 US4: the daemon's warm snapshot when a session runs for this context, else a LIST
    // scoped to the namespace filter (031: namespace-bound users can't list cluster-wide).
    let pods = match super::daemon_ctl::snapshot_source(state) {
        Some(src) => {
            super::warm_lists::list_with_daemon::<Pod>(&client, Some(&src), "pods")
                .await?
                .items
        }
        None => state.cluster.list_namespaced::<Pod>().await?,
    };
    let summaries = pods_to_summaries(pods, filter);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::workloads_pods(),
            state.cluster.current_context.clone(),
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
    let client = state.cluster.client_or_err()?;
    delete_pod_with_client(
        &client,
        state.cluster.seq.as_ref(),
        sender,
        state.cluster.current_context.as_deref(),
        namespace,
        name,
    )
    .await
}

/// The delete itself, against an explicitly supplied client rather than whatever
/// `BackendState` currently holds.
///
/// Split out for 015-mcp-agent-exposure's `delete_resource` MCP tool
/// (`crate::backend::mcp::tools_mutate`), which runs in a spawned task holding a
/// `kube::Client` but no `&BackendState`. Annotation, delete and `Notification` emission stay
/// together here so the UI-driven and MCP-driven deletes are one code path.
pub(super) async fn delete_pod_with_client(
    client: &kube::Client,
    seq: &std::sync::atomic::AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    context: Option<&str>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let api: Api<Pod> = Api::namespaced(client.clone(), namespace);

    let ts = k8s_openapi::chrono::Utc::now().to_rfc3339();
    let _ =
        interactions::annotate_action(&api, name, context, "delete", &ts, serde_json::json!({}))
            .await;

    api.delete(name, &DeleteParams::default()).await?;
    events::notify_info(seq, sender, format!("Deleted pod {}/{}", namespace, name));
    Ok(())
}

/// 025-ephemeral-debug-containers: add an ephemeral debug container to a running pod via the
/// `ephemeralcontainers` subresource (equivalent to `kubectl debug`'s `AddEphemeralContainer`).
/// This is a one-way mutation — the Kubernetes API has no way to remove an ephemeral container
/// once added, and this function does not attempt to work around that.
///
/// `target_container` must be `Some` unless the pod has exactly one existing container, in
/// which case that container is used. `ephemeral_container_name` defaults to a generated
/// `debugger-<suffix>` name (mirroring `kubectl debug`'s default naming) when not supplied.
pub(super) async fn add_ephemeral_container(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    pod_name: &str,
    target_container: Option<&str>,
    image: &str,
    ephemeral_container_name: Option<&str>,
) -> anyhow::Result<()> {
    anyhow::ensure!(!image.trim().is_empty(), "Debug image must not be empty");

    let client = state.cluster.client_or_err()?;
    let api: Api<Pod> = Api::namespaced(client, namespace);

    let target_container = match target_container {
        Some(name) => name.to_owned(),
        None => {
            let pod = api.get(pod_name).await?;
            let containers = pod
                .spec
                .as_ref()
                .map(|s| s.containers.as_slice())
                .unwrap_or(&[]);
            match containers {
                [only] => only.name.clone(),
                [] => anyhow::bail!(
                    "Pod {}/{} has no containers to share a process namespace with",
                    namespace,
                    pod_name
                ),
                _ => anyhow::bail!(
                    "Pod {}/{} has multiple containers; a target container must be specified",
                    namespace,
                    pod_name
                ),
            }
        }
    };

    let name = ephemeral_container_name
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let suffix = k8s_openapi::chrono::Utc::now()
                .timestamp_nanos_opt()
                .unwrap_or(0) as u64;
            format!("debugger-{:x}", suffix & 0x00ff_ffff)
        });

    let patch = ephemeral_container_patch(&name, image, &target_container);

    api.patch_ephemeral_containers(pod_name, &PatchParams::default(), &Patch::Strategic(patch))
        .await?;

    events::notify_info(
        state.cluster.seq.as_ref(),
        sender,
        format!(
            "Added ephemeral container {name} ({image}) to {namespace}/{pod_name}, sharing {target_container}'s process namespace. It cannot be removed."
        ),
    );
    Ok(())
}

/// Strategic-merge patch body appending one ephemeral container to a pod.
fn ephemeral_container_patch(name: &str, image: &str, target: &str) -> serde_json::Value {
    serde_json::json!({
        "spec": {
            "ephemeralContainers": [{
                "name": name,
                "image": image,
                "targetContainerName": target,
            }]
        }
    })
}

pub(super) fn pods_to_summaries(pods: Vec<Pod>, filter: &[String]) -> Vec<PodSummary> {
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
    let client = state.cluster.client_or_err()?;
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
                .map(|c| c.name.clone())
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
    let persistent_volume_claims: Vec<String> = extract_persistent_volume_claims(&pod);

    let raw_json = serde_json::to_string_pretty(&pod)?;
    let containers_lifecycle = build_container_lifecycle(&pod);

    let details = PodDetails {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        creation_timestamp,
        labels,
        annotations,
        containers,
        ports,
        resource_specs,
        persistent_volume_claims,
        owner,
        resolved_owner,
        raw_json,
        containers_lifecycle,
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
    if r.kind == "ReplicaSet"
        && let Ok(client) = state.cluster.client_or_err()
    {
        let rs_api: Api<ReplicaSet> = Api::namespaced(client, namespace);
        if let Ok(rs) = rs_api.get(&r.name).await {
            let rs_refs = rs.metadata.owner_references.unwrap_or_default();
            let rs_controller = rs_refs
                .iter()
                .find(|rr| rr.controller.unwrap_or(false))
                .or_else(|| rs_refs.first());
            if let Some(rr) = rs_controller
                && rr.kind == "Deployment"
            {
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

    (owner, None)
}

/// Start following one container's logs as its own stream (feedback #2): other streams keep
/// running. Every outcome reaches the view as events on `stream_id`: `PodLogsOpened` once the
/// API server accepts the follow, `PodLogsChunk`s, then exactly one `StreamClosed` (ended,
/// stopped, or error with the verbatim failure), including when the follow never starts.
pub(super) fn start_logs(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: String,
    name: String,
    container: Option<String>,
    requested: Option<StreamId>,
) {
    let legacy = requested.is_none();
    let stream_id = requested.unwrap_or_else(|| state.next_stream_id());
    let route = Route::namespaced(
        section_keys::workloads_pods(),
        state.cluster.current_context.clone(),
        Some(namespace.clone()),
    );
    let guard = CloseGuard::new(StreamSink {
        stream_id,
        route,
        seq: state.cluster.seq.clone(),
        sender: sender.clone(),
    });
    let refused = requested
        .filter(|id| id.0 < CLIENT_STREAM_ID_MIN)
        .map(|id| format!("Log stream id {} is reserved for the backend.", id.0));
    let client = state.cluster.client_or_err();
    let log_params = LogParams {
        follow: true,
        tail_lines: Some(200),
        container,
        ..Default::default()
    };

    state
        .log_stream
        .spawn(guard, legacy, move |mut guard| async move {
            if let Some(reason) = refused {
                guard.failed(reason);
                return;
            }
            let client = match client {
                Ok(c) => c,
                Err(e) => {
                    guard.failed(e.to_string());
                    return;
                }
            };
            let api: Api<Pod> = Api::namespaced(client, &namespace);
            let stream = match api.log_stream(&name, &log_params).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::debug!(%namespace, %name, "log follow failed to start: {e}");
                    guard.failed(format!("Failed to start logs: {e}"));
                    return;
                }
            };
            guard.sink().emit(StreamEvent::PodLogsOpened { stream_id });

            let mut lines_stream = stream.lines();
            while let Some(line) = lines_stream.next().await {
                match line {
                    Ok(line) if line.is_empty() => {}
                    Ok(line) => guard.sink().emit(StreamEvent::PodLogsChunk {
                        stream_id,
                        lines: vec![line],
                    }),
                    Err(e) => {
                        guard.failed(format!("Log stream error: {e}"));
                        return;
                    }
                }
            }
            guard.ended();
        });
}

/// Best-effort collect PVC claim names from pod.spec.volumes (volumes may be optional).
pub(super) fn extract_persistent_volume_claims(pod: &Pod) -> Vec<String> {
    pod.spec
        .as_ref()
        .and_then(|s| s.volumes.as_ref())
        .map(|vols| {
            vols.iter()
                .filter_map(|v| {
                    v.persistent_volume_claim
                        .as_ref()
                        .map(|p| p.claim_name.clone())
                })
                .collect()
        })
        .unwrap_or_default()
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

/// 008-pod-lifecycle-diagnostics: assemble per-container lifecycle detail (init containers
/// distinct from app containers, current/last state, restarts, probes, hooks) from the `Pod`
/// object `fetch_pod_details` already fetched. No additional cluster call.
pub(super) fn build_container_lifecycle(pod: &Pod) -> Vec<ContainerLifecycle> {
    let Some(spec) = pod.spec.as_ref() else {
        return Vec::new();
    };
    let status = pod.status.as_ref();

    let init_statuses = status.and_then(|s| s.init_container_statuses.as_ref());
    let app_statuses = status.and_then(|s| s.container_statuses.as_ref());

    let ephemeral_statuses = status.and_then(|s| s.ephemeral_container_statuses.as_ref());

    let init_containers = spec
        .init_containers
        .iter()
        .flatten()
        .map(|c| container_lifecycle(c, true, find_status(init_statuses, &c.name)));
    let app_containers = spec
        .containers
        .iter()
        .map(|c| container_lifecycle(c, false, find_status(app_statuses, &c.name)));
    let ephemeral_containers = spec
        .ephemeral_containers
        .iter()
        .flatten()
        .map(|c| ephemeral_container_lifecycle(c, find_status(ephemeral_statuses, &c.name)));

    init_containers
        .chain(app_containers)
        .chain(ephemeral_containers)
        .collect()
}

fn find_status<'a>(
    statuses: Option<&'a Vec<ContainerStatus>>,
    name: &str,
) -> Option<&'a ContainerStatus> {
    statuses.and_then(|list| list.iter().find(|s| s.name == name))
}

fn container_lifecycle(
    container: &Container,
    is_init: bool,
    status: Option<&ContainerStatus>,
) -> ContainerLifecycle {
    let state = status
        .and_then(|s| s.state.as_ref())
        .map(convert_state)
        .unwrap_or(ContainerState {
            phase: "waiting".to_owned(),
            reason: None,
            exit_code: None,
            started_at: None,
            finished_at: None,
        });
    let last_state = status
        .and_then(|s| s.last_state.as_ref())
        .filter(|ls| ls.running.is_some() || ls.terminated.is_some() || ls.waiting.is_some())
        .map(convert_state);

    let mut probes = Vec::new();
    if let Some(p) = &container.liveness_probe {
        probes.push(build_probe("liveness", p));
    }
    if let Some(p) = &container.readiness_probe {
        probes.push(build_probe("readiness", p));
    }
    if let Some(p) = &container.startup_probe {
        probes.push(build_probe("startup", p));
    }

    let mut hooks = Vec::new();
    if let Some(lifecycle) = &container.lifecycle {
        if let Some(h) = &lifecycle.post_start {
            hooks.push(build_hook("postStart", h));
        }
        if let Some(h) = &lifecycle.pre_stop {
            hooks.push(build_hook("preStop", h));
        }
    }

    ContainerLifecycle {
        name: container.name.clone(),
        is_init,
        image: container.image.clone().unwrap_or_default(),
        ready: status.map(|s| s.ready).unwrap_or(false),
        state,
        restart_count: status.map(|s| s.restart_count).unwrap_or(0),
        last_state,
        probes,
        hooks,
        is_ephemeral: false,
    }
}

/// 025-ephemeral-debug-containers: build a `ContainerLifecycle` entry for one
/// `status.ephemeralContainerStatuses[]` item. Ephemeral containers have no probes/hooks in
/// their spec, so those are always empty; `is_init` is always `false`.
fn ephemeral_container_lifecycle(
    spec: &k8s_openapi::api::core::v1::EphemeralContainer,
    status: Option<&ContainerStatus>,
) -> ContainerLifecycle {
    let state = status
        .and_then(|s| s.state.as_ref())
        .map(convert_state)
        .unwrap_or(ContainerState {
            phase: "waiting".to_owned(),
            reason: None,
            exit_code: None,
            started_at: None,
            finished_at: None,
        });
    let last_state = status
        .and_then(|s| s.last_state.as_ref())
        .filter(|ls| ls.running.is_some() || ls.terminated.is_some() || ls.waiting.is_some())
        .map(convert_state);

    ContainerLifecycle {
        name: spec.name.clone(),
        is_init: false,
        image: spec.image.clone().unwrap_or_default(),
        ready: status.map(|s| s.ready).unwrap_or(false),
        state,
        restart_count: status.map(|s| s.restart_count).unwrap_or(0),
        last_state,
        probes: Vec::new(),
        hooks: Vec::new(),
        is_ephemeral: true,
    }
}

fn convert_state(state: &K8sContainerState) -> ContainerState {
    if let Some(t) = &state.terminated {
        ContainerState {
            phase: "terminated".to_owned(),
            reason: t.reason.clone(),
            exit_code: Some(t.exit_code),
            started_at: t.started_at.as_ref().map(|ts| ts.0.to_rfc3339()),
            finished_at: t.finished_at.as_ref().map(|ts| ts.0.to_rfc3339()),
        }
    } else if let Some(r) = &state.running {
        ContainerState {
            phase: "running".to_owned(),
            reason: None,
            exit_code: None,
            started_at: r.started_at.as_ref().map(|ts| ts.0.to_rfc3339()),
            finished_at: None,
        }
    } else {
        // Waiting, or unspecified (k8s treats unspecified as waiting).
        ContainerState {
            phase: "waiting".to_owned(),
            reason: state.waiting.as_ref().and_then(|w| w.reason.clone()),
            exit_code: None,
            started_at: None,
            finished_at: None,
        }
    }
}

fn build_probe(probe_type: &str, probe: &Probe) -> ProbeSpec {
    let (mechanism, target) = if let Some(http) = &probe.http_get {
        let port = format_int_or_string(&http.port);
        let scheme = http.scheme.clone().unwrap_or_else(|| "HTTP".to_owned());
        let path = http.path.clone().unwrap_or_default();
        ("httpGet".to_owned(), format!("{scheme} {path}:{port}"))
    } else if let Some(tcp) = &probe.tcp_socket {
        (
            "tcpSocket".to_owned(),
            format!("tcp:{}", format_int_or_string(&tcp.port)),
        )
    } else if let Some(exec) = &probe.exec {
        (
            "exec".to_owned(),
            format!(
                "exec: {}",
                exec.command.clone().unwrap_or_default().join(" ")
            ),
        )
    } else if let Some(grpc) = &probe.grpc {
        let service = grpc.service.clone().unwrap_or_default();
        ("grpc".to_owned(), format!("grpc:{}/{}", grpc.port, service))
    } else {
        ("unknown".to_owned(), String::new())
    };

    ProbeSpec {
        probe_type: probe_type.to_owned(),
        mechanism,
        target,
        period_seconds: probe.period_seconds.unwrap_or(10),
        failure_threshold: probe.failure_threshold.unwrap_or(3),
        success_threshold: probe.success_threshold.unwrap_or(1),
    }
}

fn build_hook(hook_type: &str, handler: &LifecycleHandler) -> LifecycleHookSpec {
    let (mechanism, target) = if let Some(exec) = &handler.exec {
        (
            "exec".to_owned(),
            format!(
                "exec: {}",
                exec.command.clone().unwrap_or_default().join(" ")
            ),
        )
    } else if let Some(http) = &handler.http_get {
        let port = format_int_or_string(&http.port);
        let scheme = http.scheme.clone().unwrap_or_else(|| "HTTP".to_owned());
        let path = http.path.clone().unwrap_or_default();
        ("httpGet".to_owned(), format!("{scheme} {path}:{port}"))
    } else if let Some(sleep) = &handler.sleep {
        ("sleep".to_owned(), format!("sleep: {}s", sleep.seconds))
    } else {
        ("unknown".to_owned(), String::new())
    };

    LifecycleHookSpec {
        hook_type: hook_type.to_owned(),
        mechanism,
        target,
    }
}

fn format_int_or_string(v: &k8s_openapi::apimachinery::pkg::util::intstr::IntOrString) -> String {
    use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
    match v {
        IntOrString::Int(i) => i.to_string(),
        IntOrString::String(s) => s.clone(),
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use k8s_openapi::api::core::v1::{
        ContainerStateRunning, ContainerStateTerminated, ContainerStateWaiting, ExecAction,
        HTTPGetAction, Lifecycle, PodSpec, PodStatus,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
    use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;

    fn container(name: &str) -> Container {
        Container {
            name: name.to_owned(),
            image: Some(format!("registry.example.com/{name}:1.0")),
            ..Default::default()
        }
    }

    fn status(
        name: &str,
        ready: bool,
        restart_count: i32,
        state: K8sContainerState,
    ) -> ContainerStatus {
        ContainerStatus {
            name: name.to_owned(),
            ready,
            restart_count,
            state: Some(state),
            image: String::new(),
            image_id: String::new(),
            ..Default::default()
        }
    }

    fn terminated(exit_code: i32, reason: &str) -> K8sContainerState {
        K8sContainerState {
            terminated: Some(ContainerStateTerminated {
                exit_code,
                reason: Some(reason.to_owned()),
                started_at: Some(Time(k8s_openapi::chrono::Utc::now())),
                finished_at: Some(Time(k8s_openapi::chrono::Utc::now())),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn running() -> K8sContainerState {
        K8sContainerState {
            running: Some(ContainerStateRunning {
                started_at: Some(Time(k8s_openapi::chrono::Utc::now())),
            }),
            ..Default::default()
        }
    }

    fn waiting(reason: &str) -> K8sContainerState {
        K8sContainerState {
            waiting: Some(ContainerStateWaiting {
                reason: Some(reason.to_owned()),
                message: None,
            }),
            ..Default::default()
        }
    }

    fn pod_with(
        init: Vec<Container>,
        app: Vec<Container>,
        init_statuses: Vec<ContainerStatus>,
        app_statuses: Vec<ContainerStatus>,
    ) -> Pod {
        Pod {
            spec: Some(PodSpec {
                init_containers: if init.is_empty() { None } else { Some(init) },
                containers: app,
                ..Default::default()
            }),
            status: Some(PodStatus {
                init_container_statuses: if init_statuses.is_empty() {
                    None
                } else {
                    Some(init_statuses)
                },
                container_statuses: Some(app_statuses),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// FR-001/FR-002/FR-003: init containers are distinct (`is_init: true`) with their own
    /// status; a succeeded one shows exit code 0, a failed one shows its nonzero exit code.
    #[test]
    fn init_containers_distinct_with_exit_codes() {
        let pod = pod_with(
            vec![container("init-a"), container("init-b")],
            vec![container("app")],
            vec![
                status("init-a", false, 0, terminated(0, "Completed")),
                status("init-b", false, 0, terminated(1, "Error")),
            ],
            vec![status("app", false, 0, waiting("PodInitializing"))],
        );
        let out = build_container_lifecycle(&pod);
        assert_eq!(out.len(), 3);
        let a = out.iter().find(|c| c.name == "init-a").unwrap();
        assert!(a.is_init);
        assert_eq!(a.state.phase, "terminated");
        assert_eq!(a.state.exit_code, Some(0));
        let b = out.iter().find(|c| c.name == "init-b").unwrap();
        assert!(b.is_init);
        assert_eq!(b.state.exit_code, Some(1));
        assert_eq!(b.state.reason.as_deref(), Some("Error"));
        let app = out.iter().find(|c| c.name == "app").unwrap();
        assert!(!app.is_init);
        assert_eq!(app.state.phase, "waiting");
    }

    /// FR-004: a Pod with no init containers has zero `is_init: true` entries.
    #[test]
    fn no_init_containers_is_empty_not_error() {
        let pod = pod_with(
            vec![],
            vec![container("app")],
            vec![],
            vec![status("app", true, 0, running())],
        );
        let out = build_container_lifecycle(&pod);
        assert!(out.iter().all(|c| !c.is_init));
        assert_eq!(out.len(), 1);
    }

    /// FR-005/FR-007: all three probe types are extracted with type/mechanism/thresholds/period;
    /// a container with none configured has an empty `probes` Vec.
    #[test]
    fn probes_extracted_or_empty() {
        let mut c = container("api");
        c.liveness_probe = Some(Probe {
            http_get: Some(HTTPGetAction {
                path: Some("/healthz".to_owned()),
                port: IntOrString::Int(8080),
                ..Default::default()
            }),
            period_seconds: Some(15),
            failure_threshold: Some(5),
            success_threshold: Some(1),
            ..Default::default()
        });
        c.readiness_probe = Some(Probe {
            tcp_socket: Some(k8s_openapi::api::core::v1::TCPSocketAction {
                port: IntOrString::Int(8080),
                host: None,
            }),
            ..Default::default()
        });
        c.startup_probe = Some(Probe {
            exec: Some(ExecAction {
                command: Some(vec!["cat".to_owned(), "/tmp/ready".to_owned()]),
            }),
            ..Default::default()
        });
        let pod = pod_with(
            vec![],
            vec![c, container("sidecar")],
            vec![],
            vec![
                status("api", true, 0, running()),
                status("sidecar", true, 0, running()),
            ],
        );
        let out = build_container_lifecycle(&pod);
        let api = out.iter().find(|c| c.name == "api").unwrap();
        assert_eq!(api.probes.len(), 3);
        assert!(api.probes.iter().any(|p| p.probe_type == "liveness"
            && p.mechanism == "httpGet"
            && p.period_seconds == 15));
        assert!(
            api.probes
                .iter()
                .any(|p| p.probe_type == "readiness" && p.mechanism == "tcpSocket")
        );
        assert!(
            api.probes
                .iter()
                .any(|p| p.probe_type == "startup" && p.mechanism == "exec")
        );

        let sidecar = out.iter().find(|c| c.name == "sidecar").unwrap();
        assert!(sidecar.probes.is_empty());
    }

    /// FR-008/FR-009: nonzero restart count carries last_state (reason/exit code); zero restart
    /// count has no stale last_state.
    #[test]
    fn restart_history_attributed_per_container() {
        let mut crashed = status("crashed", true, 3, running());
        crashed.last_state = Some(terminated(137, "OOMKilled"));
        let healthy = status("healthy", true, 0, running());

        let pod = pod_with(
            vec![],
            vec![container("crashed"), container("healthy")],
            vec![],
            vec![crashed, healthy],
        );
        let out = build_container_lifecycle(&pod);
        let c = out.iter().find(|c| c.name == "crashed").unwrap();
        assert_eq!(c.restart_count, 3);
        let last = c.last_state.as_ref().unwrap();
        assert_eq!(last.exit_code, Some(137));
        assert_eq!(last.reason.as_deref(), Some("OOMKilled"));

        let h = out.iter().find(|c| c.name == "healthy").unwrap();
        assert_eq!(h.restart_count, 0);
        assert!(h.last_state.is_none());
    }

    /// Edge case: `restartPolicy: Never` pod, container terminated without `last_state` set —
    /// `state` (not `last_state`) still carries the termination info.
    #[test]
    fn terminated_without_last_state_still_shows_current_state() {
        let s = status("job", false, 0, terminated(0, "Completed"));
        let pod = pod_with(vec![], vec![container("job")], vec![], vec![s]);
        let out = build_container_lifecycle(&pod);
        let c = &out[0];
        assert_eq!(c.state.phase, "terminated");
        assert_eq!(c.state.exit_code, Some(0));
        assert!(c.last_state.is_none());
    }

    /// FR-010/FR-012: postStart/preStop hooks extracted with mechanism/target; none configured
    /// yields an empty `hooks` Vec.
    #[test]
    fn hooks_extracted_or_empty() {
        let mut c = container("api");
        c.lifecycle = Some(Lifecycle {
            post_start: Some(LifecycleHandler {
                exec: Some(ExecAction {
                    command: Some(vec![
                        "/bin/sh".to_owned(),
                        "-c".to_owned(),
                        "warm".to_owned(),
                    ]),
                }),
                ..Default::default()
            }),
            pre_stop: Some(LifecycleHandler {
                http_get: Some(HTTPGetAction {
                    path: Some("/shutdown".to_owned()),
                    port: IntOrString::Int(8080),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        });
        let pod = pod_with(
            vec![],
            vec![c, container("sidecar")],
            vec![],
            vec![
                status("api", true, 0, running()),
                status("sidecar", true, 0, running()),
            ],
        );
        let out = build_container_lifecycle(&pod);
        let api = out.iter().find(|c| c.name == "api").unwrap();
        assert_eq!(api.hooks.len(), 2);
        assert!(
            api.hooks
                .iter()
                .any(|h| h.hook_type == "postStart" && h.mechanism == "exec")
        );
        assert!(
            api.hooks
                .iter()
                .any(|h| h.hook_type == "preStop" && h.mechanism == "httpGet")
        );

        let sidecar = out.iter().find(|c| c.name == "sidecar").unwrap();
        assert!(sidecar.hooks.is_empty());
    }

    #[test]
    fn ephemeral_patch_has_expected_fields() {
        let p = ephemeral_container_patch("debugger-1", "busybox", "app");
        let c = &p["spec"]["ephemeralContainers"][0];
        assert_eq!(c["name"], "debugger-1");
        assert_eq!(c["image"], "busybox");
        assert_eq!(c["targetContainerName"], "app");
    }

    #[test]
    fn ephemeral_container_statuses_are_listed() {
        let mut pod = pod_with(
            vec![],
            vec![container("app")],
            vec![],
            vec![status("app", true, 0, running())],
        );
        pod.spec.as_mut().unwrap().ephemeral_containers =
            Some(vec![k8s_openapi::api::core::v1::EphemeralContainer {
                name: "debugger-1".to_owned(),
                image: Some("busybox".to_owned()),
                ..Default::default()
            }]);
        let out = build_container_lifecycle(&pod);
        let e = out.iter().find(|c| c.name == "debugger-1").unwrap();
        assert!(e.is_ephemeral && !e.is_init);
        assert_eq!(e.state.phase, "waiting");
        assert!(!out.iter().find(|c| c.name == "app").unwrap().is_ephemeral);
    }
}
