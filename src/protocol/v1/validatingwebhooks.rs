use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ValidatingWebhookConfigurationSummary {
    pub name: String,
    pub webhooks_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ValidatingWebhookSummary {
    pub name: String,
    pub failure_policy: Option<String>,
    pub side_effects: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ValidatingWebhookConfigurationDetails {
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub webhooks: Vec<ValidatingWebhookSummary>,
    pub raw_json: String,
}
