use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Live control-plane/kubelet version snapshot for the connected cluster (019).
///
/// Read-only, ambient status only: no historical/prior values are retained anywhere in this
/// type (spec FR-009) — each emission fully replaces the frontend's prior state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
pub struct ClusterVersionSummary {
    /// `/version` discovery `gitVersion`, e.g. "v1.29.4". `None` when discovery failed or timed
    /// out (spec FR-005 — the badge shows a neutral "unknown" state in that case).
    pub server_version: Option<String>,
    /// Distinct `status.nodeInfo.kubeletVersion` values across nodes visible to the operator's
    /// current RBAC, sorted for stable ordering. Empty when no nodes are visible/loaded.
    pub node_kubelet_versions: Vec<String>,
    /// `true` only when `server_version` is `Some` and at least one entry in
    /// `node_kubelet_versions` differs from it. MUST be `false` whenever `server_version` is
    /// `None` or `node_kubelet_versions` is empty (nothing to compare).
    pub skew_detected: bool,
}
