//! `/ws` for the hosted shell (spec 031, T026): session + exact-Origin checks before the upgrade,
//! the `DevBridgeClientMsg`/`DevBridgeServerMsg` frames, per-frame command filter, and per-session
//! teardown (close 4401 on sign-out/expiry, 1001 on shutdown).
//!
//! axum's `ws` feature is deliberately not enabled (tokio-tungstenite version conflict), so the
//! upgrade is done with hyper's `OnUpgrade` and a tokio-tungstenite 0.27 server handshake.

use async_trait::async_trait;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use fathom::protocol::{
    BackendCmd, BridgeError, DevBridgeClientMsg, DevBridgeServerMsg, LimitKind, PublicIdentity,
    PublicLimits, RequestId, ShellMode, SubscriptionInfo,
};
use futures::{SinkExt, StreamExt};
use hyper_util::rt::TokioIo;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role, WebSocketConfig};

use crate::filter;
use crate::http::{AppState, json_error, session_from_headers};
use crate::limits::LimitReached;
use crate::session::{EndReason, Session};

pub const CLOSE_SESSION_ENDED: u16 = 4401;
pub const CLOSE_TAB_LIMIT: u16 = 4429;
pub const CLOSE_ORIGIN: u16 = 4403;
const MAX_FRAME: usize = 1 << 20;

/// Why a tab could not attach to the user's backend.
#[derive(Debug)]
pub enum AttachError {
    Limit(LimitReached),
    Unavailable(String),
}

/// One browser tab's handle on the signed-in user's backend. Dropping it detaches the tab and
/// releases its tab permit.
pub trait UserConn: Send + Sync {
    /// Forward an already-filtered command to the user's backend.
    fn send(&self, cmd: BackendCmd) -> Result<RequestId, BridgeError>;
    /// Snapshot for `subscribed` (the shell fills `mode`/`identity`/`limits` itself).
    fn subscription_info(&self) -> Result<SubscriptionInfo, BridgeError>;
    /// Attach this tab to the user's event hub: flush the backlog, then stream live batches.
    fn subscribe(&self) -> Result<(), BridgeError>;
}

/// The per-user backend host (implemented on top of `tenant.rs`).
#[async_trait]
pub trait TenantHost: Send + Sync + 'static {
    /// `/readyz`: `Err(reason)` while the home cluster is unreachable.
    async fn ready(&self) -> Result<(), String>;
    /// The hosted home context (the only one a session may select).
    fn home_context(&self) -> String;
    fn public_limits(&self) -> PublicLimits;
    /// Attach a tab for `session`, creating/reusing the user's backend. Event-batch frames
    /// (`{"op":"batch",...}` JSON text) are pushed to `frames`.
    async fn attach(
        &self,
        session: &Session,
        frames: UnboundedSender<String>,
    ) -> Result<Box<dyn UserConn>, AttachError>;
}

/// Fail-closed placeholder used until a real host is wired: never ready, refuses tabs.
pub struct UnwiredHost;

#[async_trait]
impl TenantHost for UnwiredHost {
    async fn ready(&self) -> Result<(), String> {
        Err("no tenant host configured".into())
    }
    fn home_context(&self) -> String {
        "in-cluster".into()
    }
    fn public_limits(&self) -> PublicLimits {
        PublicLimits {
            max_watches: 0,
            max_streams: 0,
            max_tabs: 0,
        }
    }
    async fn attach(
        &self,
        _: &Session,
        _: UnboundedSender<String>,
    ) -> Result<Box<dyn UserConn>, AttachError> {
        Err(AttachError::Unavailable("no tenant host configured".into()))
    }
}

fn header_str<'a>(h: &'a HeaderMap, name: &str) -> Option<&'a str> {
    h.get(name).and_then(|v| v.to_str().ok())
}

fn has_token(h: &HeaderMap, name: &str, token: &str) -> bool {
    header_str(h, name).is_some_and(|v| v.split(',').any(|t| t.trim().eq_ignore_ascii_case(token)))
}

fn limit_kind_name(k: LimitKind) -> &'static str {
    match k {
        LimitKind::Sessions => "sessions",
        LimitKind::Watches => "watches",
        LimitKind::Streams => "streams",
        LimitKind::Tabs => "tabs",
    }
}

enum Out {
    Text(String),
    Close(u16, &'static str),
}

pub async fn ws_handler(State(app): State<AppState>, mut req: Request) -> Response {
    let headers = req.headers().clone();
    let Some(session) = session_from_headers(&app, &headers) else {
        return json_error(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            Some("/auth/login"),
        );
    };
    if !crate::csrf::origin_allowed(&app.origin, header_str(&headers, "origin")) {
        tracing::warn!("ws: rejected origin {:?}", header_str(&headers, "origin"));
        return json_error(StatusCode::FORBIDDEN, "origin", None);
    }
    let key = match header_str(&headers, "sec-websocket-key") {
        Some(k)
            if has_token(&headers, "connection", "upgrade")
                && has_token(&headers, "upgrade", "websocket")
                && header_str(&headers, "sec-websocket-version") == Some("13") =>
        {
            k.to_owned()
        }
        _ => return (StatusCode::BAD_REQUEST, "websocket upgrade required").into_response(),
    };
    let Some(on_upgrade) = req.extensions_mut().remove::<hyper::upgrade::OnUpgrade>() else {
        return (StatusCode::BAD_REQUEST, "upgrade not supported").into_response();
    };

    let (frames_tx, frames_rx) = unbounded_channel::<String>();
    let conn = match app.host.attach(&session, frames_tx).await {
        Ok(c) => c,
        Err(AttachError::Limit(l)) => {
            return (StatusCode::TOO_MANY_REQUESTS, axum::Json(limit_body(l))).into_response();
        }
        Err(AttachError::Unavailable(why)) => {
            tracing::warn!("ws: backend unavailable: {why}");
            return json_error(StatusCode::SERVICE_UNAVAILABLE, "unavailable", None);
        }
    };

    let app2 = app.clone();
    tokio::spawn(async move {
        let Ok(upgraded) = on_upgrade.await else {
            return;
        };
        let cfg = WebSocketConfig::default()
            .max_message_size(Some(MAX_FRAME))
            .max_frame_size(Some(MAX_FRAME));
        let ws =
            WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, Some(cfg)).await;
        run(app2, session, conn, ws, frames_rx).await;
    });

    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(header::CONNECTION, "Upgrade")
        .header(header::UPGRADE, "websocket")
        .header("sec-websocket-accept", derive_accept_key(key.as_bytes()))
        .body(Body::empty())
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn reply(tx: &UnboundedSender<Out>, msg: &DevBridgeServerMsg) {
    match serde_json::to_string(msg) {
        Ok(json) => {
            let _ = tx.send(Out::Text(json));
        }
        Err(e) => tracing::error!("ws: failed to serialize reply: {e}"),
    }
}

fn to_close(reason: EndReason) -> (u16, &'static str) {
    let _ = reason;
    (CLOSE_SESSION_ENDED, "session ended")
}

async fn run<S>(
    app: AppState,
    session: Session,
    conn: Box<dyn UserConn>,
    ws: WebSocketStream<S>,
    mut frames_rx: tokio::sync::mpsc::UnboundedReceiver<String>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut write, mut read) = ws.split();
    let (out_tx, mut out_rx) = unbounded_channel::<Out>();

    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(std::time::Duration::from_secs(30));
        ping.tick().await;
        loop {
            // Replies (`subscribed`) must precede the backlog batches pushed right after them.
            let msg = tokio::select! {
                biased;
                o = out_rx.recv() => match o {
                    Some(Out::Text(t)) => Message::text(t),
                    Some(Out::Close(code, why)) => {
                        let _ = write.send(Message::Close(Some(CloseFrame {
                            code: CloseCode::from(code),
                            reason: why.into(),
                        }))).await;
                        break;
                    }
                    None => break,
                },
                f = frames_rx.recv() => match f {
                    Some(t) => Message::text(t),
                    None => break,
                },
                _ = ping.tick() => Message::Ping(Vec::new().into()),
            };
            if write.send(msg).await.is_err() {
                break;
            }
        }
        let _ = write.close().await;
    });

    let mut ended = app.sessions.subscribe_ended();
    let mut shutdown = app.shutdown.subscribe();
    let home = app.host.home_context();
    let public_identity = PublicIdentity {
        username: session.identity.username.clone(),
        groups: session.identity.groups.clone(),
    };

    loop {
        tokio::select! {
            e = ended.recv() => match e {
                Ok(e) if e.id == session.id => {
                    let (c, w) = to_close(e.reason);
                    let _ = out_tx.send(Out::Close(c, w));
                    break;
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Missed notifications: confirm the session still exists.
                    if app.sessions.touch(&session.id).is_none() {
                        let _ = out_tx.send(Out::Close(CLOSE_SESSION_ENDED, "session ended"));
                        break;
                    }
                }
                Err(_) => break,
            },
            _ = shutdown.changed() => {
                let _ = out_tx.send(Out::Close(1001, "server shutting down"));
                break;
            }
            msg = read.next() => {
                let text = match msg {
                    Some(Ok(Message::Text(t))) => t,
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(Message::Binary(_))) => {
                        let _ = out_tx.send(Out::Close(1003, "binary frames are not supported"));
                        break;
                    }
                    Some(Ok(_)) => continue,
                };
                // Every frame counts as activity and re-validates the session.
                if app.sessions.touch(&session.id).is_none() {
                    let _ = out_tx.send(Out::Close(CLOSE_SESSION_ENDED, "session ended"));
                    break;
                }
                let parsed: DevBridgeClientMsg = match serde_json::from_str(text.as_str()) {
                    Ok(p) => p,
                    Err(_) => {
                        let _ = out_tx.send(Out::Close(1007, "malformed message"));
                        break;
                    }
                };
                match parsed {
                    DevBridgeClientMsg::Send { cmd } => {
                        let result = match filter::check(&cmd, &home) {
                            Ok(()) => conn.send(cmd),
                            Err(denied) => {
                                tracing::info!("ws: denied for {}: {denied}", session.identity.username);
                                Err(BridgeError::Forbidden(denied.to_string()))
                            }
                        };
                        reply(&out_tx, &match result {
                            Ok(request_id) => DevBridgeServerMsg::Sent { request_id },
                            Err(error) => DevBridgeServerMsg::Error { error },
                        });
                    }
                    DevBridgeClientMsg::Subscribe => match conn.subscription_info() {
                        Ok(mut info) => {
                            info.mode = Some(ShellMode::Hosted);
                            info.identity = Some(public_identity.clone());
                            info.limits = Some(app.host.public_limits());
                            reply(&out_tx, &DevBridgeServerMsg::Subscribed { info });
                            if let Err(error) = conn.subscribe() {
                                reply(&out_tx, &DevBridgeServerMsg::Error { error });
                            }
                        }
                        Err(error) => reply(&out_tx, &DevBridgeServerMsg::Error { error }),
                    },
                }
            }
        }
    }

    drop(out_tx);
    drop(conn);
    let _ = writer.await;
}

/// Public helper for the 429 body shape shared with the HTTP layer.
pub fn limit_body(l: LimitReached) -> serde_json::Value {
    serde_json::json!({"error":"limit","kind":limit_kind_name(l.kind),"max":l.max})
}
