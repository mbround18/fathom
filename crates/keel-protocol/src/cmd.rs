//! Client → server frames (contracts/protocol.md, "Envelopes" and "Commands").
//!
//! Parsing is not authorisation: the server authorises each command against the session's
//! current `authz_epoch` at the time of the command.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::passkey::PasskeyAssertion;
use crate::types::{ClientName, CsrfToken, PreviewId, RequestId};

/// The first frame on a WebSocket. A wrong `csrf_token` closes with 4403; a missing or expired
/// session with 4401; another `protocol_version` with 4400.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    /// [`crate::PROTOCOL_VERSION`] the client was built against.
    pub protocol_version: u32,
    /// Free-form client name and version, for the audit context.
    pub client: ClientName,
    /// The session's CSRF token (from the `__Host-keel_csrf` cookie).
    pub csrf_token: CsrfToken,
}

/// Every frame after `Hello`, and the body of `POST /api/v1/cmd`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CmdEnvelope {
    pub id: RequestId,
    pub cmd: Cmd,
}

/// Commands. Results arrive as events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data", deny_unknown_fields)]
pub enum Cmd {
    /// `Me`.
    GetMe,
    /// Carry out a `Preview`. `step_up` is required when the preview said so and the session has
    /// not verified within the step-up window. Replies `Confirmed` plus the command's own result.
    Confirm {
        preview_id: PreviewId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        step_up: Option<PasskeyAssertion>,
    },
}

impl Cmd {
    /// The command's name, as audited and as `Denied.action` reports it.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Cmd::GetMe => "get_me",
            Cmd::Confirm { .. } => "confirm",
        }
    }

    /// Streaming commands (subscriptions) only work over the WebSocket; `POST /api/v1/cmd`
    /// refuses them with `use_websocket`.
    #[must_use]
    pub const fn is_streaming(&self) -> bool {
        match self {
            Cmd::GetMe | Cmd::Confirm { .. } => false,
        }
    }
}
