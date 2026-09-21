use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RoleSummary {
    pub name: String,
    pub namespace: String,
    pub rules_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PolicyRuleSummary {
    pub api_groups: Vec<String>,
    pub resources: Vec<String>,
    pub verbs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RoleDetails {
    pub name: String,
    pub namespace: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub rules: Vec<PolicyRuleSummary>,
    pub raw_json: String,
}
