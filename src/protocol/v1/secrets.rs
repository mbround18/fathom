use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretSummary {
    pub name: String,
    pub namespace: String,
    pub secret_type: String,
    pub keys_count: usize,
}

/// SECURITY: This struct must never contain decoded/raw secret values.
/// Only key names and the secret type are exposed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretDetails {
    pub namespace: String,
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub secret_type: String,
    pub keys: Vec<String>,
    pub raw_json: String,
}
