use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceQuotaSummary {
    pub name: String,
    pub namespace: String,
    pub hard_limits_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceQuotaDetails {
    pub namespace: String,
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub hard: BTreeMap<String, String>,
    pub used: BTreeMap<String, String>,
    pub raw_json: String,
}
