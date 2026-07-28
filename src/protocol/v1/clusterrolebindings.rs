use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::rolebindings::SubjectSummary;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterRoleBindingSummary {
    pub name: String,
    pub role_ref: String,
    pub subjects_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterRoleBindingDetails {
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub role_ref: String,
    pub subjects: Vec<SubjectSummary>,
    pub raw_json: String,
}
