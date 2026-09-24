//! US1 (033 T011, T013, T014, T018): discovery, PKCE flow, device-code fallback, ID token
//! validation failures, refresh and session end, against the mock OIDC provider.

#[path = "support/mock_oidc.rs"]
mod mock_oidc;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fathom::backend::oidc::flow::{self, Opener};
use fathom::backend::oidc::session::{EventSink, OidcSessions};
use fathom::backend::oidc::store::{CredentialStore, SecretKind};
use fathom::backend::oidc::validate::Validator;
use fathom::protocol::v1::{
    OidcIdentityState, OidcIssuerConfig, OidcSignInMethod, OidcSignInStage, SystemEvent,
};
use mock_oidc::{CLIENT_ID, MockOidc, MockUser};
use serde_json::json;

fn cfg(m: &MockOidc) -> OidcIssuerConfig {
    serde_json::from_value(json!({
        "id": "mock",
        "issuer_url": m.issuer(),
        "client_id": CLIENT_ID,
    }))
    .unwrap()
}

async fn validator(m: &MockOidc) -> Validator {
    let http = flow::http_client(&cfg(m)).unwrap();
    let disc = flow::discover(&http, &m.issuer()).await.unwrap();
    Validator::new(
        http,
        disc.jwks_uri,
        &disc.issuer,
        CLIENT_ID,
        "preferred_username",
        "groups",
    )
    .await
    .unwrap()
}

// ---- validation (T011) --------------------------------------------------------------------

#[tokio::test]
async fn valid_token_yields_claims_and_drops_empty_groups() {
    let m = MockOidc::start().await;
    let v = validator(&m).await;
    let jwt = m.mint_id_token(
        CLIENT_ID,
        Some("n1"),
        json!({"preferred_username": "alice", "groups": ["fathom-viewers", ""]}),
    );
    let c = v.validate(&jwt, Some("n1")).await.unwrap();
    assert_eq!(c.username, "alice");
    assert_eq!(c.groups, vec!["fathom-viewers".to_owned()]);
    assert_eq!(c.email.as_deref(), Some("alice@example.test"));
    // Refresh path: no nonce expectation.
    v.validate(&jwt, None).await.unwrap();
}

#[tokio::test]
async fn validation_failures_are_specific() {
    let m = MockOidc::start().await;
    let v = validator(&m).await;
    let now = chrono::Utc::now().timestamp();

    let good = m.mint_id_token(CLIENT_ID, Some("n"), json!({}));
    let mut parts: Vec<&str> = good.split('.').collect();
    let flipped = if parts[2].starts_with('A') { 'B' } else { 'A' };
    let tampered_sig = format!("{flipped}{}", &parts[2][1..]);
    parts[2] = &tampered_sig;
    let bad_sig = parts.join(".");
    let e = v.validate(&bad_sig, Some("n")).await.unwrap_err();
    assert!(e.0.contains("signature"), "{e}");

    let jwt = m.mint_id_token(CLIENT_ID, Some("n"), json!({"iss": "https://evil.example"}));
    let e = v.validate(&jwt, Some("n")).await.unwrap_err();
    assert!(e.0.contains("issuer"), "{e}");

    let jwt = m.mint_id_token("someone-else", Some("n"), json!({}));
    let e = v.validate(&jwt, Some("n")).await.unwrap_err();
    assert!(e.0.contains("audience"), "{e}");

    let jwt = m.mint_id_token(
        CLIENT_ID,
        Some("n"),
        json!({"iat": now - 7200, "exp": now - 3600}),
    );
    let e = v.validate(&jwt, Some("n")).await.unwrap_err();
    assert!(e.0.contains("expired") && e.0.contains("clock"), "{e}");

    let jwt = m.mint_id_token(CLIENT_ID, Some("other"), json!({}));
    let e = v.validate(&jwt, Some("n")).await.unwrap_err();
    assert!(e.0.contains("nonce"), "{e}");

    // Skew: token minted by an IdP whose clock is an hour ahead.
    let jwt = m.mint_id_token(
        CLIENT_ID,
        Some("n"),
        json!({"iat": now + 3600, "exp": now + 3660}),
    );
    let e = v.validate(&jwt, Some("n")).await.unwrap_err();
    assert!(e.0.contains("clock skew"), "{e}");

    // Small skew is tolerated.
    let jwt = m.mint_id_token(
        CLIENT_ID,
        Some("n"),
        json!({"iat": now + 20, "exp": now + 80}),
    );
    v.validate(&jwt, Some("n")).await.unwrap();
}

#[tokio::test]
async fn jwks_rotation_refetches_once() {
    let m = MockOidc::start().await;
    let v = validator(&m).await;
    m.rotate_key();
    m.retire_old_keys();
    let jwt = m.mint_id_token(CLIENT_ID, None, json!({}));
    v.validate(&jwt, None).await.expect("refetched JWKS");
}

// ---- flows (T011/T013) --------------------------------------------------------------------

#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<SystemEvent>>>);

impl Events {
    fn sink(&self) -> EventSink {
        let v = self.0.clone();
        Arc::new(move |e| v.lock().unwrap().push(e))
    }

    fn snapshot(&self) -> Vec<SystemEvent> {
        self.0.lock().unwrap().clone()
    }

    async fn wait(
        &self,
        what: &str,
        secs: u64,
        pred: impl Fn(&SystemEvent) -> bool,
    ) -> SystemEvent {
        let start = Instant::now();
        loop {
            if let Some(e) = self.snapshot().into_iter().find(|e| pred(e)) {
                return e;
            }
            assert!(
                start.elapsed() < Duration::from_secs(secs),
                "timed out waiting for {what}; saw {:?}",
                self.snapshot()
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

/// A "browser": follows the authorize redirect to the loopback listener.
fn browser(urls: Arc<Mutex<Vec<String>>>) -> Opener {
    Arc::new(move |url: &str| {
        urls.lock().unwrap().push(url.to_owned());
        let url = url.to_owned();
        tokio::spawn(async move {
            let _ = reqwest::get(url).await;
        });
        Ok(())
    })
}

fn sessions(events: &Events, opener: Opener) -> (OidcSessions, Arc<CredentialStore>) {
    let store = Arc::new(CredentialStore::memory_only());
    let s = OidcSessions::new(store.clone(), events.sink()).with_opener(opener);
    (s, store)
}

fn signed_in(e: &SystemEvent) -> bool {
    matches!(e, SystemEvent::OidcIdentityUpdated { identity: Some(i), .. } if i.state == OidcIdentityState::SignedIn)
}

#[tokio::test]
async fn browser_sign_in_uses_pkce_and_publishes_identity() {
    let m = MockOidc::start().await;
    let ev = Events::default();
    let urls = Arc::new(Mutex::new(Vec::new()));
    let (s, store) = sessions(&ev, browser(urls.clone()));
    s.sign_in(cfg(&m), OidcSignInMethod::Browser);

    let SystemEvent::OidcIdentityUpdated {
        identity: Some(id), ..
    } = ev.wait("sign in", 10, signed_in).await
    else {
        unreachable!()
    };
    assert_eq!(id.sub, "alice-sub");
    assert_eq!(id.groups, vec!["platform".to_owned()]);
    ev.wait("done", 5, |e| {
        matches!(
            e,
            SystemEvent::OidcSignInProgress {
                stage: OidcSignInStage::Done,
                ..
            }
        )
    })
    .await;

    let url = urls.lock().unwrap()[0].clone();
    assert!(
        url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A"),
        "{url}"
    );
    assert!(url.contains("code_challenge_method=S256"), "{url}");
    let req = &m.token_requests()[0];
    assert_eq!(req["grant_type"], "authorization_code");
    assert!(req.contains_key("code_verifier"));
    assert!(!req.contains_key("client_secret"));

    assert!((s.token_source("mock"))().is_ok());
    assert!(store.get(SecretKind::RefreshToken, "mock").is_some());
    // No token in any emitted event.
    let dump = format!("{:?}", ev.snapshot());
    assert!(!dump.contains("eyJ"), "a JWT leaked into events");
}

#[tokio::test]
async fn browser_sign_in_can_be_cancelled() {
    let m = MockOidc::start().await;
    let ev = Events::default();
    let idle: Opener = Arc::new(|_| Ok(()));
    let (s, _) = sessions(&ev, idle);
    s.sign_in(cfg(&m), OidcSignInMethod::Browser);
    ev.wait("awaiting", 5, |e| {
        matches!(
            e,
            SystemEvent::OidcSignInProgress {
                stage: OidcSignInStage::Awaiting,
                ..
            }
        )
    })
    .await;
    s.cancel_sign_in("mock");
    ev.wait(
        "cancelled",
        5,
        |e| matches!(e, SystemEvent::OidcError { stage, .. } if stage == "cancelled"),
    )
    .await;
    assert!((s.token_source("mock"))().is_err());
}

#[tokio::test]
async fn browser_sign_in_times_out() {
    let m = MockOidc::start().await;
    let ev = Events::default();
    let idle: Opener = Arc::new(|_| Ok(()));
    let store = Arc::new(CredentialStore::memory_only());
    let s = OidcSessions::new(store, ev.sink())
        .with_opener(idle)
        .with_browser_timeout(Duration::from_millis(300));
    s.sign_in(cfg(&m), OidcSignInMethod::Browser);
    ev.wait("timeout", 5, |e| {
        matches!(e, SystemEvent::OidcError { stage, message, .. } if stage == "authorize" && message.contains("timed out"))
    })
    .await;
}

#[tokio::test]
async fn state_mismatch_is_ignored_and_real_redirect_still_works() {
    let m = MockOidc::start().await;
    let ev = Events::default();
    let opener: Opener = Arc::new(move |url: &str| {
        let url = url.to_owned();
        tokio::spawn(async move {
            // Forged callback first, then the genuine authorize round trip.
            let redirect = url
                .split("redirect_uri=")
                .nth(1)
                .and_then(|r| r.split('&').next())
                .map(|r| urlencoding::decode(r).unwrap().into_owned())
                .unwrap();
            let _ = reqwest::get(format!("{redirect}?code=evil&state=wrong")).await;
            let _ = reqwest::get(url).await;
        });
        Ok(())
    });
    let (s, _) = sessions(&ev, opener);
    s.sign_in(cfg(&m), OidcSignInMethod::Browser);
    ev.wait("sign in", 10, signed_in).await;
}

#[tokio::test]
async fn device_code_flow_shows_code_then_signs_in() {
    let m = MockOidc::start().await;
    let ev = Events::default();
    let (s, _) = sessions(&ev, Arc::new(|_| Ok(())));
    s.sign_in(cfg(&m), OidcSignInMethod::DeviceCode);
    let SystemEvent::OidcSignInProgress {
        stage: OidcSignInStage::DeviceCode { user_code, .. },
        ..
    } = ev
        .wait("device code", 5, |e| {
            matches!(
                e,
                SystemEvent::OidcSignInProgress {
                    stage: OidcSignInStage::DeviceCode { .. },
                    ..
                }
            )
        })
        .await
    else {
        unreachable!()
    };
    assert_eq!(user_code, "ABCD-EFGH");
    // The human approves once the app has polled at least once.
    let start = Instant::now();
    let device_code = loop {
        if let Some(d) = m
            .token_requests()
            .iter()
            .find_map(|r| r.get("device_code").cloned())
        {
            break d;
        }
        assert!(start.elapsed() < Duration::from_secs(8), "no poll seen");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    m.approve_device(&device_code);
    ev.wait("sign in", 10, signed_in).await;
}

#[tokio::test]
async fn unopenable_browser_falls_back_to_device_code() {
    let m = MockOidc::start().await;
    let ev = Events::default();
    let broken: Opener = Arc::new(|_| Err("no browser on this host".into()));
    let (s, _) = sessions(&ev, broken);
    s.sign_in(cfg(&m), OidcSignInMethod::Browser);
    ev.wait("device fallback", 5, |e| {
        matches!(
            e,
            SystemEvent::OidcSignInProgress {
                stage: OidcSignInStage::DeviceCode { .. },
                ..
            }
        )
    })
    .await;
}

#[tokio::test]
async fn rejected_id_token_reports_validate_stage() {
    let m = MockOidc::start().await;
    m.set_user(MockUser::alice());
    let mut c = cfg(&m);
    c.client_id = "some-other-client".into(); // aud in minted tokens is fathom-desktop
    let ev = Events::default();
    let urls = Arc::new(Mutex::new(Vec::new()));
    let (s, _) = sessions(&ev, browser(urls));
    s.sign_in(c, OidcSignInMethod::Browser);
    ev.wait("validate error", 10, |e| {
        matches!(e, SystemEvent::OidcError { stage, message, .. } if stage == "validate" && message.contains("audience"))
    })
    .await;
    assert!((s.token_source("mock"))().is_err());
}

// ---- session (T014) -----------------------------------------------------------------------

#[tokio::test]
async fn refresh_runs_before_expiry_and_rotates_the_refresh_token() {
    let m = MockOidc::start().await;
    m.set_token_ttl(4);
    let ev = Events::default();
    let urls = Arc::new(Mutex::new(Vec::new()));
    let store = Arc::new(CredentialStore::memory_only());
    let s = OidcSessions::new(store.clone(), ev.sink())
        .with_opener(browser(urls))
        .with_refresh_lead(Duration::from_secs(2));
    s.sign_in(cfg(&m), OidcSignInMethod::Browser);
    ev.wait("sign in", 10, signed_in).await;
    let first = (s.token_source("mock"))().unwrap();
    let first_refresh = store.get(SecretKind::RefreshToken, "mock").unwrap();

    ev.wait("refreshing", 10, |e| {
        matches!(e, SystemEvent::OidcIdentityUpdated { identity: Some(i), .. } if i.state == OidcIdentityState::Refreshing)
    })
    .await;
    let start = Instant::now();
    loop {
        if (s.token_source("mock"))().unwrap() != first {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "token never refreshed"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let grants: Vec<_> = m
        .token_requests()
        .iter()
        .map(|r| r["grant_type"].clone())
        .collect();
    assert!(grants.iter().any(|g| g == "refresh_token"), "{grants:?}");
    let latest = store.get(SecretKind::RefreshToken, "mock").unwrap();
    assert_ne!(
        latest.expose(),
        first_refresh.expose(),
        "rotated token persisted"
    );
    assert!(
        !ev.snapshot()
            .iter()
            .any(|e| matches!(e, SystemEvent::OidcSessionEnded { .. }))
    );
}

#[tokio::test]
async fn failed_refresh_ends_the_session_with_the_verbatim_reason() {
    let m = MockOidc::start().await;
    m.set_token_ttl(3);
    let ev = Events::default();
    let urls = Arc::new(Mutex::new(Vec::new()));
    let store = Arc::new(CredentialStore::memory_only());
    let s = OidcSessions::new(store.clone(), ev.sink())
        .with_opener(browser(urls))
        .with_refresh_lead(Duration::from_secs(2));
    s.sign_in(cfg(&m), OidcSignInMethod::Browser);
    ev.wait("sign in", 10, signed_in).await;
    m.fail_token(400, "invalid_grant");
    let SystemEvent::OidcSessionEnded { reason, .. } = ev
        .wait("session ended", 10, |e| {
            matches!(e, SystemEvent::OidcSessionEnded { .. })
        })
        .await
    else {
        unreachable!()
    };
    assert!(reason.contains("invalid_grant"), "{reason}");
    let err = (s.token_source("mock"))().unwrap_err();
    assert!(err.contains("sign in again"), "{err}");
    assert!(store.get(SecretKind::RefreshToken, "mock").is_none());
}

#[tokio::test]
async fn sign_out_clears_tokens_and_identity() {
    let m = MockOidc::start().await;
    let ev = Events::default();
    let urls = Arc::new(Mutex::new(Vec::new()));
    let (s, store) = sessions(&ev, browser(urls));
    s.sign_in(cfg(&m), OidcSignInMethod::Browser);
    ev.wait("sign in", 10, signed_in).await;
    s.sign_out(&cfg(&m), false).await;
    assert!(s.identities().is_empty());
    assert!((s.token_source("mock"))().is_err());
    assert!(store.get(SecretKind::RefreshToken, "mock").is_none());
    assert!(matches!(
        ev.snapshot().last(),
        Some(SystemEvent::OidcIdentityUpdated { identity: None, .. })
    ));
}

#[tokio::test]
async fn restore_resumes_from_the_stored_refresh_token() {
    let m = MockOidc::start().await;
    let ev = Events::default();
    let urls = Arc::new(Mutex::new(Vec::new()));
    let (s, store) = sessions(&ev, browser(urls));
    s.sign_in(cfg(&m), OidcSignInMethod::Browser);
    ev.wait("sign in", 10, signed_in).await;

    let ev2 = Events::default();
    let s2 = OidcSessions::new(store, ev2.sink());
    s2.restore(&cfg(&m)).await;
    assert_eq!(s2.identities().len(), 1);
    assert!((s2.token_source("mock"))().is_ok());
}

// ---- private CA and errors (T018) ---------------------------------------------------------

#[test]
fn ca_bundle_errors_name_the_path() {
    let mut c: OidcIssuerConfig = serde_json::from_value(json!({
        "id": "x", "issuer_url": "https://i", "client_id": "c",
        "ca_bundle_path": "/nonexistent/ca.pem"
    }))
    .unwrap();
    let e = flow::http_client(&c).unwrap_err();
    assert!(e.contains("/nonexistent/ca.pem"), "{e}");

    let dir = std::env::temp_dir().join(format!("fathom-ca-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("bad.pem");
    std::fs::write(&bad, "not a certificate").unwrap();
    c.ca_bundle_path = Some(bad.to_string_lossy().into_owned());
    let e = flow::http_client(&c).unwrap_err();
    assert!(e.contains("bad.pem"), "{e}");
    c.ca_bundle_path = None;
    assert!(flow::http_client(&c).is_ok());
}

#[tokio::test]
async fn discovery_rejects_a_mismatched_issuer() {
    let m = MockOidc::start().await;
    let http = flow::http_client(&cfg(&m)).unwrap();
    // The document says issuer=<mock uri>; ask for a different (but reachable) spelling.
    let e = flow::discover(&http, &format!("{}/", m.issuer()))
        .await
        .map(|_| ());
    assert!(e.is_ok(), "trailing slash is tolerated");
    let mut c = cfg(&m);
    c.issuer_url = "http://127.0.0.1:1".into();
    let e = flow::discover(&http, &c.issuer_url).await.unwrap_err();
    assert!(e.to_string().contains("discovery"), "{e}");
}
