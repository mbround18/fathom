//! `GET /api/v1/ws` (spec 055 T026, contracts/protocol.md "Transport").
//!
//! The upgrade is refused (403 `{"error":"origin"}`) unless `Origin` is Keel's own; a request
//! that is not an upgrade gets 426. After the upgrade the first frame must be a `Hello` within
//! [`HELLO_TIMEOUT`]:
//!
//! - no session, a timeout, or a first frame that is not a `Hello`: close `4401`;
//! - a `protocol_version` other than [`PROTOCOL_VERSION`]: close `4400`;
//! - a `csrf_token` that does not match the session: close `4403`.
//!
//! Then every command touches the session and re-reads the caller from the database, so a
//! changed `authz_epoch`, a disabled user or an expired session ends the socket at the next
//! command with a `Revoked` push and close `4401`. The socket does not wait for a command to
//! notice: an ended session, or an epoch change announced through
//! [`crate::serve::AppState::epoch_changed`], is checked straight away. Binary frames close
//! `1003`; a message over [`MAX_MESSAGE_BYTES`] ends the socket.

use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use keel_protocol::{CmdEnvelope, Event, EventEnvelope, Hello, PROTOCOL_VERSION};
use tokio::sync::broadcast::error::RecvError;
use web_auth::session::{EndReason, Session};

use crate::dispatch::{self, Channel, Request};
use crate::identity::{self, Caller};
use crate::serve::{AppState, error};
use keel_core::authz::Capability;

pub const CLOSE_GOING_AWAY: u16 = 1001;
pub const CLOSE_UNSUPPORTED: u16 = 1003;
pub const CLOSE_PROTOCOL: u16 = 4400;
pub const CLOSE_SIGNED_OUT: u16 = 4401;
pub const CLOSE_CSRF: u16 = 4403;
/// How long the client has to send its `Hello`.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// The largest client message.
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;
/// How long a closing socket waits for the client's close frame.
const CLOSE_GRACE: Duration = Duration::from_secs(1);

pub async fn upgrade(
    State(app): State<AppState>,
    headers: HeaderMap,
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    if !web_auth::csrf::origin_allowed(&app.origin, origin) {
        return error(StatusCode::FORBIDDEN, "origin");
    }
    let Ok(ws) = ws else {
        return error(StatusCode::UPGRADE_REQUIRED, "websocket_required");
    };
    let session = app.auth.session(&headers);
    ws.max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| run(app, session, socket))
        .into_response()
}

/// Sends the close frame, then waits briefly for the client's, so it is read before the
/// connection drops.
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

/// `Revoked`, then close `4401`, and the session ends everywhere.
async fn revoke(app: &AppState, session: &str, mut socket: WebSocket) {
    app.auth.sessions.end(session, EndReason::Revoked);
    app.previews.forget_session(session);
    let _ = send(&mut socket, &EventEnvelope::push(Event::Revoked)).await;
    close(socket, CLOSE_SIGNED_OUT, "revoked").await;
}

/// What re-reading the caller found.
enum Check {
    Valid(Box<Caller>),
    Gone,
    Unavailable,
}

async fn check(app: &AppState, session: &str) -> Check {
    if app.auth.sessions.touch(session).is_none() {
        return Check::Gone;
    }
    match identity::load_session(&app.db, session, app.clock.now()).await {
        Ok(Some(c)) => Check::Valid(Box::new(c)),
        Ok(None) => Check::Gone,
        Err(e) => {
            tracing::error!(error = %e, "session lookup failed");
            Check::Unavailable
        }
    }
}

async fn run(app: AppState, session: Option<Session>, mut socket: WebSocket) {
    let Some(session) = session else {
        return close(socket, CLOSE_SIGNED_OUT, "signed_out").await;
    };
    // Subscribed before the Hello, so an end that races it is not missed.
    let mut ended = app.auth.sessions.subscribe_ended();
    let mut epochs = app.revocations.subscribe();
    let mut pushed = app.repos.events.subscribe();

    let first = tokio::time::timeout(HELLO_TIMEOUT, socket.recv()).await;
    let Ok(Some(Ok(Message::Text(text)))) = first else {
        return close(socket, CLOSE_SIGNED_OUT, "hello").await;
    };
    let Ok(hello) = serde_json::from_str::<Hello>(&text) else {
        return close(socket, CLOSE_SIGNED_OUT, "hello").await;
    };
    if hello.protocol_version != PROTOCOL_VERSION {
        return close(socket, CLOSE_PROTOCOL, "protocol_version").await;
    }
    if !web_auth::csrf::csrf_token_valid(&session.csrf, Some(hello.csrf_token.as_str())) {
        return close(socket, CLOSE_CSRF, "csrf").await;
    }
    let user = match check(&app, &session.id).await {
        Check::Valid(c) => c.user.id,
        Check::Gone => return revoke(&app, &session.id, socket).await,
        Check::Unavailable => return close(socket, CLOSE_GOING_AWAY, "unavailable").await,
    };

    loop {
        tokio::select! {
            m = socket.recv() => match m {
                Some(Ok(Message::Text(text))) => {
                    let caller = match check(&app, &session.id).await {
                        Check::Valid(c) => c,
                        Check::Gone => return revoke(&app, &session.id, socket).await,
                        Check::Unavailable => {
                            let env = EventEnvelope::push(Event::op_error(
                                "unavailable",
                                "Keel cannot reach its database. Try again.",
                            ));
                            if !send(&mut socket, &env).await {
                                return;
                            }
                            continue;
                        }
                    };
                    let Ok(env) = serde_json::from_str::<CmdEnvelope>(&text) else {
                        let env = EventEnvelope::push(Event::op_error(
                            "bad_request",
                            "That message is not a command.",
                        ));
                        if !send(&mut socket, &env).await {
                            return;
                        }
                        continue;
                    };
                    let req = Request {
                        caller: &caller,
                        session: Some(&session.id),
                        channel: Channel::Socket,
                    };
                    for out in dispatch::dispatch(&app, &req, env).await {
                        if !send(&mut socket, &out).await {
                            return;
                        }
                    }
                }
                Some(Ok(Message::Binary(_))) => {
                    return close(socket, CLOSE_UNSUPPORTED, "binary").await;
                }
                Some(Ok(Message::Close(_)) | Err(_)) | None => return,
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            },
            e = ended.recv() => match e {
                Ok(e) if e.id == session.id => {
                    let _ = send(&mut socket, &EventEnvelope::push(Event::Revoked)).await;
                    return close(socket, CLOSE_SIGNED_OUT, "revoked").await;
                }
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => {
                    if app.auth.sessions.peek(&session.id).is_none() {
                        return revoke(&app, &session.id, socket).await;
                    }
                }
                Err(RecvError::Closed) => {
                    return close(socket, CLOSE_GOING_AWAY, "shutting_down").await;
                }
            },
            u = epochs.recv() => match u {
                Ok(u) if u != user => {}
                Ok(_) | Err(RecvError::Lagged(_)) => {
                    if matches!(check(&app, &session.id).await, Check::Gone) {
                        return revoke(&app, &session.id, socket).await;
                    }
                }
                Err(RecvError::Closed) => {
                    return close(socket, CLOSE_GOING_AWAY, "shutting_down").await;
                }
            },
            // Bindings and receivers (US1): pushed to callers who may view the workspace,
            // checked against the session as it is now.
            p = pushed.recv() => match p {
                Ok((workspace, event)) => match check(&app, &session.id).await {
                    Check::Valid(c) => {
                        let subject = crate::bindings::subject(&c);
                        if keel_core::authz::require(&subject, Capability::View, workspace, &[])
                            .is_ok()
                            && !send(&mut socket, &EventEnvelope::push(event)).await
                        {
                            return;
                        }
                    }
                    Check::Gone => return revoke(&app, &session.id, socket).await,
                    Check::Unavailable => {}
                },
                // Missed pushes: the page re-reads on its next command.
                Err(RecvError::Lagged(_) | RecvError::Closed) => {}
            },
        }
    }
}
