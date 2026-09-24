//! T047: SeriesStore contract, ring bound + truncation marker, metrics parsing.

use chrono::{DateTime, Duration, TimeZone, Utc};
use fathom::protocol::v1::{MetricKind, SeriesKey};
use fathom_daemon::metrics_ring::{
    MetricsRing, Reading, parse_cpu_cores, parse_memory_bytes, readings_from_list,
};
use fathom_daemon::series_store::{Sample, SeriesStore};
use serde_json::json;

fn t(s: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + s, 0).unwrap()
}

fn key(uid: &str) -> SeriesKey {
    SeriesKey {
        object_uid: uid.into(),
        metric: MetricKind::Cpu,
    }
}

fn sm(s: i64, v: f64) -> Sample {
    Sample {
        time: t(s),
        value: v,
    }
}

/// The contract every SeriesStore must satisfy.
fn contract(store: &dyn SeriesStore) {
    let k = key("u1");
    let empty = store.query_range(&k, t(0), t(100));
    assert!(empty.samples.is_empty() && empty.truncated_oldest.is_none());
    assert!(store.append(&k, sm(15, 1.0)));
    assert!(store.append(&k, sm(30, 2.0)));
    assert!(store.append(&k, sm(45, 3.0)));
    let r = store.query_range(&k, t(20), t(45));
    assert_eq!(
        r.samples,
        vec![sm(30, 2.0), sm(45, 3.0)],
        "range is inclusive, ordered"
    );
    assert!(r.truncated_oldest.is_none());
    assert!(
        store
            .query_range(&key("other"), t(0), t(100))
            .samples
            .is_empty()
    );
    assert!(store.truncate_marker(&k).is_none());
}

#[test]
fn ring_satisfies_series_store_contract() {
    contract(&MetricsRing::new(240, 1 << 20));
}

#[test]
fn ring_drops_oldest_and_marks_truncation() {
    let r = MetricsRing::new(3, 1 << 20);
    let k = key("u");
    for i in 0..5 {
        r.append(&k, sm(i * 15, i as f64));
    }
    let q = r.query_range(&k, t(0), t(1000));
    assert_eq!(q.samples.len(), 3);
    assert_eq!(q.samples[0], sm(30, 2.0));
    assert_eq!(
        q.truncated_oldest,
        Some(t(30)),
        "query reaches past what the ring kept"
    );
    assert_eq!(r.truncate_marker(&k), Some(t(30)));
    // a query that starts inside the retained window is complete
    assert!(r.query_range(&k, t(30), t(1000)).truncated_oldest.is_none());
}

#[test]
fn total_bytes_are_bounded_new_series_rejected() {
    let r = MetricsRing::new(10, 1000);
    let mut accepted = 0;
    for i in 0..100 {
        if r.append(&key(&format!("u{i}")), sm(0, 1.0)) {
            accepted += 1;
        }
    }
    assert!(accepted > 0 && accepted < 100, "accepted={accepted}");
    assert!(r.approx_bytes() <= 1000 + 200, "bytes={}", r.approx_bytes());
    // existing series still accept samples
    assert!(r.append(&key("u0"), sm(15, 2.0)));
}

#[test]
fn prune_removes_series_of_deleted_objects() {
    let r = MetricsRing::new(10, 1 << 20);
    r.append(&key("a"), sm(0, 1.0));
    r.append(&key("b"), sm(0, 1.0));
    let live: std::collections::HashSet<String> = ["a".to_string()].into();
    r.prune(&live);
    assert_eq!(r.series_count(), 1);
}

#[test]
fn quantity_parsing() {
    assert_eq!(parse_cpu_cores("250m"), Some(0.25));
    assert_eq!(parse_cpu_cores("2"), Some(2.0));
    assert!((parse_cpu_cores("1500000n").unwrap() - 0.0015).abs() < 1e-9);
    assert_eq!(parse_cpu_cores("junk"), None);
    assert_eq!(parse_memory_bytes("1024"), Some(1024.0));
    assert_eq!(parse_memory_bytes("1Ki"), Some(1024.0));
    assert_eq!(parse_memory_bytes("2Mi"), Some(2.0 * 1024.0 * 1024.0));
    assert_eq!(parse_memory_bytes("1G"), Some(1e9));
    assert_eq!(parse_memory_bytes("x"), None);
}

#[test]
fn readings_from_pod_and_node_metrics() {
    let pods = json!({"items":[{
        "metadata":{"name":"p","namespace":"a"},
        "containers":[{"name":"c1","usage":{"cpu":"100m","memory":"1Ki"}},
                      {"name":"c2","usage":{"cpu":"150m","memory":"1Ki"}}]}]});
    let r = readings_from_list(&pods, true);
    assert_eq!(
        r,
        vec![Reading {
            namespace: Some("a".into()),
            name: "p".into(),
            cpu: 0.25,
            memory: 2048.0
        }]
    );
    let nodes = json!({"items":[{"metadata":{"name":"n1"},"usage":{"cpu":"1","memory":"1Mi"}}]});
    let r = readings_from_list(&nodes, false);
    assert_eq!(r[0].namespace, None);
    assert_eq!(r[0].cpu, 1.0);
    assert!(readings_from_list(&json!({}), true).is_empty());
    let _ = Duration::seconds(1);
}
