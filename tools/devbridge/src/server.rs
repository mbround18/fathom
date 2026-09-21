//! The WebSocket server: handshake checks, the JSON op protocol, and the event sink.

use crate::auth::{HandshakeError, check_handshake};
use crate::guard::WriteGuard;
use fathom::bridge::pump::{BatchSink, Hub, SinkError};
use fathom::bridge::{BackendLink, EventBatchView};
use fathom::protocol::{BackendEventEnvelope, BridgeError, DevBridgeClientMsg, DevBridgeServerMsg};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

/// Pushes event batches to one WebSocket connection's writer task.
pub struct WsSink(UnboundedSender<Message>);

/// `{"op":"batch","batch":{...}}` without cloning the envelopes.
#[derive(Serialize)]
struct BatchFrame<'a> {
    op: &'static str,
    batch: EventBatchView<'a>,
}

impl BatchSink<BackendEventEnvelope> for WsSink {
    fn send(&mut self, envelopes: &[BackendEventEnvelope], dropped: u32) -> Result<(), SinkError> {
        let json = serde_json::to_string(&BatchFrame {
            op: "batch",
            batch: EventBatchView { envelopes, dropped },
        })
        .map_err(|e| SinkError(format!("serialize batch frame: {e}")))?;
        self.0
            .send(Message::text(json))
            .map_err(|_| SinkError("websocket connection closed".to_owned()))
    }
}

pub type WsHub = Hub<BackendEventEnvelope, WsSink>;

/// Everything a connection needs. One backend, one hub: a new `subscribe` replaces the
/// previous subscriber, exactly like a webview reload in the Tauri shell.
pub struct Bridge {
    pub link: BackendLink,
    pub hub: Arc<WsHub>,
    pub guard: WriteGuard,
    pub token: String,
}

/// Accept connections on `listener` forever.
pub async fn serve(listener: TcpListener, bridge: Arc<Bridge>) -> std::io::Result<()> {
    loop {
        let (stream, peer) = listener.accept().await?;
        let bridge = Arc::clone(&bridge);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, &bridge).await {
                tracing::debug!("devbridge connection from {peer} ended: {e}");
            }
        });
    }
}

fn reject(status: StatusCode, body: &str) -> ErrorResponse {
    let mut response = ErrorResponse::new(Some(body.to_owned()));
    *response.status_mut() = status;
    response
}

async fn handle_connection(stream: TcpStream, bridge: &Bridge) -> anyhow::Result<()> {
    // The `Err` type (`http::Response`) is dictated by tungstenite's `Callback` trait.
    #[allow(clippy::result_large_err)]
    let callback = |request: &Request, response: Response| {
        let origin = request
            .headers()
            .get("origin")
            .and_then(|v| v.to_str().ok());
        match check_handshake(origin, request.uri().query(), &bridge.token) {
            Ok(()) => Ok(response),
            Err(HandshakeError::BadOrigin) => {
                tracing::warn!("devbridge: rejected connection with origin {origin:?}");
                Err(reject(StatusCode::FORBIDDEN, "origin not allowed"))
            }
            Err(HandshakeError::BadToken) => {
                tracing::warn!("devbridge: rejected connection with a missing or bad token");
                Err(reject(StatusCode::UNAUTHORIZED, "bad token"))
            }
        }
    };
    let ws = tokio_tungstenite::accept_hdr_async(stream, callback).await?;
    let (mut write, mut read) = ws.split();

    let (tx, mut rx) = unbounded_channel::<Message>();
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let closing = matches!(msg, Message::Close(_));
            if write.send(msg).await.is_err() || closing {
                break;
            }
        }
        let _ = write.close().await;
    });

    while let Some(msg) = read.next().await {
        let text = match msg? {
            Message::Text(text) => text,
            Message::Close(_) => break,
            Message::Binary(_) => {
                close(
                    &tx,
                    CloseCode::Unsupported,
                    "binary frames are not supported",
                );
                break;
            }
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
        };
        let parsed: DevBridgeClientMsg = match serde_json::from_str(text.as_str()) {
            Ok(parsed) => parsed,
            Err(e) => {
                tracing::warn!("devbridge: malformed message, closing: {e}");
                close(&tx, CloseCode::Invalid, "malformed message");
                break;
            }
        };
        handle_message(parsed, bridge, &tx);
    }

    drop(tx);
    let _ = writer.await;
    Ok(())
}

fn close(tx: &UnboundedSender<Message>, code: CloseCode, reason: &str) {
    let _ = tx.send(Message::Close(Some(CloseFrame {
        code,
        reason: reason.to_owned().into(),
    })));
}

fn reply(tx: &UnboundedSender<Message>, msg: &DevBridgeServerMsg) {
    match serde_json::to_string(msg) {
        Ok(json) => {
            let _ = tx.send(Message::text(json));
        }
        Err(e) => tracing::error!("devbridge: failed to serialize reply: {e}"),
    }
}

/// Handle one parsed client op, replying on `tx`.
pub fn handle_message(msg: DevBridgeClientMsg, bridge: &Bridge, tx: &UnboundedSender<Message>) {
    match msg {
        DevBridgeClientMsg::Send { cmd } => {
            let result = match bridge.guard.check(&cmd) {
                Ok(()) => bridge.link.send(cmd),
                Err(reason) => {
                    tracing::warn!("devbridge: FORBIDDEN {reason}");
                    Err(BridgeError::Forbidden(reason))
                }
            };
            let reply_msg = match result {
                Ok(request_id) => DevBridgeServerMsg::Sent { request_id },
                Err(error) => DevBridgeServerMsg::Error { error },
            };
            reply(tx, &reply_msg);
        }
        DevBridgeClientMsg::Subscribe => match bridge.link.subscription_info() {
            Ok(info) => {
                // `subscribed` goes out first; the backlog flush and live batches follow.
                reply(tx, &DevBridgeServerMsg::Subscribed { info });
                if let Err(e) = bridge.hub.subscribe(WsSink(tx.clone())) {
                    tracing::warn!("devbridge: subscribe flush failed: {e}");
                }
            }
            Err(error) => reply(tx, &DevBridgeServerMsg::Error { error }),
        },
    }
}
