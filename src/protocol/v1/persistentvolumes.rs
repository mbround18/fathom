use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PersistentVolumeSummary {
    pub name: String,
    pub capacity: String,
    pub access_modes: Vec<String>,
    pub status: String,
    pub storage_class: Option<String>,
    pub reclaim_policy: String,
    pub claim_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PersistentVolumeDetails {
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub capacity: String,
    pub access_modes: Vec<String>,
    pub status: String,
    pub storage_class: Option<String>,
    pub reclaim_policy: String,
    pub claim_ref: Option<String>,
    pub volume_mode: Option<String>,
    pub csi_driver: Option<String>,
    pub raw_json: String,
}
