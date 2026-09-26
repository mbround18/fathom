//! 036-incluster-daemon US4 (FR-013): snapshot-then-delta list sources.
//!
//! When a daemon session runs for the active context (preference `auto`) and it offers a warm
//! snapshot of a kind, the initial list is served from the daemon's cache instead of a full
//! LIST against the API server; the client then watches the API server directly from the
//! snapshot's `resourceVersion`. Anything short of a fresh, synced snapshot (no daemon, not
//! offered, not synced, `ageMs` >= 30 s, HTTP error, timeout, unreadable body, or a `410 Gone`
//! when watching from it) falls back transparently to the direct LIST(+WATCH). With the
//! preference `off` none of this runs: callers keep their legacy code paths (SC-002).

use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crossbeam_channel::Sender;
use futures_util::StreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams, WatchEvent, WatchParams};
use kube::{Resource, ResourceExt};
use tokio::time::Instant;

use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, ResourceListEvent, Route, section_keys,
};

use super::daemon_client::{SnapshotSlot, SnapshotSource, source_for};
use super::{events, pods};

/// Emission batching for the delta watcher (the legacy watcher re-LISTs on every event).
const EMIT_WINDOW: Duration = Duration::from_millis(250);
const RETRY_MIN: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);

/// Where a list came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ListOrigin {
    Daemon,
    Direct,
}

/// Items of one list plus the version to watch from.
#[derive(Debug)]
pub(super) struct Listed<K> {
    pub items: Vec<K>,
    pub resource_version: Option<String>,
    pub origin: ListOrigin,
}

/// Initial list of `K`, from the daemon's warm snapshot (`daemon_kind`, e.g. `pods`) when
/// `source` is given and the snapshot is usable, otherwise a direct cluster-wide LIST.
pub(super) async fn list_with_daemon<K>(
    client: &kube::Client,
    source: Option<&SnapshotSource>,
    daemon_kind: &str,
) -> kube::Result<Listed<K>>
where
    K: Resource<DynamicType = ()> + Clone + serde::de::DeserializeOwned + Debug,
{
    if let Some(src) = source {
        match src.fetch::<K>(daemon_kind).await {
            Ok(s) => {
                tracing::debug!(
                    kind = daemon_kind,
                    items = s.items.len(),
                    age_ms = s.age_ms,
                    "list served from daemon snapshot"
                );
                return Ok(Listed {
                    items: s.items,
                    resource_version: Some(s.resource_version),
                    origin: ListOrigin::Daemon,
                });
            }
            Err(reason) => {
                tracing::debug!(kind = daemon_kind, %reason, "daemon snapshot unused; listing directly");
            }
        }
    }
    let l = Api::<K>::all(client.clone())
        .list(&ListParams::default())
        .await?;
    Ok(Listed {
        items: l.items,
        resource_version: l.metadata.resource_version,
        origin: ListOrigin::Direct,
    })
}

fn is_gone(e: &kube::Error) -> bool {
    matches!(e, kube::Error::Api(s) if s.code == 410)
}

/// Apply one watch event to `items` (keyed by uid). Returns `Some(relist)` when the watch
/// must end: `true` for `410 Gone` (the version is too old; take a new snapshot), `false`
/// for any other error (re-watch from the current version).
pub(super) fn apply_event<K: Resource>(
    items: &mut HashMap<String, K>,
    rv: &mut String,
    ev: WatchEvent<K>,
) -> Option<bool> {
    match ev {
        WatchEvent::Added(o) | WatchEvent::Modified(o) => {
            if let Some(v) = o.resource_version() {
                *rv = v;
            }
            items.insert(o.uid().unwrap_or_default(), o);
            None
        }
        WatchEvent::Deleted(o) => {
            if let Some(v) = o.resource_version() {
                *rv = v;
            }
            items.remove(&o.uid().unwrap_or_default());
            None
        }
        WatchEvent::Bookmark(b) => {
            *rv = b.metadata.resource_version;
            None
        }
        WatchEvent::Error(e) => Some(e.code == 410),
    }
}

/// Where the delta watcher's pod lists go (same route/shape as the legacy watcher).
struct PodSink {
    filter_shared: Arc<RwLock<Vec<String>>>,
    seq: Arc<AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
    context: Option<String>,
}

impl PodSink {
    fn emit(&self, items: &HashMap<String, Pod>) {
        let filter = self
            .filter_shared
            .read()
            .map(|f| f.clone())
            .unwrap_or_default();
        let mut list: Vec<Pod> = items.values().cloned().collect();
        list.sort_by(|a, b| {
            (a.metadata.namespace.as_deref(), a.metadata.name.as_deref())
                .cmp(&(b.metadata.namespace.as_deref(), b.metadata.name.as_deref()))
        });
        let summaries = pods::pods_to_summaries(list, &filter);
        events::emit(
            self.seq.as_ref(),
            &self.sender,
            Route::namespaced(section_keys::workloads_pods(), self.context.clone(), None),
            Correlation::None,
            BackendEvent::ResourceList(ResourceListEvent::PodsUpdated(summaries)),
        );
    }
}

async fn backoff(retry: &mut Duration) {
    tokio::time::sleep(*retry).await;
    *retry = (*retry * 2).min(RETRY_MAX);
}

/// Consume one watch stream into `items`, re-emitting at most every [`EMIT_WINDOW`].
/// Returns `None` when the server ended the watch normally, else as [`apply_event`].
async fn follow<S>(
    stream: S,
    items: &mut HashMap<String, Pod>,
    rv: &mut String,
    sink: &PodSink,
) -> Option<bool>
where
    S: futures_util::Stream<Item = kube::Result<WatchEvent<Pod>>>,
{
    let mut stream = std::pin::pin!(stream);
    let mut flush_at: Option<Instant> = None;
    let out = loop {
        let next = match flush_at {
            Some(t) => match tokio::time::timeout_at(t, stream.next()).await {
                Ok(n) => n,
                Err(_) => {
                    sink.emit(items);
                    flush_at = None;
                    continue;
                }
            },
            None => stream.next().await,
        };
        let ev = match next {
            None => break None,
            Some(Err(e)) => break Some(is_gone(&e)),
            Some(Ok(ev)) => ev,
        };
        let bookmark = matches!(ev, WatchEvent::Bookmark(_));
        if let Some(relist) = apply_event(items, rv, ev) {
            break Some(relist);
        }
        if !bookmark {
            flush_at.get_or_insert_with(|| Instant::now() + EMIT_WINDOW);
        }
    };
    if flush_at.is_some() {
        sink.emit(items);
    }
    out
}

/// Background pod sync used when the daemon preference is `auto`: one snapshot (daemon or
/// direct), then a single WATCH from its version with batched re-emits. A `410 Gone` takes a
/// new snapshot (again preferring the daemon); other failures re-watch with backoff.
pub(super) async fn watch_pods_delta(
    client: kube::Client,
    filter_shared: Arc<RwLock<Vec<String>>>,
    seq: Arc<AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
    context: Option<String>,
    slot: SnapshotSlot,
) {
    let sink = PodSink {
        filter_shared,
        seq,
        sender,
        context,
    };
    events::notify_info(
        sink.seq.as_ref(),
        &sink.sender,
        "Background sync: watching pods (snapshot + deltas)...".to_owned(),
    );
    let api: Api<Pod> = Api::all(client.clone());
    let mut retry = RETRY_MIN;
    loop {
        let source = source_for(&slot, sink.context.as_deref());
        let listed = match list_with_daemon::<Pod>(&client, source.as_ref(), "pods").await {
            Ok(l) => l,
            Err(e) => {
                events::notify_error(
                    sink.seq.as_ref(),
                    &sink.sender,
                    format!("Background pod refresh failed (retrying in {retry:?}): {e}"),
                );
                backoff(&mut retry).await;
                continue;
            }
        };
        tracing::debug!(origin = ?listed.origin, items = listed.items.len(), "pod snapshot");
        let mut items: HashMap<String, Pod> = listed
            .items
            .into_iter()
            .map(|p| (p.uid().unwrap_or_default(), p))
            .collect();
        sink.emit(&items);
        let Some(mut rv) = listed.resource_version.filter(|r| !r.is_empty()) else {
            backoff(&mut retry).await;
            continue;
        };
        // Follow from `rv` until the version is gone (then snapshot again).
        loop {
            let relist = match api.watch(&WatchParams::default().timeout(290), &rv).await {
                Ok(stream) => {
                    retry = RETRY_MIN;
                    follow(stream, &mut items, &mut rv, &sink).await
                }
                Err(e) => {
                    tracing::warn!(error = %e, "pod watch failed");
                    Some(is_gone(&e))
                }
            };
            match relist {
                // Server-side timeout: resume from `rv`.
                None => {}
                Some(true) => {
                    backoff(&mut retry).await;
                    break;
                }
                Some(false) => backoff(&mut retry).await,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    fn pod(uid: &str, rv: &str) -> Pod {
        Pod {
            metadata: ObjectMeta {
                uid: Some(uid.into()),
                name: Some(uid.into()),
                resource_version: Some(rv.into()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn apply_event_tracks_items_and_version() {
        let mut items = HashMap::new();
        let mut rv = "1".to_string();
        assert_eq!(
            apply_event(&mut items, &mut rv, WatchEvent::Added(pod("a", "2"))),
            None
        );
        assert_eq!(
            apply_event(&mut items, &mut rv, WatchEvent::Modified(pod("b", "3"))),
            None
        );
        assert_eq!(rv, "3");
        apply_event(&mut items, &mut rv, WatchEvent::Deleted(pod("a", "4")));
        assert_eq!(items.keys().collect::<Vec<_>>(), ["b"]);
        assert_eq!(rv, "4");
        let bookmark: WatchEvent<Pod> = serde_json::from_value(serde_json::json!({
            "type": "BOOKMARK",
            "object": {"kind": "Pod", "apiVersion": "v1", "metadata": {"resourceVersion": "9"}}
        }))
        .unwrap();
        apply_event(&mut items, &mut rv, bookmark);
        assert_eq!(rv, "9");
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn apply_event_gone_means_resnapshot() {
        let mut items: HashMap<String, Pod> = HashMap::new();
        let mut rv = "1".to_string();
        let gone = |code: u16| {
            WatchEvent::<Pod>::Error(
                serde_json::from_value(serde_json::json!({
                    "status": "Failure", "message": "too old", "reason": "Expired", "code": code
                }))
                .unwrap(),
            )
        };
        assert_eq!(apply_event(&mut items, &mut rv, gone(410)), Some(true));
        assert_eq!(apply_event(&mut items, &mut rv, gone(500)), Some(false));
    }
}
