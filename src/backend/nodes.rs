use crate::protocol::{NodeCondition, NodeDetails};
use k8s_openapi::api::core::v1::Node;

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

    // Extract roles from labels
    let roles = {
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
    };

    // Extract system information
    let node_info = status_info.node_info.as_ref();
    let kernel_version = node_info
        .map(|ni| ni.kernel_version.clone())
        .unwrap_or_else(|| "(unknown)".to_string());
    let container_runtime = node_info
        .map(|ni| ni.container_runtime_version.clone())
        .unwrap_or_else(|| "(unknown)".to_string());
    let kubelet_version = node_info
        .map(|ni| ni.kubelet_version.clone())
        .unwrap_or_else(|| "(unknown)".to_string());
    let os = node_info
        .map(|ni| ni.operating_system.clone())
        .unwrap_or_else(|| "(unknown)".to_string());
    let arch = node_info
        .map(|ni| ni.architecture.clone())
        .unwrap_or_else(|| "(unknown)".to_string());

    // Extract capacity and allocatable resources
    let (capacity_cpu, capacity_memory, allocatable_cpu, allocatable_memory) = {
        let capacity = status_info.capacity.clone().unwrap_or_default();
        let allocatable = status_info.allocatable.clone().unwrap_or_default();

        let cpu_cap = capacity
            .get("cpu")
            .map(|q| q.0.clone())
            .unwrap_or_else(|| "(unknown)".to_string());
        let mem_cap = capacity
            .get("memory")
            .map(|q| q.0.clone())
            .unwrap_or_else(|| "(unknown)".to_string());

        let cpu_alloc = allocatable
            .get("cpu")
            .map(|q| q.0.clone())
            .unwrap_or_else(|| "(unknown)".to_string());
        let mem_alloc = allocatable
            .get("memory")
            .map(|q| q.0.clone())
            .unwrap_or_else(|| "(unknown)".to_string());

        (cpu_cap, mem_cap, cpu_alloc, mem_alloc)
    };

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
    let conditions = status_info
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
        .unwrap_or_default();

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
