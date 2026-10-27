//! Runs, jobs and approval policies (spec 055 US2, contracts/protocol.md "Runs", "Jobs and
//! policies").

use std::fmt;

use keel_core::uuid::Uuid;
use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

use crate::binding::ObjectName;
use crate::types::checked_string;

checked_string!(
    /// A glob (a policy's job or limit match) or an Ansible `--limit` expression: 1 to 256
    /// printable characters.
    Pattern,
    "pattern",
    |v| {
        if v.is_empty() || v.len() > 256 {
            return Err("length");
        }
        if v.chars().any(char::is_control) { Err("bad character") } else { Ok(()) }
    }
);

impl fmt::Debug for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Pattern({:?})", self.0)
    }
}

/// A run's state (data-model "runs").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    NeedsReview,
    PendingApproval,
    Queued,
    Running,
    Succeeded,
    Failed,
    Refused,
    Cancelled,
    Denied,
    Expired,
}

/// Why a run reached its state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RunReason {
    ApprovalDenied,
    ApprovalExpired,
    BindingDisabled,
    CancelledByUser,
    RunnerLost,
    InventoryInvalid,
    ArtifactMissing,
    CredentialUnavailable,
    HostRefused,
}

/// One host's state within a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum HostPhase {
    Queued,
    Sent,
    Running,
    Succeeded,
    Failed,
    Refused,
    Lost,
    Cancelled,
}

/// What started a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RunTrigger {
    Delivery,
    Poll,
    Manual,
    Api,
    Workflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    Stdout,
    Stderr,
}

/// Ansible's per-host stats.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct HostStats {
    pub ok: u32,
    pub changed: u32,
    pub failed: u32,
    pub unreachable: u32,
    pub skipped: u32,
    pub rescued: u32,
    pub ignored: u32,
}

/// The first task that failed on a host, so an operator can act on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FirstFailure {
    pub task: String,
    pub module: Option<String>,
    pub message: String,
    pub rc: Option<i64>,
    /// The host was unreachable rather than a task failing.
    pub unreachable: bool,
}

/// A run's host counts and summed stats.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct RunSummary {
    pub hosts: u32,
    pub succeeded: u32,
    pub failed: u32,
    pub refused: u32,
    pub lost: u32,
    pub cancelled: u32,
    pub stats: HostStats,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct RunInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    #[ts(type = "string")]
    pub workspace: Uuid,
    #[ts(type = "string | null")]
    pub binding: Option<Uuid>,
    #[ts(type = "string | null")]
    pub job: Option<Uuid>,
    pub job_name: String,
    pub trigger: RunTrigger,
    #[ts(type = "string | null")]
    pub requested_by: Option<Uuid>,
    pub commit: String,
    pub inventory: String,
    #[ts(type = "string | null")]
    pub inventory_revision: Option<Uuid>,
    /// A newer revision of the inventory exists than the one this run is pinned to.
    pub newer_inventory_revision: bool,
    pub limit: Option<String>,
    pub hosts: Vec<String>,
    #[ts(type = "string | null")]
    pub policy: Option<Uuid>,
    pub state: RunPhase,
    pub reason: Option<RunReason>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub summary: Option<RunSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct RunHostInfo {
    #[ts(type = "string")]
    pub host: Uuid,
    pub name: String,
    pub state: HostPhase,
    /// The host's own refusal (`unsigned`, `base_mismatch`, …).
    pub refusal: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub stats: Option<HostStats>,
    pub first_failure: Option<FirstFailure>,
}

/// A job as synced from `.keel/jobs.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct JobInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    #[ts(type = "string")]
    pub binding: Uuid,
    pub name: String,
    pub playbook: String,
    pub profiles: Vec<String>,
    pub inventory: String,
    pub limit: Option<String>,
    /// Path globs that trigger a run; `null` means manual only.
    pub paths: Option<Vec<String>>,
    pub credentials: Vec<String>,
    pub blocked: Option<String>,
    pub defined_at_commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PolicyInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    #[ts(type = "string")]
    pub workspace: Uuid,
    pub priority: i32,
    pub job: Option<String>,
    pub inventory: Option<String>,
    pub limit: Option<String>,
    pub requires_approval: bool,
    pub approvals_needed: u16,
    pub four_eyes: bool,
    pub approval_ttl_secs: u32,
    /// The workspace catch-all: cannot be removed or narrowed.
    pub catch_all: bool,
}

/// `PreviewPolicyUpsert`: a new policy (`id` absent) or a change to one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct PolicySpec {
    #[ts(type = "string")]
    pub workspace: Uuid,
    #[serde(default)]
    #[ts(type = "string | null")]
    pub id: Option<Uuid>,
    pub priority: i32,
    #[serde(default)]
    pub job: Option<Pattern>,
    #[serde(default)]
    pub inventory: Option<ObjectName>,
    #[serde(default)]
    pub limit: Option<Pattern>,
    pub requires_approval: bool,
    pub approvals_needed: u16,
    #[serde(default)]
    pub four_eyes: bool,
    pub approval_ttl_secs: u32,
}

/// One policy as `ExplainPolicy` judged it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PolicyCheck {
    #[ts(type = "string")]
    pub policy: Uuid,
    pub priority: i32,
    pub matched: bool,
    pub why: String,
}
