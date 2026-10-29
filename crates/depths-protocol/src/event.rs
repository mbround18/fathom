//! Server → client frames (contracts/protocol.md, "Commands", result column).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::types::{
    Actor, AuditEntry, ComposeWarning, DenyReason, Entry, EnvText, FileContent, FilePreview,
    FileRefusal, FileVersion, Fingerprint, GuardException, GuardRefusal, HostSample, Mount,
    MountId, OpOutcome, OperationId, OperationKind, PasskeyInfo, PasskeyRefusedReason,
    PasskeyRequest, Preview, PreviewId, PreviewVoidReason, RelPath, RequestId, RunId, RunTrigger,
    StackDetail, StackList, StackOutcome, StackUpdatePlan, StreamCloseReason, UpdatesView,
    UserList, ValidationError, Verification, VersionId,
};
use crate::wire::{StreamId, UsageSample};

/// One server frame. `reply_to` is the `CmdEnvelope::id` that caused it; absent for live
/// updates (`StacksChanged`, `UsersChanged`, `SessionExpiring`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct EventEnvelope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reply_to: Option<RequestId>,
    pub event: Event,
}

/// Events. Adjacently tagged like [`crate::Cmd`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum Event {
    // Stacks
    Stacks(StackList),
    StacksChanged(StackList),
    /// Docker failed; the reason verbatim (FR-010).
    RuntimeUnavailable {
        reason: String,
    },
    StackDetail(StackDetail),
    EnvRevealed {
        name: String,
        env_text: String,
    },

    // Operations
    Preview(Preview),
    /// The draft did not validate.
    ValidationFailed {
        errors: Vec<ValidationError>,
    },
    /// The stack changed since the editor loaded it (FR-019). `current_env` is masked like
    /// `StackDetail`'s.
    Conflict {
        current_text: String,
        current_env: Option<EnvText>,
        current_hash: String,
    },
    PreviewVoid {
        reason: PreviewVoidReason,
    },
    /// `LintCompose`'s findings, worst first; empty when the text does not parse.
    LintResult {
        stack: String,
        findings: Vec<ComposeWarning>,
    },
    OpStarted {
        operation_id: OperationId,
    },
    OpOutput {
        operation_id: OperationId,
        chunk: String,
    },
    OpFinished {
        operation_id: OperationId,
        outcome: OpOutcome,
        /// 054: the runtime guard refused a request of this operation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        guard: Option<GuardRefusal>,
    },

    // Refusals
    /// Not permitted. Audited.
    Denied {
        reason: DenyReason,
        /// A human sentence on why, when the reason alone does not say (a container's security
        /// findings, the compose policy).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        detail: Option<String>,
    },
    /// Another operation is running on this stack (FR-014): which one, and whose.
    Busy {
        stack: String,
        operation_id: OperationId,
        kind: OperationKind,
        actor: Actor,
    },
    /// The command named something that does not exist or cannot be acted on (an invalid stack,
    /// an unknown stream). Says what, never why internally.
    Invalid {
        reason: String,
    },

    /// 054: this command needs a fresh passkey check. Answer `request` and send the command
    /// again with `assertion`. Audited `step_up_required`.
    StepUpRequired {
        request: PasskeyRequest,
    },
    /// 054: register a passkey first.
    PasskeyEnrolmentRequired,
    /// 054: a passkey answer, recovery code or removal was refused. Audited.
    PasskeyRefused {
        reason: PasskeyRefusedReason,
    },
    /// 054: the runtime guard is not answering, so nothing can reach the runtime.
    RuntimeGuardUnreachable,

    // Streams
    StreamOpened {
        stream_id: StreamId,
    },
    LogChunk {
        stream_id: StreamId,
        lines: Vec<String>,
    },
    ShellOutput {
        stream_id: StreamId,
        bytes_b64: String,
    },
    StreamClosed {
        stream_id: StreamId,
        reason: StreamCloseReason,
    },
    StatsSample {
        samples: Vec<UsageSample>,
    },
    /// Host metrics, while `WatchHost` is on.
    HostSample(HostSample),

    // Admin
    AuditPage {
        entries: Vec<AuditEntry>,
        next_cursor: Option<String>,
    },
    Users(UserList),
    UsersChanged(UserList),

    // Files (spec 053)
    Mounts {
        stack: String,
        mounts: Vec<Mount>,
    },
    /// One page of a folder, folders first then by name. `last` on the final page.
    DirPage {
        req: u32,
        mount: MountId,
        path: RelPath,
        entries: Vec<Entry>,
        page: u32,
        last: bool,
    },
    /// A file, or a kept version of it (`version`, always read-only).
    FileContent {
        req: u32,
        mount: MountId,
        path: RelPath,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        version: Option<VersionId>,
        file: FileContent,
    },
    FolderSize {
        req: u32,
        bytes: u64,
        entries: u64,
        /// The count stopped at the download limit.
        capped: bool,
    },
    FilePreview(FilePreview),
    /// A confirmed file change was applied. `fingerprint` is the written file's, for the editor's
    /// next save; `running` the services that may need a restart to see it (FR-010).
    FileOpDone {
        preview_id: PreviewId,
        mount: MountId,
        path: RelPath,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        fingerprint: Option<Fingerprint>,
        running: Vec<String>,
    },
    /// Kept versions of a file, newest first.
    History {
        mount: MountId,
        path: RelPath,
        versions: Vec<FileVersion>,
    },
    /// A file command refused for a reason of its own. Audited.
    FileRefused(FileRefusal),

    // Passkeys and the runtime guard (spec 054)
    PasskeyChallenge(PasskeyRequest),
    PasskeyRegistered(PasskeyInfo),
    Passkeys {
        items: Vec<PasskeyInfo>,
    },
    Verification(Verification),
    GuardExceptions {
        stack: String,
        items: Vec<GuardException>,
    },

    // Image updates (spec 062). Admin, except `StackDetail.updates`.
    Updates(UpdatesView),
    /// After a check pass, a run step, a settings change or a Watchtower container coming or
    /// going. At most one per second.
    UpdatesChanged(UpdatesView),
    /// On sign-in and when the count changes: drives the navigation badge.
    UpdatesBadge {
        stacks_with_updates: u32,
    },
    CheckStarted,
    /// Check now ran less than `DEPTHS_UPDATES_CHECK_GAP` ago (FR-002): try again after this
    /// many seconds.
    CheckBusy {
        retry_after_secs: u32,
    },
    /// A batch preview. Binds each stack's `content_hash`, the digests shown and the order.
    UpdatePreview {
        preview_id: PreviewId,
        stacks: Vec<StackUpdatePlan>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        step_up: Option<PasskeyRequest>,
    },
    /// The answer to a batch preview in which no service would be recreated: each stack shown
    /// is up to date or left as it is, and why. Information, not a refusal: nothing is held and
    /// nothing is audited.
    NothingToUpdate {
        stacks: Vec<StackUpdatePlan>,
    },
    UpdateRunStarted {
        run_id: RunId,
        trigger: RunTrigger,
        stacks: Vec<String>,
    },
    UpdateStackDone {
        run_id: RunId,
        stack: String,
        outcome: StackOutcome,
        /// The `at` of the stack's audit entry.
        audit_at: String,
    },
    UpdateRunFinished {
        run_id: RunId,
        summary: Vec<(String, StackOutcome)>,
    },

    // Session
    /// Sent 60 s before the session expires; refresh via `POST /auth/refresh`.
    SessionExpiring {
        at: String,
    },
}
