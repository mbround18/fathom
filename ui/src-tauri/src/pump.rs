//! Event pump: backend `gui_rx` → coalesced batches → the webview (contracts/ipc.md
//! "Pump behavior").
//!
//! Deliberately free of Tauri types so it can be unit-tested without a webview: the
//! Tauri `Channel` is plugged in through [`BatchSink`], and eviction priority through
//! [`Evictable`].

use crossbeam_channel::{Receiver, RecvTimeoutError};
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Most envelopes sent in one batch.
pub const MAX_BATCH: usize = 256;
/// Longest time the pump waits for more envelopes after the first one of a batch.
pub const BATCH_WINDOW: Duration = Duration::from_millis(16);
/// Envelopes buffered while no subscriber is attached, before eviction starts.
pub const BUFFER_CAP: usize = 10_000;

/// Eviction priority of a buffered item when the buffer is full.
pub trait Evictable {
    /// Lower ranks are evicted first (oldest first within a rank). `None` is never evicted,
    /// so the buffer may exceed its cap if it holds only such items.
    fn eviction_rank(&self) -> Option<u8>;
}

/// The subscriber rejected a batch (e.g. its webview is gone).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinkError(pub String);

impl std::fmt::Display for SinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SinkError {}

/// Where batches go: the Tauri `Channel` in production, a recorder in tests.
pub trait BatchSink<T> {
    /// Deliver `envelopes` (in order) plus the count of envelopes dropped since the last
    /// successful delivery.
    fn send(&mut self, envelopes: &[T], dropped: u32) -> Result<(), SinkError>;
}

/// Block for the first envelope, then keep collecting until `max` envelopes or `window`
/// has elapsed since the first one. Returns an empty batch only once the channel is
/// disconnected and drained.
pub fn collect_batch<T>(rx: &Receiver<T>, max: usize, window: Duration) -> Vec<T> {
    let Ok(first) = rx.recv() else {
        return Vec::new();
    };
    let deadline = Instant::now() + window;
    let mut batch = Vec::with_capacity(max.min(64));
    batch.push(first);

    while batch.len() < max {
        // Drain whatever is already queued without touching the clock.
        match rx.try_recv() {
            Ok(item) => {
                batch.push(item);
                continue;
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => break,
            Err(crossbeam_channel::TryRecvError::Empty) => {}
        }
        match rx.recv_deadline(deadline) {
            Ok(item) => batch.push(item),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
        }
    }
    batch
}

struct HubState<T, S> {
    /// Invariant: while `subscriber` is `Some`, `buffer` is empty.
    subscriber: Option<S>,
    buffer: VecDeque<T>,
    /// Evicted since the last successful delivery.
    dropped: u32,
}

/// Holds the single subscriber, or buffers envelopes while there is none.
///
/// One mutex guards subscriber, buffer, and drop count together, so a subscribe (which
/// flushes the buffer) and a delivery from the pump can never interleave or reorder.
pub struct Hub<T, S> {
    state: Mutex<HubState<T, S>>,
    cap: usize,
}

impl<T: Evictable, S: BatchSink<T>> Hub<T, S> {
    pub fn new(cap: usize) -> Self {
        Self {
            state: Mutex::new(HubState {
                subscriber: None,
                buffer: VecDeque::new(),
                dropped: 0,
            }),
            cap,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HubState<T, S>> {
        // A panic inside a sink must not wedge the pump forever.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Send `batch` to the subscriber, or buffer it if there is none. If the subscriber
    /// rejects it, the subscriber is detached and the batch is buffered.
    pub fn deliver(&self, batch: Vec<T>) {
        if batch.is_empty() {
            return;
        }
        let mut state = self.lock();
        let state = &mut *state;
        if let Some(sink) = state.subscriber.as_mut() {
            match sink.send(&batch, state.dropped) {
                Ok(()) => {
                    state.dropped = 0;
                    return;
                }
                Err(e) => {
                    tracing::warn!("Event subscriber rejected a batch; detaching: {}", e);
                    state.subscriber = None;
                }
            }
        }
        for item in batch {
            push_bounded(state, item, self.cap);
        }
    }

    /// Replace any current subscriber with `sink`, first flushing the buffered backlog to
    /// it in order (in chunks of [`MAX_BATCH`]). If the flush fails, `sink` is not
    /// attached and whatever was not delivered stays buffered.
    pub fn subscribe(&self, mut sink: S) -> Result<(), SinkError> {
        let mut state = self.lock();
        let state = &mut *state;
        state.subscriber = None;

        while !state.buffer.is_empty() {
            let n = state.buffer.len().min(MAX_BATCH);
            let chunk = &state.buffer.make_contiguous()[..n];
            sink.send(chunk, state.dropped)?;
            state.buffer.drain(..n);
            state.dropped = 0;
        }
        if state.dropped > 0 {
            // Nothing left to deliver, but the UI still needs to hear about the loss.
            sink.send(&[], state.dropped)?;
            state.dropped = 0;
        }

        state.subscriber = Some(sink);
        Ok(())
    }

    pub fn has_subscriber(&self) -> bool {
        self.lock().subscriber.is_some()
    }

    pub fn buffered_len(&self) -> usize {
        self.lock().buffer.len()
    }

    pub fn dropped(&self) -> u32 {
        self.lock().dropped
    }
}

fn push_bounded<T: Evictable, S>(state: &mut HubState<T, S>, item: T, cap: usize) {
    state.buffer.push_back(item);
    while state.buffer.len() > cap {
        let victim = state
            .buffer
            .iter()
            .enumerate()
            .filter_map(|(i, item)| item.eviction_rank().map(|rank| (rank, i)))
            // Lowest rank, then oldest (smallest index).
            .min();
        match victim {
            Some((_, i)) => {
                state.buffer.remove(i);
                state.dropped = state.dropped.saturating_add(1);
            }
            None => break,
        }
    }
}

/// Run until the backend drops its sender: collect batches from `rx` and hand them to
/// `hub`. `on_envelope` sees every envelope first, in order.
pub fn run_pump<T, S, F>(rx: &Receiver<T>, hub: &Hub<T, S>, mut on_envelope: F)
where
    T: Evictable,
    S: BatchSink<T>,
    F: FnMut(&T),
{
    loop {
        let batch = collect_batch(rx, MAX_BATCH, BATCH_WINDOW);
        if batch.is_empty() {
            tracing::info!("Backend event channel closed; pump exiting");
            return;
        }
        batch.iter().for_each(&mut on_envelope);
        hub.deliver(batch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use std::sync::Arc;

    /// `(seq, rank)`; rank `None` = never evicted (a System envelope).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Item(u32, Option<u8>);

    impl Evictable for Item {
        fn eviction_rank(&self) -> Option<u8> {
            self.1
        }
    }

    const LIST: Option<u8> = Some(0);
    const STREAM: Option<u8> = Some(2);
    const SYSTEM: Option<u8> = None;

    #[derive(Default)]
    struct Log {
        batches: Vec<(Vec<Item>, u32)>,
        /// Fail every send once this many batches have been accepted.
        fail_after: Option<usize>,
    }

    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Log>>);

    impl Recorder {
        fn failing_after(n: usize) -> Self {
            let r = Self::default();
            r.0.lock().unwrap().fail_after = Some(n);
            r
        }
        fn batches(&self) -> Vec<(Vec<Item>, u32)> {
            self.0.lock().unwrap().batches.clone()
        }
        fn seqs(&self) -> Vec<u32> {
            self.batches()
                .into_iter()
                .flat_map(|(b, _)| b.into_iter().map(|i| i.0))
                .collect()
        }
    }

    impl BatchSink<Item> for Recorder {
        fn send(&mut self, envelopes: &[Item], dropped: u32) -> Result<(), SinkError> {
            let mut log = self.0.lock().unwrap();
            if log.fail_after.is_some_and(|n| log.batches.len() >= n) {
                return Err(SinkError("webview gone".into()));
            }
            log.batches.push((envelopes.to_vec(), dropped));
            Ok(())
        }
    }

    fn items(range: std::ops::Range<u32>, rank: Option<u8>) -> Vec<Item> {
        range.map(|i| Item(i, rank)).collect()
    }

    #[test]
    fn collect_batch_caps_at_max() {
        let (tx, rx) = unbounded();
        for i in 0..600 {
            tx.send(i).unwrap();
        }
        assert_eq!(collect_batch(&rx, 256, BATCH_WINDOW).len(), 256);
        assert_eq!(collect_batch(&rx, 256, BATCH_WINDOW).len(), 256);
        assert_eq!(
            collect_batch(&rx, 256, BATCH_WINDOW),
            (512..600).collect::<Vec<_>>()
        );
    }

    #[test]
    fn collect_batch_stops_at_window() {
        let (tx, rx) = unbounded();
        tx.send(1).unwrap();
        let start = Instant::now();
        let batch = collect_batch(&rx, 256, Duration::from_millis(20));
        let elapsed = start.elapsed();
        assert_eq!(batch, vec![1]);
        assert!(elapsed >= Duration::from_millis(20), "{elapsed:?}");
        assert!(elapsed < Duration::from_millis(500), "{elapsed:?}");
        drop(tx);
    }

    #[test]
    fn collect_batch_coalesces_late_arrivals_within_window() {
        let (tx, rx) = unbounded();
        let producer = std::thread::spawn(move || {
            tx.send(1).unwrap();
            std::thread::sleep(Duration::from_millis(5));
            tx.send(2).unwrap();
        });
        let batch = collect_batch(&rx, 256, Duration::from_millis(200));
        producer.join().unwrap();
        assert_eq!(batch, vec![1, 2]);
    }

    #[test]
    fn collect_batch_returns_partial_then_empty_on_disconnect() {
        let (tx, rx) = unbounded();
        tx.send(1).unwrap();
        tx.send(2).unwrap();
        drop(tx);
        assert_eq!(collect_batch(&rx, 256, Duration::from_secs(5)), vec![1, 2]);
        assert!(collect_batch(&rx, 256, Duration::from_secs(5)).is_empty());
    }

    #[test]
    fn buffers_without_subscriber_then_flushes_in_order_and_chunks() {
        let hub: Hub<Item, Recorder> = Hub::new(BUFFER_CAP);
        hub.deliver(items(0..300, LIST));
        hub.deliver(items(300..310, SYSTEM));
        assert_eq!(hub.buffered_len(), 310);

        let rec = Recorder::default();
        hub.subscribe(rec.clone()).unwrap();
        assert!(hub.has_subscriber());
        assert_eq!(hub.buffered_len(), 0);
        let sizes: Vec<usize> = rec.batches().iter().map(|(b, _)| b.len()).collect();
        assert_eq!(sizes, vec![256, 54]);
        assert_eq!(rec.seqs(), (0..310).collect::<Vec<_>>());

        hub.deliver(items(310..312, STREAM));
        assert_eq!(rec.seqs(), (0..312).collect::<Vec<_>>());
    }

    #[test]
    fn eviction_order_list_then_stream_never_system() {
        let hub: Hub<Item, Recorder> = Hub::new(4);
        hub.deliver(vec![
            Item(0, SYSTEM),
            Item(1, STREAM),
            Item(2, LIST),
            Item(3, LIST),
        ]);
        // Over cap by one each time: oldest LIST first.
        hub.deliver(vec![Item(4, STREAM)]);
        hub.deliver(vec![Item(5, SYSTEM)]);
        // No LIST left: oldest STREAM next.
        hub.deliver(vec![Item(6, SYSTEM)]);
        assert_eq!(hub.dropped(), 3);

        let rec = Recorder::default();
        hub.subscribe(rec.clone()).unwrap();
        assert_eq!(rec.seqs(), vec![0, 4, 5, 6]);
        assert_eq!(rec.batches()[0].1, 3, "dropped count rides the first batch");
        assert_eq!(hub.dropped(), 0);
    }

    #[test]
    fn system_envelopes_may_exceed_cap() {
        let hub: Hub<Item, Recorder> = Hub::new(2);
        hub.deliver(items(0..5, SYSTEM));
        assert_eq!(hub.buffered_len(), 5);
        assert_eq!(hub.dropped(), 0);
    }

    #[test]
    fn dropped_only_is_reported_with_an_empty_batch() {
        let hub: Hub<Item, Recorder> = Hub::new(0);
        hub.deliver(items(0..3, LIST));
        assert_eq!(hub.buffered_len(), 0);
        let rec = Recorder::default();
        hub.subscribe(rec.clone()).unwrap();
        assert_eq!(rec.batches(), vec![(vec![], 3)]);
    }

    #[test]
    fn failed_send_detaches_and_rebuffers() {
        let hub: Hub<Item, Recorder> = Hub::new(BUFFER_CAP);
        let rec = Recorder::failing_after(1);
        hub.subscribe(rec.clone()).unwrap();
        hub.deliver(items(0..2, LIST));
        hub.deliver(items(2..4, LIST)); // rejected
        assert!(!hub.has_subscriber());
        assert_eq!(hub.buffered_len(), 2);
        hub.deliver(items(4..5, LIST));

        let next = Recorder::default();
        hub.subscribe(next.clone()).unwrap();
        assert_eq!(rec.seqs(), vec![0, 1]);
        assert_eq!(next.seqs(), vec![2, 3, 4]);
    }

    #[test]
    fn failed_flush_keeps_the_rest_buffered_and_does_not_attach() {
        let hub: Hub<Item, Recorder> = Hub::new(BUFFER_CAP);
        hub.deliver(items(0..600, LIST));
        let flaky = Recorder::failing_after(1);
        assert!(hub.subscribe(flaky.clone()).is_err());
        assert!(!hub.has_subscriber());
        assert_eq!(flaky.seqs(), (0..256).collect::<Vec<_>>());
        assert_eq!(hub.buffered_len(), 344);

        let next = Recorder::default();
        hub.subscribe(next.clone()).unwrap();
        assert_eq!(next.seqs(), (256..600).collect::<Vec<_>>());
    }

    #[test]
    fn new_subscriber_replaces_old() {
        let hub: Hub<Item, Recorder> = Hub::new(BUFFER_CAP);
        let first = Recorder::default();
        let second = Recorder::default();
        hub.subscribe(first.clone()).unwrap();
        hub.deliver(items(0..1, LIST));
        hub.subscribe(second.clone()).unwrap();
        hub.deliver(items(1..2, LIST));
        assert_eq!(first.seqs(), vec![0]);
        assert_eq!(second.seqs(), vec![1]);
    }

    #[test]
    fn run_pump_delivers_everything_in_order_and_exits_on_disconnect() {
        let (tx, rx) = unbounded();
        let hub: Arc<Hub<Item, Recorder>> = Arc::new(Hub::new(BUFFER_CAP));
        let rec = Recorder::default();
        hub.subscribe(rec.clone()).unwrap();

        let pump_hub = Arc::clone(&hub);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let pump_seen = Arc::clone(&seen);
        let pump = std::thread::spawn(move || {
            run_pump(&rx, &pump_hub, |item: &Item| {
                pump_seen.lock().unwrap().push(item.0)
            });
        });
        for i in 0..1000 {
            tx.send(Item(i, LIST)).unwrap();
        }
        drop(tx);
        pump.join().unwrap();

        assert_eq!(rec.seqs(), (0..1000).collect::<Vec<_>>());
        assert_eq!(*seen.lock().unwrap(), (0..1000).collect::<Vec<_>>());
        assert!(rec.batches().iter().all(|(b, _)| b.len() <= MAX_BATCH));
    }
}
