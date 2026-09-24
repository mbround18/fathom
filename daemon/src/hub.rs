//! Shared state: the event log, the topology graph and the live fan-out. Everything that
//! shares the sequence space lives under one lock so a subscriber's replay and its live
//! receiver join at exactly the same point (no gap, no duplicate).

use crate::eventlog::{Entry, EventLog, Payload, Replay};
use crate::topology::{Obj, TopologyState};
use chrono::{DateTime, Duration, Utc};
use fathom::protocol::v1::{DerivedControllerEvent, TopologyEdge, TopologyNode, VisibleScope};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::broadcast;

struct Inner {
    log: EventLog,
    topo: TopologyState,
    seen_ns: BTreeSet<String>,
}

pub struct Hub {
    inner: Mutex<Inner>,
    tx: broadcast::Sender<Arc<Entry>>,
}

impl Hub {
    /// `queue` bounds each subscriber's backlog; a subscriber that falls further behind lags
    /// (and is dropped by the stream handler) rather than blocking producers.
    pub fn new(epoch: String, max_events: usize, max_age: Duration, queue: usize) -> Self {
        let (tx, _) = broadcast::channel(queue.max(1));
        Self {
            inner: Mutex::new(Inner {
                log: EventLog::new(epoch, max_events, max_age),
                topo: TopologyState::default(),
                seen_ns: BTreeSet::new(),
            }),
            tx,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn push(&self, g: &mut Inner, payload: Payload, now: DateTime<Utc>) -> Entry {
        let e = g.log.append(payload, now);
        // Sent while holding the lock: ordering matches seq and `subscribe` cannot interleave.
        let _ = self.tx.send(Arc::new(e.clone()));
        e
    }

    pub fn epoch(&self) -> String {
        self.lock().log.epoch().to_string()
    }

    pub fn last_seq(&self) -> u64 {
        self.lock().log.last_seq()
    }

    pub fn append_event(&self, ev: DerivedControllerEvent, now: DateTime<Utc>) -> Entry {
        let mut g = self.lock();
        if let Some(ns) = &ev.namespace {
            g.seen_ns.insert(ns.clone());
        }
        self.push(&mut g, Payload::Event(ev), now)
    }

    pub fn topology_upsert(&self, obj: Obj, now: DateTime<Utc>) -> Option<Entry> {
        let mut g = self.lock();
        if let Some(ns) = &obj.namespace {
            g.seen_ns.insert(ns.clone());
        }
        let d = g.topo.upsert(obj);
        (!d.is_empty()).then(|| self.push(&mut g, Payload::Topology(d), now))
    }

    pub fn topology_remove(&self, uid: &str, now: DateTime<Utc>) -> Option<Entry> {
        let mut g = self.lock();
        let d = g.topo.remove(uid);
        (!d.is_empty()).then(|| self.push(&mut g, Payload::Topology(d), now))
    }

    pub fn subscribe(
        &self,
        epoch: Option<&str>,
        after: Option<u64>,
        now: DateTime<Utc>,
    ) -> (Replay, broadcast::Receiver<Arc<Entry>>) {
        let mut g = self.lock();
        g.log.evict(now);
        let replay = g.log.replay(epoch, after, now);
        (replay, self.tx.subscribe())
    }

    /// Snapshot filtered to `scope`, with the seq it is consistent with.
    pub fn topology_snapshot(
        &self,
        scope: &VisibleScope,
    ) -> (u64, Vec<TopologyNode>, Vec<TopologyEdge>) {
        let g = self.lock();
        let (n, e) = g.topo.snapshot(scope);
        (g.log.last_seq(), n, e)
    }

    /// Namespaces the daemon has seen (objects or events); the universe for scope resolution.
    pub fn known_namespaces(&self) -> Vec<String> {
        self.lock().seen_ns.iter().cloned().collect()
    }

    /// Namespace of a known topology object (outer `None` = unknown).
    pub fn ns_of(&self, uid: &str) -> Option<Option<String>> {
        self.lock().topo.ns_of(uid)
    }

    pub fn uid_of(&self, kind: &str, ns: Option<&str>, name: &str) -> Option<String> {
        self.lock().topo.uid_of(kind, ns, name)
    }

    /// Number of objects in the topology graph.
    pub fn object_count(&self) -> usize {
        self.lock().topo.len()
    }
}

/// Tracks which watchers finished their initial list; drives `/readyz` and `synced`.
pub struct SyncTracker {
    pending: Mutex<std::collections::HashSet<String>>,
}

impl SyncTracker {
    pub fn new(kinds: &[&str]) -> Self {
        Self {
            pending: Mutex::new(kinds.iter().map(|k| k.to_string()).collect()),
        }
    }

    pub fn mark(&self, kind: &str) {
        self.pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(kind);
    }

    pub fn is_synced(&self) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_empty()
    }
}
