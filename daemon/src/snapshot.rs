//! Warm snapshots (spec 036 US4, research R6): `GET /v1/snapshot/{pods,events}` served from the
//! daemon's own long-lived list+watch, so a client can populate a large list without its own
//! full LIST and then watch the API server from the snapshot's `resourceVersion`.
//!
//! A store is fed by exactly one list+watch loop per partition (one partition in cluster mode,
//! one per namespace in namespace-scoped mode). A relist is buffered and swapped in at the end,
//! so readers never see a half-listed store. The `resourceVersion` advances with every event
//! *and bookmark*, in order, so it is always one the store is consistent with (never ahead of
//! what it holds); a client watching from it misses nothing.
//!
//! `ageMs` is how long the store may have been behind the API server: 0 while every partition's
//! watch is established, otherwise the time since the first partition lost its watch. Clients
//! treat an age at or above their threshold (30 s) or `synced: false` as stale and go direct.

use crate::authz::{ResourceSet, namespace_visible};
use chrono::{DateTime, Utc};
use fathom::protocol::v1::VisibleScope;
use kube::{Resource, ResourceExt};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// Why a snapshot cannot be served.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    /// The initial list has not completed for every partition (HTTP 503).
    NotSynced,
    /// Serialization failed (HTTP 500).
    Encode(String),
}

/// A kind the daemon can serve a snapshot of; object-safe so HTTP holds a heterogeneous list.
pub trait SnapshotSource: Send + Sync {
    /// Path segment and capability name (`pods`, `events`).
    fn kind(&self) -> &'static str;
    /// Resources the caller must be able to `list` for an object to be included.
    fn access(&self) -> &'static ResourceSet;
    /// JSON body `{ kind, resourceVersion, ageMs, synced, items }`, filtered to `scope` and
    /// ordered by (namespace, name) like an API server list.
    fn render(&self, scope: &VisibleScope, now: DateTime<Utc>) -> Result<Vec<u8>, SnapshotError>;
    /// Distinct namespaces of the objects held, so access is resolved over every namespace
    /// the snapshot could return (not only those the topology/event hub has seen).
    fn namespaces(&self) -> Vec<String>;
    /// Objects held (all partitions, unfiltered); for perf accounting.
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// `true` when resource version `b` is newer than `a`. Resource versions are opaque, but every
/// API server in practice uses etcd revisions; compare numerically when both parse, otherwise
/// the later observation wins (callers only pass versions in observation order).
pub fn rv_newer(a: &str, b: &str) -> bool {
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(x), Ok(y)) => y > x,
        _ => true,
    }
}

struct Partition<K> {
    items: HashMap<String, Arc<K>>,
    /// Objects of an in-progress relist; swapped in at `init_done`.
    relist: Option<HashMap<String, Arc<K>>>,
    resource_version: Option<String>,
    synced: bool,
    /// Since when the watch has been down (None = established).
    stale_since: Option<DateTime<Utc>>,
}

impl<K> Partition<K> {
    fn new(now: DateTime<Utc>) -> Self {
        Self {
            items: HashMap::new(),
            relist: None,
            resource_version: None,
            synced: false,
            // Not established until the first watch connects.
            stale_since: Some(now),
        }
    }

    fn bump_rv(&mut self, rv: Option<String>) {
        if let Some(rv) = rv.filter(|r| !r.is_empty())
            && self
                .resource_version
                .as_deref()
                .is_none_or(|cur| rv_newer(cur, &rv))
        {
            self.resource_version = Some(rv);
        }
    }
}

pub struct SnapshotStore<K> {
    kind: &'static str,
    access: &'static ResourceSet,
    parts: Mutex<HashMap<String, Partition<K>>>,
    /// Average encoded size of one item in the previous render, used to size the next buffer
    /// in one allocation (a doubling `Vec` briefly holds ~2x a multi-MiB body).
    bytes_per_item: AtomicUsize,
}

impl<K> SnapshotStore<K>
where
    K: Resource + Serialize + Send + Sync + 'static,
{
    /// `partitions` are the watch keys that must all sync (e.g. `["Pod"]` or `["Pod/a","Pod/b"]`).
    pub fn new(
        kind: &'static str,
        access: &'static ResourceSet,
        partitions: &[String],
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            kind,
            access,
            parts: Mutex::new(
                partitions
                    .iter()
                    .map(|p| (p.clone(), Partition::new(now)))
                    .collect(),
            ),
            bytes_per_item: AtomicUsize::new(0),
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Partition<K>>> {
        self.parts.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn with<R>(&self, part: &str, f: impl FnOnce(&mut Partition<K>) -> R) -> Option<R> {
        self.lock().get_mut(part).map(f)
    }

    /// A (re)list of `part` starts; the current items keep being served until `init_done`.
    pub fn init(&self, part: &str) {
        self.with(part, |p| p.relist = Some(HashMap::new()));
    }

    pub fn init_apply(&self, part: &str, obj: Arc<K>) {
        self.with(part, |p| {
            let uid = obj.uid().unwrap_or_default();
            match p.relist.as_mut() {
                Some(r) => {
                    r.insert(uid, obj);
                }
                None => {
                    p.items.insert(uid, obj);
                }
            }
        });
    }

    /// The (re)list finished at `list_rv`: swap it in. The version is taken as-is (a relist
    /// may legitimately restart from any version).
    pub fn init_done(&self, part: &str, list_rv: Option<String>) {
        self.with(part, |p| {
            if let Some(r) = p.relist.take() {
                p.items = r;
            }
            if let Some(rv) = list_rv.filter(|r| !r.is_empty()) {
                p.resource_version = Some(rv);
            }
            p.synced = true;
        });
    }

    /// The watch for `part` is established (the store follows the API server again).
    pub fn established(&self, part: &str) {
        self.with(part, |p| p.stale_since = None);
    }

    /// The watch for `part` failed; the store may fall behind from `now`.
    pub fn lost(&self, part: &str, now: DateTime<Utc>) {
        self.with(part, |p| {
            p.stale_since.get_or_insert(now);
        });
    }

    pub fn apply(&self, part: &str, obj: Arc<K>) {
        self.with(part, |p| {
            p.bump_rv(obj.resource_version());
            p.items.insert(obj.uid().unwrap_or_default(), obj);
        });
    }

    pub fn delete(&self, part: &str, obj: &K) {
        self.with(part, |p| {
            p.bump_rv(obj.resource_version());
            p.items.remove(&obj.uid().unwrap_or_default());
        });
    }

    pub fn bookmark(&self, part: &str, rv: String) {
        self.with(part, |p| p.bump_rv(Some(rv)));
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Body<'a, K> {
    kind: &'static str,
    resource_version: Option<String>,
    age_ms: u64,
    synced: bool,
    items: Vec<&'a K>,
}

impl<K> SnapshotSource for SnapshotStore<K>
where
    K: Resource + Serialize + Send + Sync + 'static,
{
    fn kind(&self) -> &'static str {
        self.kind
    }

    fn access(&self) -> &'static ResourceSet {
        self.access
    }

    fn render(&self, scope: &VisibleScope, now: DateTime<Utc>) -> Result<Vec<u8>, SnapshotError> {
        // Clone the Arcs under the lock, serialize outside it so watchers are never blocked on
        // a large response.
        let (mut items, rv, age_ms) = {
            let g = self.lock();
            if g.is_empty() || g.values().any(|p| !p.synced) {
                return Err(SnapshotError::NotSynced);
            }
            let items: Vec<Arc<K>> = g
                .values()
                .flat_map(|p| p.items.values())
                .filter(|o| namespace_visible(scope, o.meta().namespace.as_deref()))
                .cloned()
                .collect();
            // With several partitions the oldest version is the one every partition is
            // consistent with (a watch from it may replay, never skip).
            let rv = g
                .values()
                .filter_map(|p| p.resource_version.clone())
                .reduce(|a, b| if rv_newer(&b, &a) { b } else { a });
            let age = g
                .values()
                .filter_map(|p| p.stale_since)
                .min()
                .map_or(0, |t| (now - t).num_milliseconds().max(0) as u64);
            (items, rv, age)
        };
        items.sort_by(|a, b| {
            (a.meta().namespace.as_deref(), a.meta().name.as_deref())
                .cmp(&(b.meta().namespace.as_deref(), b.meta().name.as_deref()))
        });
        let body = Body {
            kind: self.kind,
            resource_version: rv,
            age_ms,
            synced: true,
            items: items.iter().map(|a| a.as_ref()).collect(),
        };
        let n = body.items.len();
        let per_item = self.bytes_per_item.load(Ordering::Relaxed);
        let mut out = Vec::with_capacity(256 + n * (per_item + per_item / 16));
        serde_json::to_writer(&mut out, &body).map_err(|e| SnapshotError::Encode(e.to_string()))?;
        if let Some(avg) = out.len().checked_div(n) {
            self.bytes_per_item.store(avg, Ordering::Relaxed);
        }
        Ok(out)
    }

    fn namespaces(&self) -> Vec<String> {
        let set: std::collections::BTreeSet<String> = self
            .lock()
            .values()
            .flat_map(|p| p.items.values())
            .filter_map(|o| o.meta().namespace.clone())
            .collect();
        set.into_iter().collect()
    }

    fn len(&self) -> usize {
        self.lock().values().map(|p| p.items.len()).sum()
    }
}
