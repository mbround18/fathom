//! Saving a guided draft (045 US5, T046/T047): the save is what makes anything persistent, and
//! one sign-in is enough.
//!
//! The draft sign-in deliberately writes nothing (see `oidc_draft_signin.rs`), which left the
//! operator with a session that would end at the ID token's `exp`. `adopt_draft` closes that:
//! the refresh token the draft held in memory is committed under the id the save just created,
//! and a refresher is started, so the operator signs in once and stays signed in.
//!
//! `$HOME` is process-global, so the one test here that touches `config.json` owns it for the
//! whole binary; the others are built so they never read it.

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
    FathomRoleMap, OidcConfig, OidcIdentityState, OidcIssuerConfig, OidcSignInMethod, SystemEvent,
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

/// Records every credential write, so "committed under the saved id" is an assertion about the
/// calls made and not only about the values left behind.
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

fn draft_succeeded(e: &SystemEvent) -> bool {
    matches!(e, SystemEvent::OidcDraftSignInSucceeded { .. })
}

/// T046 (FR-028): the save persists the issuer and the role map, commits the draft's refresh
/// token under the now-saved id, and leaves the operator signed in — all from one sign-in.
///
/// Also T047's second half at the end: `save_oidc_config` still refuses to write over a
/// `config.json` it cannot parse.
#[tokio::test]
async fn saving_a_draft_adopts_its_sign_in() {
    let home = tempfile::tempdir().unwrap();
    // SAFETY: this is the only test in this binary that reads `$HOME`.
    unsafe { std::env::set_var("HOME", home.path()) };

    let m = MockOidc::start().await;
    let ev = Events::default();
    let sets = Arc::new(Mutex::new(Vec::new()));
    let store = Arc::new(CredentialStore::new(Box::new(RecordingBackend {
        sets: sets.clone(),
        map: Mutex::default(),
    })));
    let urls = Arc::new(Mutex::new(Vec::new()));
    let s = OidcSessions::new(store.clone(), ev.sink()).with_opener(browser(urls.clone()));

    // One sign-in, against a draft that has never been saved.
    s.test_sign_in(cfg(&m, "draft-whatever"), OidcSignInMethod::Browser);
    ev.wait("draft sign-in", 10, draft_succeeded).await;
    assert!(s.has_draft_session());
    assert!(
        sets.lock().unwrap().is_empty(),
        "the draft wrote a credential before the save: {:?}",
        sets.lock().unwrap()
    );

    // The save: the issuer and the role map in one write, then the adoption.
    let saved = cfg(&m, "corp");
    let role_map = FathomRoleMap {
        admins_group: "platform".to_owned(),
        ..FathomRoleMap::default()
    };
    let config = OidcConfig {
        issuers: vec![saved.clone()],
        contexts: Vec::new(),
        role_map: role_map.clone(),
    };
    save_oidc_config(&config).unwrap();
    s.adopt_draft(&saved).await.unwrap();

    // The issuer and the role map are both on disk.
    let on_disk = load_oidc_config();
    assert_eq!(on_disk.issuers, vec![saved.clone()]);
    assert_eq!(on_disk.role_map, role_map);

    // The refresh token is committed under the *saved* id, and nowhere else: never under the
    // draft key, which no saved id can take.
    assert_eq!(
        sets.lock().unwrap().clone(),
        vec!["fathom.oidc.corp/refresh_token".to_owned()],
        "exactly one credential write, under the saved id"
    );
    assert!(store.get(SecretKind::RefreshToken, "corp").is_some());
    assert!(
        store
            .get(SecretKind::RefreshToken, DRAFT_SESSION_KEY)
            .is_none()
    );

    // The operator is signed in, after exactly one sign-in: no second authorize URL was opened.
    let SystemEvent::OidcIdentityUpdated {
        identity: Some(identity),
        ..
    } = ev
        .wait("the saved issuer is signed in", 10, |e| {
            matches!(e, SystemEvent::OidcIdentityUpdated { issuer_id, identity: Some(i) }
                if issuer_id == "corp" && i.state == OidcIdentityState::SignedIn)
        })
        .await
    else {
        unreachable!()
    };
    assert_eq!(identity.issuer_id, "corp");
    assert_eq!(identity.username, "alice@example.test");
    assert!((s.token_source("corp"))().is_ok());
    assert_eq!(
        urls.lock().unwrap().len(),
        1,
        "the operator signed in twice"
    );

    // The draft is gone: adopted, not duplicated.
    assert!(!s.has_draft_session());
    assert_eq!(
        s.identities()
            .iter()
            .map(|i| i.issuer_id.clone())
            .collect::<Vec<_>>(),
        vec!["corp".to_owned()]
    );

    // No token material leaked into any event.
    assert!(
        !format!("{:?}", ev.snapshot()).contains("eyJ"),
        "a JWT leaked"
    );

    // T047: an unparseable `config.json` is still never written over — the save fails and the
    // operator's file is left exactly as it was for them to fix.
    let path = home.path().join(".fathom").join("config.json");
    std::fs::write(&path, "{ not json").unwrap();
    let err = save_oidc_config(&config).expect_err("an unparseable config.json is not overwritten");
    assert!(
        format!("{err:#}").to_lowercase().contains("config"),
        "{err:#}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
}

/// T047 (FR-007): adopting a sign-in that never happened is refused, and writes nothing.
#[tokio::test]
async fn adopting_without_a_test_sign_in_is_refused() {
    let m = MockOidc::start().await;
    let ev = Events::default();
    let sets = Arc::new(Mutex::new(Vec::new()));
    let store = Arc::new(CredentialStore::new(Box::new(RecordingBackend {
        sets: sets.clone(),
        map: Mutex::default(),
    })));
    let s = OidcSessions::new(store.clone(), ev.sink());

    let err = s
        .adopt_draft(&cfg(&m, "corp"))
        .await
        .expect_err("there is no draft session to adopt");
    assert!(err.to_lowercase().contains("test sign-in"), "{err}");
    assert!(
        sets.lock().unwrap().is_empty(),
        "{:?}",
        sets.lock().unwrap()
    );
    assert!(store.get(SecretKind::RefreshToken, "corp").is_none());
    assert!(s.identities().is_empty());
}

/// An abandoned draft takes its refresh token with it, so a later save cannot adopt it (FR-007).
#[tokio::test]
async fn abandoning_a_draft_drops_its_refresh_token() {
    let m = MockOidc::start().await;
    let ev = Events::default();
    let store = Arc::new(CredentialStore::memory_only());
    let urls = Arc::new(Mutex::new(Vec::new()));
    let s = OidcSessions::new(store.clone(), ev.sink()).with_opener(browser(urls));

    s.test_sign_in(cfg(&m, "draft"), OidcSignInMethod::Browser);
    ev.wait("draft sign-in", 10, draft_succeeded).await;
    assert!(s.has_draft_session());

    s.abandon_draft();
    assert!(!s.has_draft_session());
    assert!(s.adopt_draft(&cfg(&m, "corp")).await.is_err());
    assert!(store.get(SecretKind::RefreshToken, "corp").is_none());
}
