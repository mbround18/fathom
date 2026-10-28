//! Server → client frames (contracts/protocol.md).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use keel_core::uuid::Uuid;

use crate::binding::{BindingInfo, CredentialInfo, DeliveryInfo, JobSummary, ReceiverInfo};
use crate::passkey::{PasskeyInfo, PasskeyRefusedReason, PasskeyRequest, Verification};
use crate::types::{Me, Preview, PreviewId, RequestId};
// Inventories (US4)
use crate::inventory::{
    InventoryChange, InventoryHostInfo, InventoryInfo, InventoryProblem, InventoryRevisionInfo,
    InventoryRevisionSummary,
};
// Governance (US6)
use crate::governance::{ApiTokenInfo, TeamInfo, TokenSecret, UserInfo, WorkspaceInfo};
// Hosts and the SSH CAs (US7)
use crate::host::{EnrollToken, HostDetail, HostInfo, HostStatus, SshCaInfo};
// Signed mode: review and signers (US8)
use crate::review::{
    DiffFile, DiffPage, OverrideChallenge, ReviewCommit, ReviewDecision, ReviewFlag, ReviewInfo,
    SignerInfo,
};
// Runs (US2)
use crate::run::{
    JobInfo, LogStream, PolicyCheck, PolicyInfo, RunHostInfo, RunInfo, RunPhase, RunReason,
};

/// Every frame the server sends, and each element of `POST /api/v1/cmd`'s reply array.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct EventEnvelope {
    /// The `CmdEnvelope.id` this answers; absent for pushed events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reply_to: Option<RequestId>,
    pub event: Event,
}

/// Events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum Event {
    /// Reply to `GetMe`.
    Me(Me),
    // Bindings and receivers (US1)
    /// Reply to `ListBindings`.
    Bindings { items: Vec<BindingInfo> },
    /// Reply to `GetBinding`, and the result of a confirmed binding change.
    Binding {
        binding: BindingInfo,
        jobs: Vec<JobSummary>,
    },
    /// A sync moved the binding's head (reply to `SyncBinding`, or pushed after a delivery).
    BindingSynced {
        #[ts(type = "string")]
        binding: Uuid,
        commit: String,
        jobs_changed: bool,
        inventories_changed: bool,
    },
    /// Reply to `ListReceivers`.
    Receivers { items: Vec<ReceiverInfo> },
    /// The receiver's URL and secret, shown once after create or rotate. Never sent again.
    ReceiverSecret { url: String, secret: String },
    /// Reply to `ListDeliveries`.
    Deliveries { items: Vec<DeliveryInfo> },
    /// A receiver recorded a delivery (pushed).
    DeliveryReceived { delivery: DeliveryInfo },
    /// Reply to `ListCredentials`.
    Credentials { items: Vec<CredentialInfo> },
    // Inventories (US4)
    /// Reply to `ListInventories`, and after a deprecation.
    Inventories { items: Vec<InventoryInfo> },
    /// Reply to `GetInventoryRevision`, and the result of a confirmed create, edit, restore or
    /// schema change.
    /// `history` is every revision, newest first.
    InventoryRevision {
        inventory: InventoryInfo,
        revision: InventoryRevisionInfo,
        history: Vec<InventoryRevisionSummary>,
        /// The inventory's var schema as pretty JSON, when it has one.
        var_schema: Option<String>,
    },
    /// Reply to `DiffInventory`.
    InventoryDiff {
        #[ts(type = "string")]
        inventory: Uuid,
        from: u32,
        to: u32,
        changes: Vec<InventoryChange>,
    },
    /// Reply to `ValidateInventory`.
    InventoryValidation {
        content_hash: String,
        hosts: Vec<InventoryHostInfo>,
        errors: Vec<InventoryProblem>,
        warnings: Vec<InventoryProblem>,
    },
    /// A two-step mutation awaiting `Confirm`.
    Preview(Preview),
    /// The preview was carried out. Any result the command has follows as its own event.
    Confirmed { preview_id: PreviewId },
    /// Authorization refused the command. Audited.
    Denied { action: String, reason: String },
    /// The command failed; `message` is shown to the user verbatim.
    OpError { code: String, message: String },
    /// The user's access changed or the session ended; the server closes the socket after this.
    Revoked,

    // Passkeys, as in Depths (spec 054).
    /// A ceremony to answer with `PasskeyFinish` (or, for `verify`, on `Confirm.step_up`).
    PasskeyChallenge(PasskeyRequest),
    /// A passkey was added. `Verification` follows.
    PasskeyRegistered(PasskeyInfo),
    /// Reply to `ListPasskeys`.
    Passkeys { items: Vec<PasskeyInfo> },
    /// Where the session stands on passkeys.
    Verification(Verification),
    /// Register a passkey first.
    PasskeyEnrolmentRequired,
    /// A passkey answer, recovery code or removal was refused. Audited.
    PasskeyRefused { reason: PasskeyRefusedReason },

    // Governance (US6)
    /// Reply to `ListWorkspaces`, and after a workspace or project change.
    Workspaces { items: Vec<WorkspaceInfo> },
    /// Reply to `ListTeams`, and after a team, member or grant change.
    Teams { items: Vec<TeamInfo> },
    /// Reply to `ListUsers`, and after a role change or removal.
    Users { items: Vec<UserInfo> },
    /// Reply to `ListApiTokens`, and after a token is created or revoked.
    ApiTokens { items: Vec<ApiTokenInfo> },
    /// A token was created. `secret` is shown this once; Keel keeps only its hash.
    ApiTokenCreated {
        token: ApiTokenInfo,
        secret: TokenSecret,
    },

    // Runs (US2)
    /// Reply to `ListRuns`.
    Runs { items: Vec<RunInfo> },
    /// Reply to `GetRun` and to a confirmed `PreviewRunStart`.
    Run {
        run: RunInfo,
        hosts: Vec<RunHostInfo>,
    },
    /// Reply to `SubscribeRun`: `seq` is the last sequence number it will replay.
    RunSubscribed {
        #[ts(type = "string")]
        run: Uuid,
        seq: u64,
    },
    /// One structured event of a subscribed run. `data` is the event's JSON.
    RunEvent {
        #[ts(type = "string")]
        run: Uuid,
        seq: u64,
        at: String,
        kind: String,
        host: Option<String>,
        task: Option<String>,
        data: String,
    },
    /// A chunk of a subscribed run's output (lossy UTF-8).
    RunLog {
        #[ts(type = "string")]
        run: Uuid,
        seq: u64,
        stream: LogStream,
        text: String,
    },
    /// A run changed state (pushed to the workspace's viewers, and to subscribers).
    RunState {
        #[ts(type = "string")]
        run: Uuid,
        state: RunPhase,
        reason: Option<RunReason>,
    },
    /// Reply to `ListJobs`.
    Jobs {
        #[ts(type = "string")]
        binding: Uuid,
        items: Vec<JobInfo>,
    },
    /// Reply to `ListPolicies`, and after a policy change.
    Policies {
        #[ts(type = "string")]
        workspace: Uuid,
        items: Vec<PolicyInfo>,
    },
    /// Reply to `ExplainPolicy`.
    PolicyExplanation {
        #[ts(type = "string")]
        job: Uuid,
        limit: Option<String>,
        #[ts(type = "string")]
        winner: Uuid,
        requires_approval: bool,
        reason: String,
        checks: Vec<PolicyCheck>,
    },

    // Hosts and the SSH CAs (US7)
    /// Reply to `ListHosts`.
    Hosts { items: Vec<HostInfo> },
    /// Reply to `GetHost` and to a confirmed `PreviewHostRevoke`.
    Host(HostDetail),
    /// A confirmed `PreviewHostEnroll`. `token` is shown this once; Keel keeps only its hash.
    HostEnrollment {
        #[ts(type = "string")]
        id: Uuid,
        #[ts(type = "string")]
        workspace: Uuid,
        inventory_name: String,
        profiles: Vec<String>,
        expires_at: String,
        token: EnrollToken,
    },
    /// Reply to `GetSshCa` and to a confirmed `PreviewSshCaRotate`.
    SshCa {
        cas: Vec<SshCaInfo>,
        /// `@cert-authority * ssh-ed25519 …`: the value for `keel_runner_host_ca`.
        cert_authority: String,
    },
    /// Pushed when a host connects, disconnects or is revoked, to anyone who can see it.
    HostState {
        #[ts(type = "string")]
        host: Uuid,
        state: HostStatus,
    },
    /// Pushed when a host's `changed_outside` is set or cleared (spec 058 amendment A2), to
    /// anyone who can see it. Re-read the host for the new value.
    HostChangedOutside {
        #[ts(type = "string")]
        host: Uuid,
    },

    // Signed mode: review and signers (US8)
    /// Reply to `ListReviews`.
    Reviews { items: Vec<ReviewInfo> },
    /// Reply to `GetReview`, and after a confirmed override or deny.
    Review {
        review: ReviewInfo,
        commits: Vec<ReviewCommit>,
        flags: Vec<ReviewFlag>,
        decisions: Vec<ReviewDecision>,
    },
    /// Reply to `GetReviewDiff`: every changed file, and the requested page of one file.
    ReviewDiff {
        #[ts(type = "string")]
        group: Uuid,
        files: Vec<DiffFile>,
        page: Option<DiffPage>,
    },
    /// Follows the `Preview` of `PreviewOverride` and `PreviewBaseline`.
    OverrideChallenge(OverrideChallenge),
    /// A signed-mode run needs review (pushed to the workspace's approvers).
    ReviewRequested {
        #[ts(type = "string")]
        group: Uuid,
        #[ts(type = "string")]
        run: Uuid,
        #[ts(type = "string")]
        workspace: Uuid,
    },
    /// Reply to `ListSigners`, and after a signer change.
    Signers {
        #[ts(type = "string")]
        workspace: Uuid,
        items: Vec<SignerInfo>,
    },
}

impl EventEnvelope {
    #[must_use]
    pub fn reply(to: RequestId, event: Event) -> Self {
        Self {
            reply_to: Some(to),
            event,
        }
    }

    #[must_use]
    pub fn push(event: Event) -> Self {
        Self {
            reply_to: None,
            event,
        }
    }
}

impl Event {
    #[must_use]
    pub fn op_error(code: &str, message: impl Into<String>) -> Self {
        Event::OpError {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}
