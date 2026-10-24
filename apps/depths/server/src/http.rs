//! T037: the HTTP surface (contracts/protocol.md, "HTTP endpoints").
//!
//! | Method | Path | Auth |
//! |---|---|---|
//! | GET | `/healthz`, `/readyz` | none |
//! | GET | `/auth/login?provider=<id>`, `/auth/login/{provider}` | none (auth rate limit) |
//! | GET | `/auth/callback/{provider}` | login cookie + state (auth rate limit) |
//! | POST | `/auth/logout`, `/auth/refresh` | session + CSRF |
//! | GET | `/api/v1/me` | session (401 lists the providers) |
//! | GET | `/api/v1/ws` | Origin, then session + Hello |
//! | GET | `/*` | static files, SPA fallback to `index.html` |
//!
//! Every error body is `{"error":"<code>"}`; unknown `/api` and `/auth` paths are a JSON 404.
//! Every state-changing request passes [`web_auth::extract::CsrfLayer`]. After sign-in the only
//! redirect target is `/`.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Json;
use axum::Router;
use axum::extract::{ConnectInfo, DefaultBodyLimit, FromRef, Path, Query, Request, State};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{any, get, post};
use http::{HeaderMap, HeaderValue, StatusCode, header};
use serde::Deserialize;
use serde_json::json;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::timeout::TimeoutLayer;
use web_auth::cookie::CookieConfig;
use web_auth::extract::{AuthState, CsrfLayer, SessionUser};
use web_auth::login::LoginStore;
use web_auth::provider::{IdentityProvider, PendingLogin, ProviderError, Revalidation};
use web_auth::session::{EndReason, SessionConfig, SessionError, SessionStore};

use depths_core::ports::{ContainerRuntime, StackStore};
use depths_stacks_fs::FsStackStore;
use tokio::sync::broadcast;

use crate::audit::Audit;
use crate::client_ip::ClientIp;
use crate::config::Config;
use crate::limits::{Limits, RateLimiter, SocketCounts};
use crate::ops::Ops;
use crate::users::{Registry, SignIn};
use crate::watch::Watch;
use crate::ws::WsConfig;

pub const SESSION_COOKIE: &str = "__Host-depths_session";
pub const LOGIN_COOKIE: &str = "__Host-depths_login";
/// Largest request body anywhere (T092). Nothing Depths accepts over HTTP is bigger.
pub const MAX_BODY_BYTES: usize = 16 * 1024;
/// Whole-request deadline (T092). The WebSocket is exempt once upgraded.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub type Providers = Vec<(&'static str, Arc<dyn IdentityProvider>)>;

/// Everything a handler can reach.
#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub auth: AuthState,
    pub logins: Arc<LoginStore<PendingLogin>>,
    pub providers: Arc<Providers>,
    pub users: Arc<Registry>,
    pub limits: Arc<Limits>,
    pub client_ip: Arc<ClientIp>,
    pub store: Arc<dyn StackStore>,
    pub runtime: Arc<dyn ContainerRuntime>,
    pub audit: Arc<Audit>,
    pub watch: Watch,
    pub ops: Arc<Ops>,
    /// Fires after every users-registry change: sockets re-read access and refresh feeds.
    pub users_changed: broadcast::Sender<()>,
    pub sockets: Arc<SocketCounts>,
    pub ws: WsConfig,
}

impl FromRef<AppState> for AuthState {
    fn from_ref(s: &AppState) -> Self {
        s.auth.clone()
    }
}

impl AppState {
    /// Opens the stacks directory and the audit log; refuses (with what to fix) when either
    /// cannot be opened.
    pub fn new(
        cfg: Arc<Config>,
        providers: Providers,
        users: Arc<Registry>,
        runtime: Arc<dyn ContainerRuntime>,
    ) -> Result<Self, String> {
        let sessions = SessionStore::new(SessionConfig {
            absolute: cfg.session_max,
            idle: cfg.session_idle,
            ..SessionConfig::default()
        });
        let store: Arc<dyn StackStore> = Arc::new(
            FsStackStore::open(&cfg.stacks_dir)
                .map_err(|e| {
                    format!(
                        "cannot open the stacks directory {} (DEPTHS_STACKS_DIR): {e}",
                        cfg.stacks_dir.display()
                    )
                })?
                .with_max_file_bytes(cfg.max_file_bytes),
        );
        let audit_dir = cfg.data_dir.join("audit");
        let audit = Arc::new(
            Audit::open(audit_dir.clone(), cfg.audit_rotate_bytes).map_err(|e| {
                format!("cannot open the audit log in {}: {e}", audit_dir.display())
            })?,
        );
        let watch = Watch::new(store.clone(), runtime.clone());
        let ops = Arc::new(Ops::new(
            store.clone(),
            runtime.clone(),
            audit.clone(),
            watch.clone(),
        ));
        Ok(Self {
            store,
            runtime,
            audit,
            watch,
            ops,
            users_changed: broadcast::channel(16).0,
            sockets: Arc::new(SocketCounts::default()),
            ws: WsConfig::default(),
            auth: AuthState {
                sessions,
                cookie: CookieConfig::new(SESSION_COOKIE),
                login_url: Some("/auth/login".into()),
            },
            logins: Arc::new(LoginStore::default().with_cookie(CookieConfig::new(LOGIN_COOKIE))),
            providers: Arc::new(providers),
            limits: Arc::new(Limits::default()),
            client_ip: Arc::new(ClientIp::new(cfg.trusted_proxies.clone())),
            users,
            cfg,
        })
    }

    fn provider(&self, id: &str) -> Option<&Arc<dyn IdentityProvider>> {
        self.providers
            .iter()
            .find(|(p, _)| *p == id)
            .map(|(_, p)| p)
    }

    fn provider_ids(&self) -> Vec<&'static str> {
        self.providers.iter().map(|(p, _)| *p).collect()
    }

    /// `401 {"error":"unauthenticated","providers":[...],"loginUrl":"/auth/login"}`.
    pub fn unauthenticated(&self) -> Response {
        let body = json!({
            "error": "unauthenticated",
            "providers": self.provider_ids(),
            "loginUrl": "/auth/login",
        });
        (StatusCode::UNAUTHORIZED, Json(body)).into_response()
    }
}

/// `{"error": code}` with `status`: the one error shape.
pub fn error(status: StatusCode, code: &'static str) -> Response {
    (status, Json(json!({ "error": code }))).into_response()
}

/// The client address, resolved once per request by [`client_ip_layer`].
#[derive(Debug, Clone, Copy)]
pub struct ClientAddr(pub IpAddr);

pub fn router(state: AppState) -> Router {
    let auth_limited = Router::new()
        .route("/auth/login", get(login_query))
        .route("/auth/login/{provider}", get(login_path))
        .route("/auth/callback/{provider}", get(callback))
        .route_layer(middleware::from_fn_with_state(state.clone(), limit_auth));

    let api_limited = Router::new()
        .route("/api/v1/me", get(me))
        .route("/auth/logout", post(logout))
        .route("/auth/refresh", post(refresh))
        .route_layer(middleware::from_fn_with_state(state.clone(), limit_api));

    let ws = Router::new()
        .route("/api/v1/ws", get(crate::ws::upgrade))
        .route_layer(middleware::from_fn_with_state(state.clone(), limit_ws));

    let static_dir = &state.cfg.static_dir;
    let assets = ServeDir::new(static_dir.join("assets"));
    let spa = ServeDir::new(static_dir)
        .append_index_html_on_directories(true)
        .fallback(ServeFile::new(static_dir.join("index.html")));

    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .merge(auth_limited)
        .merge(api_limited)
        .merge(ws)
        .route("/api", any(not_found))
        .route("/api/{*rest}", any(not_found))
        .route("/auth", any(not_found))
        .route("/auth/{*rest}", any(not_found))
        .nest_service("/assets", assets)
        .fallback_service(spa)
        .method_not_allowed_fallback(method_not_allowed)
        .layer(CsrfLayer::new(state.auth.clone(), state.cfg.origin.clone()))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            client_ip_layer,
        ))
        .layer(middleware::from_fn_with_state(
            state.cfg.public_url.scheme() == "https",
            crate::headers::security_headers,
        ))
        .with_state(state)
}

// ---- middleware ---------------------------------------------------------------------------

async fn client_ip_layer(State(app): State<AppState>, mut req: Request, next: Next) -> Response {
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map_or(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0), |c| {
            c.0
        });
    let ip = app.client_ip.resolve(peer, req.headers());
    req.extensions_mut().insert(ClientAddr(ip));
    next.run(req).await
}

fn client_of(req: &Request) -> IpAddr {
    req.extensions()
        .get::<ClientAddr>()
        .map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |c| c.0)
}

fn limited(limiter: &RateLimiter<IpAddr>, req: &Request) -> Option<Response> {
    let wait = limiter.check(&client_of(req), Instant::now()).err()?;
    let mut r = error(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
    let secs = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
    if let Ok(v) = HeaderValue::from_str(&secs.max(1).to_string()) {
        r.headers_mut().insert(header::RETRY_AFTER, v);
    }
    Some(r)
}

async fn limit_auth(State(app): State<AppState>, req: Request, next: Next) -> Response {
    match limited(&app.limits.auth, &req) {
        Some(r) => r,
        None => next.run(req).await,
    }
}

async fn limit_api(State(app): State<AppState>, req: Request, next: Next) -> Response {
    match limited(&app.limits.api, &req) {
        Some(r) => r,
        None => next.run(req).await,
    }
}

async fn limit_ws(State(app): State<AppState>, req: Request, next: Next) -> Response {
    match limited(&app.limits.ws_upgrade, &req) {
        Some(r) => r,
        None => next.run(req).await,
    }
}

// ---- plain handlers -----------------------------------------------------------------------

async fn healthz() -> Response {
    Json(json!({ "status": "ok" })).into_response()
}

async fn readyz(State(app): State<AppState>) -> Response {
    if app.cfg.stacks_dir.is_dir() {
        Json(json!({ "status": "ready" })).into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "not_ready", "reason": "stacks directory is unavailable" })),
        )
            .into_response()
    }
}

async fn not_found() -> Response {
    error(StatusCode::NOT_FOUND, "not_found")
}

async fn method_not_allowed() -> Response {
    error(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed")
}

// ---- sign-in ------------------------------------------------------------------------------

#[derive(Deserialize)]
struct LoginQuery {
    provider: Option<String>,
}

async fn login_query(State(app): State<AppState>, Query(q): Query<LoginQuery>) -> Response {
    let id = match q.provider {
        Some(p) => p,
        // One provider configured: the plain link uses it.
        None if app.providers.len() == 1 => app.providers[0].0.to_owned(),
        None => return error(StatusCode::BAD_REQUEST, "provider_required"),
    };
    start_login(&app, &id).await
}

async fn login_path(State(app): State<AppState>, Path(provider): Path<String>) -> Response {
    start_login(&app, &provider).await
}

async fn start_login(app: &AppState, id: &str) -> Response {
    let Some(provider) = app.provider(id) else {
        return error(StatusCode::NOT_FOUND, "unknown_provider");
    };
    let Ok(redirect) = web_auth::oauth::callback_url(&app.cfg.public_url, provider.id()) else {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "internal");
    };
    match provider.authorize(&redirect).await {
        Ok(ar) => {
            let cookie = app.logins.begin(ar.pending, "/".to_owned());
            let mut r = Redirect::to(ar.url.as_str()).into_response();
            r.headers_mut().append(header::SET_COOKIE, cookie);
            r
        }
        Err(e) => {
            tracing::warn!(
                provider = id,
                error = category(&e),
                "sign-in could not start"
            );
            error(StatusCode::BAD_GATEWAY, "provider_unavailable")
        }
    }
}

/// A fixed page for a failed callback: nothing from the request is echoed.
fn sign_in_failed(status: StatusCode, clear: HeaderValue, msg: &'static str) -> Response {
    let body = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Sign-in \
         failed</title></head><body><h1>Sign-in failed</h1><p>{msg}</p><p><a href=\"/\">Start \
         again</a></p></body></html>"
    );
    let mut r = (
        status,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body,
    )
        .into_response();
    r.headers_mut().append(header::SET_COOKIE, clear);
    r
}

fn category(e: &ProviderError) -> &'static str {
    match e {
        ProviderError::StateMismatch => "state mismatch",
        ProviderError::Refused(_) => "refused",
        ProviderError::Rejected(_) => "rejected",
        ProviderError::Transient(_) => "transient",
        ProviderError::RateLimited(_) => "rate limited",
        ProviderError::Failed(_) => "failed",
    }
}

async fn callback(
    State(app): State<AppState>,
    Path(provider_id): Path<String>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let clear = app.logins.clear_cookie();
    let Some(provider) = app.provider(&provider_id).cloned() else {
        return error(StatusCode::NOT_FOUND, "unknown_provider");
    };
    let state = q.get("state").map(String::as_str).unwrap_or_default();
    // Consumed whatever happens next: a login is single-use.
    let pending = match app.logins.finish(&headers, state) {
        Ok((pending, _)) => pending,
        Err(refused) => {
            tracing::warn!(provider = %provider_id, reason = %refused, "sign-in callback refused");
            return sign_in_failed(
                StatusCode::BAD_REQUEST,
                clear,
                "The sign-in attempt is missing, expired or was started elsewhere.",
            );
        }
    };
    if q.contains_key("error") {
        return sign_in_failed(
            StatusCode::UNAUTHORIZED,
            clear,
            "The provider did not grant the sign-in.",
        );
    }
    let Some(code) = q.get("code") else {
        return sign_in_failed(
            StatusCode::BAD_REQUEST,
            clear,
            "The callback is missing its parameters.",
        );
    };
    let completed = match provider.complete(pending, code, state).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(provider = %provider_id, error = category(&e), "sign-in failed");
            let status = if e.is_unreachable() {
                StatusCode::BAD_GATEWAY
            } else {
                StatusCode::UNAUTHORIZED
            };
            return sign_in_failed(
                status,
                clear,
                "The provider's answer could not be accepted.",
            );
        }
    };
    let who = SignIn::from_identity(&completed.identity);
    let user = match app.users.sign_in(&who) {
        Ok(u) => u,
        Err(e) => {
            tracing::error!(error = %e, "users registry write failed");
            return sign_in_failed(
                StatusCode::INTERNAL_SERVER_ERROR,
                clear,
                "Depths could not record the sign-in.",
            );
        }
    };
    let session = match app
        .auth
        .sessions
        .create(completed.identity, completed.tokens)
    {
        Ok(s) => s,
        Err(SessionError::GlobalLimit(_)) => {
            return sign_in_failed(
                StatusCode::SERVICE_UNAVAILABLE,
                clear,
                "Too many people are signed in. Try again later.",
            );
        }
    };
    tracing::info!(user = %user.id, role = ?user.role, "signed in");
    // Only ever `/`: no return path from the request is followed.
    let mut r = Redirect::to("/").into_response();
    let h = r.headers_mut();
    h.append(
        header::SET_COOKIE,
        app.auth
            .cookie
            .set(&session.cookie, app.cfg.session_max.as_secs()),
    );
    h.append(header::SET_COOKIE, clear);
    r
}

/// Behind [`CsrfLayer`]: session, Origin and token are already checked.
async fn logout(State(app): State<AppState>, SessionUser(session): SessionUser) -> Response {
    let end = app
        .providers
        .iter()
        .find(|(id, _)| *id == session.identity.provider.as_str())
        .and_then(|(_, p)| p.end_session_url(&session))
        .map(|u| u.to_string());
    app.auth.sessions.end(&session.id, EndReason::SignedOut);
    let mut r = Json(json!({ "endSessionUrl": end })).into_response();
    r.headers_mut()
        .append(header::SET_COOKIE, app.auth.cookie.clear());
    r
}

/// T072: checks the session with its provider now. Valid keeps it (with fresh tokens); a
/// rejection ends it; an unreachable provider leaves it alone and says so.
async fn refresh(State(app): State<AppState>, SessionUser(session): SessionUser) -> Response {
    let Some(provider) = app
        .providers
        .iter()
        .find(|(id, _)| *id == session.identity.provider.as_str())
        .map(|(_, p)| p.clone())
    else {
        return error(StatusCode::UNAUTHORIZED, "unauthenticated");
    };
    match provider.revalidate(&session).await {
        Revalidation::Valid(refreshed) => {
            app.auth
                .sessions
                .update_tokens(&session.id, refreshed.tokens);
            Json(json!({ "ok": true })).into_response()
        }
        Revalidation::Rejected(e) => {
            tracing::info!(error = category(&e), "session rejected by its provider");
            app.auth.sessions.end(&session.id, EndReason::Revoked);
            let mut r = app.unauthenticated();
            r.headers_mut()
                .append(header::SET_COOKIE, app.auth.cookie.clear());
            r
        }
        Revalidation::Unreachable { retry_after, .. } => {
            let mut r = error(StatusCode::SERVICE_UNAVAILABLE, "provider_unavailable");
            let secs = retry_after.map_or(30, |d| d.as_secs().max(1));
            if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
                r.headers_mut().insert(header::RETRY_AFTER, v);
            }
            r
        }
    }
}

async fn me(State(app): State<AppState>, headers: HeaderMap) -> Response {
    let Some(s) = app.auth.session(&headers) else {
        return app.unauthenticated();
    };
    let who = SignIn::from_identity(&s.identity);
    let user = app.users.get(&who.id());
    let role = user
        .as_ref()
        .map_or(depths_protocol::Role::Pending, |u| u.role);
    let me = depths_protocol::Me {
        identity: depths_protocol::MeIdentity {
            provider: who.provider,
            subject: who.subject,
            username: who.username,
            email: who.email,
        },
        csrf_token: s.csrf.clone(),
        role,
        owner: user.as_ref().is_some_and(|u| u.owner),
        stacks: match role {
            depths_protocol::Role::User => user.map(|u| u.stacks).unwrap_or_default(),
            _ => Vec::new(),
        },
        features: depths_protocol::Features {
            shell: app.cfg.features.shell,
            edit: app.cfg.features.edit,
        },
    };
    Json(me).into_response()
}
