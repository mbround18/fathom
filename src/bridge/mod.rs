//! Shell-independent half of the UI bridge (contracts/ipc.md): the event pump, request-id
//! assignment, and backend/pump thread startup. Shared by the Tauri shell
//! (`ui/src-tauri`) and the dev WebSocket bridge (`tools/devbridge`); each shell only
//! adds its transport (a [`pump::BatchSink`]) and its command surface.

pub mod pump;

use crate::config::Args;
use crate::protocol::{
    BRIDGE_PROTOCOL_VERSION, BackendCmd, BackendEvent, BackendEventEnvelope, BridgeError,
    CmdRequest, RequestId, SubscriptionInfo, SystemEvent,
};
use crossbeam_channel::{Receiver, Sender};
use pump::{BatchSink, Evictable, Hub};
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;

impl Evictable for BackendEventEnvelope {
    /// List snapshots are superseded by the next refresh, so they go first; one-shot reads
    /// next; stream chunks after that. System events (errors, notifications, lifecycle)
    /// and command outcomes are never dropped.
    fn eviction_rank(&self) -> Option<u8> {
        match &self.event {
            BackendEvent::ResourceList(_) => Some(0),
            BackendEvent::ResourceRead(_) => Some(1),
            BackendEvent::Stream(_) => Some(2),
            BackendEvent::System(_) | BackendEvent::CommandFinished { .. } => None,
        }
    }
}

/// Borrowed view of [`crate::protocol::EventBatch`], so a batch can be serialized without
/// cloning it and re-buffered if the send fails. Serializes identically.
#[derive(Serialize)]
pub struct EventBatchView<'a> {
    pub envelopes: &'a [BackendEventEnvelope],
    pub dropped: u32,
}

/// Monotonic [`RequestId`]s starting at 1.
#[derive(Debug)]
pub struct RequestIds(AtomicU64);

impl RequestIds {
    pub fn new() -> Self {
        Self(AtomicU64::new(1))
    }

    pub fn next_id(&self) -> RequestId {
        RequestId(self.0.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for RequestIds {
    fn default() -> Self {
        Self::new()
    }
}

/// A shell's handle on the backend: the command channel, id assignment, and the
/// lifecycle flags maintained by the pump thread.
pub struct BackendLink {
    pub cmd_tx: Sender<CmdRequest>,
    pub ids: RequestIds,
    /// Set once the pump has seen `SystemEvent::BackendStarted`.
    pub backend_started: Arc<AtomicBool>,
    /// Cleared when the backend drops its event sender (the pump exits).
    pub backend_alive: Arc<AtomicBool>,
}

impl BackendLink {
    pub fn new(cmd_tx: Sender<CmdRequest>) -> Self {
        Self {
            cmd_tx,
            ids: RequestIds::new(),
            backend_started: Arc::new(AtomicBool::new(false)),
            backend_alive: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Assign the next request id and forward `cmd` to the backend.
    pub fn send(&self, cmd: BackendCmd) -> Result<RequestId, BridgeError> {
        let id = self.ids.next_id();
        self.cmd_tx
            .send(CmdRequest::new(id, cmd))
            .map_err(|_| BridgeError::BackendGone)?;
        Ok(id)
    }

    /// Reply to a successful subscribe, or `BackendGone` if the pump has exited.
    pub fn subscription_info(&self) -> Result<SubscriptionInfo, BridgeError> {
        if !self.backend_alive.load(Ordering::Acquire) {
            return Err(BridgeError::BackendGone);
        }
        Ok(SubscriptionInfo {
            protocol_version: BRIDGE_PROTOCOL_VERSION,
            backend_started: self.backend_started.load(Ordering::Acquire),
        })
    }
}

/// Run [`crate::backend::init`] on a dedicated `fathom-backend` thread.
pub fn spawn_backend(
    cmd_rx: Receiver<CmdRequest>,
    gui_tx: Sender<BackendEventEnvelope>,
    args: Args,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("fathom-backend".into())
        .spawn(move || crate::backend::init(cmd_rx, gui_tx, args))
        .expect("failed to spawn backend thread")
}

/// Run the pump on a dedicated `fathom-event-pump` thread, maintaining `link`'s lifecycle
/// flags. `on_envelope` sees every envelope first, in order.
pub fn spawn_pump<S, F>(
    gui_rx: Receiver<BackendEventEnvelope>,
    hub: Arc<Hub<BackendEventEnvelope, S>>,
    link: &BackendLink,
    mut on_envelope: F,
) -> JoinHandle<()>
where
    S: BatchSink<BackendEventEnvelope> + Send + 'static,
    F: FnMut(&BackendEventEnvelope) + Send + 'static,
{
    let backend_started = Arc::clone(&link.backend_started);
    let backend_alive = Arc::clone(&link.backend_alive);
    std::thread::Builder::new()
        .name("fathom-event-pump".into())
        .spawn(move || {
            pump::run_pump(&gui_rx, &hub, |envelope| {
                if matches!(
                    envelope.event,
                    BackendEvent::System(SystemEvent::BackendStarted)
                ) {
                    backend_started.store(true, Ordering::Release);
                }
                on_envelope(envelope);
            });
            backend_alive.store(false, Ordering::Release);
        })
        .expect("failed to spawn event pump thread")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Correlation, EventBatch, Route, section_keys};

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

    #[test]
    fn request_ids_are_monotonic_from_one() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let link = BackendLink::new(tx);
        assert_eq!(link.send(BackendCmd::RefreshPods), Ok(RequestId(1)));
        assert_eq!(link.send(BackendCmd::RefreshNodes), Ok(RequestId(2)));
        let got: Vec<Option<RequestId>> = rx.try_iter().map(|r| r.id).collect();
        assert_eq!(got, vec![Some(RequestId(1)), Some(RequestId(2))]);
        drop(rx);
        assert_eq!(
            link.send(BackendCmd::RefreshPods),
            Err(BridgeError::BackendGone)
        );
    }

    #[test]
    fn command_finished_is_never_evicted() {
        let env = BackendEventEnvelope::new(
            1,
            Route::global(section_keys::system()),
            Correlation::Request(RequestId(1)),
            BackendEvent::CommandFinished {
                ok: false,
                error: Some("x".into()),
            },
        );
        assert_eq!(env.eviction_rank(), None);
    }
}
