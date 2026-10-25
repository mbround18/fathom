//! T037: the HTTP surface (contracts/protocol.md, "HTTP endpoints").
//!
//! | Method | Path | Auth |
//! |---|---|---|
//! | GET | `/healthz`, `/readyz` | none |
//! | GET | `/auth/login?provider=<id>`, `/auth/login/{provider}` | none (auth rate limit) |
//! | GET | `/auth/callback/{provider}` | login cookie + state (auth rate limit) |
//! | POST | `/auth/logout`, `/auth/refresh` | session + CSRF |
//! | GET | `/api/v1/me` | session (401 lists the providers) |
//! | GET | `/api/v1/audit/export?stack=&actor=&action=&from=&to=` | session, `read_audit` (admin) |
//! | GET | `/api/v1/stacks/{stack}/files/download?mount=&path=` | session, as `ReadFile` |
//! | POST | `/api/v1/stacks/{stack}/files/upload?preview=&name=` | session + CSRF, a confirmed preview |
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
use crate::trust::Trust;
use crate::users::{Registry, SignIn};
use crate::watch::Watch;
use crate::ws::WsConfig;

pub const SESSION_COOKIE: &str = "__Host-depths_session";
pub const LOGIN_COOKIE: &str = "__Host-depths_login";
/// Largest request body anywhere (T092). Nothing Depths accepts over HTTP is bigger.
pub const MAX_BODY_BYTES: usize = 16 * 1024;
/// Whole-request deadline (T092). The WebSocket is exempt once upgraded.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a `/readyz` docker probe is reused.
pub const READY_PROBE_EVERY: Duration = Duration::from_secs(5);

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
    /// The network gate (spec 052).
    pub trust: Arc<Trust>,
    /// The volume explorer (spec 053).
    pub files: Arc<crate::files::Files>,
    /// The last `/readyz` docker probe and when it ran: at most one probe per
    /// [`READY_PROBE_EVERY`], so an unauthenticated caller cannot make Depths spawn docker at will.
    ready: Arc<std::sync::Mutex<Option<(Instant, bool)>>>,
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
        let trust = Arc::new(Trust::open(&cfg, audit.clone(), &sessions)?);
        let watch = Watch::new(store.clone(), runtime.clone()).with_shell_policy(cfg.shell_policy);
        let ops = Arc::new(Ops::new(
            store.clone(),
            runtime.clone(),
            audit.clone(),
            watch.clone(),
        ));
        ops.set_compose_policy(cfg.compose_policy);
        Ok(Self {
            store,
            runtime,
            audit,
            watch,
            ops,
            users_changed: broadcast::channel(16).0,
            sockets: Arc::new(SocketCounts::default()),
            ws: WsConfig::default(),
            trust,
            files: Arc::new(crate::files::Files::new(cfg.features.files)),
            ready: Arc::default(),
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
        .route("/api/v1/audit/export", get(audit_export))
        .route("/api/v1/stacks/{stack}/files/download", get(download))
        .route("/auth/logout", post(logout))
        .route("/auth/refresh", post(refresh))
        .merge(crate::network_routes::routes())
        .route_layer(middleware::from_fn_with_state(state.clone(), observe))
        .route_layer(middleware::from_fn_with_state(state.clone(), limit_api));

    // The one route with a body above MAX_BODY_BYTES, and no whole-request deadline: an upload
    // takes as long as the file does. `upload` bounds the body by its preview.
    let upload_max = usize::try_from(state.cfg.files.upload_max).unwrap_or(usize::MAX);
    let uploads = Router::new()
        .route("/api/v1/stacks/{stack}/files/upload", post(upload))
        .route_layer(middleware::from_fn_with_state(state.clone(), observe))
        .route_layer(middleware::from_fn_with_state(state.clone(), limit_api))
        .layer(DefaultBodyLimit::max(upload_max));

    let ws = Router::new()
        .route("/api/v1/ws", get(crate::ws::upgrade))
        .route_layer(middleware::from_fn_with_state(state.clone(), observe))
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
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .merge(uploads)
        .layer(CsrfLayer::new(state.auth.clone(), state.cfg.origin.clone()))
        .layer(middleware::from_fn(path_guard))
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

/// Spec 052: every request with a live session notes its address and records its network (at
/// most hourly per session), which also ends the session's shells when the network is untrusted
/// (FR-012). Requests without a session pass untouched; the handler answers 401.
async fn observe(State(app): State<AppState>, req: Request, next: Next) -> Response {
    if let Some(cookie) = app.auth.cookie.get(req.headers())
        && let Some(s) = app
            .auth
            .sessions
            .peek(&web_auth::session::session_handle(cookie))
    {
        let user = SignIn::from_identity(&s.identity).id();
        let actor = crate::audit::actor_of(&s.identity);
        app.trust.observe(&user, &actor, &s.id, client_of(&req));
    }
    next.run(req).await
}

/// The 403 for a request refused because its network is untrusted.
pub fn untrusted(d: &depths_core::network::TrustDecision) -> Response {
    (StatusCode::FORBIDDEN, Json(crate::trust::refusal(d))).into_response()
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
    let not_ready = |reason: &'static str| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "not_ready", "reason": reason })),
        )
            .into_response()
    };
    if !app.cfg.stacks_dir.is_dir() {
        return not_ready("stacks directory is unavailable");
    }
    let cached = app
        .ready
        .lock()
        .ok()
        .and_then(|g| *g)
        .filter(|(at, _)| at.elapsed() < READY_PROBE_EVERY)
        .map(|(_, ok)| ok);
    let ok = match cached {
        Some(ok) => ok,
        None => {
            let rt = app.runtime.clone();
            let ok = matches!(
                tokio::task::spawn_blocking(move || rt.containers_by_project()).await,
                Ok(Ok(_))
            );
            if let Ok(mut g) = app.ready.lock() {
                *g = Some((Instant::now(), ok));
            }
            ok
        }
    };
    if ok {
        Json(json!({ "status": "ready" })).into_response()
    } else {
        // Docker's own words reach only signed-in people (FR-013), never this public probe.
        not_ready("the container runtime is unavailable")
    }
}

/// `GET /api/v1/audit/export?stack=&actor=&action=&from=&to=`: the matching audit entries as a
/// JSON-lines download, for people who may read the audit log (admins). Others are refused and
/// the refusal is audited.
async fn audit_export(
    State(app): State<AppState>,
    SessionUser(session): SessionUser,
    ClientAddrExt(ip): ClientAddrExt,
    filter: Result<Query<depths_protocol::AuditFilter>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let who = SignIn::from_identity(&session.identity);
    let access = app.users.access(&who.id());
    if !depths_core::role::permits(&access, None, depths_core::role::Action::ReadAudit) {
        let _ = app.audit.record(&crate::audit::denied(
            crate::audit::actor_of(&session.identity),
            ip,
            None,
            "audit export needs read_audit",
        ));
        return error(StatusCode::FORBIDDEN, "forbidden");
    }
    if let Err(d) = app.trust.gate(
        &who.id(),
        &crate::audit::actor_of(&session.identity),
        &session.id,
        ip,
        depths_core::role::CommandKind::ReadAudit,
        "audit export",
    ) {
        return untrusted(&d);
    }
    let Ok(Query(filter)) = filter else {
        return error(StatusCode::BAD_REQUEST, "invalid_filter");
    };
    let audit = app.audit.clone();
    match tokio::task::spawn_blocking(move || audit.export(&filter)).await {
        Ok(Ok(body)) => (
            [
                (header::CONTENT_TYPE, "application/x-ndjson"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"depths-audit.jsonl\"",
                ),
            ],
            body,
        )
            .into_response(),
        Ok(Err(e)) => {
            tracing::error!(error = %e, "audit export failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal")
        }
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    }
}

#[derive(Deserialize)]
struct DownloadQuery {
    mount: depths_protocol::MountId,
    path: depths_protocol::RelPath,
}

/// `GET /api/v1/stacks/{stack}/files/download?mount=&path=` (spec 053): one file, or a folder as
/// `<folder>.tar`, streamed from a helper of its own as it reads, never buffered whole. Gated as
/// `ReadFile` is; audited `files.download` with the size.
async fn download(
    State(app): State<AppState>,
    SessionUser(session): SessionUser,
    ClientAddrExt(ip): ClientAddrExt,
    Path(stack): Path<String>,
    query: Result<Query<DownloadQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let (Ok(stack), Ok(Query(q))) = (depths_core::StackName::parse(&stack), query) else {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    };
    let who = who_of(&app, &session, ip);
    let a = app.clone();
    let opened = tokio::task::spawn_blocking(move || {
        crate::files::open_download(&a, &who, &stack, &q.mount, &q.path)
    })
    .await;
    let d = match opened {
        Ok(Ok(d)) => d,
        Ok(Err(event)) => return refused(&event),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    };
    let (size, name, content_type) = (d.size, d.name.clone(), d.content_type);
    // Two frames in flight: the helper reads only as fast as the client takes them.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(2);
    tokio::task::spawn_blocking(move || pump(d.chan, &tx));
    let body =
        axum::body::Body::from_stream(futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx)));
    let mut r = (
        [
            (header::CONTENT_TYPE, content_type.to_owned()),
            (header::CACHE_CONTROL, "no-store".to_owned()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
            (header::CONTENT_DISPOSITION, attachment(&name)),
        ],
        body,
    )
        .into_response();
    if let Some(size) = size {
        r.headers_mut().insert(header::CONTENT_LENGTH, size.into());
    }
    r
}

/// Who a file request comes from, as the socket's commands see them.
fn who_of(app: &AppState, session: &web_auth::session::Session, ip: IpAddr) -> crate::ops::Who {
    let sign_in = SignIn::from_identity(&session.identity);
    crate::ops::Who {
        session: session.id.clone(),
        user: sign_in.id(),
        access: app.users.access(&sign_in.id()),
        actor: crate::audit::actor_of(&session.identity),
        ip,
    }
}

#[derive(Deserialize)]
struct UploadQuery {
    preview: depths_protocol::PreviewId,
    name: depths_protocol::Name,
}

/// `POST /api/v1/stacks/{stack}/files/upload?preview=&name=` (spec 053): one file of a confirmed
/// `Upload` preview, as a raw body of the size it was previewed with, streamed to a helper of
/// its own as it arrives. 201 `{fingerprint}`; each name is accepted once.
async fn upload(
    State(app): State<AppState>,
    SessionUser(session): SessionUser,
    ClientAddrExt(ip): ClientAddrExt,
    Path(stack): Path<String>,
    query: Result<Query<UploadQuery>, axum::extract::rejection::QueryRejection>,
    req: Request,
) -> Response {
    use axum::body::HttpBody as _;
    let Ok(stack) = depths_core::StackName::parse(&stack) else {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    };
    let Ok(Query(q)) = query else {
        return error(StatusCode::BAD_REQUEST, "invalid_name");
    };
    let len = req
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok()?.parse::<u64>().ok())
        .or_else(|| req.body().size_hint().exact());
    let Some(len) = len else {
        return error(StatusCode::LENGTH_REQUIRED, "length_required");
    };
    let who = who_of(&app, &session, ip);
    // Two chunks in flight: the client sends only as fast as the helper writes.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::files::Chunk>(2);
    let a = app.clone();
    let written = tokio::task::spawn_blocking(move || {
        let mut next = || rx.blocking_recv();
        crate::files::upload(&a, &who, &stack, q.preview, &q.name, len, &mut next)
    });
    let mut body = req.into_body().into_data_stream();
    let feed = async {
        use futures_util::StreamExt as _;
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map(|b| b.to_vec()).map_err(|_| ());
            let end = chunk.is_err();
            if tx.send(chunk).await.is_err() || end {
                break;
            }
        }
        drop(tx);
    };
    let ((), written) = tokio::join!(feed, written);
    match written {
        Ok(Ok(fingerprint)) => (
            StatusCode::CREATED,
            Json(json!({ "fingerprint": fingerprint })),
        )
            .into_response(),
        Ok(Err(event)) => refused(&event),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    }
}

/// Feeds a download's Data frames to the response until End, then closes the helper. A client
/// that goes away, or a helper that breaks, kills it.
fn pump(
    mut chan: depths_runtime_cli::helper::OneShot,
    tx: &tokio::sync::mpsc::Sender<Result<Vec<u8>, std::io::Error>>,
) {
    use depths_volumes::frame::FrameKind;
    loop {
        match chan.recv() {
            Ok(f) if f.kind == FrameKind::Data => {
                if tx.blocking_send(Ok(f.body)).is_err() {
                    chan.kill();
                    return;
                }
            }
            Ok(f) if f.kind == FrameKind::End => {
                chan.close();
                return;
            }
            other => {
                if let Err(e) = other {
                    tracing::warn!(error = %e, "download broke off");
                }
                let _ = tx.blocking_send(Err(std::io::Error::other("the volume helper failed")));
                chan.kill();
                return;
            }
        }
    }
}

/// `Content-Disposition` for `name`: an ASCII fallback, and the exact name per RFC 5987.
fn attachment(name: &str) -> String {
    let plain: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut exact = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b"._-~".contains(&b) {
            exact.push(b as char);
        } else {
            exact.push_str(&format!("%{b:02X}"));
        }
    }
    format!("attachment; filename=\"{plain}\"; filename*=UTF-8''{exact}")
}

/// A file refusal as an HTTP error: `{"error": kind, "kind": kind}` with the status it means,
/// and `bytes` for `too_large`. A gate denial is 403 with the denial's kind.
fn refused(event: &depths_protocol::Event) -> Response {
    use depths_protocol::{Event, FileRefusal};
    let (status, kind, bytes) = match event {
        Event::Denied { reason, .. } => {
            let kind = serde_json::to_value(reason)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| "forbidden".to_owned());
            (StatusCode::FORBIDDEN, kind, None)
        }
        Event::FileRefused(r) => {
            let (status, bytes) = match r {
                FileRefusal::TooLarge { bytes } => (StatusCode::PAYLOAD_TOO_LARGE, *bytes),
                FileRefusal::Missing | FileRefusal::NotFound | FileRefusal::NotCreated => {
                    (StatusCode::NOT_FOUND, None)
                }
                FileRefusal::NotConfirmed
                | FileRefusal::OutsideMount
                | FileRefusal::FeatureReadOnly
                | FileRefusal::ReadOnlyMount
                | FileRefusal::NotBrowsable { .. }
                | FileRefusal::WriteBlocked { .. } => (StatusCode::FORBIDDEN, None),
                FileRefusal::InvalidName => (StatusCode::BAD_REQUEST, None),
                FileRefusal::HelperUnavailable { .. } => (StatusCode::SERVICE_UNAVAILABLE, None),
                _ => (StatusCode::CONFLICT, None),
            };
            (status, crate::files::refusal_code(r), bytes)
        }
        _ => return error(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    };
    let mut body = json!({ "error": kind, "kind": kind });
    if let Some(bytes) = bytes {
        body["bytes"] = bytes.into();
    }
    (status, Json(body)).into_response()
}

/// The resolved client address as an extractor (set by [`client_ip_layer`]).
pub struct ClientAddrExt(pub IpAddr);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for ClientAddrExt {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(
        parts: &mut http::request::Parts,
        _: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(
            parts
                .extensions
                .get::<ClientAddr>()
                .map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |c| c.0),
        ))
    }
}

/// T095: refuses, before routing, any path a static file server could be tricked with: a
/// segment that is (or decodes to) a dotfile or `..`, an empty segment (`//`), and any slash or
/// backslash smuggled in percent-encoded. Answered as the plain JSON 404, so nothing tells a
/// prober what was matched.
pub fn path_is_hostile(path: &str) -> bool {
    let Some(rest) = path.strip_prefix('/') else {
        return true;
    };
    if rest.contains('\\') {
        return true;
    }
    let segments: Vec<&str> = rest.split('/').collect();
    let last = segments.len() - 1;
    segments.iter().enumerate().any(|(i, seg)| {
        if seg.is_empty() {
            // Only a trailing slash (or the root itself) may leave an empty segment.
            return i != last;
        }
        let decoded = percent_decode(seg);
        decoded.starts_with('.') || decoded.contains('/') || decoded.contains('\\')
    })
}

/// `%XX` decoding, lossy: only to look at what a segment would mean.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(u8::try_from(h * 16 + l).unwrap_or(b'?'));
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn path_guard(req: Request, next: Next) -> Response {
    if path_is_hostile(req.uri().path()) {
        return error(StatusCode::NOT_FOUND, "not_found");
    }
    next.run(req).await
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

/// Writes one audit entry; a failed write is logged, never shown to the caller.
fn record(app: &AppState, e: depths_protocol::AuditEntry) {
    if let Err(err) = app.audit.record(&e) {
        tracing::error!(error = %err, "audit write failed");
    }
}

fn provider_kind(id: &str) -> depths_protocol::Provider {
    match id {
        "github" => depths_protocol::Provider::Github,
        "oidc" => depths_protocol::Provider::Oidc,
        _ => depths_protocol::Provider::Discord,
    }
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
    ClientAddrExt(ip): ClientAddrExt,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let clear = app.logins.clear_cookie();
    let Some(provider) = app.provider(&provider_id).cloned() else {
        return error(StatusCode::NOT_FOUND, "unknown_provider");
    };
    // T094: a refused callback is audited `denied`. Nobody is known yet, so the actor is the
    // provider with an empty subject.
    let refuse = |reason: String| {
        let actor = depths_protocol::Actor {
            provider: provider_kind(provider.id()),
            subject: String::new(),
            username: String::new(),
        };
        record(&app, crate::audit::denied(actor, ip, None, reason));
    };
    let state = q.get("state").map(String::as_str).unwrap_or_default();
    // Consumed whatever happens next: a login is single-use.
    let pending = match app.logins.finish(&headers, state) {
        Ok((pending, _)) => pending,
        Err(refused) => {
            tracing::warn!(provider = %provider_id, reason = %refused, "sign-in callback refused");
            refuse(format!("sign-in refused: {refused}"));
            return sign_in_failed(
                StatusCode::BAD_REQUEST,
                clear,
                "The sign-in attempt is missing, expired or was started elsewhere.",
            );
        }
    };
    if q.contains_key("error") {
        refuse("sign-in refused: the provider did not grant it".into());
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
                refuse(format!("sign-in refused: {}", category(&e)));
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
    let actor = crate::audit::actor_of(&completed.identity);
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
    // T099: at the session cap, someone with access takes the slot of someone who has none
    // (signed in, never admitted), so strangers cannot fill the cap and lock the admins out.
    // Access is read before the store's lock is taken: the predicate runs under it.
    let admitted: std::collections::HashSet<String> =
        if depths_core::role::has_any_access(&app.users.access(&user.id)) {
            app.users
                .all()
                .into_iter()
                .filter(|u| depths_core::role::has_any_access(&app.users.access(&u.id)))
                .map(|u| u.id)
                .collect()
        } else {
            std::collections::HashSet::new()
        };
    let evictable = |s: &web_auth::session::Session| {
        !admitted.is_empty() && !admitted.contains(&SignIn::from_identity(&s.identity).id())
    };
    let session =
        match app
            .auth
            .sessions
            .create_evicting(completed.identity, completed.tokens, evictable)
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
    record(
        &app,
        crate::audit::entry(
            actor.clone(),
            depths_protocol::AuditAction::SignIn,
            ip,
            depths_protocol::AuditOutcome::Succeeded,
        ),
    );
    // Spec 052: the session's address and browser, and the network (a first one is trusted).
    app.trust.signed_in(
        &user.id,
        &actor,
        &session.id,
        ip,
        headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok()),
    );
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
async fn logout(
    State(app): State<AppState>,
    ClientAddrExt(ip): ClientAddrExt,
    SessionUser(session): SessionUser,
) -> Response {
    record(
        &app,
        crate::audit::entry(
            crate::audit::actor_of(&session.identity),
            depths_protocol::AuditAction::SignOut,
            ip,
            depths_protocol::AuditOutcome::Succeeded,
        ),
    );
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

async fn me(
    State(app): State<AppState>,
    ClientAddrExt(ip): ClientAddrExt,
    headers: HeaderMap,
) -> Response {
    let Some(s) = app.auth.session(&headers) else {
        return app.unauthenticated();
    };
    let who = SignIn::from_identity(&s.identity);
    let decision = app.trust.decide(&who.id(), ip);
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
            // Admin only, like `WatchHost` itself: nobody else learns the setting.
            host_metrics: role == depths_protocol::Role::Admin && app.cfg.host_proc.is_some(),
            shell_policy: crate::security::shell_policy(app.cfg.shell_policy),
            compose_policy: crate::security::compose_policy(app.cfg.compose_policy),
            // 051 FR-006: admin only, from the inventory's cached probe.
            daemon: (role == depths_protocol::Role::Admin)
                .then(|| app.watch.daemon())
                .flatten()
                .map(crate::security::daemon),
            // 053 R2: `off` when the helper cannot run here; why is for admins only.
            files: match (app.files.reason(), app.cfg.features.files) {
                (Some(_), _) | (None, crate::config::FilesMode::Off) => {
                    depths_protocol::FilesMode::Off
                }
                (None, crate::config::FilesMode::ReadOnly) => depths_protocol::FilesMode::ReadOnly,
                (None, crate::config::FilesMode::Full) => depths_protocol::FilesMode::Full,
            },
            files_reason: app
                .files
                .reason()
                .filter(|_| role == depths_protocol::Role::Admin),
        },
        network: crate::trust::wire(&decision),
        needs_answer: decision.needs_answer(),
        proxy_warning: role == depths_protocol::Role::Admin && app.trust.proxy.warning(),
        limits: depths_protocol::MeLimits {
            files: depths_protocol::FileLimits {
                editor_max: app.cfg.files.editor_max,
                download_max: app.cfg.files.download_max,
                upload_max: app.cfg.files.upload_max,
            },
        },
    };
    Json(me).into_response()
}
