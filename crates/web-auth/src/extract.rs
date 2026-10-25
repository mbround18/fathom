//! axum glue: the [`SessionUser`] extractor and [`CsrfLayer`], which guards state-changing
//! requests with [`crate::csrf::check_mutation`].
//!
//! The app's state provides an [`AuthState`] through `FromRef`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::Json;
use axum::extract::{FromRef, FromRequestParts, Request};
use axum::response::{IntoResponse, Response};
use http::request::Parts;
use http::{HeaderMap, StatusCode};
use serde_json::json;
use tower::{Layer, Service};

use crate::cookie::CookieConfig;
use crate::csrf;
use crate::session::{Session, SessionStore};

/// What the extractor and the CSRF layer need from the app.
#[derive(Clone)]
pub struct AuthState {
    pub sessions: SessionStore,
    pub cookie: CookieConfig,
    /// Sent as `loginUrl` in the 401 body, so the UI knows where to sign in.
    pub login_url: Option<Arc<str>>,
}

impl AuthState {
    /// The live session for the request's cookie (refreshes the idle timer).
    pub fn session(&self, headers: &HeaderMap) -> Option<Session> {
        self.sessions.touch_cookie(self.cookie.get(headers)?)
    }

    /// `401 {"error":"unauthenticated","loginUrl":...}`.
    pub fn unauthenticated(&self) -> Response {
        let mut body = json!({ "error": "unauthenticated" });
        if let Some(u) = &self.login_url {
            body["loginUrl"] = json!(u.as_ref());
        }
        (StatusCode::UNAUTHORIZED, Json(body)).into_response()
    }
}

/// The signed-in person. Rejects with [`AuthState::unauthenticated`].
#[derive(Debug, Clone)]
pub struct SessionUser(pub Arc<Session>);

impl std::ops::Deref for SessionUser {
    type Target = Session;
    fn deref(&self) -> &Session {
        &self.0
    }
}

impl<S> FromRequestParts<S> for SessionUser
where
    AuthState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Response> {
        // Set by CsrfLayer, which already looked the session up.
        if let Some(u) = parts.extensions.get::<SessionUser>() {
            return Ok(u.clone());
        }
        let auth = AuthState::from_ref(state);
        auth.session(&parts.headers)
            .map(|s| SessionUser(Arc::new(s)))
            .ok_or_else(|| auth.unauthenticated())
    }
}

/// Lets safe methods through. Any other request needs a session (else 401) and must pass
/// [`csrf::check_mutation`] against `origin` (else `403 {"error":"csrf"}`); the session is then
/// handed to the handler's [`SessionUser`].
#[derive(Clone)]
pub struct CsrfLayer {
    auth: AuthState,
    origin: Arc<str>,
}

impl CsrfLayer {
    /// `origin` is the app's `scheme://host[:port]`, the only accepted `Origin`.
    pub fn new(auth: AuthState, origin: impl Into<Arc<str>>) -> Self {
        Self {
            auth,
            origin: origin.into(),
        }
    }
}

impl<S> Layer<S> for CsrfLayer {
    type Service = CsrfService<S>;
    fn layer(&self, inner: S) -> CsrfService<S> {
        CsrfService {
            inner,
            layer: self.clone(),
        }
    }
}

#[derive(Clone)]
pub struct CsrfService<S> {
    inner: S,
    layer: CsrfLayer,
}

fn header<'a>(h: &'a HeaderMap, name: &str) -> Option<&'a str> {
    h.get(name).and_then(|v| v.to_str().ok())
}

impl<S> Service<Request> for CsrfService<S>
where
    S: Service<Request, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Response, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request) -> Self::Future {
        // The clone is not ready; keep the one poll_ready readied.
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        if req.method().is_safe() {
            return Box::pin(inner.call(req));
        }
        let l = &self.layer;
        let h = req.headers();
        let Some(session) = l.auth.session(h) else {
            let r = l.auth.unauthenticated();
            return Box::pin(async move { Ok(r) });
        };
        let ok = csrf::check_mutation(
            &l.origin,
            header(h, "origin"),
            header(h, "sec-fetch-site"),
            &session.csrf,
            header(h, "x-csrf-token"),
        );
        if !ok {
            let r = (StatusCode::FORBIDDEN, Json(json!({ "error": "csrf" }))).into_response();
            return Box::pin(async move { Ok(r) });
        }
        req.extensions_mut().insert(SessionUser(Arc::new(session)));
        Box::pin(inner.call(req))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Identity, SessionConfig, Tokens};
    use axum::Router;
    use axum::body::Body;
    use axum::routing::{get, post};
    use secrecy::SecretString;
    use tower::ServiceExt;

    const ORIGIN: &str = "https://app.example.test";

    fn auth() -> AuthState {
        AuthState {
            sessions: SessionStore::new(SessionConfig::default()),
            cookie: CookieConfig::new("__Host-t_session"),
            login_url: Some("/auth/login".into()),
        }
    }

    fn sign_in(a: &AuthState) -> crate::session::NewSession {
        a.sessions
            .create(
                Identity {
                    provider: Default::default(),
                    issuer: "i".into(),
                    sub: "s".into(),
                    username: "alice".into(),
                    groups: vec![],
                    email: None,
                },
                Tokens {
                    id_token: SecretString::from("id".to_string()),
                    access_token: SecretString::from("at".to_string()),
                    refresh_token: None,
                    expires_at: tokio::time::Instant::now() + std::time::Duration::from_secs(300),
                },
            )
            .unwrap()
    }

    async fn whoami(SessionUser(s): SessionUser) -> String {
        s.identity.username.clone()
    }

    fn app(a: AuthState) -> Router {
        Router::new()
            .route("/me", get(whoami))
            .route(
                "/mutate",
                post(whoami).layer(CsrfLayer::new(a.clone(), ORIGIN)),
            )
            .with_state(a)
    }

    fn req(method: &str, uri: &str, headers: &[(&str, &str)]) -> Request {
        let mut b = Request::builder().method(method).uri(uri);
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        b.body(Body::empty()).unwrap()
    }

    async fn body(r: Response) -> String {
        let b = axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap();
        String::from_utf8(b.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn extractor_rejects_without_a_session_and_names_the_login_url() {
        let r = app(auth()).oneshot(req("GET", "/me", &[])).await.unwrap();
        assert_eq!(r.status(), 401);
        let v: serde_json::Value = serde_json::from_str(&body(r).await).unwrap();
        assert_eq!(v["error"], "unauthenticated");
        assert_eq!(v["loginUrl"], "/auth/login");
    }

    #[tokio::test]
    async fn extractor_finds_the_session_by_cookie_name() {
        let a = auth();
        let s = sign_in(&a);
        let wrong = format!("__Host-other={}", s.cookie);
        let r = app(a.clone())
            .oneshot(req("GET", "/me", &[("cookie", &wrong)]))
            .await
            .unwrap();
        assert_eq!(r.status(), 401);
        let c = format!("__Host-t_session={}", s.cookie);
        let r = app(a)
            .oneshot(req("GET", "/me", &[("cookie", &c)]))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(body(r).await, "alice");
    }

    #[tokio::test]
    async fn csrf_layer_guards_mutations() {
        let a = auth();
        let s = sign_in(&a);
        let c = format!("__Host-t_session={}", s.cookie);
        let call = |h: Vec<(&'static str, String)>| {
            let a = a.clone();
            async move {
                let h: Vec<(&str, &str)> = h.iter().map(|(k, v)| (*k, v.as_str())).collect();
                app(a).oneshot(req("POST", "/mutate", &h)).await.unwrap()
            }
        };
        let r = call(vec![("origin", ORIGIN.into())]).await;
        assert_eq!(r.status(), 401);
        let r = call(vec![("cookie", c.clone()), ("origin", ORIGIN.into())]).await;
        assert_eq!(r.status(), 403);
        assert_eq!(body(r).await, r#"{"error":"csrf"}"#);
        let r = call(vec![
            ("cookie", c.clone()),
            ("origin", "https://evil.example".into()),
            ("x-csrf-token", s.csrf.clone()),
        ])
        .await;
        assert_eq!(r.status(), 403);
        let r = call(vec![
            ("cookie", c.clone()),
            ("origin", ORIGIN.into()),
            ("sec-fetch-site", "cross-site".into()),
            ("x-csrf-token", s.csrf.clone()),
        ])
        .await;
        assert_eq!(r.status(), 403);
        let r = call(vec![
            ("cookie", c),
            ("origin", ORIGIN.into()),
            ("x-csrf-token", s.csrf.clone()),
        ])
        .await;
        assert_eq!(r.status(), 200);
        assert_eq!(body(r).await, "alice");
    }
}
