//! The HTTP server (spec 055 T025, T026, T029): application state, the router, static web
//! serving and the accept loop.
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | `/healthz`, `/readyz` | liveness; readiness checks the database |
//! | GET | `/auth/login[/{provider}]`, `/auth/callback/{provider}` | sign-in (`signin.rs`) |
//! | POST | `/auth/logout` | sign-out, behind the CSRF layer |
//! | GET | `/api/v1/ws` | the WebSocket (`ws.rs`) |
//! | POST | `/api/v1/cmd` | one command over HTTP (`api.rs`) |
//! | POST | `/hooks/{id}` | a source-control webhook delivery (`hooks.rs`), 1 MiB cap |
//! | GET | `/*` | static files from `--static-dir`, SPA fallback to `index.html` |
//!
//! The web build is served from disk, never embedded in the binary. Unknown `/api` and `/auth`
//! paths are JSON 404s, not the SPA.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::{ConnectInfo, DefaultBodyLimit, FromRef, FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use keel_core::ports::Clock;
use serde_json::json;
use sqlx::PgPool;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tower::Service as _;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::timeout::TimeoutLayer;
use uuid::Uuid;
use web_auth::cookie::CookieConfig;
use web_auth::extract::{AuthState, CsrfLayer};
use web_auth::login::LoginStore;
use web_auth::provider::{IdentityProvider, PendingLogin};
use web_auth::session::{SessionConfig, SessionStore};

use crate::config::Config;
use crate::dispatch::PendingOp;
use crate::passkeys::Passkeys;
use crate::preview::{Previews, StepUpVerifier};

/// How long a client may take to send its request headers.
pub const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);
/// How long open connections get to finish after a shutdown signal.
pub const DRAIN: Duration = Duration::from_secs(10);
/// The largest request body (a `CmdEnvelope`).
pub const MAX_BODY_BYTES: usize = 64 * 1024;
/// Whole-request deadline. The WebSocket is exempt once upgraded.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a started sign-in may take, and how many may be pending at once.
pub const LOGIN_TTL: Duration = Duration::from_secs(600);
pub const LOGIN_CAP: usize = 1024;

pub type Providers = Vec<(&'static str, Arc<dyn IdentityProvider>)>;

/// Cookie names: `__Host-` needs `Secure`, so plain-HTTP `--insecure-dev` drops the prefix.
#[derive(Debug, Clone, Copy)]
pub struct Cookies {
    pub session: CookieConfig,
    pub login: CookieConfig,
    /// Not `HttpOnly`: the web client reads it for `Hello.csrf_token` and `X-CSRF-Token`.
    pub csrf: &'static str,
    pub secure: bool,
}

impl Cookies {
    #[must_use]
    pub const fn new(secure: bool) -> Self {
        let (session, login, csrf) = if secure {
            (
                "__Host-keel_session",
                "__Host-keel_login",
                "__Host-keel_csrf",
            )
        } else {
            ("keel_session", "keel_login", "keel_csrf")
        };
        Self {
            session: CookieConfig {
                name: session,
                secure,
            },
            login: CookieConfig {
                name: login,
                secure,
            },
            csrf,
            secure,
        }
    }

    /// `Set-Cookie` for the CSRF token: readable by the page, strict same-site.
    #[must_use]
    pub fn set_csrf(&self, token: &str, max_age: u64) -> HeaderValue {
        let secure = if self.secure { " Secure;" } else { "" };
        HeaderValue::from_str(&format!(
            "{}={token}; Path=/;{secure} SameSite=Strict; Max-Age={max_age}",
            self.csrf
        ))
        .unwrap_or_else(|_| HeaderValue::from_static(""))
    }

    #[must_use]
    pub fn clear_csrf(&self) -> HeaderValue {
        self.set_csrf("", 0)
    }
}

/// Everything a handler can reach.
#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub auth: AuthState,
    pub cookies: Cookies,
    pub session_cfg: SessionConfig,
    pub logins: Arc<LoginStore<PendingLogin>>,
    pub providers: Arc<Providers>,
    pub db: PgPool,
    pub clock: Arc<dyn Clock>,
    pub previews: Arc<Previews<PendingOp>>,
    /// Passkey ceremonies; also the step-up verifier.
    pub passkeys: Arc<Passkeys>,
    pub step_up: Arc<dyn StepUpVerifier>,
    /// Bindings, receivers and credentials: the master key, pushed events, the poller.
    pub repos: crate::sync::Repos,
    /// A user's `authz_epoch` changed (or they were disabled): sockets of that user re-check.
    pub revocations: broadcast::Sender<Uuid>,
    /// The only accepted `Origin`.
    pub origin: Arc<str>,
    /// Runs (US2): connected runners, dispatch and run output.
    pub runs: crate::queue::Runs,
    /// The SSH CAs and the KRL, once [`crate::hosts::bootstrap`] loaded them.
    pub ssh: crate::hosts::Ssh,
}

impl FromRef<AppState> for AuthState {
    fn from_ref(s: &AppState) -> Self {
        s.auth.clone()
    }
}

impl AppState {
    /// Builds the state. Step-up checks passkey assertions ([`Passkeys`]).
    #[must_use]
    pub fn new(cfg: Arc<Config>, providers: Providers, db: PgPool, clock: Arc<dyn Clock>) -> Self {
        let cookies = Cookies::new(cfg.public_url.scheme() == "https");
        let session_cfg = SessionConfig::default();
        let sessions = SessionStore::new(session_cfg.clone());
        let auth = AuthState {
            sessions,
            cookie: cookies.session,
            login_url: Some("/auth/login".into()),
        };
        let origin: Arc<str> = cfg.origin().into();
        let (revocations, _) = broadcast::channel(256);
        let passkeys = Arc::new(Passkeys::new(&cfg, db.clone(), clock.clone()));
        let cfg_log_cap = cfg.run_log_cap;
        Self {
            previews: Arc::new(Previews::new(clock.clone())),
            logins: Arc::new(LoginStore::new(LOGIN_TTL, LOGIN_CAP).with_cookie(cookies.login)),
            providers: Arc::new(providers),
            step_up: passkeys.clone(),
            repos: crate::sync::Repos::default(),
            passkeys,
            cfg,
            auth,
            cookies,
            session_cfg,
            db,
            clock,
            revocations,
            origin,
            runs: crate::queue::Runs::new(crate::queue::RunSettings {
                log_cap: cfg_log_cap,
                ..crate::queue::RunSettings::default()
            }),
            ssh: crate::hosts::Ssh::default(),
        }
    }

    /// Seals and opens credentials and receiver secrets with `key`. Without one they fail
    /// closed.
    #[must_use]
    pub fn with_master_key(mut self, key: crate::seal::MasterKey) -> Self {
        self.repos.master_key = Some(Arc::new(key));
        self
    }

    /// Lets bindings fetch local paths and `file://` URLs. Tests only.
    #[must_use]
    pub fn allowing_local_repos(mut self) -> Self {
        self.repos.allow_local = true;
        self
    }

    /// The provider registered as `id`.
    #[must_use]
    pub fn provider(&self, id: &str) -> Option<&Arc<dyn IdentityProvider>> {
        self.providers
            .iter()
            .find(|(p, _)| *p == id)
            .map(|(_, p)| p)
    }

    /// Call after committing a change that bumped `user`'s `authz_epoch`: their sockets re-read
    /// the session, find it stale, send `Revoked` and close.
    pub fn epoch_changed(&self, user: Uuid) {
        // No receivers just means no open sockets.
        let _ = self.revocations.send(user);
    }
}

/// `{"error": code}` with `status`: the one error shape.
pub fn error(status: StatusCode, code: &'static str) -> Response {
    (status, Json(json!({ "error": code }))).into_response()
}

/// The peer address, when the accept loop recorded one.
#[derive(Debug, Clone, Copy)]
pub struct Peer(pub Option<SocketAddr>);

impl<S: Send + Sync> FromRequestParts<S> for Peer {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(Peer(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|c| c.0),
        ))
    }
}

pub fn router(app: AppState) -> Router {
    let csrf = CsrfLayer::new(app.auth.clone(), app.origin.clone());
    let static_dir = &app.cfg.static_dir;
    let assets = ServeDir::new(static_dir.join("assets"));
    let spa = ServeDir::new(static_dir)
        .append_index_html_on_directories(true)
        .fallback(ServeFile::new(static_dir.join("index.html")));

    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/auth/login", get(crate::signin::login_query))
        .route("/auth/login/{provider}", get(crate::signin::login_path))
        .route("/auth/callback/{provider}", get(crate::signin::callback))
        .route("/auth/logout", post(crate::signin::logout).layer(csrf))
        .route("/api/v1/ws", get(crate::ws::upgrade))
        .route("/api/v1/cmd", post(crate::api::cmd))
        .route(
            "/hooks/{id}",
            post(crate::hooks::receive).layer(DefaultBodyLimit::max(crate::hooks::MAX_BODY_BYTES)),
        )
        .route("/api", any(not_found))
        .route("/api/{*rest}", any(not_found))
        .route("/auth", any(not_found))
        .route("/auth/{*rest}", any(not_found))
        .nest_service("/assets", assets)
        .fallback_service(spa)
        .method_not_allowed_fallback(method_not_allowed)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .with_state(app)
}

async fn healthz() -> Response {
    Json(json!({ "ok": true })).into_response()
}

async fn readyz(State(app): State<AppState>) -> Response {
    match sqlx::query("select 1").execute(&app.db).await {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "readiness: database unreachable");
            error(StatusCode::SERVICE_UNAVAILABLE, "database_unavailable")
        }
    }
}

async fn not_found() -> Response {
    error(StatusCode::NOT_FOUND, "not_found")
}

async fn method_not_allowed() -> Response {
    error(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed")
}

/// The accept loop: hyper http1 with a header-read deadline (slowloris), WebSocket upgrades,
/// the peer address in `ConnectInfo`, and graceful shutdown on SIGTERM or Ctrl-C.
///
/// # Errors
///
/// Never at present; kept fallible for the listener's sake.
pub async fn serve(listener: TcpListener, app: Router) -> std::io::Result<()> {
    serve_until(listener, app, shutdown_signal()).await
}

/// [`serve`], stopping when `shutdown` completes (tests pass their own).
///
/// # Errors
///
/// Never at present.
pub async fn serve_until(
    listener: TcpListener,
    app: Router,
    shutdown: impl Future<Output = ()>,
) -> std::io::Result<()> {
    let graceful = GracefulShutdown::new();
    let mut shutdown = std::pin::pin!(shutdown);
    loop {
        let (stream, peer) = tokio::select! {
            r = listener.accept() => match r {
                Ok(c) => c,
                Err(e) => {
                    // EMFILE and friends: back off instead of spinning.
                    tracing::warn!(error = %e, "accept failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            },
            () = &mut shutdown => break,
        };
        let app = app.clone();
        let svc = hyper::service::service_fn(move |mut req: hyper::Request<Incoming>| {
            req.extensions_mut().insert(ConnectInfo::<SocketAddr>(peer));
            app.clone().call(req)
        });
        let mut builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
        builder
            .http1()
            .timer(TokioTimer::new())
            .header_read_timeout(HEADER_READ_TIMEOUT);
        let conn = builder
            .http1_only()
            .serve_connection_with_upgrades(TokioIo::new(stream), svc)
            .into_owned();
        let conn = graceful.watch(conn);
        tokio::spawn(async move {
            if let Err(e) = conn.await {
                tracing::debug!(error = %e, "connection ended with an error");
            }
        });
    }
    drop(listener);
    tracing::info!("shutting down");
    tokio::select! {
        () = graceful.shutdown() => {}
        () = tokio::time::sleep(DRAIN) => tracing::warn!("connections still open after drain"),
    }
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = term => {}
    }
}

/// Builds the sign-in providers from the configuration (OIDC discovery runs here).
///
/// # Errors
///
/// A provider cannot be built, or OIDC discovery failed.
pub async fn build_providers(cfg: &Config) -> Result<Providers, String> {
    use crate::config::ProviderConfig;
    let mut out: Providers = Vec::new();
    for p in &cfg.providers {
        let provider: Arc<dyn IdentityProvider> = match p {
            ProviderConfig::Discord {
                client_id,
                client_secret,
            } => Arc::new(
                web_auth::discord::DiscordProvider::new(web_auth::discord::DiscordConfig {
                    client_id: client_id.clone(),
                    client_secret: client_secret.clone(),
                    public_url: cfg.public_url.clone(),
                    request_email: false,
                })
                .map_err(|e| format!("discord provider: {e}"))?,
            ),
            ProviderConfig::Github {
                client_id,
                client_secret,
            } => Arc::new(
                web_auth::github::GithubProvider::new(web_auth::github::GithubConfig {
                    client_id: client_id.clone(),
                    client_secret: client_secret.clone(),
                    redirect_url: web_auth::oauth::callback_url(&cfg.public_url, "github")
                        .map_err(|e| format!("github provider: {e}"))?,
                })
                .map_err(|e| format!("github provider: {e}"))?,
            ),
            ProviderConfig::Oidc {
                issuer,
                client_id,
                client_secret,
                scopes,
            } => {
                let http = web_auth::oidc::build_http_client(None)
                    .map_err(|e| format!("oidc provider: {e}"))?;
                let provider = web_auth::oidc::OidcProvider::discover(
                    web_auth::oidc::OidcConfig {
                        issuer: issuer.clone(),
                        client_id: client_id.clone(),
                        client_secret: client_secret.clone(),
                        redirect_url: web_auth::oauth::callback_url(&cfg.public_url, "oidc")
                            .map_err(|e| format!("oidc provider: {e}"))?,
                        scopes: scopes.clone(),
                        post_logout_url: cfg.public_url.clone(),
                    },
                    web_auth::oidc::ClaimsConfig {
                        username: "preferred_username".into(),
                        groups: "groups".into(),
                        username_prefix: String::new(),
                        groups_prefix: String::new(),
                        groups_required: false,
                    },
                    http,
                )
                .await
                .map_err(|e| format!("oidc provider {issuer}: {e}"))?;
                Arc::new(provider)
            }
        };
        out.push((p.id(), provider));
    }
    Ok(out)
}
