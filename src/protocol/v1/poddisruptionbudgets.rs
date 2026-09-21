use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PodDisruptionBudgetSummary {
    pub namespace: String,
    pub name: String,
    pub min_available: Option<String>,
    pub max_unavailable: Option<String>,
    pub current_healthy: Option<i32>,
    pub desired_healthy: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PodDisruptionBudgetDetails {
    pub namespace: String,
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub min_available: Option<String>,
    pub max_unavailable: Option<String>,
    pub current_healthy: Option<i32>,
    pub desired_healthy: Option<i32>,
    pub disruptions_allowed: Option<i32>,
    pub expected_pods: Option<i32>,
    pub raw_json: String,
}
