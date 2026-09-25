//! Local mutation audit-log viewer, spec `026-mutation-audit-log-viewer`.
//!
//! Every mutating command already flows through the guard/routing in `mutation.rs` and
//! terminates in exactly one `BackendEvent::CommandFinished` (see `src/backend.rs`'s
//! `run_cmd`). This module adds a persisted, append-only, bounded local record of that
//! completion — never a second mutation-tracking mechanism. Persistence itself lives in
//! `src/backend/audit_log.rs` (`~/.fathom/audit-log.json`, the same flat-JSON-file
//! convention as `ports.json`/`cluster-settings.json`, see `research.md` R2).
//!
//! Retention is a fixed, non-configurable bound in v1 (500 entries, see `research.md` R3) —
//! there is deliberately no settings UI for this.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Fixed retention bound for `~/.fathom/audit-log.json` (v1, non-configurable). See
/// `research.md` R3.
pub const AUDIT_LOG_RETENTION: usize = 500;

/// Outcome of one completed mutating action, mirroring `BackendEvent::CommandFinished`'s
/// `ok`/`error` fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum AuditOutcome {
    Success,
    /// `reason` MUST be non-empty when the originating `CommandFinished.error` carried a
    /// message (data-model.md validation rule): never leave a failure reason-less if
    /// avoidable.
    Failure {
        reason: String,
    },
}

impl AuditOutcome {
    /// Build an outcome from a `CommandFinished`-style `(ok, error)` pair. A failure with
    /// no error message falls back to a generic placeholder reason rather than an empty
    /// string.
    pub fn from_result(ok: bool, error: Option<&str>) -> Self {
        if ok {
            Self::Success
        } else {
            let reason = match error {
                Some(e) if !e.trim().is_empty() => e.to_owned(),
                _ => "Unknown error".to_owned(),
            };
            Self::Failure { reason }
        }
    }
}

/// Coarse, stable category for a mutating action, used for "filter by action type" (FR-004).
/// Exhaustive over every `MutationTarget::{Namespaced, ClusterScoped, Local}`-producing
/// `BackendCmdV1` variant that the audit log records (see `mutation.rs`'s `audit_summary()`);
/// deliberately excludes pure session/UI mechanics that carry no meaningful target (e.g.
/// backend shutdown, the demo-chat subprocess lifecycle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum AuditActionKind {
    YamlApply,
    HelmUpgrade,
    HelmRollback,
    HelmUninstall,
    NodeCordon,
    NodeUncordon,
    NodeDrain,
    NodeTaint,
    Delete,
    BulkDelete,
    Scale,
    Restart,
    CronJobControl,
    PortForwardChange,
    SettingsChange,
    TeamStoreChange,
    AgentAction,
    McpToggle,
    /// Ephemeral debug container injected into a running pod (025).
    EphemeralContainer,
    /// Local kind cluster created via the provisioning UI (030).
    KindClusterCreate,
    /// Local kind cluster deleted via the provisioning UI (030).
    KindClusterDelete,
}

impl AuditActionKind {
    /// Human-readable label for display, per `data-model.md`'s `action_label` field.
    pub fn label(self) -> &'static str {
        match self {
            Self::YamlApply => "Apply YAML",
            Self::HelmUpgrade => "Helm Upgrade",
            Self::HelmRollback => "Helm Rollback",
            Self::HelmUninstall => "Helm Uninstall",
            Self::NodeCordon => "Node Cordon",
            Self::NodeUncordon => "Node Uncordon",
            Self::NodeDrain => "Node Drain",
            Self::NodeTaint => "Node Taint",
            Self::Delete => "Delete",
            Self::BulkDelete => "Bulk Delete",
            Self::Scale => "Scale",
            Self::Restart => "Restart",
            Self::CronJobControl => "CronJob Control",
            Self::PortForwardChange => "Port Forward Change",
            Self::SettingsChange => "Settings Change",
            Self::TeamStoreChange => "Team Store Change",
            Self::AgentAction => "Agent Action",
            Self::McpToggle => "MCP Server Toggle",
            Self::EphemeralContainer => "Debug Container",
            Self::KindClusterCreate => "Kind Cluster Create",
            Self::KindClusterDelete => "Kind Cluster Delete",
        }
    }
}

/// Extracted, pre-dispatch summary of a mutating command, computed by
/// `BackendCmdV1::audit_summary()` before the command is consumed by `dispatch_cmd` (see
/// `research.md` R1). Not part of the wire protocol; consumed only by
/// `src/backend/audit_log.rs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditActionSummary {
    pub kind: AuditActionKind,
    pub target_kind: Option<String>,
    pub target_namespace: Option<String>,
    pub target_name: Option<String>,
}

/// One persisted, immutable record of a completed mutating action.
/// `~/.fathom/audit-log.json` (see `src/backend/audit_log.rs`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AuditLogEntry {
    /// Unique per entry. Generated at write time (not a UUID crate dependency — a
    /// monotonic counter plus timestamp is sufficient for local, single-writer uniqueness,
    /// see `src/backend/audit_log.rs::next_id`).
    pub id: String,
    /// RFC 3339 UTC timestamp, captured when the action completed.
    pub timestamp: String,
    pub action: AuditActionKind,
    pub action_label: String,
    /// The kubeconfig context the action targeted.
    pub context: String,
    pub target_kind: Option<String>,
    pub target_namespace: Option<String>,
    pub target_name: Option<String>,
    pub outcome: AuditOutcome,
    /// The originating command's correlation id, best-effort, for cross-referencing.
    pub request_id: Option<String>,
}
