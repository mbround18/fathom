//! Hosted HTTP surface (spec 031, T025): OIDC login/callback/logout, session info, health,
//! `/ws`, and the SPA static files, with cookie flags, security headers and CSP.
//! Contract: `specs/031-hosted-server-mode/contracts/hosted-http-api.md`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use serde_json::json;
use tokio::sync::watch;
use tokio::time::Instant;
use tower_http::services::ServeDir;

use crate::config::{AccessModel, HostedConfig, KubeTokenKind};
use crate::csrf;
use crate::oidc::{LoginState, OidcClient, OidcError};
use crate::session::{EndReason, Session, SessionError, SessionStore};
use crate::ws::{self, TenantHost};

pub const SESSION_COOKIE: &str = "__Host-fathom_session";
pub const LOGIN_COOKIE: &str = "__Host-fathom_login";
const LOGIN_TTL: Duration = Duration::from_secs(600);
const LOGIN_CAP: usize = 2000;

/// Pending logins (state/nonce/PKCE verifier + return path), keyed by the login cookie value.
#[derive(Default)]
pub struct LoginStore {
    inner: Mutex<HashMap<String, (LoginState, String, Instant)>>,
}

impl LoginStore {
    pub fn put(&self, login: LoginState, return_to: String) -> String {
        let id = crate::session::random_token();
        let now = Instant::now();
        let mut m = self.inner.lock().unwrap();
        m.retain(|_, (_, _, t)| now.duration_since(*t) < LOGIN_TTL);
        let oldest = m
            .iter()
            .min_by_key(|(_, (_, _, t))| *t)
            .map(|(k, _)| k.clone());
        if let (true, Some(oldest)) = (m.len() >= LOGIN_CAP, oldest) {
            m.remove(&oldest);
        }
        m.insert(id.clone(), (login, return_to, now));
        id
    }

    /// Single use: the entry is removed whether or not the callback succeeds.
    pub fn take(&self, id: &str) -> Option<(LoginState, String)> {
        let (l, r, t) = self.inner.lock().unwrap().remove(id)?;
        (Instant::now().duration_since(t) < LOGIN_TTL).then_some((l, r))
    }
}

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<HostedConfig>,
    pub oidc: Arc<OidcClient>,
    pub sessions: SessionStore,
    pub logins: Arc<LoginStore>,
    pub host: Arc<dyn TenantHost>,
    /// `scheme://host[:port]` of `external_url`; the only accepted `Origin`.
    pub origin: Arc<str>,
    /// Flipped to `true` on shutdown so sockets close with 1001.
    pub shutdown: Arc<watch::Sender<bool>>,
}

impl AppState {
    pub fn new(cfg: Arc<HostedConfig>, oidc: Arc<OidcClient>, host: Arc<dyn TenantHost>) -> Self {
        let sessions = SessionStore::new(cfg.session_config());
        Self::with_sessions(cfg, oidc, host, sessions)
    }

    /// Like [`AppState::new`] with a session store shared with the host.
    pub fn with_sessions(
        cfg: Arc<HostedConfig>,
        oidc: Arc<OidcClient>,
        host: Arc<dyn TenantHost>,
        sessions: SessionStore,
    ) -> Self {
        let origin: Arc<str> = csrf::origin_of(&cfg.external_url).into();
        Self {
            cfg,
            oidc,
            sessions,
            logins: Arc::default(),
            host,
            origin,
            shutdown: Arc::new(watch::channel(false).0),
        }
    }
}

// ---- helpers -------------------------------------------------------------------------------

pub fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|s| s.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

fn set_cookie(name: &str, value: &str, max_age: u64) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{name}={value}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={max_age}"
    ))
    .expect("cookie value is ascii")
}

/// Live session for the request's cookie (refreshes the idle timer).
pub fn session_from_headers(app: &AppState, headers: &HeaderMap) -> Option<Session> {
    app.sessions.touch(cookie_value(headers, SESSION_COOKIE)?)
}

pub fn json_error(status: StatusCode, error: &str, login_url: Option<&str>) -> Response {
    let mut body = json!({ "error": error });
    if let Some(u) = login_url {
        body["loginUrl"] = json!(u);
    }
    (status, Json(body)).into_response()
}

fn page(status: StatusCode, title: &str, msg: &str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{title}</title>\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"></head>\
         <body style=\"font-family:system-ui;max-width:32rem;margin:4rem auto;padding:0 1rem\">\
         <h1>{title}</h1><p>{msg}</p><p><a href=\"/auth/login\">Sign in again</a></p></body></html>"
    );
    (status, Html(html)).into_response()
}

/// RFC 3339 UTC (`YYYY-MM-DDTHH:MM:SSZ`) for a unix timestamp.
pub fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

// ---- handlers ------------------------------------------------------------------------------

async fn login(State(app): State<AppState>, Query(q): Query<HashMap<String, String>>) -> Response {
    let return_to = csrf::safe_return_path(q.get("return").map(String::as_str));
    let req = app.oidc.authorize();
    let id = app.logins.put(req.login, return_to);
    let mut resp = Redirect::to(req.url.as_str()).into_response();
    resp.headers_mut().append(
        header::SET_COOKIE,
        set_cookie(LOGIN_COOKIE, &id, LOGIN_TTL.as_secs()),
    );
    resp
}

async fn callback(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let clear = set_cookie(LOGIN_COOKIE, "", 0);
    let fail = |status, title: &str, msg: &str| {
        let mut r = page(status, title, msg);
        r.headers_mut().append(header::SET_COOKIE, clear.clone());
        r
    };
    let Some((login, return_to)) =
        cookie_value(&headers, LOGIN_COOKIE).and_then(|id| app.logins.take(id))
    else {
        return fail(
            StatusCode::BAD_REQUEST,
            "Sign-in expired",
            "The sign-in attempt is missing or expired. Start again.",
        );
    };
    if q.contains_key("error") {
        tracing::warn!("oidc: provider returned an error on callback");
        return fail(
            StatusCode::UNAUTHORIZED,
            "Sign-in refused",
            "The identity provider refused the sign-in.",
        );
    }
    let (Some(code), Some(state)) = (q.get("code"), q.get("state")) else {
        return fail(
            StatusCode::BAD_REQUEST,
            "Sign-in failed",
            "The callback was missing required parameters.",
        );
    };
    let (identity, tokens) = match app.oidc.complete(code, state, &login).await {
        Ok(v) => v,
        Err(e) => {
            // Log the category only; error text can echo provider responses.
            tracing::warn!("oidc callback failed: {}", oidc_category(&e));
            return match e {
                OidcError::StateMismatch => fail(
                    StatusCode::BAD_REQUEST,
                    "Sign-in failed",
                    "The sign-in state did not match. Start again.",
                ),
                OidcError::MissingGroups(c) => fail(
                    StatusCode::FORBIDDEN,
                    "Groups claim missing",
                    &format!(
                        "The ID token has no `{}` claim. Ask an administrator to configure the identity provider to emit it.",
                        esc(&c)
                    ),
                ),
                OidcError::MissingUsername(c) => fail(
                    StatusCode::FORBIDDEN,
                    "Username claim missing",
                    &format!("The ID token has no usable `{}` claim.", esc(&c)),
                ),
                _ => fail(
                    StatusCode::BAD_GATEWAY,
                    "Sign-in failed",
                    "The identity provider response could not be validated.",
                ),
            };
        }
    };
    let session = match app.sessions.create(identity, tokens) {
        Ok(s) => s,
        Err(SessionError::GlobalLimit(_)) => {
            return fail(
                StatusCode::SERVICE_UNAVAILABLE,
                "Server busy",
                "The server has reached its session limit. Try again later.",
            );
        }
    };
    let mut resp = Redirect::to(&return_to).into_response();
    let h = resp.headers_mut();
    h.append(
        header::SET_COOKIE,
        set_cookie(
            SESSION_COOKIE,
            &session.id,
            app.cfg.session_absolute.as_secs(),
        ),
    );
    h.append(header::SET_COOKIE, clear);
    resp
}

fn oidc_category(e: &OidcError) -> &'static str {
    match e {
        OidcError::StateMismatch => "state mismatch",
        OidcError::Exchange(_) => "code exchange",
        OidcError::IdToken(_) => "id token validation",
        OidcError::MissingUsername(_) | OidcError::MissingGroups(_) => "claims",
        _ => "other",
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

async fn logout(State(app): State<AppState>, headers: HeaderMap) -> Response {
    let Some(session) = session_from_headers(&app, &headers) else {
        return json_error(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            Some("/auth/login"),
        );
    };
    let ok = csrf::check_mutation(
        &app.origin,
        headers.get("origin").and_then(|v| v.to_str().ok()),
        headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()),
        &session.csrf,
        headers.get("x-csrf-token").and_then(|v| v.to_str().ok()),
    );
    if !ok {
        return json_error(StatusCode::FORBIDDEN, "csrf", None);
    }
    app.sessions.end(&session.id, EndReason::SignedOut);
    // In pass-through with ID-token bearers the ID token *is* the user's cluster credential
    // and stays valid at the API server until it expires: never put it in a URL the browser
    // (history, page scripts, IdP access logs) can see (FR-011, FR-017).
    let id_token_is_kube_credential =
        app.cfg.access_model == AccessModel::PassThrough && app.cfg.kube_token == KubeTokenKind::Id;
    let hint = (!id_token_is_kube_credential).then_some(&session.tokens.id_token);
    let end = app.oidc.end_session_url(hint).map(|u| u.to_string());
    let mut resp = Json(json!({ "endSessionUrl": end })).into_response();
    resp.headers_mut()
        .append(header::SET_COOKIE, set_cookie(SESSION_COOKIE, "", 0));
    resp
}

async fn api_session(State(app): State<AppState>, headers: HeaderMap) -> Response {
    let Some(s) = session_from_headers(&app, &headers) else {
        return json_error(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            Some("/auth/login"),
        );
    };
    let remaining = s
        .absolute_deadline
        .saturating_duration_since(Instant::now());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let l = &app.cfg.limits;
    Json(json!({
        "username": s.identity.username,
        "groups": s.identity.groups,
        "csrf": s.csrf,
        "expiresAt": rfc3339((now + remaining).as_secs()),
        "idleTimeoutSec": app.cfg.session_idle.as_secs(),
        "accessModel": match app.cfg.access_model {
            AccessModel::PassThrough => "passthrough",
            AccessModel::Impersonation => "impersonation",
        },
        "mode": "hosted",
        "refreshCount": s.refresh_count,
        "limits": {
            "maxWatches": l.max_watches_per_user,
            "maxStreams": l.max_streams_per_user,
            "maxTabs": l.max_tabs_per_user,
        },
    }))
    .into_response()
}

async fn healthz() -> &'static str {
    "ok"
}

async fn readyz(State(app): State<AppState>) -> Response {
    match app.host.ready().await {
        Ok(()) => Json(json!({"status":"ready"})).into_response(),
        Err(reason) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status":"unready","reason":reason})),
        )
            .into_response(),
    }
}

async fn api_not_found() -> Response {
    json_error(StatusCode::NOT_FOUND, "not_found", None)
}

/// SPA entry: navigations need a session (unauthenticated -> 302 to login), then `index.html`.
async fn spa(State(app): State<AppState>, req: Request) -> Response {
    if session_from_headers(&app, req.headers()).is_none() {
        let want = req
            .uri()
            .path_and_query()
            .map(|p| p.as_str())
            .unwrap_or("/");
        let enc: String = url::form_urlencoded::byte_serialize(want.as_bytes()).collect();
        return Redirect::to(&format!("/auth/login?return={enc}")).into_response();
    }
    match tokio::fs::read(app.cfg.ui_dir.join("index.html")).await {
        Ok(bytes) => {
            let mut r = Response::new(axum::body::Body::from(bytes));
            r.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            );
            r.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            r
        }
        Err(_) => (StatusCode::NOT_FOUND, "UI bundle not found").into_response(),
    }
}

// ---- headers -------------------------------------------------------------------------------

fn csp(app: &AppState) -> String {
    let u = &app.cfg.external_url;
    let ws_scheme = if u.scheme() == "https" { "wss" } else { "ws" };
    let host = u.host_str().unwrap_or("");
    let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
    format!(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
         img-src 'self' data: blob:; font-src 'self' data:; worker-src 'self' blob:; \
         connect-src 'self' {ws_scheme}://{host}{port}; frame-ancestors 'none'; \
         base-uri 'self'; form-action 'self'; object-src 'none'"
    )
}

async fn security_headers(State(app): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_owned();
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    let mut put = |n: &'static str, v: &str| {
        if let Ok(v) = HeaderValue::from_str(v) {
            h.insert(HeaderName::from_static(n), v);
        }
    };
    put("content-security-policy", &csp(&app));
    put("x-content-type-options", "nosniff");
    put("x-frame-options", "DENY");
    put("referrer-policy", "no-referrer");
    put("cross-origin-opener-policy", "same-origin");
    put(
        "permissions-policy",
        "camera=(), microphone=(), geolocation=()",
    );
    if app.cfg.external_url.scheme() == "https" {
        put(
            "strict-transport-security",
            "max-age=31536000; includeSubDomains",
        );
    }
    if path.starts_with("/api/") || path.starts_with("/auth/") || path == "/ws" {
        put("cache-control", "no-store");
    }
    resp
}

pub fn router(app: AppState) -> Router {
    let files = ServeDir::new(&app.cfg.ui_dir)
        .append_index_html_on_directories(false)
        .fallback(get(spa).with_state(app.clone()));
    Router::new()
        .route("/auth/login", get(login))
        .route("/auth/callback", get(callback))
        .route("/auth/logout", post(logout))
        .route("/api/session", get(api_session))
        .route("/api/{*rest}", any(api_not_found))
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/ws", get(ws::ws_handler))
        .fallback_service(files)
        .layer(middleware::from_fn_with_state(
            app.clone(),
            security_headers,
        ))
        .with_state(app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_known_values() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_709_210_096), "2024-02-29T12:34:56Z");
        assert_eq!(rfc3339(4_102_444_799), "2099-12-31T23:59:59Z");
    }

    #[test]
    fn cookie_parsing_picks_exact_name() {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_static(
                "a=1; __Host-fathom_session=abc; x-__Host-fathom_session=evil",
            ),
        );
        assert_eq!(cookie_value(&h, SESSION_COOKIE), Some("abc"));
        assert_eq!(cookie_value(&h, "missing"), None);
    }

    #[test]
    fn set_cookie_carries_required_attributes() {
        let v = set_cookie(SESSION_COOKIE, "id", 60);
        let s = v.to_str().unwrap();
        for a in [
            "__Host-fathom_session=id",
            "Path=/",
            "Secure",
            "HttpOnly",
            "SameSite=Lax",
            "Max-Age=60",
        ] {
            assert!(s.contains(a), "{s}");
        }
        assert!(!s.contains("Domain"));
    }

    #[test]
    fn login_store_is_single_use() {
        let s = LoginStore::default();
        let id = s.put(
            LoginState {
                state: "s".into(),
                nonce: "n".into(),
                pkce_verifier: "v".into(),
            },
            "/x".into(),
        );
        assert!(s.take(&id).is_some());
        assert!(s.take(&id).is_none());
    }

    /// FR-017: secrets never reach logs; `Debug` on session/token types is what `tracing` would use.
    #[test]
    fn tokens_are_redacted_in_debug() {
        use crate::session::{Identity, Tokens};
        use secrecy::SecretString;
        let t = Tokens {
            id_token: SecretString::from("SECRET-ID".to_string()),
            access_token: SecretString::from("SECRET-AT".to_string()),
            refresh_token: Some(SecretString::from("SECRET-RT".to_string())),
            expires_at: Instant::now(),
        };
        let store = SessionStore::new(Default::default());
        let s = store
            .create(
                Identity {
                    issuer: "i".into(),
                    sub: "s".into(),
                    username: "u".into(),
                    groups: vec![],
                    email: None,
                },
                t,
            )
            .unwrap();
        let dbg = format!("{s:?} {:?}", s.tokens);
        assert!(!dbg.contains("SECRET"), "{dbg}");
        // The session id is the cookie value and the CSRF token its companion secret.
        assert!(!dbg.contains(&s.id), "{dbg}");
        assert!(!dbg.contains(&s.csrf), "{dbg}");
        let login = LoginState {
            state: "STATE-SECRET".into(),
            nonce: "NONCE-SECRET".into(),
            pkce_verifier: "PKCE-SECRET".into(),
        };
        assert!(!format!("{login:?}").contains("SECRET"));
    }
}
