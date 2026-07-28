use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HorizontalPodAutoscalerSummary {
    pub namespace: String,
    pub name: String,
    pub target_ref: String,
    pub min_replicas: Option<i32>,
    pub max_replicas: i32,
    pub current_replicas: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HorizontalPodAutoscalerDetails {
    pub namespace: String,
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub target_ref: String,
    pub min_replicas: Option<i32>,
    pub max_replicas: i32,
    pub current_replicas: Option<i32>,
    pub desired_replicas: i32,
    pub conditions: Vec<String>,
    pub raw_json: String,
}
