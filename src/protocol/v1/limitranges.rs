use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LimitRangeSummary {
    pub name: String,
    pub namespace: String,
    pub limits_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LimitRangeItemSummary {
    pub limit_type: String,
    pub max: BTreeMap<String, String>,
    pub min: BTreeMap<String, String>,
    pub default: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LimitRangeDetails {
    pub namespace: String,
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub limits: Vec<LimitRangeItemSummary>,
    pub raw_json: String,
}
