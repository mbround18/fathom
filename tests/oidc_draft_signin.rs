//! Draft sign-in (045 US2, T025/T026): a complete sign-in against an issuer that was never
//! saved. Two guarantees the guided path rests on, both of which the saved sign-in path would
//! break if it were reused as-is:
//!
//! 1. nothing reaches the credential store and nothing reaches `config.json` (FR-007, SC-004);
//! 2. a draft whose id collides with a saved issuer's does not disturb that issuer's session --
//!    `sign_in` cancels any in-flight sign-in keyed on `cfg.id`.
//!
//! `$HOME` is process-global, so the one test that reads `config.json` owns it for its duration.

#[path = "support/mock_oidc.rs"]
mod mock_oidc;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fathom::backend::oidc::flow::Opener;
use fathom::backend::oidc::session::{DRAFT_SESSION_KEY, EventSink, OidcSessions};
use fathom::backend::oidc::store::{BackendError, CredentialStore, SecretBackend, SecretKind};
use fathom::backend::persistence::{load_oidc_config, save_oidc_config};
use fathom::protocol::v1::{
    OidcConfig, OidcIdentityState, OidcIssuerConfig, OidcSignInMethod, SystemEvent,
};
use mock_oidc::{CLIENT_ID, MockOidc};
use serde_json::json;

fn cfg(m: &MockOidc, id: &str) -> OidcIssuerConfig {
    serde_json::from_value(json!({
        "id": id,
        "issuer_url": m.issuer(),
        "client_id": CLIENT_ID,
    }))
    .unwrap()
}

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

/// A credential backend that records every write. "The draft stores no credential" is then an
/// assertion about the calls made, not only about the values left behind.
#[derive(Default)]
struct RecordingBackend {
    sets: Arc<Mutex<Vec<String>>>,
    map: Mutex<HashMap<(String, String), String>>,
}

impl SecretBackend for RecordingBackend {
    fn get(&self, service: &str, user: &str) -> Result<Option<String>, BackendError> {
        Ok(self
            .map
            .lock()
            .unwrap()
            .get(&(service.to_owned(), user.to_owned()))
            .cloned())
    }

    fn set(&self, service: &str, user: &str, value: &str) -> Result<(), BackendError> {
        self.sets.lock().unwrap().push(format!("{service}/{user}"));
        self.map
            .lock()
            .unwrap()
            .insert((service.to_owned(), user.to_owned()), value.to_owned());
        Ok(())
    }

    fn delete(&self, service: &str, user: &str) -> Result<(), BackendError> {
        self.map
            .lock()
            .unwrap()
            .remove(&(service.to_owned(), user.to_owned()));
        Ok(())
    }
}

/// A "browser": follows the authorize redirect to the loopback listener immediately.
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

/// A "browser" that records the authorize URL without following it, so a sign-in stays in
/// flight until the test completes it with [`follow`].
fn deferred_browser(urls: Arc<Mutex<Vec<String>>>) -> Opener {
    Arc::new(move |url: &str| {
        urls.lock().unwrap().push(url.to_owned());
        Ok(())
    })
}

async fn follow(url: &str) {
    let _ = reqwest::get(url).await;
}

async fn nth_url(urls: &Arc<Mutex<Vec<String>>>, n: usize) -> String {
    let start = Instant::now();
    loop {
        if let Some(url) = urls.lock().unwrap().get(n).cloned() {
            return url;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out waiting for authorize URL {n}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn draft_succeeded(e: &SystemEvent) -> bool {
    matches!(e, SystemEvent::OidcDraftSignInSucceeded { .. })
}

/// T025 (FR-007, SC-004): the draft sign-in completes and writes nothing anywhere. Abandoning the
/// guided path after a test sign-in must leave Fathom exactly as it found it.
#[tokio::test]
async fn draft_sign_in_writes_no_credential_and_no_config() {
    let home = tempfile::tempdir().unwrap();
    // SAFETY: this is the only test in this binary that reads `$HOME`.
    unsafe { std::env::set_var("HOME", home.path()) };
    let saved = OidcConfig {
        issuers: vec![cfg(&MockOidc::start().await, "saved")],
        ..OidcConfig::default()
    };
    save_oidc_config(&saved).unwrap();
    let config_path = home.path().join(".fathom").join("config.json");
    let before = std::fs::read(&config_path).unwrap();

    let m = MockOidc::start().await;
    let ev = Events::default();
    let sets = Arc::new(Mutex::new(Vec::new()));
    let store = Arc::new(CredentialStore::new(Box::new(RecordingBackend {
        sets: sets.clone(),
        map: Mutex::default(),
    })));
    let urls = Arc::new(Mutex::new(Vec::new()));
    let s = OidcSessions::new(store.clone(), ev.sink()).with_opener(browser(urls));

    s.test_sign_in(cfg(&m, "draft-id"), OidcSignInMethod::Browser);
    let SystemEvent::OidcDraftSignInSucceeded { result } =
        ev.wait("draft sign-in", 10, draft_succeeded).await
    else {
        unreachable!()
    };

    // The sign-in really happened: it proved an identity.
    assert_eq!(result.username, "alice@example.test");
    assert_eq!(result.groups, vec!["platform".to_owned()]);
    assert!(result.claims.iter().any(|c| c.name == "groups"));
    assert!(result.expires_at > 0);

    // ... and left nothing behind. The keyring was never written, for the draft's own id or any
    // other, and the issuer list on disk is byte-identical.
    assert!(
        sets.lock().unwrap().is_empty(),
        "the draft wrote credentials: {:?}",
        sets.lock().unwrap()
    );
    assert!(store.get(SecretKind::RefreshToken, "draft-id").is_none());
    assert!(
        store
            .get(SecretKind::RefreshToken, DRAFT_SESSION_KEY)
            .is_none()
    );
    assert_eq!(std::fs::read(&config_path).unwrap(), before);
    assert_eq!(load_oidc_config(), saved);

    // No token material in anything the draft emitted.
    let dump = format!("{:?}", ev.snapshot());
    assert!(!dump.contains("eyJ"), "a JWT leaked into events");
}

/// T026: `sign_in` cancels any in-flight sign-in keyed on `cfg.id`, so a draft that reused a
/// saved issuer's id would kill the session the operator is editing from. The draft is tracked
/// under a key no saved issuer id can take.
#[tokio::test]
async fn draft_sign_in_does_not_cancel_a_saved_issuers_sign_in() {
    let m = MockOidc::start().await;
    let ev = Events::default();
    let store = Arc::new(CredentialStore::memory_only());
    let urls = Arc::new(Mutex::new(Vec::new()));
    let s = OidcSessions::new(store.clone(), ev.sink()).with_opener(deferred_browser(urls.clone()));

    // The saved issuer's sign-in is in flight: its browser has been opened and the loopback
    // listener is waiting for the redirect.
    s.sign_in(cfg(&m, "mock"), OidcSignInMethod::Browser);
    let saved_authorize = nth_url(&urls, 0).await;

    // The operator reviews that same issuer in the guided path and runs a test sign-in. Same id,
    // deliberately: this is the collision R2 found.
    s.test_sign_in(cfg(&m, "mock"), OidcSignInMethod::Browser);
    let draft_authorize = nth_url(&urls, 1).await;
    assert_ne!(saved_authorize, draft_authorize, "two separate flows");

    follow(&draft_authorize).await;
    ev.wait("draft sign-in", 10, draft_succeeded).await;

    // The saved issuer's flow was never cancelled: completing its redirect still signs it in.
    follow(&saved_authorize).await;
    let SystemEvent::OidcIdentityUpdated {
        identity: Some(identity),
        ..
    } = ev
        .wait("saved issuer signed in", 10, |e| {
            matches!(e, SystemEvent::OidcIdentityUpdated { issuer_id, identity: Some(i) }
                if issuer_id == "mock" && i.state == OidcIdentityState::SignedIn)
        })
        .await
    else {
        unreachable!()
    };
    assert_eq!(identity.issuer_id, "mock");
    assert!((s.token_source("mock"))().is_ok());
    assert!(
        !ev.snapshot()
            .iter()
            .any(|e| matches!(e, SystemEvent::OidcError { stage, .. } if stage == "cancelled")),
        "the draft cancelled a sign-in"
    );

    // The draft is not an issuer: it never appears in the identity list the UI renders.
    assert_eq!(
        s.identities()
            .iter()
            .map(|i| i.issuer_id.clone())
            .collect::<Vec<_>>(),
        vec!["mock".to_owned()]
    );
}
