use serde::{Deserialize, Serialize};

/// Reconciliation health derived from a Flux resource's `Ready` condition,
/// `spec.suspend`, and whether `status.observedGeneration` has caught up to
/// `metadata.generation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FluxReadyState {
    Ready,
    Failed,
    Reconciling,
    Suspended,
    Unknown,
}

impl FluxReadyState {
    pub fn label(&self) -> &'static str {
        match self {
            FluxReadyState::Ready => "Ready",
            FluxReadyState::Failed => "Failed",
            FluxReadyState::Reconciling => "Reconciling",
            FluxReadyState::Suspended => "Suspended",
            FluxReadyState::Unknown => "Unknown",
        }
    }
}

/// One Flux GitOps Toolkit resource (a source, a Kustomization, or a HelmRelease).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FluxResourceSummary {
    /// CRD kind, e.g. "GitRepository", "OCIRepository", "HelmRepository",
    /// "Bucket", "Kustomization", "HelmRelease".
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub ready: FluxReadyState,
    /// Message from the `Ready` condition (or a fallback status line).
    pub message: String,
    /// `status.artifact.revision` for sources, `status.lastAppliedRevision`
    /// (falling back to `status.lastAttemptedRevision`) for Kustomizations/HelmReleases.
    pub revision: String,
    /// `spec.sourceRef` formatted as "Kind/name", for Kustomizations and HelmReleases.
    pub source_ref: Option<String>,
    pub suspended: bool,
    pub interval: String,
    /// ISO8601 timestamp of the `Ready` condition's last transition.
    pub last_transition: String,
}

/// Aggregated Flux state for the dashboard: whether the toolkit is installed
/// at all, plus every source, Kustomization, and HelmRelease found.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FluxOverview {
    pub installed: bool,
    pub sources: Vec<FluxResourceSummary>,
    pub kustomizations: Vec<FluxResourceSummary>,
    pub helmreleases: Vec<FluxResourceSummary>,
}
