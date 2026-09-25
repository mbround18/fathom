//! Wire types for the Tauri bridge (`ui/src-tauri/src/bridge.rs`, contracts/ipc.md).
//!
//! They live in the core crate so the TypeScript bindings can be generated without
//! building Tauri.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{BackendCmd, BackendEventEnvelope, RequestId};

/// The protocol version the bridge speaks (`BackendEventEnvelope::version`).
pub const BRIDGE_PROTOCOL_VERSION: u32 = 1;

/// One coalesced delivery of backend events to the webview.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
pub struct EventBatch {
    /// In backend `seq` order.
    pub envelopes: Vec<BackendEventEnvelope>,
    /// Envelopes the pump discarded because of backpressure since the previous batch.
    pub dropped: u32,
}

/// Returned by `subscribe_events`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionInfo {
    #[ts(type = "1")]
    pub protocol_version: u32,
    /// Whether the pump has already seen `SystemEvent::BackendStarted`.
    pub backend_started: bool,
    /// Which shell serves this backend (031-hosted-server-mode). Absent = desktop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub mode: Option<ShellMode>,
    /// The signed-in user (hosted only). Never carries tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub identity: Option<PublicIdentity>,
    /// Read-only view of the per-user limits (hosted only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub limits: Option<PublicLimits>,
}

/// Which shell hosts the backend (031-hosted-server-mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ShellMode {
    Desktop,
    Hosted,
}

/// The signed-in user as shown in the UI. No tokens, no subject id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PublicIdentity {
    pub username: String,
    pub groups: Vec<String>,
}

/// Per-user limits exposed read-only to the UI (031-hosted-server-mode).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PublicLimits {
    pub max_watches: u32,
    pub max_streams: u32,
    pub max_tabs: u32,
}

/// Why a hosted session ended (`SystemEvent::SessionEnded`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SessionEndReason {
    SignedOut,
    Expired,
    Revoked,
    IdpUnavailable,
    LimitEvicted,
}

/// Which per-user limit was hit (`SystemEvent::LimitReached`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum LimitKind {
    Sessions,
    Watches,
    Streams,
    Tabs,
}

/// Error returned by the bridge commands. Serialized as `{ "type": "BackendGone" }` (adjacently tagged).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum BridgeError {
    /// The backend thread has stopped; the command channel is closed.
    BackendGone,
    /// `subscribe_events` could not deliver the buffered backlog to the new channel
    /// (the webview went away mid-subscribe). The backlog stays buffered for the next
    /// subscriber.
    ChannelClosed,
    /// A guarded bridge (`fathom-devbridge`) refused to forward the command; the payload
    /// says why. The command never reached the backend.
    Forbidden(String),
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::BackendGone => f.write_str("backend has stopped"),
            BridgeError::ChannelClosed => f.write_str("event channel closed during subscribe"),
            BridgeError::Forbidden(reason) => write!(f, "forbidden: {reason}"),
        }
    }
}

impl std::error::Error for BridgeError {}

/// Client → server frame of the dev WebSocket bridge (`tools/devbridge`), mirroring the
/// Tauri commands: `{"op":"send","cmd":<BackendCmd>}` and `{"op":"subscribe"}`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "op", rename_all = "lowercase")]
#[allow(clippy::large_enum_variant)] // BackendCmd grew with the 033 OIDC issuer payload
pub enum DevBridgeClientMsg {
    /// Like `send_cmd`: replied to with `sent` (the assigned id) or `error`.
    Send { cmd: BackendCmd },
    /// Like `subscribe_events`: replied to with `subscribed` (or `error`), then `batch` frames.
    Subscribe,
}

/// Server → client frame of the dev WebSocket bridge. Replies to `send`/`subscribe` arrive
/// in request order; `batch` frames interleave freely.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum DevBridgeServerMsg {
    Sent { request_id: RequestId },
    Error { error: BridgeError },
    Subscribed { info: SubscriptionInfo },
    Batch { batch: EventBatch },
}
