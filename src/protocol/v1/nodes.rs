use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct NodeSummary {
    pub name: String,
    pub status: String, // Ready, NotReady, Unknown
    pub role: String,   // master, worker, or empty
    pub pod_count: u32,
    pub pod_capacity: u32,
    pub cpu_allocatable_millicores: i64,
    pub memory_allocatable_bytes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct NodeDetails {
    pub name: String,
    pub status: String,
    pub roles: Vec<String>,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub creation_timestamp: String,
    pub kernel_version: String,
    pub container_runtime: String,
    pub kubelet_version: String,
    pub os: String,
    pub arch: String,
    pub capacity_cpu: String,
    pub capacity_memory: String,
    pub allocatable_cpu: String,
    pub allocatable_memory: String,
    pub pod_count: u32,
    pub pod_capacity: u32,
    pub conditions: Vec<NodeCondition>,
    pub raw_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct NodeCondition {
    pub condition_type: String, // Ready, MemoryPressure, DiskPressure, etc.
    pub status: String,         // True, False, Unknown
    pub reason: String,
    pub message: String,
    pub last_transition_time: String,
}
