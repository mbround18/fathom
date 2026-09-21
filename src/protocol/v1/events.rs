use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct KubernetesEvent {
    pub namespace: String,
    pub name: String,
    pub event_type: String,           // Normal, Warning
    pub involved_object_kind: String, // Pod, Deployment, Node, etc.
    pub involved_object_name: String,
    pub reason: String,
    pub message: String,
    pub count: u32,
    pub first_timestamp: String,
    pub last_timestamp: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct EventsSummary {
    pub events: Vec<KubernetesEvent>,
    pub total_count: u32,
}
