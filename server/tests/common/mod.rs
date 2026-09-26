//! Mock OpenID provider for the hosted-server integration tests: discovery, JWKS (RS256), and a
//! token endpoint whose behavior each test can steer.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use fathom_server::config::HostedConfig;
use rsa::RsaPrivateKey;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::signature::{SignatureEncoding, Signer};
use rsa::traits::PublicKeyParts;
use serde_json::{Value, json};
use sha2::Sha256;

const KEY_PEM: &str = include_str!("test_rsa.pem");
pub const CLIENT_ID: &str = "fathom";

#[derive(Clone, Debug)]
pub enum RefreshMode {
    Ok,
    InvalidGrant,
    ServerError,
    /// A 400 OAuth error response with this (possibly non-standard) `error` code.
    Error(&'static str),
}

#[derive(Clone, Debug)]
pub struct Behavior {
    /// `aud` claim of issued ID tokens.
    pub aud: String,
    /// Seconds from now to `exp` (negative = already expired).
    pub exp_in: i64,
    /// Overrides the nonce echoed in the ID token.
    pub nonce_override: Option<String>,
    pub username: String,
    pub groups: Vec<String>,
    pub refresh: RefreshMode,
    /// Token lifetime reported in `expires_in`.
    pub expires_in: u64,
}

impl Default for Behavior {
    fn default() -> Self {
        Self {
            aud: CLIENT_ID.into(),
            exp_in: 300,
            nonce_override: None,
            username: "alice".into(),
            groups: vec!["dev".into()],
            refresh: RefreshMode::Ok,
            expires_in: 300,
        }
    }
}

#[derive(Default)]
struct Inner {
    /// authorization code -> nonce the login used
    codes: HashMap<String, String>,
    behavior: Behavior,
    refreshes: u32,
    issued: u32,
}

#[derive(Clone)]
struct Shared {
    base: String,
    key: Arc<RsaPrivateKey>,
    inner: Arc<Mutex<Inner>>,
}

pub struct MockIdp {
    pub base: String,
    shared: Shared,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for MockIdp {
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
    fn sign(&self, claims: &Value) -> String {
        let header = B64.encode(json!({"alg":"RS256","typ":"JWT","kid":"k1"}).to_string());
        let payload = B64.encode(claims.to_string());
        let signing = format!("{header}.{payload}");
        let sk = SigningKey::<Sha256>::new((*self.key).clone());
        let sig = sk.sign(signing.as_bytes());
        format!("{signing}.{}", B64.encode(sig.to_bytes()))
    }

    fn id_token(&self, nonce: Option<&str>) -> String {
        let b = self.inner.lock().unwrap().behavior.clone();
        let mut c = json!({
            "iss": self.base,
            "sub": format!("sub-{}", b.username),
            "aud": b.aud,
            "iat": now() - 5,
            "exp": now() + b.exp_in,
            "preferred_username": b.username,
            "groups": b.groups,
            "email": format!("{}@example.test", b.username),
        });
        if let Some(n) = b.nonce_override.as_deref().or(nonce) {
            c["nonce"] = json!(n);
        }
        self.sign(&c)
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
        "n": B64.encode(pk.n().to_bytes_be()),
        "e": B64.encode(pk.e().to_bytes_be()),
    }]}))
}

async fn token(
    State(s): State<Shared>,
    Form(f): Form<HashMap<String, String>>,
) -> axum::response::Response {
    let grant = f.get("grant_type").map(String::as_str).unwrap_or("");
    match grant {
        "authorization_code" => {
            let nonce = {
                let mut i = s.inner.lock().unwrap();
                let n = f.get("code").and_then(|c| i.codes.remove(c));
                if n.is_some() {
                    i.issued += 1;
                }
                n
            };
            let Some(nonce) = nonce else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error":"invalid_grant"})),
                )
                    .into_response();
            };
            let exp = s.inner.lock().unwrap().behavior.expires_in;
            let id = s.id_token(Some(&nonce));
            Json(json!({
                "access_token":"at-1","token_type":"Bearer","expires_in":exp,
                "refresh_token":"rt-1","id_token":id
            }))
            .into_response()
        }
        "refresh_token" => {
            let (mode, exp) = {
                let mut i = s.inner.lock().unwrap();
                i.refreshes += 1;
                (i.behavior.refresh.clone(), i.behavior.expires_in)
            };
            match mode {
                RefreshMode::Ok => {
                    let n = s.inner.lock().unwrap().refreshes;
                    let id = s.id_token(None);
                    Json(json!({
                        "access_token":format!("at-r{n}"),"token_type":"Bearer","expires_in":exp,
                        "refresh_token":format!("rt-r{n}"),"id_token":id
                    }))
                    .into_response()
                }
                RefreshMode::InvalidGrant => (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error":"invalid_grant"})),
                )
                    .into_response(),
                RefreshMode::ServerError => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
                RefreshMode::Error(code) => {
                    (StatusCode::BAD_REQUEST, Json(json!({ "error": code }))).into_response()
                }
            }
        }
        _ => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"unsupported_grant_type"})),
        )
            .into_response(),
    }
}

impl MockIdp {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let shared = Shared {
            base: base.clone(),
            key: Arc::new(RsaPrivateKey::from_pkcs8_pem(KEY_PEM).unwrap()),
            inner: Default::default(),
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

    pub fn set(&self, f: impl FnOnce(&mut Behavior)) {
        f(&mut self.shared.inner.lock().unwrap().behavior);
    }

    /// Register an authorization code the token endpoint will accept once.
    pub fn grant_code(&self, code: &str, nonce: &str) {
        self.shared
            .inner
            .lock()
            .unwrap()
            .codes
            .insert(code.into(), nonce.into());
    }

    pub fn refresh_calls(&self) -> u32 {
        self.shared.inner.lock().unwrap().refreshes
    }

    pub fn config(&self, external_url: &str) -> HostedConfig {
        self.config_with(external_url, None)
    }

    pub fn config_with(&self, external_url: &str, ui_dir: Option<&str>) -> HostedConfig {
        let base = self.base.clone();
        let ext = external_url.to_string();
        let ui = ui_dir.map(str::to_string);
        HostedConfig::from_lookup(
            move |k| match k {
                "FATHOM_UI_DIR" => ui.clone(),
                "FATHOM_EXTERNAL_URL" => Some(ext.clone()),
                "FATHOM_OIDC_ISSUER" => Some(base.clone()),
                "FATHOM_OIDC_CLIENT_ID" => Some(CLIENT_ID.into()),
                "FATHOM_ALLOW_INSECURE_LOCALHOST" => Some("1".into()),
                _ => None,
            },
            false,
        )
        .expect("test config")
    }
}
