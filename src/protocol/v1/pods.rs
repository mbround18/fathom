use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PodSummary {
    pub name: String,
    pub namespace: String,
    pub phase: String,
    pub node: String,
    pub ready: u32,
    pub total: u32,
    pub restarts: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PodOwner {
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ContainerResourceSpec {
    pub container: String,
    pub cpu_request_millicores: Option<f64>,
    pub cpu_limit_millicores: Option<f64>,
    pub memory_request_bytes: Option<f64>,
    pub memory_limit_bytes: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PodDetails {
    pub namespace: String,
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub containers: Vec<String>,
    pub ports: Vec<ContainerPort>,
    pub resource_specs: Vec<ContainerResourceSpec>,

    /// Best-effort controller owner (from metadata.ownerReferences).
    pub owner: Option<PodOwner>,
    /// Best-effort resolved owner (e.g. ReplicaSet -> Deployment).
    pub resolved_owner: Option<PodOwner>,

    /// Names of PersistentVolumeClaims referenced by this Pod (best-effort)
    pub persistent_volume_claims: Vec<String>,

    pub raw_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PodMetricsSummary {
    pub namespace: String,
    pub name: String,
    pub containers: Vec<ContainerMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ContainerMetrics {
    pub name: String,
    pub cpu: String,
    pub memory: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct MetricsSample {
    pub timestamp: String, // ISO8601
    pub cpu_millicores: f64,
    pub memory_bytes: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ContainerMetricsHistory {
    pub name: String,
    pub samples: Vec<MetricsSample>,
    pub cpu_request_millicores: Option<f64>,
    pub cpu_limit_millicores: Option<f64>,
    pub memory_request_bytes: Option<f64>,
    pub memory_limit_bytes: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PodMetricsHistory {
    pub namespace: String,
    pub name: String,
    pub containers: Vec<ContainerMetricsHistory>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ContainerPort {
    pub container: String,
    pub name: Option<String>,
    pub port: u16,
    pub protocol: String,
}
