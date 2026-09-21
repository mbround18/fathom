//! Tauri commands and the `Channel`-backed event sink (contracts/ipc.md).
//!
//! Tauri glue only; buffering, batching, eviction, and request ids live in
//! [`fathom::bridge`].

use fathom::bridge::pump::{BatchSink, Hub, SinkError};
use fathom::bridge::{BackendLink, EventBatchView};
use fathom::protocol::{
    BackendCmd, BackendEventEnvelope, BridgeError, RequestId, SubscriptionInfo,
};
use std::sync::Arc;
use tauri::State;
use tauri::ipc::{Channel, InvokeResponseBody};

/// The webview's `onBatch` channel.
pub struct ChannelSink(pub Channel<InvokeResponseBody>);

impl BatchSink<BackendEventEnvelope> for ChannelSink {
    fn send(&mut self, envelopes: &[BackendEventEnvelope], dropped: u32) -> Result<(), SinkError> {
        let json = serde_json::to_string(&EventBatchView { envelopes, dropped })
            .map_err(|e| SinkError(format!("serialize EventBatch: {e}")))?;
        self.0
            .send(InvokeResponseBody::Json(json))
            .map_err(|e| SinkError(e.to_string()))
    }
}

pub type EventHub = Hub<BackendEventEnvelope, ChannelSink>;

/// Managed Tauri state shared by the commands and the pump thread.
pub struct BridgeState {
    pub link: BackendLink,
    pub hub: Arc<EventHub>,
}

/// Forward `cmd` to the backend and return its [`RequestId`]. Results arrive as events
/// correlated with that id, ending with exactly one `CommandFinished`.
#[tauri::command]
pub fn send_cmd(state: State<'_, BridgeState>, cmd: BackendCmd) -> Result<RequestId, BridgeError> {
    state.link.send(cmd)
}

/// Attach `on_batch` as the single event subscriber (replacing any previous one, e.g.
/// after a webview reload) and flush the buffered backlog to it.
#[tauri::command]
pub async fn subscribe_events(
    state: State<'_, BridgeState>,
    on_batch: Channel<InvokeResponseBody>,
) -> Result<SubscriptionInfo, BridgeError> {
    state.hub.subscribe(ChannelSink(on_batch)).map_err(|e| {
        tracing::warn!("Event subscription failed during backlog flush: {}", e);
        BridgeError::ChannelClosed
    })?;
    state.link.subscription_info()
}
