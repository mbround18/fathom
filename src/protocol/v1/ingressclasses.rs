use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct IngressClassSummary {
    pub name: String,
    pub controller: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct IngressClassDetails {
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub controller: String,
    pub parameters: Option<IngressClassParameters>,
    pub raw_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct IngressClassParameters {
    pub api_group: Option<String>,
    pub kind: String,
    pub namespace: Option<String>,
    pub name: String,
}
