use std::collections::{BTreeMap, HashMap};

use crossbeam_channel::Sender;
use k8s_openapi::api::core::v1::{Node, NodeStatus, Pod, Taint};
use kube::api::{Api, EvictParams, ListParams, Patch, PatchParams};
use serde_json::json;

use crate::protocol::v1::NodeSummary;
use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, NodeCondition, NodeDetails, ResourceListEvent,
    ResourceReadEvent, Route, StreamEvent, TaintOp, section_keys,
};

use super::events;
use super::state::BackendState;

/// Extract detailed information from a Node resource
pub fn extract_node_details(node: &Node) -> anyhow::Result<NodeDetails> {
    let meta = &node.metadata;
    let status = node.status.as_ref();

    let name = meta.name.clone().unwrap_or_default();
    let creation_timestamp = meta
        .creation_timestamp
        .as_ref()
        .map(|t| super::workload_common::kube_rfc3339(&t))
        .unwrap_or_default();

    let labels = meta.labels.clone().unwrap_or_default();
    let annotations = meta.annotations.clone().unwrap_or_default();

    // Extract node status information
    let default_status = Default::default();
    let status_info = status.unwrap_or(&default_status);

    let node_status = status_info
        .conditions
        .as_ref()
        .and_then(|conds| {
            conds
                .iter()
                .find(|c| c.type_ == "Ready")
                .map(|c| c.status.clone())
        })
        .unwrap_or_else(|| "Unknown".to_string());

    let roles = node_roles(&labels);

    // Extract system information
    let SystemInfo {
        kernel_version,
        container_runtime,
        kubelet_version,
        os,
        arch,
    } = system_info(status_info);

    // Extract capacity and allocatable resources
    let (capacity_cpu, capacity_memory, allocatable_cpu, allocatable_memory) =
        capacity_and_allocatable(status_info);

    // Extract pod count and capacity
    let pod_count = 0; // TODO: query actual running pods for this node
    let pod_capacity = status_info
        .allocatable
        .as_ref()
        .and_then(|a| a.get("pods"))
        .map(|q| q.0.clone())
        .as_deref()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);

    // Extract conditions
    let conditions = node_conditions(status_info);

    let raw_json = serde_json::to_string_pretty(&node)?;

    Ok(NodeDetails {
        name,
        status: node_status,
        roles,
        labels,
        annotations,
        creation_timestamp,
        kernel_version,
        container_runtime,
        kubelet_version,
        os,
        arch,
        capacity_cpu,
        capacity_memory,
        allocatable_cpu,
        allocatable_memory,
        pod_count,
        pod_capacity,
        conditions,
        raw_json,
    })
}

/// Extract roles from node labels (defaults to `worker`).
fn node_roles(labels: &BTreeMap<String, String>) -> Vec<String> {
    let mut r = Vec::new();
    if labels.contains_key("node-role.kubernetes.io/control-plane")
        || labels.contains_key("node-role.kubernetes.io/master")
    {
        r.push("control-plane".to_string());
    }
    if labels.contains_key("node-role.kubernetes.io/worker") {
        r.push("worker".to_string());
    }
    if r.is_empty() {
        r.push("worker".to_string());
    }
    r
}

/// Node system information, with `(unknown)` for missing fields.
struct SystemInfo {
    kernel_version: String,
    container_runtime: String,
    kubelet_version: String,
    os: String,
    arch: String,
}

fn system_info(status_info: &NodeStatus) -> SystemInfo {
    let node_info = status_info.node_info.as_ref();
    let field = |f: fn(&k8s_openapi::api::core::v1::NodeSystemInfo) -> &String| {
        node_info
            .map(|ni| f(ni).clone())
            .unwrap_or_else(|| "(unknown)".to_string())
    };
    SystemInfo {
        kernel_version: field(|ni| &ni.kernel_version),
        container_runtime: field(|ni| &ni.container_runtime_version),
        kubelet_version: field(|ni| &ni.kubelet_version),
        os: field(|ni| &ni.operating_system),
        arch: field(|ni| &ni.architecture),
    }
}

/// Returns `(capacity_cpu, capacity_memory, allocatable_cpu, allocatable_memory)` as raw
/// quantity strings, with `(unknown)` for missing values.
fn capacity_and_allocatable(status_info: &NodeStatus) -> (String, String, String, String) {
    let capacity = status_info.capacity.clone().unwrap_or_default();
    let allocatable = status_info.allocatable.clone().unwrap_or_default();
    let quantity =
        |map: &BTreeMap<String, k8s_openapi::apimachinery::pkg::api::resource::Quantity>,
         key: &str| {
            map.get(key)
                .map(|q| q.0.clone())
                .unwrap_or_else(|| "(unknown)".to_string())
        };

    (
        quantity(&capacity, "cpu"),
        quantity(&capacity, "memory"),
        quantity(&allocatable, "cpu"),
        quantity(&allocatable, "memory"),
    )
}

fn node_conditions(status_info: &NodeStatus) -> Vec<NodeCondition> {
    status_info
        .conditions
        .as_ref()
        .map(|conds| {
            conds
                .iter()
                .map(|c| NodeCondition {
                    condition_type: c.type_.clone(),
                    status: c.status.clone(),
                    reason: c.reason.clone().unwrap_or_default(),
                    message: c.message.clone().unwrap_or_default(),
                    last_transition_time: c
                        .last_transition_time
                        .as_ref()
                        .map(|t| super::workload_common::kube_rfc3339(&t))
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Count pods per node name (pods without a scheduled node are skipped).
pub(super) fn pod_counts_by_node(pods: Vec<Pod>) -> HashMap<String, u32> {
    let mut counts: HashMap<String, u32> = HashMap::new();
    for pod in pods {
        if let Some(node_name) = pod.spec.as_ref().and_then(|s| s.node_name.clone()) {
            *counts.entry(node_name).or_default() += 1;
        }
    }
    counts
}

/// Build the list-view summary for a node.
pub(super) fn node_summary(node: Node, pod_counts: &HashMap<String, u32>) -> NodeSummary {
    let meta = &node.metadata;
    let status = &node.status;
    let node_name = meta.name.clone().unwrap_or_default();
    let allocatable = status
        .as_ref()
        .and_then(|s| s.allocatable.as_ref())
        .cloned()
        .unwrap_or_default();

    // Parse CPU quantity: handles "4" (cores), "500m" (millicores), "1.5" (cores)
    let cpu_millicores = allocatable
        .get("cpu")
        .map(|q| parse_cpu_to_millicores(&q.0))
        .unwrap_or(0);

    // Parse memory quantity: handles Ki, Mi, Gi, M, G
    let mem_bytes = allocatable
        .get("memory")
        .map(|q| parse_memory_to_bytes(&q.0))
        .unwrap_or(0);

    // Parse node Ready status from conditions
    let node_status = status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .and_then(|conds| {
            conds.iter().find(|c| c.type_ == "Ready").map(|c| {
                if c.status == "True" {
                    "Ready"
                } else {
                    "NotReady"
                }
            })
        })
        .unwrap_or("Unknown")
        .to_string();

    let pod_count = pod_counts.get(&node_name).copied().unwrap_or(0);

    // Parse pod capacity from allocatable.pods
    let pod_capacity = allocatable
        .get("pods")
        .and_then(|q| q.0.parse::<u32>().ok())
        .unwrap_or(110); // 110 is the Kubernetes default

    NodeSummary {
        name: node_name,
        status: node_status,
        role: meta
            .labels
            .as_ref()
            .and_then(|l| l.get("node-role.kubernetes.io/master"))
            .or_else(|| {
                meta.labels
                    .as_ref()
                    .and_then(|l| l.get("node-role.kubernetes.io/control-plane"))
            })
            .map(|_| "control-plane".to_string())
            .unwrap_or_else(|| "worker".to_string()),
        pod_count,
        pod_capacity,
        cpu_allocatable_millicores: cpu_millicores,
        memory_allocatable_bytes: mem_bytes,
    }
}

/// Parse a Kubernetes CPU quantity string into millicores.
/// Handles: "4" (cores), "4000m" (millicores), "1.5" (cores as float)
fn parse_cpu_to_millicores(q: &str) -> i64 {
    if let Some(stripped) = q.strip_suffix('m') {
        // Already in millicores: "500m" -> 500
        stripped.parse::<i64>().unwrap_or(0)
    } else if let Ok(cores) = q.parse::<f64>() {
        // Whole or fractional cores: "4" -> 4000, "1.5" -> 1500
        (cores * 1000.0) as i64
    } else {
        0
    }
}

/// Parse a Kubernetes memory quantity string into bytes.
/// Handles: "2Gi", "2048Mi", "1984568Ki", "2G", "2M", "2K"
fn parse_memory_to_bytes(q: &str) -> i64 {
    let suffixes: &[(&str, i64)] = &[
        ("Gi", 1 << 30),
        ("Mi", 1 << 20),
        ("Ki", 1 << 10),
        ("G", 1_000_000_000),
        ("M", 1_000_000),
        ("K", 1_000),
    ];
    for (suffix, multiplier) in suffixes {
        if let Some(stripped) = q.strip_suffix(suffix) {
            return stripped.parse::<i64>().unwrap_or(0) * multiplier;
        }
    }
    q.parse::<i64>().unwrap_or(0)
}

/// Route for cluster-wide node data fetched from `context`.
fn nodes_route(context: Option<String>) -> Route {
    Route::cluster(section_keys::nodes(), context)
}

/// Best-effort: fetch node list and emit event.
pub(super) async fn refresh_nodes(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    tracing::debug!("RefreshNodes command received");
    let Some(client) = &state.cluster.client else {
        return;
    };
    // Capture the context at request start so a late result keeps its origin (FR-016).
    let context = state.cluster.current_context.clone();
    match fetch_node_summaries(client).await {
        Ok(summaries) => {
            tracing::debug!("Emitting NodesUpdated event with {} nodes", summaries.len());
            events::emit(
                state.cluster.seq.as_ref(),
                sender,
                nodes_route(context),
                Correlation::None,
                BackendEvent::ResourceList(ResourceListEvent::NodesUpdated(summaries)),
            );
        }
        Err(e) => {
            tracing::warn!("Failed to list nodes: {}", e);
            events::report_error(
                state.cluster.seq.as_ref(),
                sender,
                format!("Failed to list nodes: {}", e),
                super::ns_scope::is_forbidden(&e),
            );
        }
    }
}

/// List nodes and pods concurrently and build per-node summaries (with pod counts).
async fn fetch_node_summaries(client: &::kube::Client) -> Result<Vec<NodeSummary>, ::kube::Error> {
    let node_api = Api::<Node>::all(client.clone());
    let pod_api = Api::<Pod>::all(client.clone());
    // Fetch nodes and pods concurrently for pod_count per node
    let lp = ListParams::default();
    let (nodes_result, pods_result) = tokio::join!(node_api.list(&lp), pod_api.list(&lp));
    let nodes = nodes_result?;

    // Build a map of node_name -> pod count from running pods
    let pod_counts = pods_result
        .map(|pods| pod_counts_by_node(pods.items))
        .unwrap_or_default();

    tracing::debug!("Fetched {} nodes from API", nodes.items.len());
    Ok(nodes
        .items
        .into_iter()
        .map(|node| node_summary(node, &pod_counts))
        .collect())
}

/// Fetch node details.
pub(super) async fn select_node(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
) -> anyhow::Result<()> {
    tracing::debug!("SelectNode command received for: {}", name);
    let Some(client) = &state.cluster.client else {
        return Ok(());
    };
    let context = state.cluster.current_context.clone();
    let api = Api::<Node>::all(client.clone());
    match api.get(name).await {
        Ok(node) => {
            let details = extract_node_details(&node)?;
            events::emit(
                state.cluster.seq.as_ref(),
                sender,
                nodes_route(context),
                Correlation::None,
                BackendEvent::ResourceRead(ResourceReadEvent::NodeDetailsUpdated(details)),
            );
        }
        Err(e) => {
            tracing::warn!("Failed to get node details: {}", e);
            events::report_error(
                state.cluster.seq.as_ref(),
                sender,
                format!("Failed to get node details: {}", e),
                super::ns_scope::is_forbidden(&e),
            );
        }
    }
    Ok(())
}

/// Build the JSON merge-patch body toggling `spec.unschedulable`.
fn unschedulable_patch(unschedulable: bool) -> serde_json::Value {
    json!({ "spec": { "unschedulable": unschedulable } })
}

async fn set_unschedulable(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
    unschedulable: bool,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api = Api::<Node>::all(client);
    let patch = unschedulable_patch(unschedulable);
    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    refresh_nodes(state, sender).await;
    select_node(state, sender, name).await?;
    Ok(())
}

/// Cordon a node (mark it unschedulable).
pub(super) async fn cordon_node(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
) -> anyhow::Result<()> {
    set_unschedulable(state, sender, name, true).await
}

/// Uncordon a node (mark it schedulable again).
pub(super) async fn uncordon_node(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
) -> anyhow::Result<()> {
    set_unschedulable(state, sender, name, false).await
}

/// Compute the new full taint list for a node after adding or removing `taint`, per kubectl
/// semantics: taints are matched by `(key, effect)`; on remove, `value` is informational only
/// (not required to match) since kubectl's `-` suffix removal also ignores value.
fn apply_taint(
    existing: &[Taint],
    op: TaintOp,
    key: &str,
    value: Option<&str>,
    effect: &str,
) -> Vec<Taint> {
    let mut taints: Vec<Taint> = existing
        .iter()
        .filter(|t| !(t.key == key && t.effect == effect))
        .cloned()
        .collect();
    if op == TaintOp::Add {
        taints.push(Taint {
            effect: effect.to_string(),
            key: key.to_string(),
            time_added: None,
            value: value.map(|v| v.to_string()),
        });
    }
    taints
}

/// Build the JSON merge-patch body replacing `spec.taints` wholesale (the field uses an
/// atomic-list merge strategy server-side, so a partial/keyed merge is not possible).
fn taints_patch(taints: &[Taint]) -> serde_json::Value {
    json!({ "spec": { "taints": taints } })
}

/// Add or remove a taint on a node.
pub(super) async fn taint_node(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
    op: TaintOp,
    key: String,
    value: Option<String>,
    effect: String,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api = Api::<Node>::all(client);
    let node = api.get(name).await?;
    let existing = node
        .spec
        .as_ref()
        .and_then(|s| s.taints.clone())
        .unwrap_or_default();
    let taints = apply_taint(&existing, op, &key, value.as_deref(), &effect);
    let patch = taints_patch(&taints);
    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    refresh_nodes(state, sender).await;
    select_node(state, sender, name).await?;
    Ok(())
}

/// True if `pod` has no owning controller (a "bare" pod): draining it would delete it
/// permanently with nothing to recreate it, so `kubectl drain`-style tooling refuses by default.
fn is_bare_pod(pod: &Pod) -> bool {
    pod.metadata
        .owner_references
        .as_ref()
        .is_none_or(|owners| owners.is_empty())
}

/// True if `pod` is owned (directly or not — we only need the immediate owner reference,
/// matching kubectl's own check) by a DaemonSet.
fn is_daemonset_pod(pod: &Pod) -> bool {
    pod.metadata
        .owner_references
        .as_ref()
        .is_some_and(|owners| owners.iter().any(|o| o.kind == "DaemonSet"))
}

/// True if `pod` is still active on the node (not already terminal/terminating), i.e. a pod
/// that draining actually needs to act on.
fn is_active_pod(pod: &Pod) -> bool {
    if pod.metadata.deletion_timestamp.is_some() {
        return false;
    }
    let phase = pod
        .status
        .as_ref()
        .and_then(|s| s.phase.as_deref())
        .unwrap_or("");
    !matches!(phase, "Succeeded" | "Failed")
}

/// Find the first bare (unowned) active pod in `pods`, if any.
fn find_bare_pod(pods: &[Pod]) -> Option<&Pod> {
    pods.iter()
        .filter(|p| is_active_pod(p))
        .find(|p| is_bare_pod(p))
}

/// List active pods scheduled on `name`.
async fn active_node_pods(client: &::kube::Client, name: &str) -> Result<Vec<Pod>, ::kube::Error> {
    let api = Api::<Pod>::all(client.clone());
    let lp = ListParams::default().fields(&format!("spec.nodeName={name}"));
    let pods = api.list(&lp).await?;
    Ok(pods.items.into_iter().filter(is_active_pod).collect())
}

/// Emit an incremental `DrainProgress` snapshot for the in-flight drain identified by
/// `request_id`.
#[allow(clippy::too_many_arguments)]
fn emit_drain_progress(
    seq: &std::sync::atomic::AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    context: Option<String>,
    request_id: String,
    total_pods: u32,
    evicted: Vec<String>,
    blocked_on: Vec<(String, String)>,
    ignored_daemonset_pods: Vec<String>,
) {
    events::emit(
        seq,
        sender,
        nodes_route(context),
        Correlation::None,
        BackendEvent::Stream(StreamEvent::DrainProgress {
            request_id,
            total_pods,
            evicted,
            blocked_on,
            ignored_daemonset_pods,
        }),
    );
}

fn emit_node_action_finished(
    seq: &std::sync::atomic::AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    context: Option<String>,
    request_id: String,
    ok: bool,
    error: Option<String>,
) {
    events::emit(
        seq,
        sender,
        nodes_route(context),
        Correlation::None,
        BackendEvent::Stream(StreamEvent::NodeActionFinished {
            request_id,
            ok,
            error,
        }),
    );
}

/// Drain a node: evict every active, non-DaemonSet pod scheduled on it.
///
/// Refuses up front (before any eviction call) if the node hosts a bare pod (no owning
/// controller), matching `kubectl drain`'s default (non-`--force`) behavior. DaemonSet pods are
/// filtered out when `ignore_daemonsets` is set (never evicted, reported separately) — otherwise
/// they are treated like any other pod and the eviction call is left to fail naturally.
/// PodDisruptionBudget blocks are surfaced verbatim from the Eviction API's own 429 response,
/// never recomputed client-side. Runs as a background task tracked in
/// `BackendState::node_actions` so `CancelDrainNode` can abort it, preserving whatever partial
/// progress was already reported.
pub(super) async fn drain_node(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: String,
    ignore_daemonsets: bool,
    _force_delete_empty_dir: bool,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let context = state.cluster.current_context.clone();
    let request_id = events::current_request_id_string();

    let pods = active_node_pods(&client, &name).await?;

    if let Some(bare) = find_bare_pod(&pods) {
        let pod_name = bare.metadata.name.clone().unwrap_or_default();
        let message = format!(
            "Refusing to drain '{name}': pod '{pod_name}' has no owning controller (bare pod). \
             Delete it manually first if you really want to drain this node."
        );
        events::notify_error(state.cluster.seq.as_ref(), sender, message.clone());
        emit_node_action_finished(
            state.cluster.seq.as_ref(),
            sender,
            context,
            request_id,
            false,
            Some(message),
        );
        return Ok(());
    }

    let (to_evict, ignored_daemonset_pods): (Vec<Pod>, Vec<String>) = {
        let mut evict_list = Vec::new();
        let mut ignored = Vec::new();
        for pod in pods {
            if ignore_daemonsets && is_daemonset_pod(&pod) {
                ignored.push(pod.metadata.name.clone().unwrap_or_default());
            } else {
                evict_list.push(pod);
            }
        }
        (evict_list, ignored)
    };

    let total_pods = to_evict.len() as u32;
    let sender = sender.clone();
    let seq = state.cluster.seq.clone();
    let request_id_for_task = request_id.clone();
    let context_for_task = context.clone();

    let handle = tokio::spawn(async move {
        let pod_api = Api::<Pod>::all(client);
        let ep = EvictParams::default();
        let mut evicted: Vec<String> = Vec::new();
        let mut blocked_on: Vec<(String, String)> = Vec::new();

        emit_drain_progress(
            seq.as_ref(),
            &sender,
            context_for_task.clone(),
            request_id_for_task.clone(),
            total_pods,
            evicted.clone(),
            blocked_on.clone(),
            ignored_daemonset_pods.clone(),
        );

        for pod in &to_evict {
            let pod_name = pod.metadata.name.clone().unwrap_or_default();
            match pod_api.evict(&pod_name, &ep).await {
                Ok(_) => evicted.push(pod_name),
                Err(e) => blocked_on.push((pod_name, e.to_string())),
            }
            emit_drain_progress(
                seq.as_ref(),
                &sender,
                context_for_task.clone(),
                request_id_for_task.clone(),
                total_pods,
                evicted.clone(),
                blocked_on.clone(),
                ignored_daemonset_pods.clone(),
            );
        }

        emit_node_action_finished(
            seq.as_ref(),
            &sender,
            context_for_task,
            request_id_for_task,
            true,
            None,
        );
    });

    state.node_actions.drain_tasks.insert(request_id, handle);
    Ok(())
}

/// Cancel an in-flight drain, aborting its background task (preserving whatever `DrainProgress`
/// was already reported) and notifying the frontend the drain ended.
pub(super) fn cancel_drain_node(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    _name: &str,
    request_id: &str,
) {
    let context = state.cluster.current_context.clone();
    let cancelled = state.node_actions.cancel(request_id);
    if cancelled {
        emit_node_action_finished(
            state.cluster.seq.as_ref(),
            sender,
            context,
            request_id.to_string(),
            false,
            Some("Drain cancelled by operator.".to_string()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ScopeKind;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};

    #[test]
    fn nodes_route_carries_originating_context() {
        let route = nodes_route(Some("kind-b".to_owned()));
        assert_eq!(route.context.as_deref(), Some("kind-b"));
        assert_eq!(route.scope, ScopeKind::Cluster);
        assert_eq!(route.section, section_keys::nodes());
    }

    fn owned_pod(name: &str, owner_kind: &str) -> Pod {
        Pod {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                owner_references: Some(vec![OwnerReference {
                    api_version: "apps/v1".to_string(),
                    kind: owner_kind.to_string(),
                    name: "owner".to_string(),
                    uid: "uid".to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn bare_pod(name: &str) -> Pod {
        Pod {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                owner_references: None,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn bare_pod_precheck_refuses_before_any_eviction_call() {
        // A node hosting one bare pod (no owner) and one ReplicaSet-owned pod must be
        // detected as having a bare pod, so `drain_node` can refuse up front (T033) —
        // this test asserts the pure detection helper used for that up-front check,
        // independent of any live eviction call.
        let pods = vec![owned_pod("owned-1", "ReplicaSet"), bare_pod("bare-1")];
        let found = find_bare_pod(&pods);
        assert_eq!(
            found.map(|p| p.metadata.name.clone().unwrap()),
            Some("bare-1".to_string())
        );
    }

    #[test]
    fn bare_pod_precheck_passes_when_all_pods_are_owned() {
        let pods = vec![
            owned_pod("owned-1", "ReplicaSet"),
            owned_pod("owned-2", "DaemonSet"),
        ];
        assert!(find_bare_pod(&pods).is_none());
    }

    #[test]
    fn daemonset_pod_detection() {
        let ds_pod = owned_pod("ds-1", "DaemonSet");
        let rs_pod = owned_pod("rs-1", "ReplicaSet");
        assert!(is_daemonset_pod(&ds_pod));
        assert!(!is_daemonset_pod(&rs_pod));
    }

    #[test]
    fn cordon_patch_sets_unschedulable_true() {
        let patch = unschedulable_patch(true);
        assert_eq!(patch, json!({ "spec": { "unschedulable": true } }));
    }

    #[test]
    fn uncordon_patch_sets_unschedulable_false() {
        let patch = unschedulable_patch(false);
        assert_eq!(patch, json!({ "spec": { "unschedulable": false } }));
    }

    fn taint(key: &str, value: Option<&str>, effect: &str) -> Taint {
        Taint {
            effect: effect.to_string(),
            key: key.to_string(),
            time_added: None,
            value: value.map(|v| v.to_string()),
        }
    }

    #[test]
    fn taint_add_appends_new_taint() {
        let existing = vec![taint("existing", Some("v1"), "NoSchedule")];
        let result = apply_taint(
            &existing,
            TaintOp::Add,
            "new-key",
            Some("new-val"),
            "NoExecute",
        );
        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|t| t.key == "new-key"
            && t.effect == "NoExecute"
            && t.value.as_deref() == Some("new-val")));
        assert!(result.iter().any(|t| t.key == "existing"));
    }

    #[test]
    fn taint_add_replaces_existing_same_key_and_effect() {
        let existing = vec![taint("key-a", Some("old"), "NoSchedule")];
        let result = apply_taint(&existing, TaintOp::Add, "key-a", Some("new"), "NoSchedule");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].value.as_deref(), Some("new"));
    }

    #[test]
    fn taint_remove_drops_matching_key_and_effect_ignoring_value() {
        let existing = vec![
            taint("key-a", Some("v1"), "NoSchedule"),
            taint("key-b", Some("v2"), "NoExecute"),
        ];
        // Value passed on remove is informational only — must not need to match.
        let result = apply_taint(
            &existing,
            TaintOp::Remove,
            "key-a",
            Some("does-not-match"),
            "NoSchedule",
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].key, "key-b");
    }

    #[test]
    fn taint_remove_leaves_taints_with_different_effect_untouched() {
        let existing = vec![
            taint("key-a", None, "NoSchedule"),
            taint("key-a", None, "NoExecute"),
        ];
        let result = apply_taint(&existing, TaintOp::Remove, "key-a", None, "NoSchedule");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].effect, "NoExecute");
    }

    #[test]
    fn taints_patch_serializes_whole_array() {
        let taints = vec![taint("key-a", Some("v"), "NoSchedule")];
        let patch = taints_patch(&taints);
        assert_eq!(patch["spec"]["taints"][0]["key"], "key-a");
        assert_eq!(patch["spec"]["taints"][0]["effect"], "NoSchedule");
    }
}
