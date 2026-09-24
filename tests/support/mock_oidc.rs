//! Mock OIDC provider (033 T009). No live IdP needed.
//!
//! Serves discovery, JWKS, authorize (302 with code+state), token (authorization_code,
//! refresh_token with rotation, device_code) and device authorization. Signing keys can be
//! rotated at runtime and individual failure modes injected, so sign-in, refresh, reuse
//! detection and key-rotation paths are all testable offline. Mirrors the Rauthy behaviours
//! measured in research.md (RS256, short tokens, rotated refresh tokens).
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use rand08::rngs::OsRng;
use rsa::pkcs1v15::SigningKey;
use rsa::signature::{RandomizedSigner, SignatureEncoding};
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};
use serde_json::{Value, json};
use sha2::Sha256;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

pub const CLIENT_ID: &str = "fathom-desktop";

/// Key generation is slow in debug builds; generate a small pool once per process.
fn pooled_key(i: usize) -> RsaPrivateKey {
    static POOL: OnceLock<Mutex<Vec<RsaPrivateKey>>> = OnceLock::new();
    let pool = POOL.get_or_init(|| Mutex::new(Vec::new()));
    let mut pool = pool.lock().unwrap();
    while pool.len() <= i {
        pool.push(RsaPrivateKey::new(&mut OsRng, 2048).expect("rsa keygen"));
    }
    pool[i].clone()
}

#[derive(Clone)]
pub struct MockKey {
    pub kid: String,
    private: RsaPrivateKey,
}

impl MockKey {
    fn jwk(&self) -> Value {
        let public = RsaPublicKey::from(&self.private);
        json!({
            "kty": "RSA", "use": "sig", "alg": "RS256", "kid": self.kid,
            "n": B64.encode(public.n().to_bytes_be()),
            "e": B64.encode(public.e().to_bytes_be()),
        })
    }

    fn sign(&self, claims: &Value) -> String {
        let header = json!({"alg": "RS256", "typ": "JWT", "kid": self.kid});
        let signing_input = format!(
            "{}.{}",
            B64.encode(header.to_string()),
            B64.encode(claims.to_string())
        );
        let key = SigningKey::<Sha256>::new(self.private.clone());
        let sig = key.sign_with_rng(&mut OsRng, signing_input.as_bytes());
        format!("{signing_input}.{}", B64.encode(sig.to_bytes()))
    }
}

/// The user the provider authenticates.
#[derive(Clone)]
pub struct MockUser {
    pub sub: String,
    pub email: String,
    pub groups: Vec<String>,
}

impl MockUser {
    pub fn alice() -> Self {
        Self {
            sub: "alice-sub".into(),
            email: "alice@example.test".into(),
            groups: vec!["platform".into()],
        }
    }
}

#[derive(Default)]
struct State {
    keys: Vec<MockKey>,
    key_counter: usize,
    user: Option<MockUser>,
    /// code -> (nonce, client redirect_uri)
    codes: HashMap<String, Option<String>>,
    /// live refresh tokens (rotation: each is single-use)
    refresh_live: HashMap<String, ()>,
    /// refresh tokens already spent (presenting one is reuse)
    refresh_spent: Vec<String>,
    /// device_code -> approved?
    devices: HashMap<String, bool>,
    counter: u64,
    token_ttl_secs: i64,
    fail_token_with: Option<(u16, String)>,
    token_requests: Vec<HashMap<String, String>>,
    issuer: String,
}

pub struct MockOidc {
    pub server: MockServer,
    state: Arc<Mutex<State>>,
}

impl MockOidc {
    pub async fn start() -> Self {
        let server = MockServer::start().await;
        let state = Arc::new(Mutex::new(State {
            token_ttl_secs: 60,
            issuer: server.uri(),
            user: Some(MockUser::alice()),
            ..State::default()
        }));
        let this = Self { server, state };
        this.rotate_key(); // initial key
        this.mount().await;
        this
    }

    pub fn issuer(&self) -> String {
        self.server.uri()
    }

    // ---- test controls -------------------------------------------------------------------

    /// Add a new signing key and make it the active one. Older keys stay in the JWKS unless
    /// [`Self::retire_old_keys`] is called.
    pub fn rotate_key(&self) -> String {
        let mut s = self.state.lock().unwrap();
        let idx = s.key_counter;
        s.key_counter += 1;
        let kid = format!("key-{idx}");
        s.keys.push(MockKey {
            kid: kid.clone(),
            private: pooled_key(idx),
        });
        kid
    }

    pub fn retire_old_keys(&self) {
        let mut s = self.state.lock().unwrap();
        let last = s.keys.pop();
        s.keys.clear();
        s.keys.extend(last);
    }

    pub fn set_user(&self, user: MockUser) {
        self.state.lock().unwrap().user = Some(user);
    }

    pub fn set_token_ttl(&self, secs: i64) {
        self.state.lock().unwrap().token_ttl_secs = secs;
    }

    /// Make the token endpoint return this status + OAuth error code until cleared.
    pub fn fail_token(&self, status: u16, error: &str) {
        self.state.lock().unwrap().fail_token_with = Some((status, error.to_owned()));
    }

    pub fn clear_failure(&self) {
        self.state.lock().unwrap().fail_token_with = None;
    }

    /// Approve a pending device authorization (the human "entered the code").
    pub fn approve_device(&self, device_code: &str) {
        self.state
            .lock()
            .unwrap()
            .devices
            .insert(device_code.to_owned(), true);
    }

    pub fn token_requests(&self) -> Vec<HashMap<String, String>> {
        self.state.lock().unwrap().token_requests.clone()
    }

    /// Mint an ID token directly (for validator tests): custom claims merged over the defaults.
    pub fn mint_id_token(&self, audience: &str, nonce: Option<&str>, overrides: Value) -> String {
        let s = self.state.lock().unwrap();
        mint(&s, audience, nonce, overrides)
    }

    // ---- wiring --------------------------------------------------------------------------

    async fn mount(&self) {
        let base = self.issuer();
        let doc = json!({
            "issuer": base,
            "authorization_endpoint": format!("{base}/authorize"),
            "token_endpoint": format!("{base}/token"),
            "device_authorization_endpoint": format!("{base}/device"),
            "jwks_uri": format!("{base}/jwks"),
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256"],
            "grant_types_supported": [
                "authorization_code", "refresh_token",
                "urn:ietf:params:oauth:grant-type:device_code"
            ],
            "code_challenge_methods_supported": ["S256"],
        });
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(doc))
            .mount(&self.server)
            .await;
        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(Jwks(self.state.clone()))
            .mount(&self.server)
            .await;
        Mock::given(method("GET"))
            .and(path("/authorize"))
            .respond_with(Authorize(self.state.clone()))
            .mount(&self.server)
            .await;
        Mock::given(method("POST"))
            .and(path("/device"))
            .respond_with(Device(self.state.clone()))
            .mount(&self.server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(Token(self.state.clone()))
            .mount(&self.server)
            .await;
    }
}

fn mint(s: &State, audience: &str, nonce: Option<&str>, overrides: Value) -> String {
    let now = chrono::Utc::now().timestamp();
    let user = s.user.clone().unwrap_or_else(MockUser::alice);
    let mut claims = json!({
        "iss": s.issuer, "sub": user.sub, "aud": audience,
        "iat": now, "exp": now + s.token_ttl_secs,
        "email": user.email, "groups": user.groups,
    });
    if let Some(n) = nonce {
        claims["nonce"] = json!(n);
    }
    if let (Some(dst), Some(src)) = (claims.as_object_mut(), overrides.as_object()) {
        for (k, v) in src {
            if v.is_null() {
                dst.remove(k);
            } else {
                dst.insert(k.clone(), v.clone());
            }
        }
    }
    s.keys.last().expect("a signing key").sign(&claims)
}

fn form(req: &Request) -> HashMap<String, String> {
    String::from_utf8_lossy(&req.body)
        .split('&')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            let dec = |x: &str| urlencoding::decode(&x.replace('+', " ")).map(|c| c.into_owned());
            Some((dec(k).ok()?, dec(v).ok()?))
        })
        .collect()
}

struct Jwks(Arc<Mutex<State>>);
impl Respond for Jwks {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let s = self.0.lock().unwrap();
        let keys: Vec<Value> = s.keys.iter().map(MockKey::jwk).collect();
        ResponseTemplate::new(200).set_body_json(json!({ "keys": keys }))
    }
}

struct Authorize(Arc<Mutex<State>>);
impl Respond for Authorize {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let q: HashMap<String, String> = req.url.query_pairs().into_owned().collect();
        let (Some(redirect), Some(state)) = (q.get("redirect_uri"), q.get("state")) else {
            return ResponseTemplate::new(400);
        };
        let mut s = self.0.lock().unwrap();
        s.counter += 1;
        let code = format!("code-{}", s.counter);
        s.codes.insert(code.clone(), q.get("nonce").cloned());
        let sep = if redirect.contains('?') { '&' } else { '?' };
        ResponseTemplate::new(302).insert_header(
            "location",
            format!(
                "{redirect}{sep}code={code}&state={}",
                urlencoding::encode(state)
            ),
        )
    }
}

struct Device(Arc<Mutex<State>>);
impl Respond for Device {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let mut s = self.0.lock().unwrap();
        s.counter += 1;
        let device_code = format!("device-{}", s.counter);
        s.devices.insert(device_code.clone(), false);
        let base = s.issuer.clone();
        ResponseTemplate::new(200).set_body_json(json!({
            "device_code": device_code,
            "user_code": "ABCD-EFGH",
            "verification_uri": format!("{base}/device/verify"),
            "expires_in": 300,
            "interval": 1,
        }))
    }
}

struct Token(Arc<Mutex<State>>);
impl Token {
    fn error(status: u16, code: &str) -> ResponseTemplate {
        ResponseTemplate::new(status).set_body_json(json!({ "error": code }))
    }
}
impl Respond for Token {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let f = form(req);
        let mut s = self.0.lock().unwrap();
        s.token_requests.push(f.clone());
        if let Some((status, code)) = s.fail_token_with.clone() {
            return Self::error(status, &code);
        }
        let grant = f.get("grant_type").map(String::as_str).unwrap_or("");
        let nonce = match grant {
            "authorization_code" => {
                let Some(code) = f.get("code") else {
                    return Self::error(400, "invalid_request");
                };
                match s.codes.remove(code) {
                    Some(nonce) => nonce, // codes are single-use
                    None => return Self::error(400, "invalid_grant"),
                }
            }
            "refresh_token" => {
                let Some(rt) = f.get("refresh_token") else {
                    return Self::error(400, "invalid_request");
                };
                if s.refresh_spent.contains(rt) {
                    // Reuse of a rotated token: the whole family is revoked (Rauthy).
                    s.refresh_live.clear();
                    return Self::error(400, "invalid_grant");
                }
                if s.refresh_live.remove(rt).is_none() {
                    return Self::error(400, "invalid_grant");
                }
                s.refresh_spent.push(rt.clone());
                None
            }
            "urn:ietf:params:oauth:grant-type:device_code" => {
                match f.get("device_code").and_then(|d| s.devices.get(d)) {
                    Some(true) => None,
                    Some(false) => return Self::error(400, "authorization_pending"),
                    None => return Self::error(400, "invalid_grant"),
                }
            }
            _ => return Self::error(400, "unsupported_grant_type"),
        };
        s.counter += 1;
        let refresh = format!("refresh-{}", s.counter);
        s.refresh_live.insert(refresh.clone(), ());
        let id_token = mint(&s, CLIENT_ID, nonce.as_deref(), json!({}));
        ResponseTemplate::new(200).set_body_json(json!({
            "access_token": mint(&s, CLIENT_ID, None, json!({})),
            "token_type": "Bearer",
            "expires_in": s.token_ttl_secs,
            "refresh_token": refresh,
            "id_token": id_token,
        }))
    }
}
