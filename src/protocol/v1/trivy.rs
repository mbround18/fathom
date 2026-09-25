//! Aqua Trivy Operator vulnerability data (021-trivy-vulnerability-pane): read-only surfacing
//! of `VulnerabilityReport` (and sibling `ExposedSecretReport`/`SbomReport`/
//! `RbacAssessmentReport`) CRDs, API group `aquasecurity.github.io`. Detection and CRD reads
//! happen only in the backend (`src/backend/trivy.rs`), mirroring the Flux GitOps Toolkit
//! integration's detect-CRD-then-show-live-pane-or-honest-empty-state pattern
//! (`src/protocol/v1/flux.rs`). Fathom never triggers or configures a Trivy scan; it only reads
//! reports the operator already produced. See specs/021-trivy-vulnerability-pane/spec.md.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A CVE's severity bucket, using Trivy's own vocabulary verbatim so operators recognize the
/// same labels they'd see via `kubectl get vulnerabilityreports`. `Ord` is declared worst-first
/// (`Critical < High < Medium < Low < Unknown`) so ascending sorts put the worst severity first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "UPPERCASE")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Unknown,
}

/// One CVE entry within a `VulnerabilityReport`, as surfaced in the Inspector's Vulnerabilities
/// tab (FR-003, FR-004).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct VulnerabilityFinding {
    pub cve_id: String,
    pub severity: Severity,
    pub package: String,
    pub installed_version: String,
    /// `None` when no fix is published yet.
    pub fixed_version: Option<String>,
    /// CVSS score, when Trivy reports one.
    pub score: Option<f64>,
    /// Which container/image this finding applies to (FR-004).
    pub container: String,
    pub image: String,
}

/// Per-workload or per-image severity aggregation (FR-002), used both by the Inspector tab and
/// by the cluster-wide summary's worst-offenders ranking.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, TS)]
pub struct SeveritySummary {
    pub critical: u32,
    pub high: u32,
    pub medium: u32,
    pub low: u32,
    pub unknown: u32,
}

impl SeveritySummary {
    /// Not serialized — a convenience accessor for the frontend/tests to sum the buckets.
    pub fn total(&self) -> u32 {
        self.critical + self.high + self.medium + self.low + self.unknown
    }
}

/// Result of inspecting one Pod's vulnerabilities (`InspectPodVulnerabilities`, US1).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PodVulnerabilityInspection {
    pub namespace: String,
    pub pod: String,
    /// The workload kind Trivy's reports were attributed to (owner-chain resolution).
    pub owner_kind: String,
    pub owner_name: String,
    pub summary: SeveritySummary,
    /// Capped at 50 items, sorted severity desc then score desc (FR-016).
    pub top_findings: Vec<VulnerabilityFinding>,
    /// The true count before the 50-item cap (FR-016).
    pub total_findings: u32,
    /// RFC3339 timestamp of the most recent contributing report.
    pub generated_at: String,
}

/// Every failure mode the Vulnerabilities tab must render as an explicit, distinguishable state
/// (FR-007/FR-008/FR-013) rather than a blank or indefinitely-loading panel.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "detail")]
pub enum VulnInspectError {
    NotInstalled(String),
    NoReportYet(String),
    Forbidden(String),
    Other(String),
}

impl VulnInspectError {
    /// Backend errors MUST reach the operator verbatim (constitution I) — this is the message
    /// shown alongside the error kind's label in the UI.
    pub fn detail(&self) -> &str {
        match self {
            Self::NotInstalled(d) | Self::NoReportYet(d) | Self::Forbidden(d) | Self::Other(d) => d,
        }
    }
}

/// One row in the cluster-wide "worst offenders" summary (US2).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct WorkloadVulnSummary {
    pub namespace: String,
    /// e.g. "Deployment", "StatefulSet", "Pod".
    pub kind: String,
    pub name: String,
    pub summary: SeveritySummary,
    /// Number of distinct images contributing findings to this workload.
    pub image_count: u32,
}

/// Aggregated cluster-wide Trivy Operator state (`SystemEvent::TrivyOverviewUpdated`), mirroring
/// `FluxOverview`'s installed/live-data shape.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
pub struct TrivyOverview {
    /// Any of the four Trivy CRDs (`VulnerabilityReport`, `ExposedSecretReport`, `SbomReport`,
    /// `RbacAssessmentReport`) found and served (FR-001, FR-014).
    pub installed: bool,
    /// CRDs are installed but reading report instances was denied (FR-013) — distinct from
    /// "not installed" so an operator fixes RBAC, not "installs an operator that's already there".
    pub forbidden: bool,
    /// Ranked worst-first (critical desc, then high, medium, low, total; tie-break
    /// namespace/name asc).
    pub workloads: Vec<WorkloadVulnSummary>,
}
