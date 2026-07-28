use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleBindingSummary {
    pub name: String,
    pub namespace: String,
    pub role_ref: String,
    pub subjects_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubjectSummary {
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleBindingDetails {
    pub name: String,
    pub namespace: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub role_ref: String,
    pub subjects: Vec<SubjectSummary>,
    pub raw_json: String,
}
