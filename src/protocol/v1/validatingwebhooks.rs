use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatingWebhookConfigurationSummary {
    pub name: String,
    pub webhooks_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatingWebhookSummary {
    pub name: String,
    pub failure_policy: Option<String>,
    pub side_effects: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatingWebhookConfigurationDetails {
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub webhooks: Vec<ValidatingWebhookSummary>,
    pub raw_json: String,
}
