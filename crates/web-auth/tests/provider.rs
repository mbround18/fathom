//! `IdentityProvider` conformance for OIDC (spec 050, T016), against an in-process fake issuer
//! (discovery, RS256 JWKS, token endpoint). The conformance test itself is in `common`.

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use common::{Caps, Driver, conformance};
use rsa::RsaPrivateKey;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::sha2::Sha256;
use rsa::signature::{SignatureEncoding, Signer};
use rsa::traits::PublicKeyParts;
use serde_json::{Value, json};
use url::Url;
use web_auth::oidc::{ClaimsConfig, OidcConfig, OidcProvider, build_http_client};
use web_auth::provider::{AuthRedirect, IdentityProvider};
use web_auth::session::{SessionConfig, SessionStore};

const KEY_PEM: &str = include_str!("fixtures/test_rsa.pem");
const CLIENT_ID: &str = "web-auth-test";

#[derive(Default)]
struct Inner {
    /// authorization code -> nonce of the login it was granted for
    codes: HashMap<String, String>,
    /// `redirect_uri` sent with each code exchange
    redirects: Vec<String>,
    groups: Vec<String>,
    reject_refresh: bool,
}

#[derive(Clone)]
struct Shared {
    base: String,
    key: Arc<RsaPrivateKey>,
    inner: Arc<Mutex<Inner>>,
}

struct FakeIssuer {
    base: String,
    shared: Shared,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeIssuer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

impl Shared {
    fn id_token(&self, nonce: Option<&str>) -> String {
        let groups = self.inner.lock().unwrap().groups.clone();
        let mut c = json!({
            "iss": self.base, "sub": "sub-alice", "aud": CLIENT_ID,
            "iat": now() - 5, "exp": now() + 300,
            "preferred_username": "alice", "groups": groups, "email": "alice@example.test",
        });
        if let Some(n) = nonce {
            c["nonce"] = json!(n);
        }
        let header = B64.encode(json!({"alg":"RS256","typ":"JWT","kid":"k1"}).to_string());
        let signing = format!("{header}.{}", B64.encode(c.to_string()));
        let sig = SigningKey::<Sha256>::new((*self.key).clone()).sign(signing.as_bytes());
        format!("{signing}.{}", B64.encode(sig.to_bytes()))
    }
}

async fn discovery(State(s): State<Shared>) -> Json<Value> {
    Json(json!({
        "issuer": s.base,
        "authorization_endpoint": format!("{}/authorize", s.base),
        "token_endpoint": format!("{}/token", s.base),
        "jwks_uri": format!("{}/jwks", s.base),
        "end_session_endpoint": format!("{}/logout", s.base),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
    }))
}

async fn jwks(State(s): State<Shared>) -> Json<Value> {
    let pk = s.key.to_public_key();
    Json(json!({"keys":[{
        "kty":"RSA","use":"sig","alg":"RS256","kid":"k1",
        "n": B64.encode(pk.n().to_bytes_be()), "e": B64.encode(pk.e().to_bytes_be()),
    }]}))
}

fn oauth_error(code: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": code }))).into_response()
}

async fn token(State(s): State<Shared>, Form(f): Form<HashMap<String, String>>) -> Response {
    match f.get("grant_type").map(String::as_str) {
        Some("authorization_code") => {
            let nonce = {
                let mut i = s.inner.lock().unwrap();
                i.redirects
                    .push(f.get("redirect_uri").cloned().unwrap_or_default());
                f.get("code").and_then(|c| i.codes.remove(c))
            };
            let Some(nonce) = nonce else {
                return oauth_error("invalid_grant");
            };
            Json(json!({
                "access_token":"at-1","token_type":"Bearer","expires_in":300,
                "refresh_token":"rt-1","id_token": s.id_token(Some(&nonce)),
            }))
            .into_response()
        }
        Some("refresh_token") => {
            if s.inner.lock().unwrap().reject_refresh {
                return oauth_error("invalid_grant");
            }
            Json(json!({
                "access_token":"at-2","token_type":"Bearer","expires_in":300,
                "refresh_token":"rt-2","id_token": s.id_token(None),
            }))
            .into_response()
        }
        _ => oauth_error("unsupported_grant_type"),
    }
}

impl FakeIssuer {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let shared = Shared {
            base: base.clone(),
            key: Arc::new(RsaPrivateKey::from_pkcs8_pem(KEY_PEM).unwrap()),
            inner: Arc::new(Mutex::new(Inner {
                groups: vec!["dev".into()],
                ..Default::default()
            })),
        };
        let app = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks", get(jwks))
            .route("/token", post(token))
            .with_state(shared.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self { base, shared, task }
    }

    fn with<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        f(&mut self.shared.inner.lock().unwrap())
    }
}

impl Driver for FakeIssuer {
    fn grant(&self, code: &str, auth: &AuthRedirect) {
        let nonce = auth
            .pending
            .nonce
            .clone()
            .expect("OIDC logins carry a nonce");
        self.with(|i| i.codes.insert(code.into(), nonce));
    }
    fn set_groups(&self, groups: &[&str]) {
        self.with(|i| i.groups = groups.iter().map(|g| g.to_string()).collect());
    }
    fn reject_refresh(&self) {
        self.with(|i| i.reject_refresh = true);
    }
    fn redirects(&self) -> Vec<String> {
        self.with(|i| i.redirects.clone())
    }
}

async fn oidc_provider(idp: &FakeIssuer) -> OidcProvider {
    let cfg = OidcConfig {
        issuer: idp.base.clone(),
        client_id: CLIENT_ID.into(),
        client_secret: None,
        redirect_url: Url::parse("https://configured.example.test/auth/callback").unwrap(),
        scopes: vec!["openid".into(), "profile".into()],
        post_logout_url: Url::parse("https://app.example.test/").unwrap(),
    };
    let claims = ClaimsConfig {
        username: "preferred_username".into(),
        groups: "groups".into(),
        username_prefix: String::new(),
        groups_prefix: String::new(),
        groups_required: true,
    };
    OidcProvider::discover(cfg, claims, build_http_client(None).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn oidc_provider_conforms() {
    let idp = FakeIssuer::start().await;
    let p = oidc_provider(&idp).await;
    assert_eq!(p.id(), "oidc");
    let redirect = Url::parse("https://app.example.test/auth/callback").unwrap();
    let caps = Caps {
        reads_groups: true,
        rotates_tokens: true,
    };
    conformance(&p, &idp, caps, &redirect).await;
}

#[tokio::test]
async fn oidc_end_session_url_names_the_client() {
    let idp = FakeIssuer::start().await;
    let oidc = oidc_provider(&idp).await;
    // As a trait object: OidcProvider's inherent methods share these names.
    let p: &dyn IdentityProvider = &oidc;
    let redirect = Url::parse("https://app.example.test/auth/callback").unwrap();
    let auth = p.authorize(&redirect).await.unwrap();
    idp.grant("c1", &auth);
    let state = auth.pending.state.clone();
    let done = p.complete(auth.pending, "c1", &state).await.unwrap();
    let session = SessionStore::new(SessionConfig::default())
        .create(done.identity, done.tokens)
        .unwrap();
    let u = p.end_session_url(&session).expect("endpoint advertised");
    assert!(u.as_str().starts_with(&format!("{}/logout", idp.base)));
    assert!(
        u.query()
            .unwrap()
            .contains(&format!("client_id={CLIENT_ID}"))
    );
}
