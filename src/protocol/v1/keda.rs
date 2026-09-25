use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

/// Dashboard-level KEDA aggregate (029-keda-autoscaler-pane), analogous to `FluxOverview`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct KedaOverview {
    /// True when `scaledobjects.keda.sh` or `scaledjobs.keda.sh` is a served CRD.
    pub installed: bool,
    pub scaled_objects: Vec<KedaResourceSummary>,
    pub scaled_jobs: Vec<KedaResourceSummary>,
}

/// One row in the Autoscalers list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct KedaResourceSummary {
    /// `"ScaledObject"` or `"ScaledJob"`.
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub target_kind: String,
    pub target_name: String,
    /// False when the target workload no longer exists.
    pub target_found: bool,
    pub current_replicas: i32,
    pub min_replicas: i32,
    pub max_replicas: i32,
    pub paused: bool,
    pub ready: bool,
    pub trigger_types: Vec<String>,
    pub age: String,
}

/// Inspector payload for `SelectScaledObject`/`SelectScaledJob`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct KedaResourceDetails {
    pub summary: KedaResourceSummary,
    pub triggers: Vec<KedaTrigger>,
    pub reasoning: ScaleReasoning,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct KedaTrigger {
    pub trigger_type: String,
    pub metadata: BTreeMap<String, String>,
    pub auth_ref: Option<TriggerAuthRef>,
    pub active: Option<bool>,
    /// Only populated for `prometheus` triggers when the metrics integration is reachable.
    pub current_value: Option<f64>,
    pub threshold: Option<String>,
}

/// Reference to a (Cluster)TriggerAuthentication by name only; never carries credentials.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct TriggerAuthRef {
    pub name: String,
    pub cluster_scoped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum ScaleReasoningState {
    AtFloor,
    AtCeiling,
    MidRange,
    Paused,
    ScaledToZero,
    TargetNotFound,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ScaleReasoning {
    pub state: ScaleReasoningState,
    pub message: String,
}
