use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::applications::RelatedResourceRef;
use super::flux::FluxReadyState;
use super::route::SectionKey;

/// Mirrors Helm's own `status` field verbatim (no reinterpretation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum HelmReleaseStatus {
    Deployed,
    Failed,
    PendingInstall,
    PendingUpgrade,
    PendingRollback,
    Superseded,
    Uninstalled,
    Uninstalling,
    Unknown,
}

/// Which GitOps controller produced a [`GitOpsOriginRef`]. A one-variant enum today so a
/// future `Argo` variant is additive, not a breaking rename.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum GitOpsController {
    Flux,
}

/// One row in the Helm Releases list, read directly from Helm 3's own release-storage
/// Secret/ConfigMap objects (no `helm` binary shellout).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct HelmReleaseSummary {
    pub name: String,
    pub namespace: String,
    pub chart: String,
    pub chart_version: String,
    pub app_version: Option<String>,
    pub revision: i64,
    pub status: HelmReleaseStatus,
    pub updated: String,
    pub degraded: bool,
}

/// One prior (or current) revision's storage object (list-only, no diffing).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct HelmRevisionSummary {
    pub revision: i64,
    pub status: HelmReleaseStatus,
    pub description: Option<String>,
    pub updated: String,
    pub app_version: Option<String>,
    pub chart_version: String,
}

/// Generic origin-source pointer — must not be Flux-specific in shape, even though Flux is
/// the only source implemented today.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct GitOpsOriginRef {
    pub controller: GitOpsController,
    pub section: SectionKey,
    pub kind: String,
    pub namespace: String,
    pub name: String,
    pub ready: FluxReadyState,
    pub message: Option<String>,
}

/// Full detail for one selected Helm release.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct HelmReleaseDetails {
    pub name: String,
    pub namespace: String,
    pub chart: String,
    pub chart_version: String,
    pub app_version: Option<String>,
    pub revision: i64,
    pub status: HelmReleaseStatus,
    pub updated: String,
    pub description: Option<String>,
    pub values_user_supplied: String,
    pub values_effective: String,
    pub manifest: String,
    pub revisions: Vec<HelmRevisionSummary>,
    pub origin: Option<GitOpsOriginRef>,
    pub related: Vec<RelatedResourceRef>,
    pub degraded: bool,
}
