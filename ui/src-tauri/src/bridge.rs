//! Tauri commands and the `Channel`-backed event sink (contracts/ipc.md).
//!
//! Tauri glue only; buffering, batching, and eviction live in [`crate::pump`].

use crate::pump::{BatchSink, Evictable, Hub, SinkError};
use crossbeam_channel::Sender;
use fathom::protocol::{
    BRIDGE_PROTOCOL_VERSION, BackendCmd, BackendEvent, BackendEventEnvelope, BridgeError,
    SubscriptionInfo,
};
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::State;
use tauri::ipc::{Channel, InvokeResponseBody};

impl Evictable for BackendEventEnvelope {
    /// List snapshots are superseded by the next refresh, so they go first; one-shot reads
    /// next; stream chunks after that. System events (errors, notifications, lifecycle)
    /// are never dropped.
    fn eviction_rank(&self) -> Option<u8> {
        match &self.event {
            BackendEvent::ResourceList(_) => Some(0),
            BackendEvent::ResourceRead(_) => Some(1),
            BackendEvent::Stream(_) => Some(2),
            BackendEvent::System(_) => None,
        }
    }
}

/// Borrowed view of `fathom::protocol::EventBatch`, so a batch can be serialized without
/// cloning it and re-buffered if the send fails. Must serialize identically.
#[derive(Serialize)]
struct EventBatchView<'a> {
    envelopes: &'a [BackendEventEnvelope],
    dropped: u32,
}

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
    pub cmd_tx: Sender<BackendCmd>,
    pub hub: Arc<EventHub>,
    /// Set once the pump has seen `SystemEvent::BackendStarted`.
    pub backend_started: Arc<AtomicBool>,
    /// Cleared when the backend drops its event sender (the pump exits).
    pub backend_alive: Arc<AtomicBool>,
}

/// Forward `cmd` to the backend. Fire-and-forget: results arrive as events.
#[tauri::command]
pub fn send_cmd(state: State<'_, BridgeState>, cmd: BackendCmd) -> Result<(), BridgeError> {
    state.cmd_tx.send(cmd).map_err(|_| BridgeError::BackendGone)
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
    if !state.backend_alive.load(Ordering::Acquire) {
        return Err(BridgeError::BackendGone);
    }
    Ok(SubscriptionInfo {
        protocol_version: BRIDGE_PROTOCOL_VERSION,
        backend_started: state.backend_started.load(Ordering::Acquire),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fathom::protocol::{Correlation, EventBatch, Route, SystemEvent, section_keys};

    #[test]
    fn batch_view_serializes_like_event_batch() {
        let envelopes = vec![BackendEventEnvelope::new(
            3,
            Route::global(section_keys::system()),
            Correlation::None,
            BackendEvent::System(SystemEvent::BackendStarted),
        )];
        let view = serde_json::to_value(EventBatchView {
            envelopes: &envelopes,
            dropped: 2,
        })
        .unwrap();
        let owned = serde_json::to_value(EventBatch {
            envelopes,
            dropped: 2,
        })
        .unwrap();
        assert_eq!(view, owned);
    }
}
