//! Server → client frames (contracts/protocol.md, "Commands", result column).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::types::{
    Actor, AuditEntry, ComposeWarning, DenyReason, Entry, EnvText, FileContent, FilePreview,
    FileRefusal, FileVersion, Fingerprint, HostSample, Mount, MountId, OpOutcome, OperationId,
    OperationKind, Preview, PreviewId, PreviewVoidReason, RelPath, RequestId, StackDetail,
    StackList, StreamCloseReason, UserList, ValidationError, VersionId,
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

    // Session
    /// Sent 60 s before the session expires; refresh via `POST /auth/refresh`.
    SessionExpiring {
        at: String,
    },
}
