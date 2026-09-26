//! T014 (+T046/T048 HTTP parts): axum test client against the router with a mock reviewer.

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use axum::Extension;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{Duration as CDur, Utc};
use fathom::protocol::v1::{
    DaemonFrame, DerivedCategory, DerivedControllerEvent, DerivedOrigin, DerivedSeverity,
    MetricKind, SeriesKey, TopologyHealth, TruncationReason,
};
use fathom_daemon::authz::{AccessCheck, AccessReviewer, ScopeResolver};
use fathom_daemon::http::{AppState, HttpConfig, api_router, health_router};
use fathom_daemon::hub::{Hub, SyncTracker};
use fathom_daemon::identity::{CallerIdentity, PeerVerified};
use fathom_daemon::metrics_ring::{MetricsAvailability, MetricsRing};
use fathom_daemon::series_store::{Sample, SeriesStore};
use fathom_daemon::topology::Obj;
use http_body_util::BodyExt;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tower::ServiceExt;

#[derive(Default)]
struct Mock {
    ns: Mutex<HashMap<String, HashSet<String>>>,
    cluster: Mutex<HashSet<String>>,
    fail: AtomicBool,
}

#[async_trait]
impl AccessReviewer for Mock {
    async fn allowed(&self, id: &CallerIdentity, c: &AccessCheck) -> Result<bool> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(anyhow!("sar boom"));
        }
        if c.cluster_scoped {
            return Ok(self.cluster.lock().unwrap().contains(&id.user));
        }
        let m = self.ns.lock().unwrap();
        let s = m.get(&id.user).cloned().unwrap_or_default();
        Ok(match &c.namespace {
            None => s.contains("*"),
            Some(n) => s.contains("*") || s.contains(n),
        })
    }
}

struct Fx {
    state: AppState,
    hub: Arc<Hub>,
    mock: Arc<Mock>,
    ring: Arc<MetricsRing>,
    metrics: Arc<MetricsAvailability>,
    sync: Arc<SyncTracker>,
}

fn fx_with(cfg: HttpConfig, ttl_ms: u64) -> Fx {
    let hub = Arc::new(Hub::new("ep1".into(), 100, CDur::hours(1), 64));
    let mock = Arc::new(Mock::default());
    mock.ns
        .lock()
        .unwrap()
        .insert("alice".into(), ["a".to_string()].into());
    mock.ns
        .lock()
        .unwrap()
        .insert("bob".into(), ["b".to_string()].into());
    let resolver = Arc::new(ScopeResolver::new(
        mock.clone(),
        Duration::from_millis(ttl_ms),
        64,
    ));
    let ring = Arc::new(MetricsRing::new(10, 1 << 20));
    let metrics = Arc::new(MetricsAvailability::default());
    let sync = Arc::new(SyncTracker::new(&[]));
    let state = AppState::new(
        hub.clone(),
        resolver,
        ring.clone(),
        metrics.clone(),
        sync.clone(),
        cfg,
    );
    Fx {
        state,
        hub,
        mock,
        ring,
        metrics,
        sync,
    }
}

fn fx() -> Fx {
    fx_with(HttpConfig::default(), 60_000)
}

fn verified(state: &AppState) -> Router {
    api_router(state.clone()).layer(Extension(PeerVerified(true)))
}

fn get(path: &str, user: Option<&str>) -> Request<Body> {
    let mut r = Request::builder().uri(path);
    if let Some(u) = user {
        r = r.header("x-remote-user", u);
    }
    r.body(Body::empty()).unwrap()
}

async fn json(resp: axum::response::Response) -> (StatusCode, serde_json::Value) {
    let st = resp.status();
    let b = resp.into_body().collect().await.unwrap().to_bytes();
    (
        st,
        serde_json::from_slice(&b).unwrap_or(serde_json::Value::Null),
    )
}

fn ev(ns: &str, n: u32) -> DerivedControllerEvent {
    DerivedControllerEvent {
        origin: DerivedOrigin::Derived,
        kind: "Deployment".into(),
        namespace: Some(ns.into()),
        name: format!("d{n}"),
        uid: format!("u{n}"),
        category: DerivedCategory::Rollout,
        severity: DerivedSeverity::Info,
        reason: "r".into(),
        message: "m".into(),
        repeat: 1,
    }
}

fn now() -> chrono::DateTime<Utc> {
    Utc::now()
}

/// Reads NDJSON frames from a streaming body.
struct Frames {
    body: Body,
    buf: Vec<u8>,
}

impl Frames {
    fn new(resp: axum::response::Response) -> Self {
        Self {
            body: resp.into_body(),
            buf: vec![],
        }
    }

    async fn next(&mut self) -> Option<DaemonFrame> {
        loop {
            if let Some(i) = self.buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.buf.drain(..=i).collect();
                return Some(serde_json::from_slice(&line[..line.len() - 1]).expect("frame json"));
            }
            match tokio::time::timeout(Duration::from_secs(3), self.body.frame()).await {
                Ok(Some(Ok(f))) => {
                    if let Some(d) = f.data_ref() {
                        self.buf.extend_from_slice(d);
                    }
                }
                _ => return None,
            }
        }
    }
}

fn pod(uid: &str, ns: &str, node: Option<&str>) -> Obj {
    Obj {
        uid: uid.into(),
        kind: "Pod".into(),
        namespace: Some(ns.into()),
        name: uid.into(),
        health: TopologyHealth::Ok,
        owners: vec![],
        labels: BTreeMap::new(),
        selector: None,
        node_name: node.map(str::to_string),
        scale_target: None,
    }
}

#[tokio::test]
async fn healthz_and_readyz() {
    let hub = Arc::new(Hub::new("e".into(), 10, CDur::hours(1), 8));
    let mock = Arc::new(Mock::default());
    let sync = Arc::new(SyncTracker::new(&["Pod"]));
    let state = AppState::new(
        hub,
        Arc::new(ScopeResolver::new(mock, Duration::from_secs(1), 8)),
        Arc::new(MetricsRing::new(1, 1000)),
        Arc::new(MetricsAvailability::default()),
        sync.clone(),
        HttpConfig::default(),
    );
    let app = health_router(state);
    let r = app.clone().oneshot(get("/healthz", None)).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let r = app.clone().oneshot(get("/readyz", None)).await.unwrap();
    assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
    sync.mark("Pod");
    let r = app.oneshot(get("/readyz", None)).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
}

#[tokio::test]
async fn capabilities_requires_verified_identity() {
    let f = fx();
    let (st, body) = json(
        verified(&f.state)
            .oneshot(get("/v1/capabilities", Some("alice")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["protocolMajor"], 1);
    assert_eq!(body["epoch"], "ep1");
    assert_eq!(body["perUserFiltering"], true);
    assert_eq!(body["metrics"], "metricsUnavailable");
    assert_eq!(body["topology"], true);

    // verified peer but no X-Remote-User
    let (st, body) = json(
        verified(&f.state)
            .oneshot(get("/v1/capabilities", None))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    assert!(body["error"].as_str().unwrap().starts_with("noAccess"));

    // unverified peer (no PeerVerified extension) with spoofed header and query param
    let unverified = api_router(f.state.clone());
    let (st, _) = json(
        unverified
            .clone()
            .oneshot(get("/v1/capabilities?user=alice", Some("alice")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let unv = api_router(f.state.clone()).layer(Extension(PeerVerified(false)));
    let (st, _) = json(
        unv.oneshot(get("/v1/capabilities", Some("alice")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn every_data_endpoint_fails_closed_without_identity() {
    let f = fx();
    f.hub.append_event(ev("a", 1), now());
    for p in [
        "/v1/stream",
        "/v1/topology",
        "/v1/metrics?uid=x&metric=cpu",
        "/v1/capabilities",
    ] {
        let (st, body) = json(
            api_router(f.state.clone())
                .oneshot(get(p, Some("alice")))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{p}");
        assert!(body["error"].is_string(), "{p}: JSON error");
    }
}

#[tokio::test]
async fn aggregated_prefix_and_discovery() {
    let f = fx();
    let app = verified(&f.state);
    let (st, _) = json(
        app.clone()
            .oneshot(get(
                "/apis/daemon.fathom.dev/v1/v1/capabilities",
                Some("alice"),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    // discovery needs no identity (static, exposes nothing)
    let (st, body) = json(
        api_router(f.state.clone())
            .oneshot(get("/apis/daemon.fathom.dev/v1", None))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["groupVersion"], "daemon.fathom.dev/v1");
}

#[tokio::test]
async fn unknown_paths_and_bad_input_are_json() {
    let f = fx();
    let (st, body) = json(
        verified(&f.state)
            .oneshot(get("/nope", Some("alice")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert!(body["error"].is_string());
    let (st, body) = json(
        verified(&f.state)
            .oneshot(get("/v1/stream?after=abc", Some("alice")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(body["error"].is_string());
    let (st, body) = json(
        verified(&f.state)
            .oneshot(get("/v1/snapshot/pods", Some("alice")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert!(body["error"].is_string());
}

#[tokio::test]
async fn stream_hello_replay_then_live_filtered_per_user() {
    let f = fx();
    f.hub.append_event(ev("a", 1), now());
    f.hub.append_event(ev("b", 2), now());
    f.hub.append_event(ev("a", 3), now());
    let resp = verified(&f.state)
        .oneshot(get("/v1/stream?epoch=ep1&after=0", Some("alice")))
        .await
        .unwrap();
    assert_eq!(resp.headers()["content-type"], "application/x-ndjson");
    let mut fr = Frames::new(resp);
    assert!(matches!(fr.next().await, Some(DaemonFrame::Hello { .. })));
    // replay: alice sees seq 1 and 3 only (ns b filtered), strictly ordered
    let mut seqs = vec![];
    for _ in 0..2 {
        match fr.next().await {
            Some(DaemonFrame::Event { seq, event, .. }) => {
                assert_eq!(event.namespace.as_deref(), Some("a"));
                seqs.push(seq);
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(seqs, vec![1, 3]);
    // live
    f.hub.append_event(ev("b", 4), now());
    f.hub.append_event(ev("a", 5), now());
    match fr.next().await {
        Some(DaemonFrame::Event { seq, .. }) => assert_eq!(seq, 5, "b's live event never leaks"),
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn stream_without_after_is_live_only() {
    let f = fx();
    f.hub.append_event(ev("a", 1), now());
    let mut fr = Frames::new(
        verified(&f.state)
            .oneshot(get("/v1/stream", Some("alice")))
            .await
            .unwrap(),
    );
    assert!(matches!(fr.next().await, Some(DaemonFrame::Hello { .. })));
    f.hub.append_event(ev("a", 2), now());
    match fr.next().await {
        Some(DaemonFrame::Event { seq, .. }) => assert_eq!(seq, 2),
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn stream_truncated_on_epoch_mismatch() {
    let f = fx();
    f.hub.append_event(ev("a", 1), now());
    let mut fr = Frames::new(
        verified(&f.state)
            .oneshot(get("/v1/stream?epoch=old&after=99", Some("alice")))
            .await
            .unwrap(),
    );
    assert!(matches!(fr.next().await, Some(DaemonFrame::Hello { .. })));
    match fr.next().await {
        Some(DaemonFrame::Truncated {
            reason, oldest_seq, ..
        }) => {
            assert_eq!(reason, TruncationReason::EpochChanged);
            assert_eq!(oldest_seq, 1);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn stream_heartbeat() {
    let cfg = HttpConfig {
        heartbeat: Duration::from_millis(40),
        ..HttpConfig::default()
    };
    let f = fx_with(cfg, 60_000);
    let mut fr = Frames::new(
        verified(&f.state)
            .oneshot(get("/v1/stream", Some("alice")))
            .await
            .unwrap(),
    );
    assert!(matches!(fr.next().await, Some(DaemonFrame::Hello { .. })));
    assert!(matches!(
        fr.next().await,
        Some(DaemonFrame::Heartbeat { .. })
    ));
}

#[tokio::test]
async fn stream_cap_returns_429() {
    let cfg = HttpConfig {
        max_streams: 1,
        ..HttpConfig::default()
    };
    let f = fx_with(cfg, 60_000);
    let first = verified(&f.state)
        .oneshot(get("/v1/stream", Some("alice")))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let (st, body) = json(
        verified(&f.state)
            .oneshot(get("/v1/stream", Some("alice")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::TOO_MANY_REQUESTS);
    assert!(body["error"].is_string());
    drop(first);
    // permit is released when the client goes away
    let mut ok = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let r = verified(&f.state)
            .oneshot(get("/v1/stream", Some("alice")))
            .await
            .unwrap();
        if r.status() == StatusCode::OK {
            ok = true;
            break;
        }
    }
    assert!(ok, "permit released after disconnect");
}

#[tokio::test]
async fn sar_error_is_reported_verbatim_and_fails_closed() {
    let f = fx();
    f.mock.fail.store(true, Ordering::SeqCst);
    let (st, body) = json(
        verified(&f.state)
            .oneshot(get("/v1/stream", Some("alice")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    assert!(body["error"].as_str().unwrap().contains("sar boom"));
}

#[tokio::test]
async fn revoked_access_mid_stream_emits_scope_changed_and_stops_data() {
    let cfg = HttpConfig {
        rescope: Duration::from_millis(40),
        ..HttpConfig::default()
    };
    let f = fx_with(cfg, 10);
    f.hub.topology_upsert(pod("pa", "a", None), now());
    let mut fr = Frames::new(
        verified(&f.state)
            .oneshot(get("/v1/stream", Some("alice")))
            .await
            .unwrap(),
    );
    assert!(matches!(fr.next().await, Some(DaemonFrame::Hello { .. })));
    f.mock.ns.lock().unwrap().clear();
    match fr.next().await {
        Some(DaemonFrame::ScopeChanged { namespaces }) => assert!(namespaces.is_empty()),
        other => panic!("unexpected {other:?}"),
    }
    f.hub.append_event(ev("a", 9), now());
    let next = fr.next().await; // heartbeat window is long; expect nothing (timeout => None)
    assert!(!matches!(next, Some(DaemonFrame::Event { .. })), "{next:?}");
}

#[tokio::test]
async fn topology_snapshot_is_filtered_and_deltas_stream() {
    let f = fx();
    f.hub.topology_upsert(pod("pa", "a", None), now());
    f.hub.topology_upsert(pod("pb", "b", None), now());
    let (st, body) = json(
        verified(&f.state)
            .oneshot(get("/v1/topology", Some("alice")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let uids: Vec<&str> = body["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["uid"].as_str().unwrap())
        .collect();
    assert_eq!(uids, vec!["pa"]);
    assert_eq!(body["seq"], 2);

    let mut fr = Frames::new(
        verified(&f.state)
            .oneshot(get("/v1/stream", Some("alice")))
            .await
            .unwrap(),
    );
    assert!(matches!(fr.next().await, Some(DaemonFrame::Hello { .. })));
    f.hub.topology_upsert(pod("pb2", "b", None), now());
    f.hub.topology_upsert(pod("pa2", "a", None), now());
    match fr.next().await {
        Some(DaemonFrame::TopologyDelta { seq, added, .. }) => {
            assert_eq!(seq, 4);
            assert_eq!(added.len(), 1);
            assert_eq!(added[0].uid, "pa2");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn metrics_filtered_per_caller_and_flags() {
    let f = fx();
    f.hub.topology_upsert(pod("pa", "a", None), now());
    let k = SeriesKey {
        object_uid: "pa".into(),
        metric: MetricKind::Cpu,
    };
    f.ring.append(
        &k,
        Sample {
            time: Utc::now() - CDur::minutes(1),
            value: 0.5,
        },
    );

    // metrics API absent => explicit flag, not empty data
    let (st, body) = json(
        verified(&f.state)
            .oneshot(get("/v1/metrics?uid=pa&metric=cpu", Some("alice")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["metricsUnavailable"], true);

    f.metrics.set(true);
    let (_, body) = json(
        verified(&f.state)
            .oneshot(get("/v1/metrics?uid=pa&metric=cpu", Some("alice")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(body["metricsUnavailable"], false);
    assert_eq!(body["samples"].as_array().unwrap().len(), 1);
    assert_eq!(body["samples"][0]["value"], 0.5);

    // bob cannot see ns a: same answer as an unknown object
    let (st, _) = json(
        verified(&f.state)
            .oneshot(get("/v1/metrics?uid=pa&metric=cpu", Some("bob")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = json(
        verified(&f.state)
            .oneshot(get("/v1/metrics?uid=zzz&metric=cpu", Some("alice")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = json(
        verified(&f.state)
            .oneshot(get("/v1/metrics?uid=pa&metric=disk", Some("alice")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let _ = &f.sync;
}

// ---- T033: warm snapshots (US4) ----

use fathom_daemon::authz::{EVENTS_ONLY, PODS_ONLY};
use fathom_daemon::snapshot::{SnapshotSource, SnapshotStore};
use k8s_openapi::api::core::v1::{Event as KEvent, Pod};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

fn spod(ns: &str, name: &str, rv: &str) -> Arc<Pod> {
    Arc::new(Pod {
        metadata: ObjectMeta {
            namespace: Some(ns.into()),
            name: Some(name.into()),
            uid: Some(format!("{ns}/{name}")),
            resource_version: Some(rv.into()),
            ..Default::default()
        },
        ..Default::default()
    })
}

type SnapFx = (Fx, Arc<SnapshotStore<Pod>>, Arc<SnapshotStore<KEvent>>);

fn snap_fx(parts: &[&str]) -> SnapFx {
    let parts: Vec<String> = parts.iter().map(|s| s.to_string()).collect();
    let pods = Arc::new(SnapshotStore::<Pod>::new("pods", &PODS_ONLY, &parts, now()));
    let events = Arc::new(SnapshotStore::<KEvent>::new(
        "events",
        &EVENTS_ONLY,
        &["Event".to_string()],
        now(),
    ));
    let mut f = fx();
    let sources: Vec<Arc<dyn SnapshotSource>> = vec![pods.clone(), events.clone()];
    f.state = f.state.clone().with_snapshots(sources);
    (f, pods, events)
}

async fn snap(f: &Fx, kind: &str, user: Option<&str>) -> (StatusCode, serde_json::Value) {
    json(
        verified(&f.state)
            .oneshot(get(&format!("/v1/snapshot/{kind}"), user))
            .await
            .unwrap(),
    )
    .await
}

fn names(body: &serde_json::Value) -> Vec<String> {
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            format!(
                "{}/{}",
                i["metadata"]["namespace"].as_str().unwrap(),
                i["metadata"]["name"].as_str().unwrap()
            )
        })
        .collect()
}

#[tokio::test]
async fn snapshot_capability_unknown_kind_and_identity() {
    let (f, _, _) = snap_fx(&["Pod"]);
    let (st, caps) = json(
        verified(&f.state)
            .oneshot(get("/v1/capabilities", Some("alice")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(caps["snapshots"], serde_json::json!(["pods", "events"]));
    let (st, body) = snap(&f, "secrets", Some("alice")).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert!(body["error"].is_string());
    let (st, _) = snap(&f, "pods", None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn snapshot_unsynced_is_503_until_every_partition_lists() {
    let (f, pods, _) = snap_fx(&["Pod/a", "Pod/b"]);
    let (st, body) = snap(&f, "pods", Some("alice")).await;
    assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body["error"].as_str().unwrap().contains("not synced"));
    pods.init("Pod/a");
    pods.init_apply("Pod/a", spod("a", "p1", "10"));
    pods.init_done("Pod/a", Some("10".into()));
    let (st, _) = snap(&f, "pods", Some("alice")).await;
    assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE, "Pod/b not listed yet");
    pods.init("Pod/b");
    pods.init_done("Pod/b", Some("12".into()));
    let (st, body) = snap(&f, "pods", Some("alice")).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["synced"], true);
    // Oldest partition version: a watch from it may replay, never skip.
    assert_eq!(body["resourceVersion"], "10");
}

#[tokio::test]
async fn snapshot_synced_is_filtered_sorted_and_tracks_version() {
    let (f, pods, _) = snap_fx(&["Pod"]);
    pods.init("Pod");
    for (ns, n) in [("b", "z"), ("a", "y"), ("a", "x"), ("c", "w")] {
        pods.init_apply("Pod", spod(ns, n, "5"));
    }
    pods.init_done("Pod", Some("100".into()));
    pods.established("Pod");
    let (st, body) = snap(&f, "pods", Some("alice")).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["kind"], "pods");
    assert_eq!(body["ageMs"], 0);
    assert_eq!(body["resourceVersion"], "100");
    assert_eq!(names(&body), ["a/x", "a/y"]);
    let (_, body) = snap(&f, "pods", Some("bob")).await;
    assert_eq!(names(&body), ["b/z"]);
    // A caller with no access gets an empty list, like a direct list would be filtered.
    let (st, body) = snap(&f, "pods", Some("eve")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(names(&body).is_empty());
    // Events, deletes and bookmarks move the version forward, never backwards.
    pods.apply("Pod", spod("a", "v", "101"));
    pods.delete("Pod", &spod("a", "x", "102"));
    pods.bookmark("Pod", "150".into());
    pods.bookmark("Pod", "140".into());
    let (_, body) = snap(&f, "pods", Some("alice")).await;
    assert_eq!(names(&body), ["a/v", "a/y"]);
    assert_eq!(body["resourceVersion"], "150");
    assert_eq!(pods.len(), 4);
}

#[tokio::test]
async fn snapshot_age_grows_while_watch_is_down() {
    let (f, pods, _) = snap_fx(&["Pod"]);
    pods.init("Pod");
    pods.init_done("Pod", Some("1".into()));
    // Listed, but no watch established yet: age counts from store creation.
    let (_, body) = snap(&f, "pods", Some("alice")).await;
    assert!(body["ageMs"].as_u64().unwrap() < 5_000);
    pods.established("Pod");
    let (_, body) = snap(&f, "pods", Some("alice")).await;
    assert_eq!(body["ageMs"], 0);
    pods.lost("Pod", now() - CDur::seconds(45));
    pods.lost("Pod", now()); // the first loss time is kept
    let (st, body) = snap(&f, "pods", Some("alice")).await;
    assert_eq!(st, StatusCode::OK);
    let age = body["ageMs"].as_u64().unwrap();
    assert!((45_000..60_000).contains(&age), "age {age}");
    pods.established("Pod");
    let (_, body) = snap(&f, "pods", Some("alice")).await;
    assert_eq!(body["ageMs"], 0);
}

#[tokio::test]
async fn snapshot_relist_swaps_atomically() {
    let (f, pods, _) = snap_fx(&["Pod"]);
    pods.init("Pod");
    pods.init_apply("Pod", spod("a", "old", "1"));
    pods.init_done("Pod", Some("1".into()));
    pods.init("Pod"); // relist after 410
    pods.init_apply("Pod", spod("a", "new", "9"));
    let (_, body) = snap(&f, "pods", Some("alice")).await;
    assert_eq!(
        names(&body),
        ["a/old"],
        "a half-listed relist is never served"
    );
    pods.init_done("Pod", Some("9".into()));
    let (_, body) = snap(&f, "pods", Some("alice")).await;
    assert_eq!(names(&body), ["a/new"]);
    assert_eq!(body["resourceVersion"], "9");
}

struct PodsOnlyForCarol;

#[async_trait]
impl AccessReviewer for PodsOnlyForCarol {
    async fn allowed(&self, id: &CallerIdentity, c: &AccessCheck) -> Result<bool> {
        Ok(id.user == "carol" && c.resource == "pods")
    }
}

#[tokio::test]
async fn snapshot_access_is_per_kind_and_fails_closed() {
    let (mut f, pods, events) = snap_fx(&["Pod"]);
    f.state.resolver = Arc::new(ScopeResolver::new(
        Arc::new(PodsOnlyForCarol),
        Duration::from_secs(60),
        64,
    ));
    pods.init("Pod");
    pods.init_apply("Pod", spod("a", "p", "1"));
    pods.init_done("Pod", Some("1".into()));
    events.init("Event");
    events.init_done("Event", Some("1".into()));
    // carol may list pods but not the other watched kinds: the pods snapshot still matches
    // what her own direct pod list would return (namespace "a" is only known via the store).
    let (st, body) = snap(&f, "pods", Some("carol")).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(names(&body), ["a/p"]);
    let (st, body) = snap(&f, "events", Some("carol")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(names(&body).is_empty());

    let (f, pods, _) = snap_fx(&["Pod"]);
    pods.init("Pod");
    pods.init_done("Pod", Some("1".into()));
    f.mock.fail.store(true, Ordering::SeqCst);
    let (st, body) = snap(&f, "pods", Some("alice")).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    assert!(body["error"].as_str().unwrap().contains("sar boom"));
}

/// Renders are bounded (memory), not rejected: more concurrent callers than render permits all
/// get the same full body; a second render reuses the learned per-item size.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn snapshot_concurrent_callers_queue_and_all_succeed() {
    let (f, pods, _) = snap_fx(&["Pod"]);
    pods.init("Pod");
    for i in 0..200 {
        pods.init_apply("Pod", spod("a", &format!("p{i:03}"), "5"));
    }
    pods.init_done("Pod", Some("100".into()));
    pods.established("Pod");
    let n = fathom_daemon::http::MAX_CONCURRENT_RENDERS * 4;
    let mut set = tokio::task::JoinSet::new();
    for _ in 0..n {
        let state = f.state.clone();
        set.spawn(async move {
            json(
                verified(&state)
                    .oneshot(get("/v1/snapshot/pods", Some("alice")))
                    .await
                    .unwrap(),
            )
            .await
        });
    }
    let mut bodies = Vec::new();
    while let Some(r) = set.join_next().await {
        let (st, body) = r.unwrap();
        assert_eq!(st, StatusCode::OK);
        bodies.push(body);
    }
    assert_eq!(bodies.len(), n);
    assert_eq!(names(&bodies[0]).len(), 200);
    assert!(bodies.iter().all(|b| b == &bodies[0]));
}
