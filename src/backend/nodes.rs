use std::collections::{BTreeMap, HashMap};

use crate::protocol::v1::NodeSummary;
use crate::protocol::{NodeCondition, NodeDetails};
use k8s_openapi::api::core::v1::{Node, NodeStatus, Pod};

/// Extract detailed information from a Node resource
pub fn extract_node_details(node: &Node) -> anyhow::Result<NodeDetails> {
    let meta = &node.metadata;
    let status = node.status.as_ref();

    let name = meta.name.clone().unwrap_or_default();
    let creation_timestamp = meta
        .creation_timestamp
        .as_ref()
        .map(|t| t.0.to_rfc3339())
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
                        .map(|t| t.0.to_rfc3339())
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
