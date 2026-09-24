//! T042: identity is parsed only from the verified front-proxy set.

use axum::http::{HeaderMap, HeaderValue};
use fathom_daemon::identity::{CallerIdentity, identity_from_headers};

fn hm(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
    let mut h = HeaderMap::new();
    for (k, v) in pairs {
        h.append(*k, HeaderValue::from_static(v));
    }
    h
}

#[test]
fn verified_peer_with_user_and_groups() {
    let h = hm(&[
        ("x-remote-user", "alice"),
        ("x-remote-group", "dev"),
        ("x-remote-group", "system:authenticated"),
        ("x-remote-extra-scopes", "a"),
    ]);
    let id = identity_from_headers(&h, true).expect("identity");
    assert_eq!(id.user, "alice");
    assert_eq!(id.groups, vec!["dev", "system:authenticated"]);
    assert_eq!(id.extra.get("scopes").unwrap(), &vec!["a".to_string()]);
}

#[test]
fn unverified_peer_rejects_spoofed_headers() {
    let h = hm(&[
        ("x-remote-user", "admin"),
        ("x-remote-group", "system:masters"),
    ]);
    assert!(identity_from_headers(&h, false).is_none());
}

#[test]
fn missing_or_blank_user_yields_none() {
    assert!(identity_from_headers(&hm(&[]), true).is_none());
    assert!(identity_from_headers(&hm(&[("x-remote-group", "g")]), true).is_none());
    assert!(identity_from_headers(&hm(&[("x-remote-user", "  ")]), true).is_none());
}

#[test]
fn authorization_header_is_never_identity() {
    let h = hm(&[("authorization", "Bearer abc")]);
    assert!(identity_from_headers(&h, true).is_none());
}

#[test]
fn key_distinguishes_users_groups_and_extra_regardless_of_group_order() {
    let mk = |u: &str, g: &[&str]| CallerIdentity {
        user: u.into(),
        groups: g.iter().map(|s| s.to_string()).collect(),
        extra: Default::default(),
    };
    assert_eq!(mk("a", &["x", "y"]).key(), mk("a", &["y", "x"]).key());
    assert_ne!(mk("a", &["x"]).key(), mk("b", &["x"]).key());
    assert_ne!(mk("a", &["x"]).key(), mk("a", &["x", "y"]).key());
    // A user literally named like a joined key cannot collide with user+groups.
    assert_ne!(mk("a\0x", &[]).key(), mk("a", &["x"]).key());
}
