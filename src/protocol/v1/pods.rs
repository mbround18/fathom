use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodSummary {
    pub name: String,
    pub namespace: String,
    pub phase: String,
    pub node: String,
    pub ready: u32,
    pub total: u32,
    pub restarts: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodDetails {
    pub namespace: String,
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub containers: Vec<String>,
    pub raw_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodMetricsSummary {
    pub namespace: String,
    pub name: String,
    pub containers: Vec<ContainerMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerMetrics {
    pub name: String,
    pub cpu: String,
    pub memory: String,
}
