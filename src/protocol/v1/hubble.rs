use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Coarse classification of a Hubble flow's verdict, mirrored from Cilium's
/// `flow.Verdict` enum (vendored under `proto/hubble/flow/flow.proto`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum HubbleVerdict {
    Forwarded,
    Dropped,
    Audit,
    Error,
    Other,
}

/// One observed `(source namespace, destination namespace)` pair and how many
/// flows in the sampled window matched it.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct HubbleFlowPair {
    pub source_namespace: String,
    pub dest_namespace: String,
    pub count: u32,
}

/// One drop reason (`flow.DropReason` name, e.g. "POLICY_DENIED") and how many
/// dropped flows in the sampled window carried it.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct HubbleDropReason {
    pub reason: String,
    pub count: u32,
}

/// Aggregated Hubble/Cilium traffic summary for the dashboard, built from a
/// bounded `Observer.GetFlows` sample (last N flows, not a live stream).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
pub struct HubbleTrafficSummary {
    pub installed: bool,
    /// ISO8601 timestamp this summary was generated at.
    pub generated_at: String,
    pub total_flows: u32,
    pub forwarded: u32,
    pub dropped: u32,
    pub top_pairs: Vec<HubbleFlowPair>,
    pub top_drop_reasons: Vec<HubbleDropReason>,
}
