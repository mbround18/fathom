use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::PodSummary;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentSummary {
    pub name: String,
    pub namespace: String,

    pub desired: i32,
    pub ready: i32,
    pub updated: i32,
    pub available: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentDetails {
    pub namespace: String,
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub containers: Vec<String>,

    // Best-effort association: derived from `.spec.selector.matchLabels`.
    // If empty, we could not derive a selector.
    pub pod_selector: String,
    pub pods: Vec<PodSummary>,

    pub raw_json: String,
}
