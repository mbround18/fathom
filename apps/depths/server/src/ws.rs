//! T038 (first slice): `GET /api/v1/ws`.
//!
//! The upgrade is refused (403 JSON) unless `Origin` is the app's own. After the upgrade the
//! first frame must be `Hello{protocol_version, csrf_token}` within [`HELLO_TIMEOUT`]:
//!
//! - no session, a wrong token, a timeout or a non-Hello first frame: close `4401` (signed out);
//! - a `pending` person: close `4403` (no access);
//! - otherwise every command is answered with `Invalid{reason:"not available yet"}` until the
//!   stack commands land. The socket closes `4401` the moment the session ends.

use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::response::{IntoResponse, Response};
use depths_protocol::{CmdEnvelope, Event, EventEnvelope, Hello, PROTOCOL_VERSION, Role};
use http::{HeaderMap, StatusCode, header};
use web_auth::session::Session;

use crate::http::{AppState, error};
use crate::users::SignIn;

pub const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
pub const CLOSE_SIGNED_OUT: u16 = 4401;
pub const CLOSE_NO_ACCESS: u16 = 4403;
pub const CLOSE_TOO_MANY: u16 = 4429;
/// Largest client frame. Commands are small; file saves are bounded by `max_file_bytes` later.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

pub async fn upgrade(
    State(app): State<AppState>,
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
    let session = app.auth.session(&headers);
    ws.max_message_size(MAX_FRAME_BYTES)
        .max_frame_size(MAX_FRAME_BYTES)
        .on_upgrade(move |socket| run(app, session, socket))
        .into_response()
}

async fn close(mut socket: WebSocket, code: u16, reason: &'static str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

async fn send(
    socket: &mut WebSocket,
    reply_to: Option<depths_protocol::RequestId>,
    event: Event,
) -> bool {
    let Ok(text) = serde_json::to_string(&EventEnvelope { reply_to, event }) else {
        return false;
    };
    socket.send(Message::Text(text.into())).await.is_ok()
}

async fn run(app: AppState, session: Option<Session>, mut socket: WebSocket) {
    let Some(session) = session else {
        return close(socket, CLOSE_SIGNED_OUT, "signed_out").await;
    };
    // Subscribe before the Hello so an end in between is not missed.
    let mut ended = app.auth.sessions.subscribe_ended();

    let first = tokio::time::timeout(HELLO_TIMEOUT, socket.recv()).await;
    let Ok(Some(Ok(Message::Text(text)))) = first else {
        return close(socket, CLOSE_SIGNED_OUT, "signed_out").await;
    };
    let Ok(hello) = serde_json::from_str::<Hello>(text.as_str()) else {
        return close(socket, CLOSE_SIGNED_OUT, "signed_out").await;
    };
    if !web_auth::csrf::csrf_token_valid(&session.csrf, Some(hello.csrf_token.as_str())) {
        return close(socket, CLOSE_SIGNED_OUT, "signed_out").await;
    }
    if hello.protocol_version != PROTOCOL_VERSION {
        tracing::info!(
            client = hello.protocol_version,
            "client protocol version differs"
        );
    }
    // Still alive after the wait?
    if app.auth.sessions.touch(&session.id).is_none() {
        return close(socket, CLOSE_SIGNED_OUT, "signed_out").await;
    }
    let who = SignIn::from_identity(&session.identity);
    let role = app.users.get(&who.id()).map_or(Role::Pending, |u| u.role);
    if role == Role::Pending {
        return close(socket, CLOSE_NO_ACCESS, "no_access").await;
    }

    loop {
        tokio::select! {
            e = ended.recv() => match e {
                Ok(e) if e.id == session.id => {
                    return close(socket, CLOSE_SIGNED_OUT, "signed_out").await;
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    if app.auth.sessions.touch(&session.id).is_none() {
                        return close(socket, CLOSE_SIGNED_OUT, "signed_out").await;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            },
            msg = socket.recv() => {
                let Some(Ok(msg)) = msg else { return };
                match msg {
                    Message::Text(text) => {
                        if app.auth.sessions.touch(&session.id).is_none() {
                            return close(socket, CLOSE_SIGNED_OUT, "signed_out").await;
                        }
                        let reply = match serde_json::from_str::<CmdEnvelope>(text.as_str()) {
                            Ok(env) => (Some(env.id), "not available yet"),
                            Err(_) => (None, "malformed command"),
                        };
                        let event = Event::Invalid { reason: reply.1.into() };
                        if !send(&mut socket, reply.0, event).await {
                            return;
                        }
                    }
                    Message::Close(_) => return,
                    // Binary frames are not part of the protocol.
                    Message::Binary(_) => {
                        return close(socket, 1003, "unsupported").await;
                    }
                    Message::Ping(_) | Message::Pong(_) => {}
                }
            }
        }
    }
}
