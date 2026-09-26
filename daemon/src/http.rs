//! HTTP surface (contracts/daemon-http-api.md). Every data endpoint resolves the caller's
//! [`VisibleScope`] first and filters with it; there is no unfiltered path. Errors are JSON.

use crate::authz::{ScopeResolver, empty_scope, event_visible, namespace_visible};
use crate::eventlog::{Entry, Payload, Replay};
use crate::hub::{Hub, SyncTracker};
use crate::identity::{CallerIdentity, PeerVerified, identity_from_headers};
use crate::metrics_ring::MetricsAvailability;
use crate::series_store::SeriesStore;
use crate::snapshot::{SnapshotError, SnapshotSource};
use axum::body::{Body, Bytes};
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use chrono::{DateTime, Duration as ChronoDuration, SecondsFormat, Utc};
use fathom::protocol::v1::{
    DAEMON_PROTOCOL_MAJOR, DAEMON_PROTOCOL_MINOR, DaemonCapabilities, DaemonFrame,
    DaemonMetricSample, DaemonMetricsAvailability, DaemonRetention, DaemonScope, DaemonScopeMode,
    MetricKind, SeriesKey, TruncationReason, VisibleScope,
};
use serde_json::json;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Semaphore, broadcast, mpsc};
use tokio::time::{MissedTickBehavior, interval_at};
use tokio_stream::wrappers::ReceiverStream;

pub const API_PREFIX: &str = "/apis/daemon.fathom.dev/v1";

#[derive(Clone)]
pub struct HttpConfig {
    pub heartbeat: Duration,
    /// How often an open stream re-resolves its caller's scope.
    pub rescope: Duration,
    pub max_streams: usize,
    pub kinds: Vec<String>,
    pub scope: DaemonScope,
    pub retention: DaemonRetention,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            heartbeat: Duration::from_secs(15),
            rescope: Duration::from_secs(30),
            max_streams: 8,
            kinds: vec![],
            scope: DaemonScope {
                mode: DaemonScopeMode::Cluster,
                namespaces: vec![],
            },
            retention: DaemonRetention {
                max_events: 10_000,
                max_age_secs: 86_400,
            },
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub hub: Arc<Hub>,
    pub resolver: Arc<ScopeResolver>,
    pub series: Arc<dyn SeriesStore>,
    pub metrics: Arc<MetricsAvailability>,
    pub sync: Arc<SyncTracker>,
    pub cfg: HttpConfig,
    permits: Arc<Semaphore>,
    /// Warm snapshot kinds (US4); empty = the endpoint answers 404 for every kind.
    snapshots: Arc<Vec<Arc<dyn SnapshotSource>>>,
    /// Bounds concurrent snapshot renders: each holds a full encoded body (~13 MiB for 5,000
    /// pods), so unbounded fan-in would blow the pod's memory limit.
    renders: Arc<Semaphore>,
}

/// Snapshot renders allowed at once; further callers queue (a render takes tens of ms).
pub const MAX_CONCURRENT_RENDERS: usize = 2;

impl AppState {
    /// Serve `/v1/snapshot/{kind}` for these sources and advertise them in capabilities.
    #[must_use]
    pub fn with_snapshots(mut self, sources: Vec<Arc<dyn SnapshotSource>>) -> Self {
        self.snapshots = Arc::new(sources);
        self
    }

    pub fn new(
        hub: Arc<Hub>,
        resolver: Arc<ScopeResolver>,
        series: Arc<dyn SeriesStore>,
        metrics: Arc<MetricsAvailability>,
        sync: Arc<SyncTracker>,
        cfg: HttpConfig,
    ) -> Self {
        let permits = Arc::new(Semaphore::new(cfg.max_streams));
        Self {
            hub,
            resolver,
            series,
            metrics,
            sync,
            cfg,
            permits,
            snapshots: Arc::default(),
            renders: Arc::new(Semaphore::new(MAX_CONCURRENT_RENDERS)),
        }
    }

    pub fn capabilities(&self) -> DaemonCapabilities {
        DaemonCapabilities {
            protocol_major: DAEMON_PROTOCOL_MAJOR,
            protocol_minor: DAEMON_PROTOCOL_MINOR,
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            epoch: self.hub.epoch(),
            kinds: self.cfg.kinds.clone(),
            scope: self.cfg.scope.clone(),
            retention: self.cfg.retention,
            snapshots: self
                .snapshots
                .iter()
                .map(|s| s.kind().to_string())
                .collect(),
            synced: self.sync.is_synced(),
            topology: true,
            metrics: if self.metrics.get() {
                DaemonMetricsAvailability::Available
            } else {
                DaemonMetricsAvailability::MetricsUnavailable
            },
            per_user_filtering: true,
        }
    }
}

struct ApiError(StatusCode, String);

impl ApiError {
    fn new(code: StatusCode, msg: impl Into<String>) -> Self {
        Self(code, msg.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse_query(q: Option<String>) -> HashMap<String, String> {
    fn pct(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%'
                && i + 2 < b.len()
                && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
            {
                out.push(v);
                i += 3;
                continue;
            }
            out.push(b[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }
    q.unwrap_or_default()
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (pct(k), pct(v)),
            None => (pct(p), String::new()),
        })
        .collect()
}

/// Identity from verified front-proxy headers, else 403 `noAccess` (fail closed).
fn identity(headers: &HeaderMap, peer: Option<PeerVerified>) -> Result<CallerIdentity, ApiError> {
    identity_from_headers(headers, peer.is_some_and(|p| p.0)).ok_or_else(|| {
        ApiError::new(
            StatusCode::FORBIDDEN,
            "noAccess: no verified caller identity (front-proxy client certificate and X-Remote-User required)",
        )
    })
}

async fn authorize(
    st: &AppState,
    headers: &HeaderMap,
    peer: Option<PeerVerified>,
) -> Result<(CallerIdentity, VisibleScope), ApiError> {
    let id = identity(headers, peer)?;
    let universe = st.hub.known_namespaces();
    match st.resolver.resolve(&id, &universe).await {
        Ok(s) => Ok((id, s)),
        Err(e) => Err(ApiError::new(
            StatusCode::FORBIDDEN,
            format!("noAccess: access review failed: {e:#}"),
        )),
    }
}

type Peer = Option<Extension<PeerVerified>>;

fn peer_of(p: Peer) -> Option<PeerVerified> {
    p.map(|Extension(v)| v)
}

async fn capabilities(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Json<DaemonCapabilities>, ApiError> {
    identity(&headers, peer_of(peer))?;
    Ok(Json(st.capabilities()))
}

async fn topology(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (_, scope) = authorize(&st, &headers, peer_of(peer)).await?;
    let (seq, nodes, edges) = st.hub.topology_snapshot(&scope);
    Ok(Json(json!({ "seq": seq, "nodes": nodes, "edges": edges })))
}

async fn metrics(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    RawQuery(q): RawQuery,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (_, scope) = authorize(&st, &headers, peer_of(peer)).await?;
    let q = parse_query(q);
    let bad = |m: &str| ApiError::new(StatusCode::BAD_REQUEST, m.to_string());
    let uid = q
        .get("uid")
        .filter(|u| !u.is_empty())
        .ok_or_else(|| bad("uid is required"))?;
    let metric = match q.get("metric").map(String::as_str) {
        Some("cpu") => MetricKind::Cpu,
        Some("memory") => MetricKind::Memory,
        _ => return Err(bad("metric must be cpu or memory")),
    };
    let time = |k: &str, default: DateTime<Utc>| -> Result<DateTime<Utc>, ApiError> {
        match q.get(k) {
            None => Ok(default),
            Some(v) => DateTime::parse_from_rfc3339(v)
                .map(|t| t.with_timezone(&Utc))
                .map_err(|e| bad(&format!("invalid {k}: {e}"))),
        }
    };
    let now = Utc::now();
    let to = time("to", now)?;
    let from = time("from", to - ChronoDuration::hours(1))?;
    // Unknown and invisible objects are indistinguishable (no existence leak).
    let visible = st
        .hub
        .ns_of(uid)
        .is_some_and(|ns| namespace_visible(&scope, ns.as_deref()));
    if !visible {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "object not found"));
    }
    if !st.metrics.get() {
        return Ok(Json(json!({ "metricsUnavailable": true, "samples": [] })));
    }
    let key = SeriesKey {
        object_uid: uid.clone(),
        metric,
    };
    let r = st.series.query_range(&key, from, to);
    let samples: Vec<DaemonMetricSample> = r
        .samples
        .iter()
        .map(|s| DaemonMetricSample {
            time: ts(s.time),
            value: s.value,
        })
        .collect();
    let mut out = json!({ "metricsUnavailable": false, "samples": samples });
    if let Some(o) = r.truncated_oldest {
        out["truncated"] = json!({ "oldestTime": ts(o) });
    }
    Ok(Json(out))
}

fn frame_for(scope: &VisibleScope, e: &Entry) -> Option<DaemonFrame> {
    match &e.payload {
        Payload::Event(ev) => event_visible(scope, ev).then(|| DaemonFrame::Event {
            seq: e.seq,
            time: ts(e.time),
            event: ev.clone(),
        }),
        Payload::Topology(d) => d.filter(scope, e.seq).map(|t| DaemonFrame::TopologyDelta {
            seq: t.seq,
            added: t.added,
            removed: t.removed,
            edges: t.edges,
        }),
    }
}

type Out = mpsc::Sender<Result<Bytes, Infallible>>;

async fn send(out: &Out, f: &DaemonFrame) -> bool {
    let mut line = match serde_json::to_vec(f) {
        Ok(l) => l,
        Err(_) => return false,
    };
    line.push(b'\n');
    out.send(Ok(Bytes::from(line))).await.is_ok()
}

async fn stream_task(
    st: AppState,
    id: CallerIdentity,
    mut scope: VisibleScope,
    replay: Replay,
    mut rx: broadcast::Receiver<Arc<Entry>>,
    out: Out,
    _permit: tokio::sync::OwnedSemaphorePermit,
) {
    let hello = DaemonFrame::Hello {
        capabilities: st.capabilities(),
    };
    if !send(&out, &hello).await {
        return;
    }
    if let Some(t) = &replay.truncated {
        let f = DaemonFrame::Truncated {
            oldest_seq: t.oldest_seq,
            oldest_time: ts(t.oldest_time),
            dropped: t.dropped,
            reason: t.reason,
        };
        if !send(&out, &f).await {
            return;
        }
    }
    for e in &replay.entries {
        if let Some(f) = frame_for(&scope, e)
            && !send(&out, &f).await
        {
            return;
        }
    }
    let start = tokio::time::Instant::now();
    let mut hb = interval_at(start + st.cfg.heartbeat, st.cfg.heartbeat);
    hb.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut rescope = interval_at(start + st.cfg.rescope, st.cfg.rescope);
    rescope.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            r = rx.recv() => match r {
                Ok(e) => {
                    if let Some(f) = frame_for(&scope, &e)
                        && !send(&out, &f).await { return; }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let _ = send(&out, &DaemonFrame::Bye { reason: "slow consumer".into() }).await;
                    return;
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            _ = hb.tick() => {
                let f = DaemonFrame::Heartbeat { seq: st.hub.last_seq(), time: ts(Utc::now()) };
                if !send(&out, &f).await { return; }
            }
            _ = rescope.tick() => {
                let universe = st.hub.known_namespaces();
                // Fail closed: a failed re-check drops everything visible.
                let new = st.resolver.resolve(&id, &universe).await.unwrap_or_else(|_| empty_scope());
                if new.namespaces != scope.namespaces || new.cluster_scoped != scope.cluster_scoped {
                    scope = new;
                    let f = DaemonFrame::ScopeChanged { namespaces: scope.namespaces.clone() };
                    if !send(&out, &f).await { return; }
                }
            }
        }
    }
}

async fn stream(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    RawQuery(q): RawQuery,
) -> Result<Response, ApiError> {
    let (id, scope) = authorize(&st, &headers, peer_of(peer)).await?;
    let q = parse_query(q);
    let after = match q.get("after") {
        None => None,
        Some(a) => Some(a.parse::<u64>().map_err(|_| {
            ApiError::new(StatusCode::BAD_REQUEST, "after must be an unsigned integer")
        })?),
    };
    let permit =
        st.permits.clone().try_acquire_owned().map_err(|_| {
            ApiError::new(StatusCode::TOO_MANY_REQUESTS, "too many concurrent streams")
        })?;
    let (replay, rx) = st
        .hub
        .subscribe(q.get("epoch").map(String::as_str), after, Utc::now());
    let (tx, body_rx) = mpsc::channel(256);
    tokio::spawn(stream_task(st, id, scope, replay, rx, tx, permit));
    Ok((
        [(header::CONTENT_TYPE, "application/x-ndjson")],
        Body::from_stream(ReceiverStream::new(body_rx)),
    )
        .into_response())
}

/// Warm snapshot (US4). Identity first (fail closed), then the kind, then the caller's scope
/// for exactly that kind, so the result matches what a direct `list` would return them.
async fn snapshot(
    State(st): State<AppState>,
    Path(kind): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    let id = identity(&headers, peer_of(peer))?;
    let Some(src) = st.snapshots.iter().find(|s| s.kind() == kind).cloned() else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("snapshot for {kind:?} is not supported by this daemon"),
        ));
    };
    let mut universe = st.hub.known_namespaces();
    universe.extend(src.namespaces());
    universe.sort();
    universe.dedup();
    let scope = st
        .resolver
        .resolve_for(&id, &universe, src.access())
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::FORBIDDEN,
                format!("noAccess: access review failed: {e:#}"),
            )
        })?;
    // Serializing thousands of objects is CPU work; keep it off the async workers.
    let permit = st
        .renders
        .clone()
        .acquire_owned()
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let body = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        src.render(&scope, Utc::now())
    })
    .await
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    match body {
        Ok(b) => Ok(([(header::CONTENT_TYPE, "application/json")], b).into_response()),
        Err(SnapshotError::NotSynced) => Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            format!("snapshot for {kind:?} is not synced yet"),
        )),
        Err(SnapshotError::Encode(e)) => Err(ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

async fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "not found")
}

async fn method_not_allowed() -> ApiError {
    ApiError::new(StatusCode::METHOD_NOT_ALLOWED, "method not allowed")
}

/// Static aggregated-API discovery; exposes nothing, so it needs no identity. The API server
/// requests it to decide whether the `APIService` is available.
async fn discovery() -> Json<serde_json::Value> {
    Json(json!({
        "kind": "APIResourceList",
        "apiVersion": "v1",
        "groupVersion": "daemon.fathom.dev/v1",
        "resources": [],
    }))
}

fn data_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/capabilities", get(capabilities))
        .route("/v1/stream", get(stream))
        .route("/v1/topology", get(topology))
        .route("/v1/metrics", get(metrics))
        .route("/v1/snapshot/{kind}", get(snapshot))
}

/// The identity-gated API, served both at the root and under the aggregated-API prefix.
pub fn api_router(state: AppState) -> Router {
    let prefixed = format!("{API_PREFIX}/");
    Router::new()
        .merge(data_routes())
        .nest(API_PREFIX, data_routes())
        .route(API_PREFIX, get(discovery))
        .route(&prefixed, get(discovery))
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .with_state(state)
}

/// Kubelet probes on a separate plain-HTTP port; no data.
pub fn health_router(state: AppState) -> Router {
    async fn ready(State(st): State<AppState>) -> Response {
        if st.sync.is_synced() {
            (StatusCode::OK, "ready").into_response()
        } else {
            (StatusCode::SERVICE_UNAVAILABLE, "not synced").into_response()
        }
    }
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(ready))
        .with_state(state)
}

/// Why a replay's `truncated` is worth a log line.
pub fn describe(r: TruncationReason) -> &'static str {
    match r {
        TruncationReason::EpochChanged => "epoch changed",
        TruncationReason::CursorTooOld => "cursor too old",
    }
}
