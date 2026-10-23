//! `IdentityProvider` conformance (spec 050, T016): every provider must pass [`conformance`]
//! against an in-process fake of its IdP. No test reaches the network.
#![allow(dead_code)]

use std::collections::HashMap;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use secrecy::ExposeSecret;
use sha2::{Digest, Sha256};
use url::Url;
use web_auth::provider::{AuthRedirect, IdentityProvider, ProviderError, Revalidation};
use web_auth::session::{SessionConfig, SessionStore};

/// How a test drives the provider's IdP: accept `code` for the login `auth` started, change the
/// person's groups, or make the next refresh fail.
pub trait Driver {
    fn grant(&self, code: &str, auth: &AuthRedirect);
    fn set_groups(&self, groups: &[&str]);
    fn reject_refresh(&self);
    /// `redirect_uri` sent with each code exchange.
    fn redirects(&self) -> Vec<String>;
}

/// What a provider can do beyond the common contract.
#[derive(Clone, Copy)]
pub struct Caps {
    /// Fills `groups` (OIDC).
    pub reads_groups: bool,
    /// A refresh issues a new access token (OIDC; Discord once the token expired). GitHub's
    /// classic tokens do not rotate.
    pub rotates_tokens: bool,
}

/// The behaviour every `IdentityProvider` must have. `redirect` is the callback it is
/// configured for.
pub async fn conformance(p: &dyn IdentityProvider, idp: &dyn Driver, caps: Caps, redirect: &Url) {
    // authorize: a URL carrying state and a PKCE S256 challenge, and the callback asked for.
    let auth = p.authorize(redirect).await.unwrap();
    let q: HashMap<String, String> = auth.url.query_pairs().into_owned().collect();
    assert_eq!(q.get("state"), Some(&auth.pending.state));
    assert_eq!(
        q.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
    assert_eq!(q.get("redirect_uri"), Some(&redirect.to_string()));
    assert_eq!(&auth.pending.redirect, redirect);
    assert!(!format!("{:?}", auth.pending).contains(&auth.pending.state));
    assert_eq!(B64.decode(&auth.pending.state).unwrap().len(), 32);

    // A forged state is refused before any exchange.
    idp.grant("c1", &auth);
    let err = p
        .complete(auth.pending.clone(), "c1", "forged")
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::StateMismatch), "{err:?}");
    assert!(
        idp.redirects().is_empty(),
        "no exchange on a state mismatch"
    );

    // Happy path: the identity names this provider, and the exchange used the same callback.
    let state = auth.pending.state.clone();
    let done = p
        .complete(auth.pending.clone(), "c1", &state)
        .await
        .unwrap();
    assert_eq!(done.identity.provider.as_str(), p.id());
    assert!(!done.identity.sub.is_empty());
    assert_eq!(done.identity.username, "alice");
    assert_eq!(idp.redirects(), [redirect.to_string()]);
    if caps.reads_groups {
        assert_eq!(done.identity.groups, ["dev"]);
    }

    // A code is single use.
    let err = p
        .complete(auth.pending.clone(), "c1", &state)
        .await
        .unwrap_err();
    assert!(!matches!(err, ProviderError::StateMismatch), "{err:?}");

    // refresh revalidates the token; OIDC re-reads the groups.
    let store = SessionStore::new(SessionConfig::default());
    let session = store.create(done.identity, done.tokens).unwrap();
    idp.set_groups(&["dev", "ops"]);
    let r = p.refresh(&session).await.unwrap();
    if caps.rotates_tokens {
        assert_ne!(
            r.tokens.access_token.expose_secret(),
            session.tokens.access_token.expose_secret()
        );
    }
    if caps.reads_groups {
        assert_eq!(r.identity.expect("identity re-read").groups, ["dev", "ops"]);
    }
    assert!(matches!(
        p.revalidate(&session).await,
        Revalidation::Valid(_)
    ));

    // A revoked grant ends the session.
    idp.reject_refresh();
    let err = p.refresh(&session).await.unwrap_err();
    assert!(matches!(err, ProviderError::Rejected(_)), "{err:?}");
    assert!(!err.is_unreachable());
    assert!(matches!(
        p.revalidate(&session).await,
        Revalidation::Rejected(_)
    ));

    // end_session_url never panics.
    let _ = p.end_session_url(&session);
}

/// An in-process HTTP server for a fake IdP, stopped on drop.
pub struct Server {
    pub base: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn serve(app: axum::Router) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Server { base, task }
}

/// A local port nothing listens on.
pub async fn closed_port_url() -> Url {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    drop(l);
    Url::parse(&format!("http://{addr}")).unwrap()
}

/// The S256 challenge of `verifier`.
pub fn s256(verifier: &str) -> String {
    B64.encode(Sha256::digest(verifier.as_bytes()))
}

/// The `code_challenge` an authorize URL carries.
pub fn challenge_of(auth: &AuthRedirect) -> String {
    auth.url
        .query_pairs()
        .find(|(k, _)| k == "code_challenge")
        .map(|(_, v)| v.into_owned())
        .expect("authorize URL carries a PKCE challenge")
}

/// A body bigger than the crate's response cap.
pub fn oversized() -> String {
    format!(
        "{{\"id\":1,\"login\":\"alice\",\"pad\":\"{}\"}}",
        "x".repeat(web_auth::oauth::MAX_RESPONSE_BYTES + 1)
    )
}
