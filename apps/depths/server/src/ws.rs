//! T038, T047, T090, T092: `GET /api/v1/ws` (contracts/protocol.md, "Transport").
//!
//! The upgrade is refused (403 JSON) unless `Origin` is the app's own. After the upgrade the
//! first frame must be `Hello{protocol_version, csrf_token}` within [`WsConfig::hello_timeout`]:
//!
//! - no session, a timeout or a non-Hello first frame: close `4401` (signed out);
//! - a CSRF token that does not match the session: close `4403`;
//! - a person with no access (`pending`, or a `user` with no stacks): close `4403`;
//! - more than [`SOCKETS_PER_SESSION`] sockets for the session, or [`SOCKETS_PER_IP`] for the
//!   address: close `4429`.
//!
//! Then one task owns the socket. Every command touches the session (gone: `4401`), re-reads
//! the person's access from the users registry (none left: `4403`), and is authorised against
//! that. More than [`WS_COMMAND_BURST`] commands at once, or [`WS_COMMANDS_PER_MINUTE`] a
//! minute after that, closes `4429`. Binary frames close `1003`; a message over
//! [`max_message_bytes`] ends the socket. The server pings every [`WsConfig::ping_every`] and
//! closes `1001` after [`MAX_MISSED_PONGS`] unanswered pings. `SessionExpiring` goes out
//! [`WsConfig::expiring_lead`] before the session would expire.
//!
//! Every refusal of an action is audited as `denied` and answered with `Denied` carrying the
//! command's `reply_to`.

use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use axum::Extension;
use axum::extract::State;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::response::{IntoResponse, Response};
use depths_core::role::{self, Access, Action};
use depths_protocol::{
    Actor, AuditAction, AuditOutcome, Cmd, CmdEnvelope, DenyReason, Event, EventEnvelope, Hello,
    PROTOCOL_VERSION, RequestId, UserId,
};
use http::{HeaderMap, StatusCode, header};
use tokio::sync::{broadcast, mpsc};
use tokio::time::Instant;
use web_auth::session::{EndReason, Session};

use crate::audit;
use crate::http::{AppState, ClientAddr, error};
use crate::inventory;
#[allow(unused_imports)] // SOCKETS_*: named in the module docs.
use crate::limits::{
    RateLimiter, SOCKETS_PER_IP, SOCKETS_PER_SESSION, WS_COMMAND_BURST, WS_COMMANDS_PER_MINUTE,
};
use crate::ops::Who;
use crate::users::{self, SignIn};
use crate::watch::{Subscription, Update};

mod streams;
pub use streams::{MAX_COLS, MAX_ROWS, SHELL_INPUT_QUEUE, STREAM_BUDGET, STREAM_QUEUE};

pub const CLOSE_GOING_AWAY: u16 = 1001;
pub const CLOSE_UNSUPPORTED: u16 = 1003;
pub const CLOSE_TOO_BIG: u16 = 1009;
pub const CLOSE_SIGNED_OUT: u16 = 4401;
pub const CLOSE_NO_ACCESS: u16 = 4403;
pub const CLOSE_TOO_MANY: u16 = 4429;
/// Unanswered pings before the socket is given up on (T092).
pub const MAX_MISSED_PONGS: u32 = 2;
/// Events queued for one socket (operation output) before the producer waits.
const OUT_QUEUE: usize = 256;
/// How long a closing socket waits for the client's close frame.
const CLOSE_GRACE: Duration = Duration::from_secs(1);
/// Shell input and resizes per socket: 100 a second, burst 400. Apart from the command limit,
/// because key repeat alone sends 30 a second.
pub const WS_SHELL_PER_MINUTE: u32 = 100 * 60;
pub const WS_SHELL_BURST: u32 = 400;

/// The socket's timers. Tests shorten them.
#[derive(Debug, Clone, Copy)]
pub struct WsConfig {
    pub hello_timeout: Duration,
    pub ping_every: Duration,
    pub expiring_lead: Duration,
    /// How often a stats watch samples.
    pub stats_every: Duration,
}

impl Default for WsConfig {
    fn default() -> Self {
        Self {
            hello_timeout: Duration::from_secs(10),
            ping_every: Duration::from_secs(30),
            expiring_lead: Duration::from_secs(60),
            stats_every: Duration::from_secs(2),
        }
    }
}

/// The largest client message (T092): a save carries a compose file and a `.env`, each at most
/// `max_file_bytes`, plus room for the envelope.
pub fn max_message_bytes(max_file_bytes: u64) -> usize {
    let n = max_file_bytes.saturating_mul(2).saturating_add(64 * 1024);
    usize::try_from(n).unwrap_or(usize::MAX)
}

pub async fn upgrade(
    State(app): State<AppState>,
    addr: Option<Extension<ClientAddr>>,
    headers: HeaderMap,
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    if !web_auth::csrf::origin_allowed(&app.cfg.origin, origin) {
        return error(StatusCode::FORBIDDEN, "origin");
    }
    let Ok(ws) = ws else {
        return error(StatusCode::UPGRADE_REQUIRED, "websocket_required");
    };
    let ip = addr.map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |Extension(a)| a.0);
    let session = app.auth.session(&headers);
    let max = max_message_bytes(app.cfg.max_file_bytes);
    ws.max_message_size(max)
        .max_frame_size(max)
        .on_upgrade(move |socket| run(app, session, ip, socket))
        .into_response()
}

/// Sends the close frame, then waits (briefly) for the client's, so the frame is read before
/// the connection drops.
async fn close(mut socket: WebSocket, code: u16, reason: &'static str) {
    let sent = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
    if sent.is_ok() {
        let _ = tokio::time::timeout(CLOSE_GRACE, async {
            while let Some(Ok(m)) = socket.recv().await {
                if matches!(m, Message::Close(_)) {
                    break;
                }
            }
        })
        .await;
    }
}

async fn send(socket: &mut WebSocket, env: &EventEnvelope) -> bool {
    let Ok(text) = serde_json::to_string(env) else {
        return false;
    };
    socket.send(Message::Text(text.into())).await.is_ok()
}

fn reply(id: Option<RequestId>, event: Event) -> EventEnvelope {
    EventEnvelope {
        reply_to: id,
        event,
    }
}

/// How the loop ended: the close code and reason to send.
enum Flow {
    Close(u16, &'static str),
}

/// One socket's fixed facts and its mutable subscriptions.
struct Conn {
    app: AppState,
    session: String,
    user: String,
    actor: Actor,
    ip: IpAddr,
    out: mpsc::Sender<EventEnvelope>,
    stacks: Option<Subscription>,
    users_feed: bool,
    /// Log follows, shells and the stats watch. Dropped with the socket, which stops them all.
    streams: streams::Registry,
}

impl Conn {
    fn access(&self) -> Access {
        self.app.users.access(&self.user)
    }

    fn who(&self, access: Access) -> Who {
        Who {
            session: self.session.clone(),
            user: self.user.clone(),
            access,
            actor: self.actor.clone(),
            ip: self.ip,
        }
    }

    /// Audits a refusal and answers it.
    fn deny(&self, id: Option<RequestId>, stack: Option<&str>, why: String) -> EventEnvelope {
        let _ = self
            .app
            .audit
            .record(&audit::denied(self.actor.clone(), self.ip, stack, why));
        reply(
            id,
            Event::Denied {
                reason: DenyReason::NotPermitted,
            },
        )
    }

    fn deny_reason(
        &self,
        id: Option<RequestId>,
        stack: Option<&str>,
        reason: DenyReason,
        why: String,
    ) -> EventEnvelope {
        let _ = self
            .app
            .audit
            .record(&audit::denied(self.actor.clone(), self.ip, stack, why));
        reply(id, Event::Denied { reason })
    }
}

fn invalid(id: Option<RequestId>, reason: impl Into<String>) -> EventEnvelope {
    reply(
        id,
        Event::Invalid {
            reason: reason.into(),
        },
    )
}

/// The registry id of a session's person.
fn user_of(s: &Session) -> String {
    SignIn::from_identity(&s.identity).id()
}

/// The wall-clock form of a session deadline.
fn wall_clock(at: Instant) -> String {
    let left = at.saturating_duration_since(Instant::now());
    let at = chrono::Utc::now() + chrono::Duration::from_std(left).unwrap_or_default();
    at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

async fn recv_stacks(
    sub: &mut Option<Subscription>,
) -> Result<Update, broadcast::error::RecvError> {
    match sub {
        Some(s) => s.rx.recv().await,
        None => std::future::pending().await,
    }
}

#[allow(clippy::too_many_lines)]
async fn run(app: AppState, session: Option<Session>, ip: IpAddr, mut socket: WebSocket) {
    let Some(session) = session else {
        return close(socket, CLOSE_SIGNED_OUT, "signed_out").await;
    };
    // Subscribe before the Hello so an end in between is not missed.
    let mut ended = app.auth.sessions.subscribe_ended();
    let mut users_changed = app.users_changed.subscribe();

    let first = tokio::time::timeout(app.ws.hello_timeout, socket.recv()).await;
    let Ok(Some(Ok(Message::Text(text)))) = first else {
        return close(socket, CLOSE_SIGNED_OUT, "signed_out").await;
    };
    let Ok(hello) = serde_json::from_str::<Hello>(text.as_str()) else {
        return close(socket, CLOSE_SIGNED_OUT, "signed_out").await;
    };
    if !web_auth::csrf::csrf_token_valid(&session.csrf, Some(hello.csrf_token.as_str())) {
        return close(socket, CLOSE_NO_ACCESS, "csrf").await;
    }
    if hello.protocol_version != PROTOCOL_VERSION {
        tracing::info!(
            client = hello.protocol_version,
            "client protocol version differs"
        );
    }
    // Still alive after the wait?
    let Some(session) = app.auth.sessions.touch(&session.id) else {
        return close(socket, CLOSE_SIGNED_OUT, "signed_out").await;
    };
    let user = user_of(&session);
    if !role::has_any_access(&app.users.access(&user)) {
        return close(socket, CLOSE_NO_ACCESS, "no_access").await;
    }
    let Some(_permit) = app.sockets.acquire(&session.id, ip) else {
        return close(socket, CLOSE_TOO_MANY, "too_many_sockets").await;
    };

    let (out_tx, mut out_rx) = mpsc::channel::<EventEnvelope>(OUT_QUEUE);
    let (registry, mut stream_rx) = streams::Registry::new();
    let mut conn = Conn {
        actor: audit::actor_of(&session.identity),
        session: session.id.clone(),
        user,
        ip,
        out: out_tx,
        stacks: None,
        users_feed: false,
        streams: registry,
        app: app.clone(),
    };
    let commands = RateLimiter::<()>::new(WS_COMMANDS_PER_MINUTE, WS_COMMAND_BURST, 1);
    let keys = RateLimiter::<()>::new(WS_SHELL_PER_MINUTE, WS_SHELL_BURST, 1);
    let ping_every = app.ws.ping_every;
    let mut ping = tokio::time::interval_at(Instant::now() + ping_every, ping_every);
    let mut missed: u32 = 0;
    let mut deadline = app.auth.sessions.deadline(&session);
    let mut warned_for: Option<Instant> = None;

    let flow = loop {
        let wake = match warned_for {
            Some(d) if d == deadline => deadline,
            _ => deadline
                .checked_sub(app.ws.expiring_lead)
                .unwrap_or(deadline),
        };
        tokio::select! {
            msg = socket.recv() => {
                let msg = match msg {
                    None | Some(Ok(Message::Close(_))) => return,
                    Some(Err(e)) => {
                        tracing::debug!(error = %e, "websocket read failed");
                        break Flow::Close(CLOSE_TOO_BIG, "message_refused");
                    }
                    Some(Ok(m)) => m,
                };
                let text = match msg {
                    Message::Text(t) => t,
                    Message::Binary(_) => break Flow::Close(CLOSE_UNSUPPORTED, "unsupported"),
                    Message::Pong(_) => { missed = 0; continue; }
                    Message::Ping(_) | Message::Close(_) => continue,
                };
                let limit = if streams::is_shell_traffic(text.as_str()) { &keys } else { &commands };
                if limit.check(&(), std::time::Instant::now()).is_err() {
                    break Flow::Close(CLOSE_TOO_MANY, "rate_limited");
                }
                let Some(s) = app.auth.sessions.touch(&conn.session) else {
                    break Flow::Close(CLOSE_SIGNED_OUT, "signed_out");
                };
                deadline = app.auth.sessions.deadline(&s);
                let access = conn.access();
                if !role::has_any_access(&access) {
                    break Flow::Close(CLOSE_NO_ACCESS, "no_access");
                }
                let replies = dispatch(&mut conn, access, text.as_str()).await;
                for r in &replies {
                    if !send(&mut socket, r).await {
                        return;
                    }
                }
            }
            Some(env) = out_rx.recv() => {
                if !send(&mut socket, &env).await {
                    return;
                }
            }
            Some(o) = stream_rx.recv() => {
                let env = match o {
                    streams::Outgoing::Data { stream, env, _credit } => {
                        conn.streams.is_open(stream).then_some(env)
                    }
                    streams::Outgoing::Stats { generation, env, _credit } => {
                        conn.streams.is_watching(generation).then_some(env)
                    }
                    streams::Outgoing::Host { generation, env, _credit } => {
                        conn.streams.is_watching_host(generation).then_some(env)
                    }
                    streams::Outgoing::End { stream, reason } => conn.streams.close(stream).then(|| {
                        reply(None, Event::StreamClosed { stream_id: stream, reason })
                    }),
                };
                if let Some(env) = env
                    && !send(&mut socket, &env).await
                {
                    return;
                }
            }
            e = ended.recv() => match e {
                Ok(e) if e.id == conn.session => {
                    let revoked = e.reason == EndReason::Revoked
                        && !role::has_any_access(&conn.access());
                    break if revoked {
                        Flow::Close(CLOSE_NO_ACCESS, "no_access")
                    } else {
                        Flow::Close(CLOSE_SIGNED_OUT, "signed_out")
                    };
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    if app.auth.sessions.peek(&conn.session).is_none() {
                        break Flow::Close(CLOSE_SIGNED_OUT, "signed_out");
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            u = users_changed.recv() => {
                if matches!(u, Err(broadcast::error::RecvError::Closed)) {
                    continue;
                }
                let access = conn.access();
                if !role::has_any_access(&access) {
                    break Flow::Close(CLOSE_NO_ACCESS, "no_access");
                }
                let mut evs = streams::revoke(&mut conn, &access);
                if conn.stacks.is_some()
                    && let Some(list) = app.watch.last()
                {
                    let ev = Event::StacksChanged(inventory::filter(&list, &access));
                    evs.push(reply(None, ev));
                }
                if conn.users_feed {
                    if role::permits(&access, None, Action::ManageUsers) {
                        evs.push(reply(None, Event::UsersChanged(app.users.list())));
                    } else {
                        conn.users_feed = false;
                    }
                }
                for r in &evs {
                    if !send(&mut socket, r).await {
                        return;
                    }
                }
            }
            u = recv_stacks(&mut conn.stacks) => {
                let access = conn.access();
                if !role::has_any_access(&access) {
                    break Flow::Close(CLOSE_NO_ACCESS, "no_access");
                }
                let ev = match u {
                    Ok(Update::Stacks(list)) => Event::StacksChanged(inventory::filter(&list, &access)),
                    Ok(Update::Unavailable(reason)) => Event::RuntimeUnavailable { reason },
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let inv = app.watch.inventory().await;
                        Event::StacksChanged(inventory::filter(&inv.list, &access))
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        conn.stacks = None;
                        continue;
                    }
                };
                if !send(&mut socket, &reply(None, ev)).await {
                    return;
                }
            }
            _ = ping.tick() => {
                if missed >= MAX_MISSED_PONGS {
                    break Flow::Close(CLOSE_GOING_AWAY, "ping_timeout");
                }
                missed += 1;
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    return;
                }
            }
            () = tokio::time::sleep_until(wake) => {
                // Other requests (HTTP, other sockets) may have moved the deadline.
                let Some(s) = app.auth.sessions.peek(&conn.session) else {
                    break Flow::Close(CLOSE_SIGNED_OUT, "signed_out");
                };
                deadline = app.auth.sessions.deadline(&s);
                let now = Instant::now();
                if now >= deadline {
                    break Flow::Close(CLOSE_SIGNED_OUT, "signed_out");
                }
                if warned_for != Some(deadline)
                    && now + app.ws.expiring_lead >= deadline
                {
                    warned_for = Some(deadline);
                    let ev = Event::SessionExpiring { at: wall_clock(deadline) };
                    if !send(&mut socket, &reply(None, ev)).await {
                        return;
                    }
                }
            }
        }
    };
    let Flow::Close(code, reason) = flow;
    close(socket, code, reason).await;
}

/// The `id` of a frame that did not parse, when it has one.
fn id_of(v: &serde_json::Value) -> Option<RequestId> {
    v.get("id")
        .and_then(serde_json::Value::as_u64)
        .map(RequestId)
}

/// A stack name in a frame that did not parse, when it is there and invalid: probing for
/// directory names is audited (T094).
fn bad_stack_name(v: &serde_json::Value) -> Option<String> {
    let data = v.get("cmd")?.get("data")?;
    ["name", "stack"].iter().find_map(|k| {
        let s = data.get(*k)?.as_str()?;
        depths_core::StackName::parse(s)
            .is_err()
            .then(|| s.chars().take(64).collect())
    })
}

#[allow(clippy::too_many_lines)]
async fn dispatch(conn: &mut Conn, access: Access, text: &str) -> Vec<EventEnvelope> {
    let env = match serde_json::from_str::<CmdEnvelope>(text) {
        Ok(env) => env,
        Err(_) => {
            let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
            let id = id_of(&v);
            if let Some(name) = bad_stack_name(&v) {
                let _ = conn.app.audit.record(&audit::denied(
                    conn.actor.clone(),
                    conn.ip,
                    None,
                    format!("invalid stack name {name:?}"),
                ));
            }
            return vec![invalid(id, "malformed command")];
        }
    };
    let id = Some(env.id);
    let app = conn.app.clone();
    match env.cmd {
        Cmd::ListStacks => {
            if conn.stacks.is_none() {
                conn.stacks = Some(app.watch.subscribe());
            }
            let inv = app.watch.inventory().await;
            let mut out = vec![reply(
                id,
                Event::Stacks(inventory::filter(&inv.list, &access)),
            )];
            if let Some(reason) = inv.unavailable {
                out.push(reply(None, Event::RuntimeUnavailable { reason }));
            }
            out
        }
        Cmd::GetStack { name } => vec![get_stack(conn, &access, id, name.as_str()).await],
        Cmd::RevealEnv { name } => vec![reveal_env(conn, &access, id, name.as_str()).await],
        Cmd::PreviewOp { stack, op } => {
            let who = conn.who(access);
            match app.ops.preview(&who, &stack, &op).await {
                Ok(p) => vec![reply(id, Event::Preview(p))],
                Err(r) => vec![reply(id, r.into())],
            }
        }
        Cmd::Confirm { preview_id } => {
            let who = conn.who(access);
            match app.ops.confirm(&who, preview_id).await {
                Ok(job) => {
                    let started = reply(
                        id,
                        Event::OpStarted {
                            operation_id: job.operation_id,
                        },
                    );
                    // Its output goes through the socket's queue, so it follows `OpStarted`.
                    let tx = conn.out.clone();
                    tokio::task::spawn_blocking(move || job.run(tx));
                    vec![started]
                }
                Err(r) => vec![reply(id, r.into())],
            }
        }
        Cmd::Cancel { operation_id } => {
            let who = conn.who(access);
            match app.ops.cancel(&who, operation_id) {
                // `OpFinished{cancelled}` follows on the socket that confirmed it.
                Ok(()) => Vec::new(),
                Err(r) => vec![reply(id, r.into())],
            }
        }
        Cmd::PreviewSave { draft, deploy } => {
            if app.cfg.features.edit {
                let who = conn.who(access);
                match app.ops.preview_save(&who, &draft, deploy).await {
                    Ok(p) => vec![reply(id, Event::Preview(p))],
                    Err(r) => vec![reply(id, r.into())],
                }
            } else {
                vec![conn.deny_reason(
                    id,
                    None,
                    DenyReason::FeatureDisabled,
                    "editing is disabled (DEPTHS_FEATURE_EDIT)".into(),
                )]
            }
        }
        Cmd::OpenShell {
            container,
            shell,
            cols,
            rows,
        } => {
            if app.cfg.features.shell {
                vec![streams::open_shell(conn, &access, id, container, shell, cols, rows).await]
            } else {
                vec![conn.deny_reason(
                    id,
                    None,
                    DenyReason::FeatureDisabled,
                    "the shell is disabled (DEPTHS_FEATURE_SHELL)".into(),
                )]
            }
        }
        Cmd::OpenLogs {
            target,
            tail,
            since,
        } => vec![streams::open_logs(conn, &access, id, target, tail, since).await],
        Cmd::ShellInput {
            stream_id,
            bytes_b64,
        } => streams::shell_input(conn, id, stream_id, bytes_b64.as_str()),
        Cmd::ShellResize {
            stream_id,
            cols,
            rows,
        } => streams::shell_resize(conn, id, stream_id, cols, rows),
        Cmd::WatchStats { targets } => streams::watch_stats(conn, &access, id, targets).await,
        Cmd::UnwatchStats => {
            streams::unwatch_stats(conn);
            Vec::new()
        }
        // Permission before the setting: someone who may not watch the host never learns whether
        // it could be watched.
        Cmd::WatchHost => {
            if !role::permits(&access, None, Action::ViewHost) {
                vec![conn.deny(id, None, "WatchHost needs view_host".into())]
            } else if let Some(proc_dir) = app.cfg.host_proc.clone() {
                streams::watch_host(conn, id, proc_dir)
            } else {
                vec![conn.deny_reason(
                    id,
                    None,
                    DenyReason::FeatureDisabled,
                    "host metrics are off (DEPTHS_HOST_PROC)".into(),
                )]
            }
        }
        Cmd::UnwatchHost => {
            streams::unwatch_host(conn);
            Vec::new()
        }
        Cmd::CloseStream { stream_id } => vec![streams::close_stream(conn, id, stream_id)],
        Cmd::GetAudit { filter, cursor } => {
            if !role::permits(&access, None, Action::ReadAudit) {
                return vec![conn.deny(id, None, "GetAudit needs read_audit".into())];
            }
            let a = app.audit.clone();
            let page = tokio::task::spawn_blocking(move || a.page(&filter, cursor.as_ref())).await;
            match page {
                Ok(Ok((entries, next))) => vec![reply(
                    id,
                    Event::AuditPage {
                        entries,
                        next_cursor: next.map(|c| c.as_str().to_owned()),
                    },
                )],
                Ok(Err(audit::PageError::BadCursor)) => {
                    vec![invalid(id, "the audit cursor is not valid")]
                }
                Ok(Err(audit::PageError::Io(e))) => {
                    tracing::error!(error = %e, "audit log read failed");
                    vec![invalid(id, "the audit log could not be read")]
                }
                Err(_) => vec![invalid(id, "the audit log could not be read")],
            }
        }
        Cmd::GetUsers => {
            if !role::permits(&access, None, Action::ManageUsers) {
                return vec![conn.deny(id, None, "GetUsers needs manage_users".into())];
            }
            conn.users_feed = true;
            vec![reply(id, Event::Users(app.users.list()))]
        }
        Cmd::PutUser {
            id: target,
            role,
            stacks,
        } => {
            if !role::permits(&access, None, Action::ManageUsers) {
                return vec![conn.deny(id, None, "PutUser needs manage_users".into())];
            }
            let r = app.users.put(&target, role, &stacks);
            vec![user_changed(conn, id, &target, r)]
        }
        Cmd::RemoveUser { id: target } => {
            if !role::permits(&access, None, Action::ManageUsers) {
                return vec![conn.deny(id, None, "RemoveUser needs manage_users".into())];
            }
            let r = app.users.remove(&target);
            vec![user_changed(conn, id, &target, r)]
        }
    }
}

/// The answer to `PutUser`/`RemoveUser`: audit, end the person's sessions when they are left
/// with no access (their sockets close `4403`), tell every socket.
fn user_changed(
    conn: &Conn,
    id: Option<RequestId>,
    target: &UserId,
    r: Result<users::Changed, users::Refusal>,
) -> EventEnvelope {
    let app = &conn.app;
    match r {
        Ok(changed) => {
            if let Err(e) = app.audit.record(&audit::user_change(
                conn.actor.clone(),
                conn.ip,
                changed.added,
                changed.change,
            )) {
                tracing::error!(error = %e, "audit write failed for a users change");
            }
            if changed.revoke {
                let gone = app
                    .auth
                    .sessions
                    .end_matching(|s| user_of(s) == target.as_str(), EndReason::Revoked);
                tracing::info!(user = %target, sessions = gone.len(), "access revoked");
            }
            let _ = app.users_changed.send(());
            reply(id, Event::Users(app.users.list()))
        }
        Err(users::Refusal::Denied(reason)) => conn.deny_reason(
            id,
            None,
            reason,
            format!("users change for {target} refused: {reason:?}"),
        ),
        Err(users::Refusal::Invalid(why)) => invalid(id, why),
        Err(users::Refusal::Failed(e)) => {
            tracing::error!(error = %e, "users registry write failed");
            invalid(id, "could not write the users registry")
        }
    }
}

async fn read_files(
    conn: &Conn,
    name: &depths_core::StackName,
) -> Result<depths_core::ports::StackFiles, String> {
    let store = conn.app.store.clone();
    let n = name.clone();
    match tokio::task::spawn_blocking(move || store.read(&n)).await {
        Ok(Ok(f)) => Ok(f),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("the server could not complete the request".into()),
    }
}

async fn get_stack(
    conn: &Conn,
    access: &Access,
    id: Option<RequestId>,
    name: &str,
) -> EventEnvelope {
    if !inventory::visible(access, name) {
        return conn.deny(id, Some(name), "GetStack on a stack not granted".into());
    }
    let Ok(core) = depths_core::StackName::parse(name) else {
        return invalid(id, "invalid stack name");
    };
    let list = match conn.app.watch.last() {
        Some(l) if l.stacks.iter().any(|s| s.name == name) => l,
        _ => conn.app.watch.inventory().await.list,
    };
    let Some(stack) = list.stacks.iter().find(|s| s.name == name).cloned() else {
        return invalid(id, format!("no stack named {name}"));
    };
    let files = read_files(conn, &core).await.ok();
    reply(id, Event::StackDetail(inventory::detail(stack, files)))
}

/// `RevealEnv`: audited before anything is sent; if the audit cannot be written, nothing is.
async fn reveal_env(
    conn: &Conn,
    access: &Access,
    id: Option<RequestId>,
    name: &str,
) -> EventEnvelope {
    let Ok(core) = depths_core::StackName::parse(name) else {
        return conn.deny(id, Some(name), "RevealEnv on an invalid stack name".into());
    };
    if !role::permits(access, Some(&core), Action::RevealEnv) {
        return conn.deny(id, Some(name), "RevealEnv on a stack not granted".into());
    }
    let files = match read_files(conn, &core).await {
        Ok(f) => f,
        Err(e) => return invalid(id, e),
    };
    let Some(env_text) = files.env else {
        return invalid(id, format!("{name} has no .env file"));
    };
    let mut e = audit::entry(
        conn.actor.clone(),
        AuditAction::RevealEnv,
        conn.ip,
        AuditOutcome::Succeeded,
    );
    e.stack = Some(name.to_owned());
    // Which secrets were shown, by key name only: never a value.
    let keys = depths_core::secrets::secret_keys(&env_text);
    e.diff_summary = Some(if keys.is_empty() {
        "revealed .env: no secret-looking keys".into()
    } else {
        format!("revealed .env keys: {}", keys.join(", "))
    });
    if let Err(err) = conn.app.audit.record(&e) {
        tracing::error!(error = %err, "audit write failed; not revealing");
        return invalid(id, "the audit log could not be written");
    }
    reply(
        id,
        Event::EnvRevealed {
            name: name.to_owned(),
            env_text,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_cap_is_two_files_and_an_envelope() {
        assert_eq!(max_message_bytes(1024 * 1024), 2 * 1024 * 1024 + 64 * 1024);
        assert_eq!(max_message_bytes(u64::MAX), usize::MAX);
    }

    #[test]
    fn a_malformed_frame_keeps_its_id_and_names_a_bad_stack() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"id":7,"cmd":{"type":"GetStack","data":{"name":"../etc"}}}"#)
                .unwrap();
        assert_eq!(id_of(&v), Some(RequestId(7)));
        assert_eq!(bad_stack_name(&v).as_deref(), Some("../etc"));
        let ok: serde_json::Value =
            serde_json::from_str(r#"{"id":1,"cmd":{"type":"GetStack","data":{"name":"web"}}}"#)
                .unwrap();
        assert_eq!(bad_stack_name(&ok), None);
    }
}
