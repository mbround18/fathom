use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::roles::PolicyRuleSummary;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterRoleSummary {
    pub name: String,
    pub rules_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterRoleDetails {
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub rules: Vec<PolicyRuleSummary>,
    pub raw_json: String,
}
