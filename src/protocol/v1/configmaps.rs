use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ConfigMapSummary {
    pub name: String,
    pub namespace: String,
    pub keys_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ConfigMapDetails {
    pub namespace: String,
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub keys: Vec<String>,
    pub data: BTreeMap<String, String>,
    pub raw_json: String,
}
