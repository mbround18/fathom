//! The Discord provider (spec 050, T088) against an in-process fake Discord: conformance, then
//! the ways Discord can say no, fail or misbehave. No test reaches the network.

mod common;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Form, Router};
use common::{Caps, Driver, challenge_of, closed_port_url, conformance, oversized, s256, serve};
use secrecy::SecretString;
use serde_json::{Value, json};
use url::Url;
use web_auth::discord::{DiscordConfig, DiscordProvider, Endpoints};
use web_auth::oauth::{Timeouts, build_oauth_client};
use web_auth::provider::{
    AuthRedirect, Completed, IdentityProvider, ProviderError, Result, Revalidation,
};
use web_auth::session::{NewSession, Provider, SessionConfig, SessionStore};

const SECRET: &str = "discord-client-s3cret";
const PUBLIC: &str = "https://app.example.test";
const CALLBACK: &str = "https://app.example.test/auth/callback/discord";
const SNOWFLAKE: &str = "80351110224678912";

#[derive(Clone)]
struct Reply {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: String,
}

fn reply(status: u16, body: impl Into<String>) -> Option<Reply> {
    Some(Reply {
        status,
        headers: vec![],
        body: body.into(),
    })
}

impl IntoResponse for Reply {
    fn into_response(self) -> Response {
        let mut r = (
            StatusCode::from_u16(self.status).unwrap(),
            [(header::CONTENT_TYPE, "application/json")],
            self.body,
        )
            .into_response();
        for (k, v) in self.headers {
            r.headers_mut().insert(k, v.parse().unwrap());
        }
        r
    }
}

struct Inner {
    /// authorization code -> PKCE challenge of the login it was granted for
    codes: HashMap<String, String>,
    redirects: Vec<String>,
    paths: Vec<String>,
    /// grant types seen at the token endpoint, in order
    grants: Vec<String>,
    issued: u32,
    access: HashSet<String>,
    refresh: HashSet<String>,
    user: Value,
    /// the `scope` the token endpoint reports (what the person ticked)
    scope: String,
    expires_in: u64,
    revoked: bool,
    token_reply: Option<Reply>,
    me_reply: Option<Reply>,
}

#[derive(Clone)]
struct Fake(Arc<Mutex<Inner>>);

impl Fake {
    fn with<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        f(&mut self.0.lock().unwrap())
    }
    fn paths(&self) -> Vec<String> {
        self.with(|i| i.paths.clone())
    }
    fn grants(&self) -> Vec<String> {
        self.with(|i| i.grants.clone())
    }
}

fn issue(i: &mut Inner) -> Response {
    i.issued += 1;
    let (at, rt) = (format!("at-{}", i.issued), format!("rt-{}", i.issued));
    i.access.insert(at.clone());
    i.refresh.insert(rt.clone());
    reply(
        200,
        json!({
            "access_token": at, "token_type": "Bearer", "expires_in": i.expires_in,
            "refresh_token": rt, "scope": i.scope,
        })
        .to_string(),
    )
    .unwrap()
    .into_response()
}

fn invalid_grant() -> Response {
    reply(400, r#"{"error":"invalid_grant"}"#)
        .unwrap()
        .into_response()
}

async fn token(State(f): State<Fake>, Form(form): Form<HashMap<String, String>>) -> Response {
    f.with(|i| {
        i.paths.push("/api/oauth2/token".into());
        i.grants
            .push(form.get("grant_type").cloned().unwrap_or_default());
    });
    if let Some(r) = f.with(|i| i.token_reply.clone()) {
        return r.into_response();
    }
    let client_ok = form.get("client_id").map(String::as_str) == Some("dc-cid")
        && form.get("client_secret").map(String::as_str) == Some(SECRET);
    if !client_ok {
        return reply(401, r#"{"error":"invalid_client"}"#)
            .unwrap()
            .into_response();
    }
    f.with(|i| match form.get("grant_type").map(String::as_str) {
        Some("authorization_code") => {
            i.redirects
                .push(form.get("redirect_uri").cloned().unwrap_or_default());
            let challenge = form.get("code").and_then(|c| i.codes.remove(c));
            let ok =
                challenge.is_some_and(|c| form.get("code_verifier").is_some_and(|v| s256(v) == c));
            if ok { issue(i) } else { invalid_grant() }
        }
        Some("refresh_token") => {
            let known = form
                .get("refresh_token")
                .is_some_and(|t| i.refresh.contains(t));
            if i.revoked || !known {
                invalid_grant()
            } else {
                issue(i)
            }
        }
        _ => reply(400, r#"{"error":"unsupported_grant_type"}"#)
            .unwrap()
            .into_response(),
    })
}

async fn me(State(f): State<Fake>, h: HeaderMap) -> Response {
    f.with(|i| i.paths.push("/api/v10/users/@me".into()));
    if let Some(r) = f.with(|i| i.me_reply.clone()) {
        return r.into_response();
    }
    let bearer = h
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);
    let ok = f.with(|i| !i.revoked && bearer.is_some_and(|b| i.access.contains(&b)));
    if !ok {
        return reply(401, r#"{"message":"401: Unauthorized","code":0}"#)
            .unwrap()
            .into_response();
    }
    reply(200, f.with(|i| i.user.to_string()))
        .unwrap()
        .into_response()
}

async fn anything_else(State(f): State<Fake>, uri: Uri) -> StatusCode {
    f.with(|i| i.paths.push(uri.path().to_string()));
    StatusCode::NOT_FOUND
}

impl Driver for Fake {
    fn grant(&self, code: &str, auth: &AuthRedirect) {
        let c = challenge_of(auth);
        self.with(|i| i.codes.insert(code.into(), c));
    }
    fn set_groups(&self, _groups: &[&str]) {}
    fn reject_refresh(&self) {
        self.with(|i| i.revoked = true);
    }
    fn redirects(&self) -> Vec<String> {
        self.with(|i| i.redirects.clone())
    }
}

struct Env {
    fake: Fake,
    p: DiscordProvider,
    _server: common::Server,
}

fn cfg(request_email: bool) -> DiscordConfig {
    DiscordConfig {
        client_id: "dc-cid".into(),
        client_secret: SecretString::from(SECRET.to_string()),
        public_url: Url::parse(PUBLIC).unwrap(),
        request_email,
    }
}

fn endpoints(base: &str) -> Endpoints {
    Endpoints {
        authorize: Url::parse(&format!("{base}/oauth2/authorize")).unwrap(),
        token: Url::parse(&format!("{base}/api/oauth2/token")).unwrap(),
        api: Url::parse(&format!("{base}/api/v10")).unwrap(),
    }
}

async fn env_with(request_email: bool) -> Env {
    let fake = Fake(Arc::new(Mutex::new(Inner {
        codes: HashMap::new(),
        redirects: vec![],
        paths: vec![],
        grants: vec![],
        issued: 0,
        access: HashSet::new(),
        refresh: HashSet::new(),
        user: json!({
            "id": SNOWFLAKE, "username": "alice", "global_name": "Alice",
            "email": "alice@example.test", "verified": true,
        }),
        scope: "identify email".into(),
        // Every access token is already expired, so each refresh rotates it.
        expires_in: 0,
        revoked: false,
        token_reply: None,
        me_reply: None,
    })));
    let app = Router::new()
        .route("/api/oauth2/token", post(token))
        .route("/api/v10/users/@me", get(me))
        .fallback(anything_else)
        .with_state(fake.clone());
    let server = serve(app).await;
    let http = build_oauth_client(Timeouts::default(), true).unwrap();
    let p =
        DiscordProvider::with_endpoints(cfg(request_email), endpoints(&server.base), http).unwrap();
    Env {
        fake,
        p,
        _server: server,
    }
}

async fn env() -> Env {
    env_with(true).await
}

fn callback() -> Url {
    Url::parse(CALLBACK).unwrap()
}

async fn sign_in(e: &Env) -> Result<Completed> {
    let auth = e.p.authorize(&callback()).await?;
    e.fake.grant("code-1", &auth);
    let state = auth.pending.state.clone();
    e.p.complete(auth.pending, "code-1", &state).await
}

fn session(done: Completed) -> NewSession {
    SessionStore::new(SessionConfig::default())
        .create(done.identity, done.tokens)
        .unwrap()
}

#[tokio::test]
async fn discord_provider_conforms() {
    let e = env().await;
    assert_eq!(e.p.id(), "discord");
    assert_eq!(e.p.redirect_url().as_str(), CALLBACK);
    let caps = Caps {
        reads_groups: false,
        rotates_tokens: true,
    };
    conformance(&e.p, &e.fake, caps, &callback()).await;
}

#[tokio::test]
async fn identity_is_the_snowflake_and_a_verified_email() {
    let e = env().await;
    let done = sign_in(&e).await.unwrap();
    assert_eq!(done.identity.provider, Provider::Discord);
    assert_eq!(done.identity.issuer, "https://discord.com");
    assert_eq!(done.identity.sub, SNOWFLAKE);
    assert_eq!(done.identity.username, "alice");
    assert!(done.identity.groups.is_empty());
    assert_eq!(done.identity.email.as_deref(), Some("alice@example.test"));
}

#[tokio::test]
async fn no_guild_calls() {
    let e = env().await;
    let s = session(sign_in(&e).await.unwrap());
    e.p.refresh(&s).await.unwrap();
    for p in e.fake.paths() {
        assert!(
            ["/api/oauth2/token", "/api/v10/users/@me"].contains(&p.as_str()),
            "unexpected call to {p}"
        );
    }
}

#[tokio::test]
async fn an_unverified_email_is_dropped() {
    for verified in [json!(false), json!(null)] {
        let e = env().await;
        e.fake.with(|i| {
            i.user = json!({
                "id": SNOWFLAKE, "username": "alice",
                "email": "alice@example.test", "verified": verified,
            })
        });
        assert_eq!(
            sign_in(&e).await.unwrap().identity.email,
            None,
            "{verified}"
        );
    }
}

#[tokio::test]
async fn without_the_email_scope_only_identify_is_asked() {
    let e = env_with(false).await;
    e.fake.with(|i| i.scope = "identify".into());
    let auth = e.p.authorize(&callback()).await.unwrap();
    let q: HashMap<String, String> = auth.url.query_pairs().into_owned().collect();
    assert_eq!(q["scope"], "identify");
    assert_eq!(q["response_type"], "code");
    // Even if Discord sent one, an email nobody asked for is not kept.
    let done = sign_in(&e).await.unwrap();
    assert_eq!(done.identity.email, None);
}

#[tokio::test]
async fn a_grant_without_identify_is_refused() {
    let e = env().await;
    e.fake.with(|i| i.scope = "email".into());
    let err = sign_in(&e).await.unwrap_err();
    assert!(matches!(err, ProviderError::Refused(_)), "{err:?}");
    assert!(
        !e.fake.paths().contains(&"/api/v10/users/@me".to_string()),
        "no profile read without identify"
    );
}

#[tokio::test]
async fn an_unusable_id_is_refused() {
    for id in [
        json!(""),
        json!("alice"),
        json!("12a4"),
        json!("0"),
        json!(80351110224678912u64),
        json!(null),
    ] {
        let e = env().await;
        e.fake
            .with(|i| i.user = json!({"id": id, "username": "alice"}));
        let err = sign_in(&e).await.unwrap_err();
        assert!(matches!(err, ProviderError::Refused(_)), "{id}: {err:?}");
    }
    let e = env().await;
    e.fake.with(|i| i.user = json!({"username": "alice"}));
    assert!(matches!(
        sign_in(&e).await.unwrap_err(),
        ProviderError::Refused(_)
    ));
}

#[tokio::test]
async fn a_live_access_token_is_not_refreshed() {
    let e = env().await;
    e.fake.with(|i| i.expires_in = 604_800);
    let s = session(sign_in(&e).await.unwrap());
    let r = e.p.refresh(&s).await.unwrap();
    assert_eq!(e.fake.grants(), ["authorization_code"]);
    assert_eq!(r.identity.unwrap().sub, SNOWFLAKE);
}

#[tokio::test]
async fn an_expired_token_is_refreshed_with_the_refresh_grant() {
    let e = env().await;
    let s = session(sign_in(&e).await.unwrap());
    let r = e.p.refresh(&s).await.unwrap();
    assert_eq!(e.fake.grants(), ["authorization_code", "refresh_token"]);
    assert!(r.tokens.refresh_token.is_some());
}

#[tokio::test]
async fn a_401_on_refresh_is_rejected() {
    let e = env().await;
    let s = session(sign_in(&e).await.unwrap());
    e.fake
        .with(|i| i.token_reply = reply(401, r#"{"error":"invalid_client"}"#));
    assert!(matches!(
        e.p.revalidate(&s).await,
        Revalidation::Rejected(ProviderError::Rejected(_))
    ));
}

#[tokio::test]
async fn a_revoked_grant_is_rejected() {
    let e = env().await;
    let s = session(sign_in(&e).await.unwrap());
    e.fake.with(|i| i.revoked = true);
    assert!(matches!(
        e.p.revalidate(&s).await,
        Revalidation::Rejected(_)
    ));
}

#[tokio::test]
async fn a_rate_limit_keeps_the_session_and_says_when_to_retry() {
    let e = env().await;
    let s = session(sign_in(&e).await.unwrap());

    // Retry-After header.
    e.fake.with(|i| {
        i.token_reply = Some(Reply {
            status: 429,
            headers: vec![("retry-after", "3".into())],
            body: "{}".into(),
        })
    });
    match e.p.revalidate(&s).await {
        Revalidation::Unreachable { error, retry_after } => {
            assert!(matches!(error, ProviderError::RateLimited(_)), "{error:?}");
            assert_eq!(retry_after, Some(Duration::from_secs(3)));
        }
        other => panic!("{other:?}"),
    }

    // Discord's JSON body, fractional seconds.
    e.fake.with(|i| {
        i.token_reply = reply(
            429,
            r#"{"message":"You are being rate limited.","retry_after":2.5,"global":false}"#,
        )
    });
    match e.p.revalidate(&s).await {
        Revalidation::Unreachable { retry_after, .. } => {
            assert_eq!(retry_after, Some(Duration::from_millis(2500)))
        }
        other => panic!("{other:?}"),
    }

    // Absurd values are clamped to an hour.
    e.fake.with(|i| {
        i.token_reply = Some(Reply {
            status: 429,
            headers: vec![("retry-after", "999999".into())],
            body: "{}".into(),
        })
    });
    match e.p.revalidate(&s).await {
        Revalidation::Unreachable { retry_after, .. } => {
            assert_eq!(retry_after, Some(Duration::from_secs(3600)))
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn discord_down_on_refresh_keeps_the_session() {
    let e = env().await;
    let s = session(sign_in(&e).await.unwrap());
    e.fake.with(|i| i.me_reply = reply(502, "bad gateway"));
    assert!(matches!(
        e.p.revalidate(&s).await,
        Revalidation::Unreachable {
            error: ProviderError::Transient(_),
            ..
        }
    ));
}

#[tokio::test]
async fn token_endpoint_401_is_rejected_and_503_is_transient() {
    let e = env().await;
    e.fake
        .with(|i| i.token_reply = reply(401, r#"{"error":"invalid_client"}"#));
    let err = sign_in(&e).await.unwrap_err();
    assert!(matches!(err, ProviderError::Rejected(_)), "{err:?}");

    e.fake.with(|i| i.token_reply = reply(503, "unavailable"));
    let err = sign_in(&e).await.unwrap_err();
    assert!(matches!(err, ProviderError::Transient(_)), "{err:?}");
    assert!(err.is_unreachable());
}

#[tokio::test]
async fn an_oversized_response_fails() {
    let e = env().await;
    e.fake.with(|i| i.me_reply = reply(200, oversized()));
    let err = sign_in(&e).await.unwrap_err();
    assert!(matches!(err, ProviderError::Failed(_)), "{err:?}");

    let e = env().await;
    e.fake.with(|i| i.token_reply = reply(200, oversized()));
    let err = sign_in(&e).await.unwrap_err();
    assert!(matches!(err, ProviderError::Failed(_)), "{err:?}");
}

#[tokio::test]
async fn a_redirect_is_not_followed() {
    let e = env().await;
    e.fake.with(|i| {
        i.token_reply = Some(Reply {
            status: 307,
            headers: vec![("location", "/elsewhere".into())],
            body: String::new(),
        })
    });
    let err = sign_in(&e).await.unwrap_err();
    assert!(matches!(err, ProviderError::Failed(_)), "{err:?}");
    assert!(!e.fake.paths().contains(&"/elsewhere".to_string()));
}

#[tokio::test]
async fn an_unreachable_provider_is_transient() {
    let dead = closed_port_url().await;
    let http = build_oauth_client(Timeouts::default(), true).unwrap();
    let p = DiscordProvider::with_endpoints(
        cfg(true),
        endpoints(dead.as_str().trim_end_matches('/')),
        http,
    )
    .unwrap();
    let auth = p.authorize(&callback()).await.unwrap();
    let state = auth.pending.state.clone();
    let err = p.complete(auth.pending, "c", &state).await.unwrap_err();
    assert!(err.is_unreachable(), "{err:?}");
}

#[tokio::test]
async fn the_exchange_sends_the_secret_and_the_exact_callback() {
    let e = env().await;
    sign_in(&e).await.unwrap();
    assert_eq!(e.fake.redirects(), [CALLBACK]);
    let err = DiscordProvider::new(DiscordConfig {
        client_secret: SecretString::from(String::new()),
        ..cfg(true)
    })
    .err()
    .expect("an empty secret is refused");
    assert!(matches!(err, ProviderError::Failed(_)));
}
