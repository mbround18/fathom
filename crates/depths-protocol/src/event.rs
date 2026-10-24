//! Server → client frames (contracts/protocol.md, "Commands", result column).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::types::{
    Actor, AuditEntry, ComposeWarning, DenyReason, EnvText, HostSample, OpOutcome, OperationId,
    OperationKind, Preview, PreviewVoidReason, RequestId, StackDetail, StackList,
    StreamCloseReason, UserList, ValidationError,
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

    // Session
    /// Sent 60 s before the session expires; refresh via `POST /auth/refresh`.
    SessionExpiring {
        at: String,
    },
}
