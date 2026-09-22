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

// --- Helm mutate (006): rollback, upgrade, uninstall preview/apply/failure payloads ---

/// Which of the three mutate flows a preview/apply/failure event belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum HelmMutationAction {
    Rollback,
    Upgrade,
    Uninstall,
}

/// Which stage of a mutate flow failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum HelmMutationStage {
    Preview,
    Apply,
}

/// Emitted after a rollback/upgrade diff-preview is computed, before any mutation.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct HelmMutationPreviewReady {
    pub namespace: String,
    pub release: String,
    pub action: HelmMutationAction,
    /// The live revision the preview was computed against.
    pub base_revision: i64,
    pub current_manifest: String,
    /// Rollback: target revision's stored manifest verbatim.
    /// Upgrade: freshly `helm upgrade --dry-run` rendered manifest.
    pub target_manifest: String,
    /// Rollback only: the revision being rolled back to.
    pub target_revision: Option<i64>,
    /// Upgrade only: chart version the upgrade would produce, if changed.
    pub target_chart_version: Option<String>,
    /// Reuses `HelmReleaseDetails.origin` verbatim; `None` means no GitOps warning.
    pub gitops_warning: Option<GitOpsOriginRef>,
}

/// Emitted after a rollback/upgrade/uninstall mutation succeeds.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct HelmMutationApplied {
    pub namespace: String,
    pub release: String,
    pub action: HelmMutationAction,
    /// Rollback/upgrade: the new revision number. Uninstall: `None` (release is gone).
    pub new_revision: Option<i64>,
}

/// Emitted when a preview or apply stage of a mutate flow fails.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct HelmMutationFailed {
    pub namespace: String,
    pub release: String,
    pub action: HelmMutationAction,
    pub stage: HelmMutationStage,
    /// Verbatim backend error (Constitution Principle I).
    pub message: String,
}
