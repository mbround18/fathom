//! Converge: apply every job a host's profiles name, in recipe order (spec 068,
//! contracts/protocol.md). Wire types only; planning lives in `keel_core::converge`.
//!
//! Dates are RFC 3339 strings, as elsewhere in this crate.

use std::collections::BTreeMap;

use keel_core::uuid::Uuid;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::run::RunInfo;

/// Which hosts of a binding a converge targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TargetSelection {
    /// Every enrolled, unrevoked host the binding's inventory lists.
    All,
    /// These hosts only.
    Hosts {
        #[ts(type = "Array<string>")]
        ids: Vec<Uuid>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ConvergeTarget {
    #[ts(type = "string")]
    pub binding_id: Uuid,
    pub hosts: TargetSelection,
    /// Run `ansible-playbook --check`.
    #[serde(default)]
    pub check: bool,
}

/// One step of a host's plan: a job, and the profiles that brought it in (outermost first).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PlannedStep {
    /// 1-based position in the host's plan.
    pub position: u16,
    pub job: String,
    pub via: Vec<String>,
    pub params: BTreeMap<String, String>,
    pub credential_names: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum StepRefusalReason {
    /// The job's profiles are outside the host's own ceiling.
    OutsideCeiling,
    /// The host's runner does not advertise `manifest-v068` and the step needs it.
    RunnerTooOld,
    HostRevoked,
    /// The host's profiles expand to no steps.
    NoSteps,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct StepRefusal {
    pub job: String,
    pub reason: StepRefusalReason,
}

/// Why a converge leaves out a job the host's profiles reach (spec 068 T074).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum StepSkipReason {
    /// The job has a required param (a `choice` with no default): only Start can run it.
    NeedsValue,
}

/// A job the plan leaves out without refusing the host: start it by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct StepSkip {
    pub job: String,
    pub reason: StepSkipReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct HostPlan {
    #[ts(type = "string")]
    pub host_id: Uuid,
    pub host: String,
    pub profiles: Vec<String>,
    pub steps: Vec<PlannedStep>,
    pub refusals: Vec<StepRefusal>,
    /// Jobs the profiles reach that are not steps (spec 068 T074). Not a refusal: the host's
    /// steps still run.
    #[serde(default)]
    pub skipped: Vec<StepSkip>,
}

/// The approval a converge needs: the strictest terms across its steps' policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ApprovalTerms {
    pub requires_approval: bool,
    pub approvals_needed: u16,
    pub four_eyes: bool,
    pub approval_ttl_secs: u32,
}

/// Signed mode: whether one child run's commit still needs review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ReviewGroupSummary {
    pub job: String,
    pub commit: String,
    pub needs_review: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ConvergePlan {
    #[ts(type = "string")]
    pub binding_id: Uuid,
    pub commit: String,
    #[ts(type = "string")]
    pub inventory_revision: Uuid,
    pub check: bool,
    pub hosts: Vec<HostPlan>,
    pub policy: ApprovalTerms,
    pub review: Vec<ReviewGroupSummary>,
    /// Unapproved converges this one supersedes.
    #[ts(type = "Array<string>")]
    pub replaces: Vec<Uuid>,
    /// Approved or running converges that refuse it.
    #[ts(type = "Array<string>")]
    pub blocked_by: Vec<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ConvergeHostState {
    Waiting,
    Holding,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Blocked,
    Queued,
    Sent,
    Running,
    Succeeded,
    Failed,
    Refused,
    Lost,
    Cancelled,
    Skipped,
    Covered,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ConvergeStepView {
    pub position: u16,
    pub job: String,
    #[ts(type = "string")]
    pub run_id: Uuid,
    pub state: StepState,
    pub reason: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ConvergeHostView {
    #[ts(type = "string")]
    pub host_id: Uuid,
    pub host: String,
    pub state: ConvergeHostState,
    pub steps: Vec<ConvergeStepView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ConvergeView {
    pub run: RunInfo,
    pub hosts: Vec<ConvergeHostView>,
    #[ts(type = "string | null")]
    pub retry_of: Option<Uuid>,
}

/// A host's converge as the hosts list shows it: "step 2 of 5".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ConvergeHostSummary {
    #[ts(type = "string")]
    pub run_id: Uuid,
    pub state: ConvergeHostState,
    pub step: u16,
    pub of: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionAction {
    Add,
    Remove,
}

/// A profile the inventory's `groups:` suggests adding to or removing from a host. Never applied
/// without an admin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ProfileSuggestion {
    #[ts(type = "string")]
    pub host_id: Uuid,
    pub profile: String,
    pub action: SuggestionAction,
    pub groups: Vec<String>,
    pub first_seen_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct HostProfilesChange {
    #[ts(type = "string")]
    pub host_id: Uuid,
    pub add: Vec<String>,
    pub remove: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct HostProfilesPreview {
    pub change: HostProfilesChange,
    /// Assigned, but outside the host's own ceiling: the runner will refuse them.
    pub outside_ceiling: Vec<String>,
    pub principals_after: Vec<String>,
    pub plan_after: Vec<PlannedStep>,
}

/// A commit status context a binding requires before a converge dispatches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct RequiredCheck {
    pub context: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    Pending,
    Success,
    Failure,
    Error,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct RunCheckView {
    pub context: String,
    pub state: CheckState,
    pub target_url: Option<String>,
}
