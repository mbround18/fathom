use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Backend-classified expiry bucket for a Certificate's `status.notAfter`, relative to a
/// 14-day "expiring soon" threshold (spec 023 Assumptions / research.md R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum CertExpiryState {
    /// Further from expiry than the threshold.
    Valid,
    /// Within the threshold, but not yet expired.
    ExpiringSoon,
    /// `notAfter` is in the past.
    Expired,
    /// The certificate has never been issued (`status.notAfter` absent).
    Unknown,
}

/// Backend-classified ready/renewal status for a Certificate row (spec 023 FR-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum CertReadyState {
    /// `Ready` condition is `True`, no renewal in progress.
    Ready,
    /// `Ready` condition is `True`, but a newer in-progress CertificateRequest exists.
    Renewing,
    /// `Ready` condition is `False`.
    Failed,
    /// `Ready` condition is absent/`Unknown` (initial issuance in progress).
    Pending,
}

impl CertReadyState {
    pub fn label(&self) -> &'static str {
        match self {
            CertReadyState::Ready => "Ready",
            CertReadyState::Renewing => "Renewing",
            CertReadyState::Failed => "Failed",
            CertReadyState::Pending => "Pending",
        }
    }
}

/// One Certificate resource, summarized for the Certificates pane list (spec 023 FR-003).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CertificateSummary {
    pub name: String,
    pub namespace: String,
    /// `"Issuer/<name>"` or `"ClusterIssuer/<name>"`, formatted from `spec.issuerRef`.
    pub issuer_ref: String,
    /// RFC3339 `status.notAfter`, `None` if the certificate has never been issued.
    pub not_after: Option<String>,
    pub expiry_state: CertExpiryState,
    /// Signed seconds until expiry (negative if already expired); `None` iff `not_after` is `None`.
    pub expiry_seconds: Option<i64>,
    pub ready: CertReadyState,
    /// `Ready` condition's message (falls back to reason), same convention as `FluxResourceSummary`.
    pub message: String,
    /// RFC3339 `status.renewalTime`, when present.
    pub renewal_time: Option<String>,
    pub dns_names: Vec<String>,
}

/// Aggregated cert-manager state for the Certificates pane: whether cert-manager is
/// installed, plus every Certificate found (spec 023 FR-001/FR-002).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
pub struct CertManagerOverview {
    pub installed: bool,
    pub certificates: Vec<CertificateSummary>,
}

/// Which link in the Certificate lifecycle chain a [`ChainStage`] represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum ChainStageKind {
    Certificate,
    CertificateRequest,
    Order,
    Challenge,
}

impl ChainStageKind {
    pub fn label(&self) -> &'static str {
        match self {
            ChainStageKind::Certificate => "Certificate",
            ChainStageKind::CertificateRequest => "CertificateRequest",
            ChainStageKind::Order => "Order",
            ChainStageKind::Challenge => "Challenge",
        }
    }
}

/// One resolved link in a Certificate's lifecycle chain (spec 023 FR-006/FR-007).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ChainStage {
    pub kind: ChainStageKind,
    pub name: String,
    pub namespace: String,
    /// Human label: cert-manager's own `status.state` for Order/Challenge, the `Ready`
    /// condition's status for Certificate/CertificateRequest.
    pub state: String,
    pub reason: String,
    pub message: String,
    /// True for exactly the one stage identified as the earliest non-terminal-success link
    /// (research.md R4), or none, if the Certificate is healthy.
    pub is_stuck: bool,
}

/// A Certificate's resolved lifecycle chain, returned by `SelectCertificateChain`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CertificateChain {
    pub certificate_name: String,
    pub certificate_namespace: String,
    /// In chain order (Certificate, CertificateRequest, Order, Challenge); omits links that
    /// don't exist (already garbage-collected after success, or never created).
    pub stages: Vec<ChainStage>,
    /// True if the Certificate itself could no longer be found (deleted between list and select).
    pub not_found: bool,
}
