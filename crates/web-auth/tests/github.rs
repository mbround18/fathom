//! The GitHub provider (spec 050, T071) against an in-process fake GitHub: conformance, then
//! the ways GitHub can say no, fail or misbehave. No test reaches the network.

mod common;

use std::collections::HashMap;
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
use web_auth::github::{Endpoints, GithubConfig, GithubProvider};
use web_auth::oauth::{Timeouts, build_oauth_client};
use web_auth::provider::{
    AuthRedirect, Completed, IdentityProvider, ProviderError, Result, Revalidation,
};
use web_auth::session::{NewSession, Provider, SessionConfig, SessionStore};

const SECRET: &str = "gh-client-s3cret";
const TOKEN: &str = "gho_live_token";
const CALLBACK: &str = "https://app.example.test/auth/callback/github";

/// A canned answer that replaces a route's normal one.
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
    /// every path requested, in order
    paths: Vec<String>,
    user: Value,
    emails: Value,
    /// the person revoked the app: the token no longer works
    revoked: bool,
    token_reply: Option<Reply>,
    user_reply: Option<Reply>,
    emails_reply: Option<Reply>,
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
}

fn authorized(f: &Fake, h: &HeaderMap) -> bool {
    let want = format!("Bearer {TOKEN}");
    !f.with(|i| i.revoked)
        && h.get(header::AUTHORIZATION)
            .is_some_and(|v| v == want.as_str())
}

async fn token(State(f): State<Fake>, Form(form): Form<HashMap<String, String>>) -> Response {
    f.with(|i| i.paths.push("/login/oauth/access_token".into()));
    if let Some(r) = f.with(|i| i.token_reply.clone()) {
        return r.into_response();
    }
    let challenge = f.with(|i| {
        i.redirects
            .push(form.get("redirect_uri").cloned().unwrap_or_default());
        form.get("code").and_then(|c| i.codes.remove(c))
    });
    let verifier_ok =
        challenge.is_some_and(|c| form.get("code_verifier").is_some_and(|v| s256(v) == c));
    let client_ok = form.get("client_id").map(String::as_str) == Some("gh-cid")
        && form.get("client_secret").map(String::as_str) == Some(SECRET);
    if !verifier_ok || !client_ok {
        // GitHub answers a bad code with 200 and an error field.
        return reply(200, json!({"error": "bad_verification_code"}).to_string())
            .unwrap()
            .into_response();
    }
    reply(
        200,
        json!({"access_token": TOKEN, "token_type": "bearer", "scope": "read:user,user:email"})
            .to_string(),
    )
    .unwrap()
    .into_response()
}

async fn user(State(f): State<Fake>, h: HeaderMap) -> Response {
    f.with(|i| i.paths.push("/user".into()));
    if let Some(r) = f.with(|i| i.user_reply.clone()) {
        return r.into_response();
    }
    if !authorized(&f, &h) {
        return reply(401, r#"{"message":"Bad credentials"}"#)
            .unwrap()
            .into_response();
    }
    reply(200, f.with(|i| i.user.to_string()))
        .unwrap()
        .into_response()
}

async fn emails(State(f): State<Fake>, h: HeaderMap) -> Response {
    f.with(|i| i.paths.push("/user/emails".into()));
    if let Some(r) = f.with(|i| i.emails_reply.clone()) {
        return r.into_response();
    }
    if !authorized(&f, &h) {
        return reply(401, "{}").unwrap().into_response();
    }
    reply(200, f.with(|i| i.emails.to_string()))
        .unwrap()
        .into_response()
}

async fn anything_else(State(f): State<Fake>, uri: Uri) -> StatusCode {
    f.with(|i| i.paths.push(uri.path().to_string()));
    StatusCode::NOT_FOUND
}

async fn hang() -> StatusCode {
    tokio::time::sleep(Duration::from_secs(30)).await;
    StatusCode::OK
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
    p: GithubProvider,
    _server: common::Server,
}

fn cfg() -> GithubConfig {
    GithubConfig {
        client_id: "gh-cid".into(),
        client_secret: SecretString::from(SECRET.to_string()),
        redirect_url: Url::parse(CALLBACK).unwrap(),
    }
}

async fn env_with(timeouts: Timeouts) -> Env {
    let fake = Fake(Arc::new(Mutex::new(Inner {
        codes: HashMap::new(),
        redirects: vec![],
        paths: vec![],
        user: json!({"id": 583231, "login": "alice", "name": "Alice"}),
        emails: json!([
            {"email": "old@example.test", "primary": false, "verified": true},
            {"email": "alice@example.test", "primary": true, "verified": true},
        ]),
        revoked: false,
        token_reply: None,
        user_reply: None,
        emails_reply: None,
    })));
    let app = Router::new()
        .route("/login/oauth/access_token", post(token))
        .route("/user", get(user))
        .route("/user/emails", get(emails))
        .route("/hang", post(hang))
        .fallback(anything_else)
        .with_state(fake.clone());
    let server = serve(app).await;
    let base = &server.base;
    let ep = Endpoints {
        authorize: Url::parse(&format!("{base}/login/oauth/authorize")).unwrap(),
        token: Url::parse(&format!("{base}/login/oauth/access_token")).unwrap(),
        api: Url::parse(base).unwrap(),
    };
    let http = build_oauth_client(timeouts, true).unwrap();
    Env {
        fake,
        p: GithubProvider::with_endpoints(cfg(), ep, http),
        _server: server,
    }
}

async fn env() -> Env {
    env_with(Timeouts::default()).await
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
async fn github_provider_conforms() {
    let e = env().await;
    assert_eq!(e.p.id(), "github");
    let caps = Caps {
        reads_groups: false,
        rotates_tokens: false,
    };
    conformance(&e.p, &e.fake, caps, &callback()).await;
}

#[tokio::test]
async fn identity_is_the_numeric_id_and_the_primary_verified_email() {
    let e = env().await;
    let done = sign_in(&e).await.unwrap();
    assert_eq!(done.identity.provider, Provider::Github);
    assert_eq!(done.identity.issuer, "https://github.com");
    assert_eq!(done.identity.sub, "583231");
    assert_eq!(done.identity.username, "alice");
    assert!(done.identity.groups.is_empty());
    assert_eq!(done.identity.email.as_deref(), Some("alice@example.test"));
}

#[tokio::test]
async fn no_org_or_team_calls() {
    let e = env().await;
    let s = session(sign_in(&e).await.unwrap());
    e.p.refresh(&s).await.unwrap();
    for p in e.fake.paths() {
        assert!(
            ["/login/oauth/access_token", "/user", "/user/emails"].contains(&p.as_str()),
            "unexpected call to {p}"
        );
    }
}

#[tokio::test]
async fn an_unverified_or_secondary_email_is_not_kept() {
    let e = env().await;
    e.fake.with(|i| {
        i.emails = json!([
            {"email": "alice@example.test", "primary": true, "verified": false},
            {"email": "other@example.test", "primary": false, "verified": true},
        ])
    });
    assert_eq!(sign_in(&e).await.unwrap().identity.email, None);
}

#[tokio::test]
async fn a_token_without_email_access_still_signs_in() {
    let e = env().await;
    e.fake
        .with(|i| i.emails_reply = reply(403, r#"{"message":"forbidden"}"#));
    let done = sign_in(&e).await.unwrap();
    assert_eq!(done.identity.sub, "583231");
    assert_eq!(done.identity.email, None);
}

#[tokio::test]
async fn an_unusable_id_is_refused() {
    for id in [
        json!("583231"),
        json!(0),
        json!(-4),
        json!(1.5),
        json!(null),
        json!(""),
    ] {
        let e = env().await;
        e.fake
            .with(|i| i.user = json!({"id": id, "login": "alice"}));
        let err = sign_in(&e).await.unwrap_err();
        assert!(matches!(err, ProviderError::Refused(_)), "{id}: {err:?}");
    }
    let e = env().await;
    e.fake.with(|i| i.user = json!({"login": "alice"}));
    assert!(matches!(
        sign_in(&e).await.unwrap_err(),
        ProviderError::Refused(_)
    ));
}

#[tokio::test]
async fn a_renamed_login_keeps_the_account() {
    let e = env().await;
    let s = session(sign_in(&e).await.unwrap());
    e.fake
        .with(|i| i.user = json!({"id": 583231, "login": "alice-renamed"}));
    match e.p.revalidate(&s).await {
        Revalidation::Valid(r) => {
            let id = r.identity.unwrap();
            assert_eq!(id.sub, "583231");
            assert_eq!(id.username, "alice-renamed");
            assert_eq!(id.email.as_deref(), Some("alice@example.test"));
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn another_account_behind_the_token_is_rejected() {
    let e = env().await;
    let s = session(sign_in(&e).await.unwrap());
    e.fake
        .with(|i| i.user = json!({"id": 9001, "login": "alice"}));
    assert!(matches!(
        e.p.revalidate(&s).await,
        Revalidation::Rejected(_)
    ));
}

#[tokio::test]
async fn a_revoked_token_is_rejected() {
    let e = env().await;
    let s = session(sign_in(&e).await.unwrap());
    e.fake.with(|i| i.revoked = true);
    let err = e.p.refresh(&s).await.unwrap_err();
    assert!(matches!(err, ProviderError::Rejected(_)), "{err:?}");
    assert!(matches!(
        e.p.revalidate(&s).await,
        Revalidation::Rejected(_)
    ));
}

#[tokio::test]
async fn github_down_on_refresh_keeps_the_session() {
    let e = env().await;
    let s = session(sign_in(&e).await.unwrap());
    e.fake.with(|i| i.user_reply = reply(503, "{}"));
    match e.p.revalidate(&s).await {
        Revalidation::Unreachable { error, retry_after } => {
            assert!(matches!(error, ProviderError::Transient(_)), "{error:?}");
            assert_eq!(retry_after, None);
        }
        other => panic!("{other:?}"),
    }
    e.fake.with(|i| {
        i.user_reply = Some(Reply {
            status: 429,
            headers: vec![("retry-after", "7".into())],
            body: "{}".into(),
        })
    });
    match e.p.revalidate(&s).await {
        Revalidation::Unreachable { retry_after, .. } => {
            assert_eq!(retry_after, Some(Duration::from_secs(7)))
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn token_endpoint_401_is_rejected_and_503_is_transient() {
    let e = env().await;
    e.fake
        .with(|i| i.token_reply = reply(401, r#"{"error":"invalid_client"}"#));
    let err = sign_in(&e).await.unwrap_err();
    assert!(matches!(err, ProviderError::Rejected(_)), "{err:?}");
    assert!(!err.is_unreachable());

    e.fake.with(|i| i.token_reply = reply(503, "upstream down"));
    let err = sign_in(&e).await.unwrap_err();
    assert!(matches!(err, ProviderError::Transient(_)), "{err:?}");
    assert!(err.is_unreachable());
}

#[tokio::test]
async fn an_error_in_a_200_token_response_is_rejected() {
    let e = env().await;
    let auth = e.p.authorize(&callback()).await.unwrap();
    e.fake.grant("code-1", &auth);
    let state = auth.pending.state.clone();
    // A code GitHub never issued.
    let err =
        e.p.complete(auth.pending, "nope", &state)
            .await
            .unwrap_err();
    assert!(matches!(err, ProviderError::Rejected(_)), "{err:?}");
}

#[tokio::test]
async fn a_wrong_pkce_verifier_is_refused_by_the_fake() {
    let e = env().await;
    let auth = e.p.authorize(&callback()).await.unwrap();
    e.fake.grant("code-1", &auth);
    let state = auth.pending.state.clone();
    let mut pending = auth.pending;
    pending.pkce_verifier = "not-the-verifier".into();
    assert!(e.p.complete(pending, "code-1", &state).await.is_err());
}

#[tokio::test]
async fn an_oversized_response_fails() {
    let e = env().await;
    e.fake.with(|i| i.user_reply = reply(200, oversized()));
    let err = sign_in(&e).await.unwrap_err();
    assert!(matches!(err, ProviderError::Failed(_)), "{err:?}");
}

#[tokio::test]
async fn a_redirect_is_not_followed() {
    let e = env().await;
    e.fake.with(|i| {
        i.token_reply = Some(Reply {
            status: 302,
            headers: vec![("location", "/elsewhere".into())],
            body: String::new(),
        })
    });
    let err = sign_in(&e).await.unwrap_err();
    assert!(matches!(err, ProviderError::Failed(_)), "{err:?}");
    assert!(!e.fake.paths().contains(&"/elsewhere".to_string()));
}

#[tokio::test]
async fn a_hung_provider_times_out_as_transient() {
    let e = env_with(Timeouts {
        connect: Duration::from_secs(1),
        total: Duration::from_millis(200),
    })
    .await;
    let base = e._server.base.clone();
    let ep = Endpoints {
        authorize: Url::parse(&format!("{base}/login/oauth/authorize")).unwrap(),
        token: Url::parse(&format!("{base}/hang")).unwrap(),
        api: Url::parse(&base).unwrap(),
    };
    let http = build_oauth_client(
        Timeouts {
            connect: Duration::from_secs(1),
            total: Duration::from_millis(200),
        },
        true,
    )
    .unwrap();
    let p = GithubProvider::with_endpoints(cfg(), ep, http);
    let auth = p.authorize(&callback()).await.unwrap();
    let state = auth.pending.state.clone();
    let err = p.complete(auth.pending, "c", &state).await.unwrap_err();
    assert!(matches!(err, ProviderError::Transient(_)), "{err:?}");
}

#[tokio::test]
async fn an_unreachable_provider_is_transient() {
    let dead = closed_port_url().await;
    let ep = Endpoints {
        authorize: dead.join("/login/oauth/authorize").unwrap(),
        token: dead.join("/login/oauth/access_token").unwrap(),
        api: dead.clone(),
    };
    let http = build_oauth_client(Timeouts::default(), true).unwrap();
    let p = GithubProvider::with_endpoints(cfg(), ep, http);
    let auth = p.authorize(&callback()).await.unwrap();
    let state = auth.pending.state.clone();
    let err = p.complete(auth.pending, "c", &state).await.unwrap_err();
    assert!(err.is_unreachable(), "{err:?}");
}

#[tokio::test]
async fn errors_never_carry_the_secret_or_the_token() {
    let e = env().await;
    e.fake.with(|i| {
        i.token_reply = reply(
            400,
            json!({
                "error": "invalid_request",
                "error_description": format!("you sent {SECRET} and {TOKEN}"),
            })
            .to_string(),
        )
    });
    let err = sign_in(&e).await.unwrap_err();
    let shown = format!("{err} {err:?}");
    assert!(!shown.contains(SECRET), "{shown}");
    assert!(!shown.contains(TOKEN), "{shown}");
}
