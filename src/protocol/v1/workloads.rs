use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

use super::PodSummary;

/// Generic workload summary used by multiple workload kinds.
///
/// The numeric fields are intentionally normalized to match the Deployments UI columns.
/// For kinds that don't have a perfect 1:1 mapping, values are best-effort.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct WorkloadSummary {
    pub name: String,
    pub namespace: String,

    pub desired: i32,
    pub ready: i32,
    pub updated: i32,
    pub available: i32,
}

/// Generic workload details used by multiple workload kinds.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct WorkloadDetails {
    pub namespace: String,
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub containers: Vec<String>,

    // Container ports from pod template spec
    pub ports: Vec<super::ContainerPort>,

    // Best-effort association: derived from selectors (usually `.spec.selector.matchLabels`).
    // If empty, we could not derive a selector.
    pub pod_selector: String,
    pub pods: Vec<PodSummary>,

    pub raw_json: String,
}
