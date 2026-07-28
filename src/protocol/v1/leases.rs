use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseSummary {
    pub namespace: String,
    pub name: String,
    pub holder_identity: Option<String>,
    pub renew_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseDetails {
    pub namespace: String,
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub holder_identity: Option<String>,
    pub lease_duration_seconds: Option<i32>,
    pub acquire_time: Option<String>,
    pub renew_time: Option<String>,
    pub raw_json: String,
}
