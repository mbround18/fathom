use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct NamespaceSummary {
    pub name: String,
    pub status: String, // Active, Terminating
    pub pod_count: u32,
    pub deployment_count: u32,
    pub creation_timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct NamespaceDetails {
    pub name: String,
    pub status: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub creation_timestamp: String,
    pub resource_quota_names: Vec<String>,
    pub limit_range_names: Vec<String>,
    pub raw_json: String,
}
