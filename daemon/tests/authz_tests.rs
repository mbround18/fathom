//! T043: `VisibleScope` via a mocked `SubjectAccessReview`.

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use fathom::protocol::v1::{
    DerivedCategory, DerivedControllerEvent, DerivedOrigin, DerivedSeverity,
};
use fathom_daemon::authz::{
    AccessCheck, AccessReviewer, ScopeResolver, event_visible, namespace_visible,
};
use fathom_daemon::identity::CallerIdentity;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

#[derive(Default)]
struct Mock {
    /// user -> namespaces they may list (all kinds). "*" = cluster-wide namespaced kinds.
    ns: Mutex<HashMap<String, HashSet<String>>>,
    /// users that may list cluster-scoped kinds.
    cluster: Mutex<HashSet<String>>,
    calls: AtomicUsize,
    fail: AtomicBool,
}

#[async_trait]
impl AccessReviewer for Mock {
    async fn allowed(&self, id: &CallerIdentity, check: &AccessCheck) -> Result<bool> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::task::yield_now().await;
        if self.fail.load(Ordering::SeqCst) {
            return Err(anyhow!("sar boom"));
        }
        if check.cluster_scoped {
            return Ok(self.cluster.lock().unwrap().contains(&id.user));
        }
        let ns = self.ns.lock().unwrap();
        let set = ns.get(&id.user).cloned().unwrap_or_default();
        Ok(match &check.namespace {
            None => set.contains("*"),
            Some(n) => set.contains("*") || set.contains(n),
        })
    }
}

fn who(u: &str) -> CallerIdentity {
    CallerIdentity {
        user: u.into(),
        groups: vec![],
        extra: Default::default(),
    }
}

fn universe() -> Vec<String> {
    vec!["a".into(), "b".into(), "c".into()]
}

fn setup(ttl_ms: u64, max: usize) -> (Arc<Mock>, ScopeResolver) {
    let m = Arc::new(Mock::default());
    m.ns.lock()
        .unwrap()
        .insert("alice".into(), ["a".to_string()].into());
    m.ns.lock()
        .unwrap()
        .insert("bob".into(), ["b".to_string(), "c".to_string()].into());
    let r = ScopeResolver::new(m.clone(), Duration::from_millis(ttl_ms), max);
    (m, r)
}

#[tokio::test]
async fn two_users_see_different_namespaces() {
    let (_m, r) = setup(60_000, 64);
    let a = r.resolve(&who("alice"), &universe()).await.unwrap();
    let b = r.resolve(&who("bob"), &universe()).await.unwrap();
    assert_eq!(a.namespaces, vec!["a"]);
    assert_eq!(b.namespaces, vec!["b", "c"]);
    let nobody = r.resolve(&who("eve"), &universe()).await.unwrap();
    assert!(nobody.namespaces.is_empty() && !nobody.cluster_scoped);
}

#[tokio::test(start_paused = true)]
async fn scope_is_cached_for_ttl_then_refreshed() {
    let (m, r) = setup(60_000, 64);
    r.resolve(&who("alice"), &universe()).await.unwrap();
    let first = m.calls.load(Ordering::SeqCst);
    assert!(first > 0);
    r.resolve(&who("alice"), &universe()).await.unwrap();
    assert_eq!(m.calls.load(Ordering::SeqCst), first, "served from cache");
    tokio::time::advance(Duration::from_secs(61)).await;
    r.resolve(&who("alice"), &universe()).await.unwrap();
    assert!(
        m.calls.load(Ordering::SeqCst) > first,
        "re-resolved after ttl"
    );
}

#[tokio::test]
async fn revoked_access_is_seen_after_expiry() {
    let (m, r) = setup(30, 64);
    assert_eq!(
        r.resolve(&who("alice"), &universe())
            .await
            .unwrap()
            .namespaces,
        vec!["a"]
    );
    m.ns.lock().unwrap().clear();
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert!(
        r.resolve(&who("alice"), &universe())
            .await
            .unwrap()
            .namespaces
            .is_empty()
    );
}

#[tokio::test]
async fn error_fails_closed_and_is_not_cached() {
    let (m, r) = setup(60_000, 64);
    m.fail.store(true, Ordering::SeqCst);
    let err = r.resolve(&who("alice"), &universe()).await.unwrap_err();
    assert!(err.to_string().contains("sar boom"), "verbatim: {err}");
    m.fail.store(false, Ordering::SeqCst);
    assert_eq!(
        r.resolve(&who("alice"), &universe())
            .await
            .unwrap()
            .namespaces,
        vec!["a"]
    );
}

#[tokio::test]
async fn cluster_scoped_needs_cluster_list() {
    let (m, r) = setup(60_000, 64);
    assert!(
        !r.resolve(&who("alice"), &universe())
            .await
            .unwrap()
            .cluster_scoped
    );
    m.cluster.lock().unwrap().insert("carol".into());
    let c = r.resolve(&who("carol"), &universe()).await.unwrap();
    assert!(c.cluster_scoped);
    assert!(
        c.namespaces.is_empty(),
        "cluster list does not grant namespaces"
    );
}

#[tokio::test]
async fn cluster_wide_grant_short_circuits_to_whole_universe() {
    let (m, r) = setup(60_000, 64);
    m.ns.lock()
        .unwrap()
        .insert("root".into(), ["*".to_string()].into());
    let s = r.resolve(&who("root"), &universe()).await.unwrap();
    assert_eq!(s.namespaces, universe());
    // 10 namespaced kinds cluster-wide + cluster-scoped kinds, not universe x kinds.
    assert!(m.calls.load(Ordering::SeqCst) <= 12);
}

#[tokio::test]
async fn concurrent_identical_resolutions_coalesce() {
    let (m, r) = setup(60_000, 64);
    let r = Arc::new(r);
    let id = who("alice");
    let u = universe();
    let (x, y) = tokio::join!(r.resolve(&id, &u), r.resolve(&id, &u));
    assert_eq!(x.unwrap(), y.unwrap());
    let single = {
        let (m2, r2) = setup(60_000, 64);
        r2.resolve(&who("alice"), &universe()).await.unwrap();
        m2.calls.load(Ordering::SeqCst)
    };
    assert_eq!(m.calls.load(Ordering::SeqCst), single);
}

#[tokio::test]
async fn cache_is_bounded() {
    let (_m, r) = setup(60_000, 2);
    for u in ["u1", "u2", "u3", "u4", "u5"] {
        r.resolve(&who(u), &universe()).await.unwrap();
    }
    assert!(r.cache_len() <= 2);
}

fn ev(ns: Option<&str>) -> DerivedControllerEvent {
    DerivedControllerEvent {
        origin: DerivedOrigin::Raw,
        kind: "Pod".into(),
        namespace: ns.map(str::to_string),
        name: "x".into(),
        uid: "u".into(),
        category: DerivedCategory::Event,
        severity: DerivedSeverity::Info,
        reason: "r".into(),
        message: "m".into(),
        repeat: 1,
    }
}

#[tokio::test]
async fn filter_rules() {
    let (m, r) = setup(60_000, 64);
    m.cluster.lock().unwrap().insert("alice".into());
    let s = r.resolve(&who("alice"), &universe()).await.unwrap();
    assert!(event_visible(&s, &ev(Some("a"))));
    assert!(!event_visible(&s, &ev(Some("b"))));
    assert!(
        event_visible(&s, &ev(None)),
        "cluster-scoped needs cluster list"
    );
    let b = r.resolve(&who("bob"), &universe()).await.unwrap();
    assert!(!event_visible(&b, &ev(None)));
    assert!(namespace_visible(&b, Some("c")));
    assert!(!namespace_visible(&b, Some("a")));
}
