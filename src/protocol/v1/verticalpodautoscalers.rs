use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

/// A cpu/memory pair as Kubernetes quantity strings (empty when the resource is not reported).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct VpaResources {
    pub cpu: String,
    pub memory: String,
}

/// One container's recommendation from `status.recommendation.containerRecommendations`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct VpaContainerRecommendation {
    pub container: String,
    pub lower_bound: VpaResources,
    pub target: VpaResources,
    pub upper_bound: VpaResources,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct VerticalPodAutoscalerSummary {
    pub namespace: String,
    pub name: String,
    /// `Kind/name` of `spec.targetRef`.
    pub target_ref: String,
    /// `spec.updatePolicy.updateMode` (`Auto` when unset).
    pub update_mode: String,
    pub recommendations: Vec<VpaContainerRecommendation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct VerticalPodAutoscalerDetails {
    pub namespace: String,
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub target_ref: String,
    pub update_mode: String,
    pub recommendations: Vec<VpaContainerRecommendation>,
    /// `Type=Status (Reason)` per status condition.
    pub conditions: Vec<String>,
    pub raw_json: String,
}
