use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PriorityClassSummary {
    pub name: String,
    pub value: i32,
    pub global_default: bool,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PriorityClassDetails {
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub value: i32,
    pub global_default: bool,
    pub description: Option<String>,
    pub preemption_policy: Option<String>,
    pub raw_json: String,
}
