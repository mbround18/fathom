use super::*;
use crate::protocol::{
    DaemonMetricsAvailability, DaemonRetention, DaemonScope, DaemonScopeMode, DerivedCategory,
    DerivedControllerEvent, DerivedOrigin, DerivedSeverity, TopologyHealth, TopologyRelation,
    TruncationReason,
};
use std::sync::atomic::AtomicUsize;

pub(super) fn caps(major: u32, topology: bool) -> DaemonCapabilities {
    DaemonCapabilities {
        protocol_major: major,
        protocol_minor: 0,
        daemon_version: "0.1.0".into(),
        epoch: "e1".into(),
        kinds: vec![],
        scope: DaemonScope {
            mode: DaemonScopeMode::Cluster,
            namespaces: vec![],
        },
        retention: DaemonRetention {
            max_events: 10,
            max_age_secs: 10,
        },
        snapshots: vec![],
        synced: true,
        topology,
        metrics: DaemonMetricsAvailability::Available,
        per_user_filtering: true,
    }
}

pub(crate) fn ev_frame(seq: u64) -> String {
    serde_json::to_string(&DaemonFrame::Event {
        seq,
        time: "2026-01-01T00:00:00Z".into(),
        event: DerivedControllerEvent {
            origin: DerivedOrigin::Derived,
            kind: "Deployment".into(),
            namespace: Some("d".into()),
            name: "web".into(),
            uid: "u".into(),
            category: DerivedCategory::Rollout,
            severity: DerivedSeverity::Info,
            reason: "r".into(),
            message: "m".into(),
            repeat: 1,
        },
    })
    .unwrap()
}

/// Scripted transport: canned GET responses, canned stream lines, request counting.
pub(crate) struct Fake {
    pub caps: Mutex<Result<String, TransportError>>,
    pub topology: Option<String>,
    pub snapshot: Option<Result<String, TransportError>>,
    pub lines: Mutex<Vec<String>>,
    pub hold_open: bool,
    pub requests: AtomicUsize,
    pub paths: Mutex<Vec<String>>,
}

impl Fake {
    pub fn new(caps: Result<String, TransportError>, lines: Vec<String>) -> Arc<Self> {
        Arc::new(Self {
            caps: Mutex::new(caps),
            topology: None,
            snapshot: None,
            lines: Mutex::new(lines),
            hold_open: true,
            requests: AtomicUsize::new(0),
            paths: Mutex::new(vec![]),
        })
    }
}

#[async_trait]
impl DaemonTransport for Fake {
    async fn get_text(&self, path: &str) -> Result<String, TransportError> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.paths.lock().unwrap().push(path.to_owned());
        if path.contains("/v1/snapshot/") {
            return self.snapshot.clone().unwrap_or(Err(TransportError::Http {
                code: 404,
                message: "no".into(),
            }));
        }
        if path.ends_with("/v1/topology") {
            return self.topology.clone().ok_or(TransportError::Http {
                code: 404,
                message: "no".into(),
            });
        }
        self.caps.lock().unwrap().clone()
    }
    async fn open_stream(&self, path: &str) -> Result<LineStream, TransportError> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.paths.lock().unwrap().push(path.to_owned());
        let lines: Vec<_> = self.lines.lock().unwrap().drain(..).map(Ok).collect();
        let s = futures::stream::iter(lines);
        if self.hold_open {
            Ok(s.chain(futures::stream::pending()).boxed())
        } else {
            Ok(s.boxed())
        }
    }
}

fn http(code: u16) -> Result<String, TransportError> {
    Err(TransportError::Http {
        code,
        message: format!("m{code}"),
    })
}

#[test]
fn classification_table() {
    assert_eq!(classify_probe(http(404)).state, DaemonState::NotInstalled);
    assert_eq!(classify_probe(http(503)).state, DaemonState::NotInstalled);
    let na = classify_probe(http(403));
    assert_eq!(na.state, DaemonState::NoAccess);
    assert_eq!(na.detail.as_deref(), Some("m403"));
    assert_eq!(
        classify_probe(Err(TransportError::Timeout)).state,
        DaemonState::Unavailable
    );
    assert_eq!(
        classify_probe(Err(TransportError::Other("refused".into()))).state,
        DaemonState::Unavailable
    );
    let ok = classify_probe(Ok(serde_json::to_string(&caps(1, false)).unwrap()));
    assert_eq!(ok.state, DaemonState::Available);
    assert!(ok.capabilities.is_some());
    let mm = classify_probe(Ok(serde_json::to_string(&caps(2, false)).unwrap()));
    assert_eq!(mm.state, DaemonState::VersionMismatch);
    assert_eq!(
        classify_probe(Ok("nope".into())).state,
        DaemonState::Unavailable
    );
}

#[test]
fn backoff_is_5s_to_5min() {
    assert_eq!(
        backoff_with(0, BACKOFF_BASE, BACKOFF_CAP),
        Duration::from_secs(5)
    );
    assert_eq!(
        backoff_with(1, BACKOFF_BASE, BACKOFF_CAP),
        Duration::from_secs(10)
    );
    assert_eq!(
        backoff_with(5, BACKOFF_BASE, BACKOFF_CAP),
        Duration::from_secs(160)
    );
    assert_eq!(
        backoff_with(6, BACKOFF_BASE, BACKOFF_CAP),
        Duration::from_secs(300)
    );
    assert_eq!(
        backoff_with(50, BACKOFF_BASE, BACKOFF_CAP),
        Duration::from_secs(300)
    );
    assert_eq!(PROBE_TIMEOUT, Duration::from_secs(5));
}

#[test]
fn frames_ignore_unknown_and_garbage() {
    assert!(parse_frame("").is_none());
    assert!(parse_frame("{\"type\":\"future\",\"x\":1}").is_none());
    assert!(parse_frame("not json").is_none());
    assert!(matches!(
        parse_frame(&ev_frame(3)),
        Some(DaemonFrame::Event { seq: 3, .. })
    ));
}

#[test]
fn stream_path_carries_cursor() {
    assert_eq!(stream_path(None), format!("{API_BASE}/v1/stream"));
    let c = EventCursor {
        epoch: "e1".into(),
        seq: 9,
    };
    assert!(stream_path(Some(&c)).ends_with("/v1/stream?epoch=e1&after=9"));
}

fn node(uid: &str) -> TopologyNode {
    TopologyNode {
        uid: uid.into(),
        kind: "Pod".into(),
        namespace: None,
        name: uid.into(),
        health: TopologyHealth::Ok,
    }
}

fn edge(a: &str, b: &str) -> TopologyEdge {
    TopologyEdge {
        from: a.into(),
        to: b.into(),
        relation: TopologyRelation::Owns,
    }
}

#[test]
fn merge_delta_coalesces() {
    let mut acc = TopologyDelta {
        seq: 1,
        added: vec![node("a"), node("b")],
        removed: vec![],
        edges: EdgeDelta {
            added: vec![edge("a", "b")],
            removed: vec![],
        },
    };
    merge_delta(
        &mut acc,
        TopologyDelta {
            seq: 2,
            added: vec![],
            removed: vec!["b".into(), "z".into()],
            edges: EdgeDelta {
                added: vec![],
                removed: vec![edge("a", "b")],
            },
        },
    );
    assert_eq!(acc.seq, 2);
    assert_eq!(acc.added.len(), 1);
    assert_eq!(acc.removed, vec!["b".to_owned(), "z".to_owned()]);
    assert!(acc.edges.added.is_empty() && acc.edges.removed.is_empty());
}

#[test]
fn metrics_parsing_and_truncation_marker() {
    let s = parse_metrics(
        "u",
        MetricKind::Cpu,
        r#"{"samples":[{"time":"2026-01-01T00:00:00Z","value":1.5}],"truncated":"2026-01-01T00:00:00Z"}"#,
    )
    .unwrap();
    assert_eq!(s.samples.len(), 1);
    assert_eq!(
        s.truncated_oldest_time.as_deref(),
        Some("2026-01-01T00:00:00Z")
    );
    let s = parse_metrics("u", MetricKind::Memory, r#"{"samples":[]}"#).unwrap();
    assert!(s.truncated_oldest_time.is_none());
    assert!(parse_metrics("u", MetricKind::Cpu, "{}").is_none());
    let p = metrics_path(
        "u1",
        MetricKind::Memory,
        chrono::Utc::now(),
        chrono::Utc::now(),
    );
    assert!(p.contains("uid=u1&metric=memory&from="));
}

// ---- runner tests (paused time) ------------------------------------------------------

fn session(
    fake: Arc<Fake>,
    cursor: Option<EventCursor>,
    current: u64,
    id: u64,
) -> (Session, crossbeam_channel::Receiver<SystemEvent>) {
    let (tx, rx) = crossbeam_channel::unbounded();
    let emit: Emitter = Arc::new(move |e| {
        let _ = tx.send(e);
    });
    let s = Session {
        context: "ctx".into(),
        id,
        current: Arc::new(AtomicU64::new(current)),
        transport: fake,
        emit,
        save_cursor: Arc::new(|_| {}),
        timeline: Arc::new(Mutex::new(VecDeque::new())),
        cursor,
        timings: Timings::default(),
        caps: caps_cell(),
    };
    (s, rx)
}

fn drain(rx: &crossbeam_channel::Receiver<SystemEvent>) -> Vec<SystemEvent> {
    rx.try_iter().collect()
}

fn states(evs: &[SystemEvent]) -> Vec<DaemonState> {
    evs.iter()
        .filter_map(|e| match e {
            SystemEvent::DaemonStatusUpdated { status, .. } => Some(status.state),
            _ => None,
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn not_installed_backs_off_and_reprobes() {
    let fake = Fake::new(http(404), vec![]);
    let (s, rx) = session(fake.clone(), None, 1, 1);
    let h = tokio::spawn(s.run());
    tokio::time::sleep(Duration::from_secs(6)).await;
    tokio::time::sleep(Duration::from_secs(11)).await;
    h.abort();
    let evs = drain(&rx);
    let st = states(&evs);
    assert_eq!(st[0], DaemonState::Probing);
    assert_eq!(st[1], DaemonState::NotInstalled);
    // 5 s then 10 s waits: three probes within 17 s.
    assert_eq!(fake.requests.load(Ordering::SeqCst), 3);
}

#[tokio::test(start_paused = true)]
async fn probe_times_out_after_5s() {
    struct Hang;
    #[async_trait]
    impl DaemonTransport for Hang {
        async fn get_text(&self, _: &str) -> Result<String, TransportError> {
            futures::future::pending().await
        }
        async fn open_stream(&self, _: &str) -> Result<LineStream, TransportError> {
            futures::future::pending().await
        }
    }
    let (tx, rx) = crossbeam_channel::unbounded();
    let mut s = session(Fake::new(http(404), vec![]), None, 1, 1).0;
    s.transport = Arc::new(Hang);
    s.emit = Arc::new(move |e| {
        let _ = tx.send(e);
    });
    let h = tokio::spawn(s.run());
    tokio::time::sleep(Duration::from_millis(5100)).await;
    h.abort();
    assert!(states(&drain(&rx)).contains(&DaemonState::Unavailable));
}

#[tokio::test(start_paused = true)]
async fn streams_batches_and_resumes_from_cursor() {
    let mut lines = vec![
        serde_json::to_string(&DaemonFrame::Hello {
            capabilities: caps(1, false),
        })
        .unwrap(),
    ];
    for i in 1..=450 {
        lines.push(ev_frame(i));
    }
    let fake = Fake::new(Ok(serde_json::to_string(&caps(1, false)).unwrap()), lines);
    let cursor = Some(EventCursor {
        epoch: "e1".into(),
        seq: 0,
    });
    let (s, rx) = session(fake.clone(), cursor, 1, 1);
    let h = tokio::spawn(s.run());
    tokio::time::sleep(Duration::from_millis(400)).await;
    h.abort();
    let evs = drain(&rx);
    let batches: Vec<usize> = evs
        .iter()
        .filter_map(|e| match e {
            SystemEvent::DaemonEventsBatch {
                events, session, ..
            } => {
                assert_eq!(*session, 1);
                Some(events.len())
            }
            _ => None,
        })
        .collect();
    assert_eq!(batches, vec![200, 200, 50]);
    assert!(
        fake.paths
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.ends_with("/v1/stream?epoch=e1&after=0"))
    );
    assert!(states(&evs).contains(&DaemonState::Available));
}

#[tokio::test(start_paused = true)]
async fn truncation_is_forwarded() {
    let lines = vec![
        serde_json::to_string(&DaemonFrame::Truncated {
            oldest_seq: 10,
            oldest_time: "t".into(),
            dropped: 4,
            reason: TruncationReason::CursorTooOld,
        })
        .unwrap(),
    ];
    let fake = Fake::new(Ok(serde_json::to_string(&caps(1, false)).unwrap()), lines);
    let (s, rx) = session(fake, None, 1, 1);
    let h = tokio::spawn(s.run());
    tokio::time::sleep(Duration::from_millis(300)).await;
    h.abort();
    let got = drain(&rx).into_iter().find_map(|e| match e {
        SystemEvent::DaemonEventsBatch { truncated, .. } => truncated,
        _ => None,
    });
    assert_eq!(got.unwrap().dropped, 4);
}

#[tokio::test(start_paused = true)]
async fn superseded_session_emits_nothing() {
    let fake = Fake::new(
        Ok(serde_json::to_string(&caps(1, false)).unwrap()),
        vec![ev_frame(1)],
    );
    // Session id 1 but the context's current session is already 2.
    let (s, rx) = session(fake.clone(), None, 2, 1);
    let h = tokio::spawn(s.run());
    tokio::time::sleep(Duration::from_secs(1)).await;
    h.abort();
    assert!(drain(&rx).is_empty());
    assert_eq!(fake.requests.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn stream_end_reports_unavailable_then_resumes_after_last_seq() {
    let mut fake = Fake::new(
        Ok(serde_json::to_string(&caps(1, false)).unwrap()),
        vec![ev_frame(7)],
    );
    Arc::get_mut(&mut fake).unwrap().hold_open = false;
    let (s, rx) = session(fake.clone(), None, 1, 1);
    let h = tokio::spawn(s.run());
    tokio::time::sleep(Duration::from_secs(6)).await;
    h.abort();
    let st = states(&drain(&rx));
    assert!(st.contains(&DaemonState::Unavailable));
    assert!(
        fake.paths
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.ends_with("/v1/stream?epoch=e1&after=7"))
    );
}

#[tokio::test(start_paused = true)]
async fn topology_snapshot_loaded_when_advertised() {
    let mut fake = Fake::new(Ok(serde_json::to_string(&caps(1, true)).unwrap()), vec![]);
    Arc::get_mut(&mut fake).unwrap().topology =
        Some(r#"{"seq":4,"nodes":[{"uid":"a","kind":"Pod","namespace":null,"name":"a","health":"ok"}],"edges":[]}"#.into());
    let (s, rx) = session(fake, None, 1, 1);
    let h = tokio::spawn(s.run());
    tokio::time::sleep(Duration::from_millis(100)).await;
    h.abort();
    assert!(drain(&rx).iter().any(|e| matches!(
        e,
        SystemEvent::DaemonTopologyUpdated { delta, .. } if delta.seq == 4 && delta.added.len() == 1
    )));
}

pub(crate) fn caps_json() -> String {
    serde_json::to_string(&caps(1, false)).unwrap()
}

// ---- T033 (client): warm snapshot use and fallback ----

fn snap_body(age_ms: u64, synced: bool, rv: Option<&str>) -> String {
    serde_json::json!({
        "kind": "pods",
        "resourceVersion": rv,
        "ageMs": age_ms,
        "synced": synced,
        "items": [{"metadata": {"name": "p", "namespace": "a", "uid": "u"}}]
    })
    .to_string()
}

fn caps_with_snapshots(kinds: &[&str]) -> DaemonCapabilities {
    let mut c = caps(1, false);
    c.snapshots = kinds.iter().map(|k| k.to_string()).collect();
    c
}

type SnapResult = Result<DaemonSnapshot<k8s_openapi::api::core::v1::Pod>, SnapshotFallback>;

async fn fetch_with(
    probe: ProbeState,
    snapshot: Option<Result<String, TransportError>>,
) -> (SnapResult, Arc<Fake>) {
    let mut fake = Fake::new(Ok(caps_json()), vec![]);
    Arc::get_mut(&mut fake).unwrap().snapshot = snapshot;
    let cell = caps_cell();
    cell.send_replace(probe);
    let r = fetch_snapshot(fake.as_ref(), &cell, "pods", SnapshotBudget::default()).await;
    (r, fake)
}

type Val = serde_json::Value;

#[test]
fn parse_snapshot_threshold_is_30s() {
    let ok = parse_snapshot::<Val>(&snap_body(29_999, true, Some("7")), SNAPSHOT_MAX_AGE).unwrap();
    assert_eq!(ok.resource_version, "7");
    assert_eq!(ok.items.len(), 1);
    assert_eq!(
        parse_snapshot::<Val>(&snap_body(30_000, true, Some("7")), SNAPSHOT_MAX_AGE).unwrap_err(),
        SnapshotFallback::Stale { age_ms: 30_000 }
    );
    assert_eq!(
        parse_snapshot::<Val>(&snap_body(0, false, Some("7")), SNAPSHOT_MAX_AGE).unwrap_err(),
        SnapshotFallback::NotSynced
    );
    assert!(matches!(
        parse_snapshot::<Val>(&snap_body(0, true, None), SNAPSHOT_MAX_AGE),
        Err(SnapshotFallback::Unreadable(_))
    ));
    assert!(matches!(
        parse_snapshot::<Val>("not json", SNAPSHOT_MAX_AGE),
        Err(SnapshotFallback::Unreadable(_))
    ));
}

#[tokio::test]
async fn fetch_snapshot_uses_fresh_snapshot() {
    let (r, fake) = fetch_with(
        ProbeState::Available(caps_with_snapshots(&["pods", "events"])),
        Some(Ok(snap_body(10, true, Some("42")))),
    )
    .await;
    let s = r.unwrap();
    assert_eq!(s.resource_version, "42");
    assert_eq!(s.items[0].metadata.name.as_deref(), Some("p"));
    assert_eq!(
        fake.paths.lock().unwrap().as_slice(),
        ["/apis/daemon.fathom.dev/v1/v1/snapshot/pods"]
    );
}

#[tokio::test]
async fn fetch_snapshot_falls_back() {
    let fresh = || Some(Ok(snap_body(10, true, Some("42"))));
    let avail = || ProbeState::Available(caps_with_snapshots(&["pods"]));
    // Not advertised: no request at all.
    let (r, fake) = fetch_with(ProbeState::Available(caps_with_snapshots(&[])), fresh()).await;
    assert_eq!(r.unwrap_err(), SnapshotFallback::NotOffered);
    assert_eq!(fake.requests.load(Ordering::SeqCst), 0);
    // No daemon.
    let (r, fake) = fetch_with(ProbeState::Unavailable, fresh()).await;
    assert_eq!(r.unwrap_err(), SnapshotFallback::NotOffered);
    assert_eq!(fake.requests.load(Ordering::SeqCst), 0);
    // Stale (>= 30 s) and unsynced snapshots are rejected.
    let (r, _) = fetch_with(avail(), Some(Ok(snap_body(31_000, true, Some("1"))))).await;
    assert_eq!(r.unwrap_err(), SnapshotFallback::Stale { age_ms: 31_000 });
    let (r, _) = fetch_with(
        avail(),
        Some(Err(TransportError::Http {
            code: 503,
            message: "not synced".into(),
        })),
    )
    .await;
    assert_eq!(r.unwrap_err(), SnapshotFallback::NotSynced);
    // HTTP errors and timeouts.
    let (r, _) = fetch_with(
        avail(),
        Some(Err(TransportError::Http {
            code: 403,
            message: "noAccess".into(),
        })),
    )
    .await;
    assert!(matches!(r.unwrap_err(), SnapshotFallback::Http(m) if m.contains("403")));
    let (r, _) = fetch_with(avail(), Some(Err(TransportError::Timeout))).await;
    assert_eq!(r.unwrap_err(), SnapshotFallback::Timeout);
}

#[tokio::test(start_paused = true)]
async fn fetch_snapshot_waits_briefly_for_probe() {
    // Probe never finishes: give up after the probe wait, without a request.
    let (r, fake) = fetch_with(ProbeState::Probing, None).await;
    assert_eq!(r.unwrap_err(), SnapshotFallback::Probing);
    assert_eq!(fake.requests.load(Ordering::SeqCst), 0);

    // Probe finishes within the wait: the snapshot is used.
    let mut fake = Fake::new(Ok(caps_json()), vec![]);
    Arc::get_mut(&mut fake).unwrap().snapshot = Some(Ok(snap_body(0, true, Some("5"))));
    let cell = caps_cell();
    let c2 = cell.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        c2.send_replace(ProbeState::Available(caps_with_snapshots(&["pods"])));
    });
    let r: SnapResult =
        fetch_snapshot(fake.as_ref(), &cell, "pods", SnapshotBudget::default()).await;
    assert_eq!(r.unwrap().resource_version, "5");
}

#[tokio::test(start_paused = true)]
async fn session_publishes_probe_state() {
    let fake = Fake::new(
        Ok(serde_json::to_string(&caps_with_snapshots(&["pods"])).unwrap()),
        vec![],
    );
    let (s, _rx) = session(fake, None, 1, 1);
    let cell = s.caps.clone();
    let h = tokio::spawn(s.run());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(matches!(&*cell.borrow(), ProbeState::Available(c) if c.snapshots == ["pods"]));
    h.abort();

    let fake = Fake::new(
        Err(TransportError::Http {
            code: 404,
            message: "nf".into(),
        }),
        vec![],
    );
    let (s, _rx) = session(fake, None, 1, 1);
    let cell = s.caps.clone();
    let h = tokio::spawn(s.run());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(matches!(&*cell.borrow(), ProbeState::Unavailable));
    h.abort();
}
