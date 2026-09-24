//! T009: EventLog append/eviction/replay and Hub fan-out.

use chrono::{Duration, TimeZone, Utc};
use fathom::protocol::v1::{
    DerivedCategory, DerivedControllerEvent, DerivedOrigin, DerivedSeverity, TruncationReason,
};
use fathom_daemon::eventlog::{EventLog, Payload};
use fathom_daemon::hub::Hub;

fn ev(n: u32) -> Payload {
    Payload::Event(DerivedControllerEvent {
        origin: DerivedOrigin::Derived,
        kind: "Deployment".into(),
        namespace: Some("a".into()),
        name: format!("d{n}"),
        uid: format!("u{n}"),
        category: DerivedCategory::Rollout,
        severity: DerivedSeverity::Info,
        reason: "r".into(),
        message: "m".into(),
        repeat: 1,
    })
}

fn t0() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

fn seqs(r: &fathom_daemon::eventlog::Replay) -> Vec<u64> {
    r.entries.iter().map(|e| e.seq).collect()
}

#[test]
fn seq_is_contiguous_from_one() {
    let mut l = EventLog::new("e1".into(), 100, Duration::hours(1));
    for i in 0..5 {
        assert_eq!(l.append(ev(i), t0()).seq, i as u64 + 1);
    }
    assert_eq!(l.oldest_seq(), Some(1));
    assert_eq!(l.last_seq(), 5);
}

#[test]
fn evicts_by_count() {
    let mut l = EventLog::new("e1".into(), 3, Duration::hours(1));
    for i in 0..5 {
        l.append(ev(i), t0());
    }
    assert_eq!(l.oldest_seq(), Some(3));
    assert_eq!(l.len(), 3);
}

#[test]
fn evicts_by_age() {
    let mut l = EventLog::new("e1".into(), 100, Duration::seconds(60));
    l.append(ev(0), t0());
    l.append(ev(1), t0() + Duration::seconds(30));
    l.append(ev(2), t0() + Duration::seconds(100));
    // ev(0) is 100s old, ev(1) is 70s old, both past 60s
    assert_eq!(l.oldest_seq(), Some(3));
    l.evict(t0() + Duration::seconds(200));
    assert_eq!(l.len(), 0);
    assert_eq!(l.last_seq(), 3, "seq keeps counting after eviction");
}

#[test]
fn replay_strictly_after_cursor() {
    let mut l = EventLog::new("e1".into(), 100, Duration::hours(1));
    for i in 0..5 {
        l.append(ev(i), t0());
    }
    let r = l.replay(Some("e1"), Some(2), t0());
    assert!(r.truncated.is_none());
    assert_eq!(seqs(&r), vec![3, 4, 5]);
    let r = l.replay(Some("e1"), Some(5), t0());
    assert!(r.truncated.is_none() && r.entries.is_empty());
}

#[test]
fn no_cursor_is_live_only() {
    let mut l = EventLog::new("e1".into(), 100, Duration::hours(1));
    l.append(ev(0), t0());
    let r = l.replay(None, None, t0());
    assert!(r.truncated.is_none() && r.entries.is_empty());
}

#[test]
fn epoch_mismatch_is_truncated_then_retained() {
    let mut l = EventLog::new("e2".into(), 100, Duration::hours(1));
    l.append(ev(0), t0());
    let r = l.replay(Some("old"), Some(40), t0());
    let t = r.truncated.clone().expect("truncated");
    assert_eq!(t.reason, TruncationReason::EpochChanged);
    assert_eq!(t.oldest_seq, 1);
    assert_eq!(seqs(&r), vec![1]);
}

#[test]
fn cursor_too_old_is_truncated_with_dropped_estimate() {
    let mut l = EventLog::new("e1".into(), 3, Duration::hours(1));
    for i in 0..10 {
        l.append(ev(i), t0());
    }
    // retained 8,9,10; cursor 2 -> events 3..7 lost
    let r = l.replay(Some("e1"), Some(2), t0());
    let t = r.truncated.clone().unwrap();
    assert_eq!(t.reason, TruncationReason::CursorTooOld);
    assert_eq!(t.oldest_seq, 8);
    assert_eq!(t.dropped, 5);
    assert_eq!(seqs(&r), vec![8, 9, 10]);
    // cursor exactly oldest-1 is fine
    let r = l.replay(Some("e1"), Some(7), t0());
    assert!(r.truncated.is_none());
    assert_eq!(seqs(&r), vec![8, 9, 10]);
}

#[test]
fn cursor_from_the_future_is_truncated() {
    let mut l = EventLog::new("e1".into(), 3, Duration::hours(1));
    l.append(ev(0), t0());
    let r = l.replay(Some("e1"), Some(99), t0());
    assert_eq!(
        r.truncated.clone().unwrap().reason,
        TruncationReason::CursorTooOld
    );
}

#[tokio::test]
async fn hub_subscribe_has_no_gap_or_duplicate() {
    let hub = Hub::new("e1".into(), 100, Duration::hours(1), 16);
    hub.append_event(ev_inner(0), t0());
    hub.append_event(ev_inner(1), t0());
    let (replay, mut rx) = hub.subscribe(Some("e1"), Some(1), t0());
    assert_eq!(seqs(&replay), vec![2]);
    hub.append_event(ev_inner(2), t0());
    let live = rx.recv().await.unwrap();
    assert_eq!(live.seq, 3);
}

#[tokio::test]
async fn slow_subscriber_lags_instead_of_blocking() {
    let hub = Hub::new("e1".into(), 100, Duration::hours(1), 2);
    let (_r, mut rx) = hub.subscribe(None, None, t0());
    for i in 0..10 {
        hub.append_event(ev_inner(i), t0());
    }
    let mut lagged = false;
    for _ in 0..12 {
        match rx.try_recv() {
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                lagged = true;
                break;
            }
            Err(_) => break,
            Ok(_) => {}
        }
    }
    assert!(lagged);
}

fn ev_inner(n: u32) -> DerivedControllerEvent {
    match ev(n) {
        Payload::Event(e) => e,
        _ => unreachable!(),
    }
}
