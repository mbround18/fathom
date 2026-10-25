//! Pod log follows, several at once, keyed by `StreamId` (feedback #2).
//!
//! Every open logs view in the UI (the inspector's Logs tab, the dock's Logs tool, the tools
//! sheet) owns one stream under a client-chosen id, so starting or stopping one never touches
//! another. Each stream's task holds a [`CloseGuard`] that emits the stream's single
//! `StreamClosed` when the task ends for any reason: the follow finished, it failed, or the
//! task was aborted by `StopPodLogs`, a context switch or shutdown. Replacing a live id in
//! place (a restart under the same id) silences the old guard so the view sees no close.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crossbeam_channel::Sender;
use tokio::task::JoinHandle;

use super::events;
use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, Route, StreamEvent, StreamId,
};

/// `StreamClosed.reason` values for log streams (documented on `StreamEvent::StreamClosed`).
pub(super) const REASON_ENDED: &str = "ended";
pub(super) const REASON_STOPPED: &str = "stopped";
pub(super) const REASON_ERROR: &str = "error";

/// Where a log stream's events go.
#[derive(Clone)]
pub(super) struct StreamSink {
    pub(super) stream_id: StreamId,
    pub(super) route: Route,
    pub(super) seq: Arc<AtomicU64>,
    pub(super) sender: Sender<BackendEventEnvelope>,
}

impl StreamSink {
    pub(super) fn emit(&self, event: StreamEvent) {
        events::emit(
            self.seq.as_ref(),
            &self.sender,
            self.route.clone(),
            Correlation::Stream(self.stream_id),
            BackendEvent::Stream(event),
        );
    }
}

/// The shared container runtime's log follows and sessions write here (spec 050 T008). Its
/// events are the container subset of Fathom's own, so each maps 1:1 onto the same variant.
impl container_runtime::sink::StreamSink for StreamSink {
    fn emit(&self, ev: container_runtime::sink::StreamEvent) -> bool {
        // The inherent `emit`: Fathom's event channel never reports a gone listener.
        StreamSink::emit(self, ev.into());
        true
    }
}

impl From<container_runtime::sink::StreamEvent> for StreamEvent {
    fn from(ev: container_runtime::sink::StreamEvent) -> Self {
        use container_runtime::sink::StreamEvent as Crate;
        match ev {
            Crate::ContainerLogBatch {
                stream_id,
                lines,
                truncated,
            } => Self::ContainerLogBatch {
                stream_id,
                lines,
                truncated,
            },
            Crate::ContainerSessionOutput { stream_id, data } => {
                Self::ContainerSessionOutput { stream_id, data }
            }
            Crate::ContainerSessionClosed { stream_id, error } => {
                Self::ContainerSessionClosed { stream_id, error }
            }
        }
    }
}

/// Emits the stream's `StreamClosed` when dropped, unless silenced. Dropped without an
/// outcome (the task was aborted) it reports `"stopped"`.
pub(super) struct CloseGuard {
    sink: StreamSink,
    outcome: Option<(&'static str, Option<String>)>,
    silenced: Arc<AtomicBool>,
}

impl CloseGuard {
    pub(super) fn new(sink: StreamSink) -> Self {
        Self {
            sink,
            outcome: None,
            silenced: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(super) fn sink(&self) -> &StreamSink {
        &self.sink
    }

    /// The follow finished normally.
    pub(super) fn ended(&mut self) {
        self.outcome = Some((REASON_ENDED, None));
    }

    /// The follow failed; `error` is shown to the operator verbatim.
    pub(super) fn failed(&mut self, error: String) {
        self.outcome = Some((REASON_ERROR, Some(error)));
    }

    fn silencer(&self) -> Arc<AtomicBool> {
        self.silenced.clone()
    }
}

impl Drop for CloseGuard {
    fn drop(&mut self) {
        if self.silenced.load(Ordering::Acquire) {
            return;
        }
        let (reason, error) = self.outcome.take().unwrap_or((REASON_STOPPED, None));
        self.sink.emit(StreamEvent::StreamClosed {
            stream_id: self.sink.stream_id,
            reason: reason.to_owned(),
            error,
        });
    }
}

struct Entry {
    task: JoinHandle<()>,
    silenced: Arc<AtomicBool>,
}

impl Entry {
    /// Abort; with `quiet`, the stream's guard emits nothing.
    fn stop(self, quiet: bool) {
        if quiet {
            self.silenced.store(true, Ordering::Release);
        }
        self.task.abort();
    }
}

/// The live log streams.
#[derive(Default)]
pub(super) struct LogStreams {
    streams: HashMap<StreamId, Entry>,
    /// The stream started without a client id (older clients): the next id-less start
    /// replaces it, which was the singleton behaviour those clients rely on.
    legacy: Option<StreamId>,
}

impl LogStreams {
    /// Spawn `run` as stream `guard.sink().stream_id` and register it. A live stream under the
    /// same id is replaced without a close; with `legacy`, the previous id-less stream is
    /// stopped (and reports `"stopped"`).
    pub(super) fn spawn<F, Fut>(&mut self, guard: CloseGuard, legacy: bool, run: F)
    where
        F: FnOnce(CloseGuard) -> Fut,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.prune();
        let id = guard.sink().stream_id;
        if let Some(prev) = self.streams.remove(&id) {
            prev.stop(true);
        }
        if legacy && let Some(prev_id) = self.legacy.take() {
            self.stop(prev_id);
        }
        let silenced = guard.silencer();
        let task = tokio::spawn(run(guard));
        self.streams.insert(id, Entry { task, silenced });
        if legacy {
            self.legacy = Some(id);
        }
    }

    /// Stop one stream; it reports `StreamClosed { reason: "stopped" }` unless it already
    /// ended. Unknown ids are a no-op.
    pub(super) fn stop(&mut self, id: StreamId) {
        if let Some(entry) = self.streams.remove(&id) {
            entry.stop(false);
        }
        if self.legacy == Some(id) {
            self.legacy = None;
        }
    }

    /// Stop every stream (`StopPodLogs`, context switch, shutdown).
    pub(super) fn stop_all(&mut self) {
        for (_, entry) in self.streams.drain() {
            entry.stop(false);
        }
        self.legacy = None;
    }

    /// Forget streams whose task already finished (their guard has reported the close).
    fn prune(&mut self) {
        self.streams.retain(|_, e| !e.task.is_finished());
        if let Some(id) = self.legacy
            && !self.streams.contains_key(&id)
        {
            self.legacy = None;
        }
    }

    #[cfg(test)]
    fn live(&mut self) -> Vec<StreamId> {
        self.prune();
        let mut ids: Vec<_> = self.streams.keys().copied().collect();
        ids.sort_by_key(|id| id.0);
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{SectionKey, StreamId};
    use crossbeam_channel::{Receiver, unbounded};
    use std::time::Duration;

    fn sink(id: u64, tx: &Sender<BackendEventEnvelope>) -> StreamSink {
        StreamSink {
            stream_id: StreamId(id),
            route: Route::global(SectionKey::new("workloads/pods")),
            seq: Arc::new(AtomicU64::new(0)),
            sender: tx.clone(),
        }
    }

    /// A stream that opens, then idles until aborted (a quiet pod).
    async fn idle(guard: CloseGuard) {
        guard.sink().emit(StreamEvent::PodLogsOpened {
            stream_id: guard.sink().stream_id,
        });
        std::future::pending::<()>().await;
        drop(guard);
    }

    /// Stream events received within a short window, as (id, kind, reason, error).
    async fn drain(
        rx: &Receiver<BackendEventEnvelope>,
    ) -> Vec<(u64, String, String, Option<String>)> {
        let mut out = Vec::new();
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(5)).await;
            while let Ok(env) = rx.try_recv() {
                match env.event {
                    BackendEvent::Stream(StreamEvent::PodLogsOpened { stream_id }) => {
                        out.push((stream_id.0, "opened".into(), String::new(), None));
                    }
                    BackendEvent::Stream(StreamEvent::PodLogsChunk { stream_id, lines }) => {
                        out.push((stream_id.0, "chunk".into(), lines.join("|"), None));
                    }
                    BackendEvent::Stream(StreamEvent::StreamClosed {
                        stream_id,
                        reason,
                        error,
                    }) => out.push((stream_id.0, "closed".into(), reason, error)),
                    _ => {}
                }
            }
        }
        out
    }

    fn closes(events: &[(u64, String, String, Option<String>)], id: u64) -> Vec<String> {
        events
            .iter()
            .filter(|e| e.0 == id && e.1 == "closed")
            .map(|e| e.2.clone())
            .collect()
    }

    /// The shared crate's events arrive on Fathom's channel as the same variant, under the
    /// stream's own correlation, exactly as when Fathom produced them itself.
    #[test]
    fn a_crate_event_arrives_as_fathoms_own() {
        use container_runtime::sink::StreamEvent as Crate;
        let (tx, rx) = unbounded();
        let s = sink(5, &tx);
        let delivered = container_runtime::sink::StreamSink::emit(
            &s,
            Crate::ContainerLogBatch {
                stream_id: StreamId(5),
                lines: vec!["hello".into()],
                truncated: true,
            },
        );
        assert!(delivered);
        let env = rx.try_recv().expect("one envelope");
        assert_eq!(env.correlation, Correlation::Stream(StreamId(5)));
        let BackendEvent::Stream(StreamEvent::ContainerLogBatch {
            stream_id,
            lines,
            truncated,
        }) = env.event
        else {
            panic!("not a log batch: {:?}", env.event);
        };
        assert_eq!(
            (stream_id, lines, truncated),
            (StreamId(5), vec!["hello".to_owned()], true)
        );
    }

    #[tokio::test]
    async fn concurrent_streams_are_independent() {
        let (tx, rx) = unbounded();
        let mut reg = LogStreams::default();
        reg.spawn(CloseGuard::new(sink(1 << 32, &tx)), false, idle);
        reg.spawn(CloseGuard::new(sink((1 << 32) + 1, &tx)), false, idle);
        let ev = drain(&rx).await;
        assert_eq!(ev.iter().filter(|e| e.1 == "opened").count(), 2);
        assert_eq!(reg.live(), vec![StreamId(1 << 32), StreamId((1 << 32) + 1)]);

        // Stopping one leaves the other running.
        reg.stop(StreamId(1 << 32));
        let ev = drain(&rx).await;
        assert_eq!(closes(&ev, 1 << 32), vec![REASON_STOPPED]);
        assert!(closes(&ev, (1 << 32) + 1).is_empty());
        assert_eq!(reg.live(), vec![StreamId((1 << 32) + 1)]);
    }

    #[tokio::test]
    async fn abort_emits_exactly_one_stopped_close() {
        let (tx, rx) = unbounded();
        let mut reg = LogStreams::default();
        reg.spawn(CloseGuard::new(sink(7, &tx)), false, idle);
        reg.stop(StreamId(7));
        reg.stop(StreamId(7)); // repeat stop is a no-op
        let ev = drain(&rx).await;
        assert_eq!(closes(&ev, 7), vec![REASON_STOPPED]);
    }

    #[tokio::test]
    async fn failure_and_end_carry_their_reason_once() {
        let (tx, rx) = unbounded();
        let mut reg = LogStreams::default();
        reg.spawn(CloseGuard::new(sink(1, &tx)), false, |mut g| async move {
            g.failed("pods \"x\" not found".into());
        });
        reg.spawn(CloseGuard::new(sink(2, &tx)), false, |mut g| async move {
            g.sink().emit(StreamEvent::PodLogsChunk {
                stream_id: StreamId(2),
                lines: vec!["hello".into()],
            });
            g.ended();
        });
        let ev = drain(&rx).await;
        let err: Vec<_> = ev.iter().filter(|e| e.0 == 1 && e.1 == "closed").collect();
        assert_eq!(err.len(), 1);
        assert_eq!(err[0].2, REASON_ERROR);
        assert_eq!(err[0].3.as_deref(), Some("pods \"x\" not found"));
        assert_eq!(closes(&ev, 2), vec![REASON_ENDED]);
        // A finished stream is forgotten, and stopping it afterwards emits nothing more.
        assert!(reg.live().is_empty());
        reg.stop(StreamId(2));
        assert!(closes(&drain(&rx).await, 2).is_empty());
    }

    #[tokio::test]
    async fn restarting_a_live_id_replaces_it_without_a_close() {
        let (tx, rx) = unbounded();
        let mut reg = LogStreams::default();
        reg.spawn(CloseGuard::new(sink(9, &tx)), false, idle);
        drain(&rx).await;
        reg.spawn(CloseGuard::new(sink(9, &tx)), false, idle);
        let ev = drain(&rx).await;
        assert!(closes(&ev, 9).is_empty(), "{ev:?}");
        assert_eq!(ev.iter().filter(|e| e.1 == "opened").count(), 1);
        assert_eq!(reg.live(), vec![StreamId(9)]);
    }

    #[tokio::test]
    async fn legacy_start_replaces_only_the_previous_idless_stream() {
        let (tx, rx) = unbounded();
        let mut reg = LogStreams::default();
        reg.spawn(CloseGuard::new(sink(1 << 32, &tx)), false, idle);
        reg.spawn(CloseGuard::new(sink(3, &tx)), true, idle);
        reg.spawn(CloseGuard::new(sink(4, &tx)), true, idle);
        let ev = drain(&rx).await;
        assert_eq!(closes(&ev, 3), vec![REASON_STOPPED]);
        assert!(closes(&ev, 1 << 32).is_empty());
        assert_eq!(reg.live(), vec![StreamId(4), StreamId(1 << 32)]);
    }

    #[tokio::test]
    async fn stop_all_closes_everything() {
        let (tx, rx) = unbounded();
        let mut reg = LogStreams::default();
        reg.spawn(CloseGuard::new(sink(1, &tx)), false, idle);
        reg.spawn(CloseGuard::new(sink(2, &tx)), true, idle);
        reg.stop_all();
        let ev = drain(&rx).await;
        assert_eq!(closes(&ev, 1), vec![REASON_STOPPED]);
        assert_eq!(closes(&ev, 2), vec![REASON_STOPPED]);
        assert!(reg.live().is_empty());
    }
}
