//! Server → client frames (contracts/protocol.md).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::types::{Me, Preview, PreviewId, RequestId};

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
