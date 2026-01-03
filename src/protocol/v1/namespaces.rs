use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceSummary {
    pub name: String,
    pub status: String, // Active, Terminating
    pub pod_count: u32,
    pub deployment_count: u32,
    pub creation_timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceDetails {
    pub name: String,
    pub status: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub creation_timestamp: String,
    pub pod_count: u32,
    pub deployment_count: u32,
    pub statefulset_count: u32,
    pub daemonset_count: u32,
    pub job_count: u32,
    pub cronjob_count: u32,
    pub service_count: u32,
    pub configmap_count: u32,
    pub secret_count: u32,
    pub raw_json: String,
}
