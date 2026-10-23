//! Client → server frames (contracts/protocol.md, "Commands").
//!
//! Everything here is parsed from a hostile network: every type denies unknown fields, every enum
//! is tagged (an unknown variant fails to parse), every client string is a bounded newtype from
//! [`crate::types`], and no field passes JSON through untyped. Parsing is not authorisation: the
//! server authorises each command against the users registry at the time of the command.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::types::{
    AuditCursor, AuditFilter, ContainerRef, CsrfToken, LogTail, LogTarget, MAX_STATS_TARGETS,
    MAX_USER_STACKS, OperationId, PreviewId, RequestId, Role, Shell, ShellBytes, StackDraft,
    StackName, StackOp, StatsTarget, Timestamp, UserId, at_most,
};
use crate::wire::StreamId;

/// The first frame on a socket. A wrong `csrf_token` closes with 4403, a missing or expired
/// session with 4401.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    /// [`crate::PROTOCOL_VERSION`] the client was built against.
    pub protocol_version: u32,
    pub csrf_token: CsrfToken,
}

/// Every frame after `Hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CmdEnvelope {
    /// Echoed as `reply_to` on the events this command produces.
    pub id: RequestId,
    pub cmd: Cmd,
}

/// Commands. Results arrive as events.
///
/// Adjacently tagged: `{ "type": "ListStacks" }`,
/// `{ "type": "GetStack", "data": { "name": "web" } }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data", deny_unknown_fields)]
pub enum Cmd {
    /// `Stacks`, then live `StacksChanged`; `RuntimeUnavailable` when docker fails.
    ListStacks,
    /// `StackDetail`.
    GetStack {
        name: StackName,
    },
    /// `EnvRevealed`. Audited.
    RevealEnv {
        name: StackName,
    },
    /// `Preview`, or `Denied` / `Busy` / `Invalid`.
    PreviewOp {
        stack: StackName,
        op: StackOp,
    },
    /// `Preview` (validate, then `up` when `deploy`), `ValidationFailed` or `Conflict`. A draft
    /// without `base_hash` creates a stack (admin).
    PreviewSave {
        draft: StackDraft,
        deploy: bool,
    },
    /// `OpStarted`, `OpOutput`*, `OpFinished`; or `PreviewVoid`.
    Confirm {
        preview_id: PreviewId,
    },
    /// `OpFinished{outcome: cancelled}`. The actor or an admin.
    Cancel {
        operation_id: OperationId,
    },
    /// `StreamOpened`, `LogChunk`*, `StreamClosed`.
    OpenLogs {
        target: LogTarget,
        tail: LogTail,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        since: Option<Timestamp>,
    },
    /// `StreamOpened`, `ShellOutput`*, `StreamClosed`. Only when `DEPTHS_FEATURE_SHELL` is on.
    OpenShell {
        container: ContainerRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        shell: Option<Shell>,
        cols: u16,
        rows: u16,
    },
    /// No reply. Owner of the stream only.
    ShellInput {
        stream_id: StreamId,
        bytes_b64: ShellBytes,
    },
    /// No reply. Owner of the stream only.
    ShellResize {
        stream_id: StreamId,
        cols: u16,
        rows: u16,
    },
    /// `StatsSample` every 2 s while watched. At most 64 targets.
    WatchStats {
        #[serde(deserialize_with = "at_most::<_, _, MAX_STATS_TARGETS>")]
        targets: Vec<StatsTarget>,
    },
    UnwatchStats,
    /// `StreamClosed`.
    CloseStream {
        stream_id: StreamId,
    },
    /// `AuditPage`. Admin.
    GetAudit {
        #[serde(default)]
        filter: AuditFilter,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        cursor: Option<AuditCursor>,
    },
    /// `Users`, then live `UsersChanged`. Admin.
    GetUsers,
    /// `Users` (audited `user_added` or `user_changed`). An unknown id is an invite. `Denied`
    /// for an owner, or for a role other than `user`/`admin`. Admin.
    PutUser {
        id: UserId,
        role: Role,
        #[serde(deserialize_with = "at_most::<_, _, MAX_USER_STACKS>")]
        stacks: Vec<StackName>,
    },
    /// `Users` (audited `user_removed`). `Denied` for an owner. Admin.
    RemoveUser {
        id: UserId,
    },
}
