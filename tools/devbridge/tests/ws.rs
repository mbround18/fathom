//! End-to-end checks of the WebSocket surface on an ephemeral loopback port, with a fake
//! backend (a bare command channel). No cluster involved.

// tungstenite's `Error` is what `connect_async` returns; boxing it would only obscure the tests.
#![allow(clippy::result_large_err)]

use fathom::bridge::BackendLink;
use fathom::bridge::pump::BUFFER_CAP;
use fathom::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, CmdRequest, Correlation, RequestId, Route,
    SystemEvent, section_keys,
};
use fathom_devbridge::guard::WriteGuard;
use fathom_devbridge::server::{Bridge, WsHub, serve};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const TOKEN: &str = "s3cret+token";
const GOOD_ORIGIN: &str = "http://localhost:5173";

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Harness {
    addr: SocketAddr,
    cmds: crossbeam_channel::Receiver<CmdRequest>,
    bridge: Arc<Bridge>,
}

async fn start() -> Harness {
    let (tx, cmds) = crossbeam_channel::unbounded();
    let bridge = Arc::new(Bridge {
        link: BackendLink::new(tx),
        hub: Arc::new(WsHub::new(BUFFER_CAP)),
        guard: WriteGuard::new("kind-dev", "fathom-dev"),
        token: TOKEN.to_owned(),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(serve(listener, Arc::clone(&bridge)));
    Harness { addr, cmds, bridge }
}

async fn connect(
    addr: SocketAddr,
    token: Option<&str>,
    origin: Option<&str>,
) -> Result<Ws, WsError> {
    let url = match token {
        Some(t) => format!("ws://{addr}/?token={}", urlencoding::encode(t)),
        None => format!("ws://{addr}/"),
    };
    let mut request = url.into_client_request().unwrap();
    if let Some(origin) = origin {
        request
            .headers_mut()
            .insert("origin", HeaderValue::from_str(origin).unwrap());
    }
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(ws, _)| ws)
}

fn rejected_status(result: Result<Ws, WsError>) -> u16 {
    match result {
        Err(WsError::Http(response)) => response.status().as_u16(),
        Err(e) => panic!("expected an HTTP rejection, got {e}"),
        Ok(_) => panic!("expected the handshake to be rejected"),
    }
}

async fn recv_json(ws: &mut Ws) -> Value {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("timed out waiting for a frame")
            .expect("stream ended")
            .expect("websocket error");
        match msg {
            Message::Text(text) => return serde_json::from_str(text.as_str()).unwrap(),
            Message::Ping(_) | Message::Pong(_) => {}
            other => panic!("unexpected frame {other:?}"),
        }
    }
}

async fn send_json(ws: &mut Ws, value: Value) {
    ws.send(Message::text(value.to_string())).await.unwrap();
}

fn envelope(seq: u64) -> BackendEventEnvelope {
    BackendEventEnvelope::new(
        seq,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::BackendStarted),
    )
}

#[tokio::test]
async fn rejects_foreign_or_missing_origin() {
    let h = start().await;
    for origin in [
        Some("http://evil.example"),
        Some("https://localhost:5173"),
        Some("http://localhost.evil.example"),
        None,
    ] {
        let status = rejected_status(connect(h.addr, Some(TOKEN), origin).await);
        assert_eq!(status, 403, "{origin:?}");
    }
}

#[tokio::test]
async fn rejects_missing_or_wrong_token() {
    let h = start().await;
    for token in [None, Some("wrong"), Some("")] {
        let status = rejected_status(connect(h.addr, token, Some(GOOD_ORIGIN)).await);
        assert_eq!(status, 401, "{token:?}");
    }
}

#[tokio::test]
async fn accepts_both_local_origins() {
    let h = start().await;
    for origin in ["http://localhost:5173", "http://127.0.0.1:4173"] {
        connect(h.addr, Some(TOKEN), Some(origin))
            .await
            .unwrap_or_else(|e| panic!("{origin}: {e}"));
    }
}

#[tokio::test]
async fn send_returns_request_ids_and_forwards_allowed_commands() {
    let h = start().await;
    let mut ws = connect(h.addr, Some(TOKEN), Some(GOOD_ORIGIN))
        .await
        .unwrap();

    send_json(&mut ws, json!({"op":"send","cmd":{"type":"RefreshPods"}})).await;
    assert_eq!(
        recv_json(&mut ws).await,
        json!({"op":"sent","request_id":1})
    );

    send_json(
        &mut ws,
        json!({"op":"send","cmd":{"type":"ScaleDeployment","data":{
            "namespace":"fathom-dev","name":"scale-me","replicas":2}}}),
    )
    .await;
    assert_eq!(
        recv_json(&mut ws).await,
        json!({"op":"sent","request_id":2})
    );

    let forwarded: Vec<CmdRequest> = h.cmds.try_iter().collect();
    assert_eq!(forwarded.len(), 2);
    assert_eq!(forwarded[0].id, Some(RequestId(1)));
    assert!(matches!(forwarded[0].cmd, BackendCmd::RefreshPods));
    assert_eq!(forwarded[1].id, Some(RequestId(2)));
}

#[tokio::test]
async fn guarded_commands_are_forbidden_and_never_reach_the_backend() {
    let h = start().await;
    let mut ws = connect(h.addr, Some(TOKEN), Some(GOOD_ORIGIN))
        .await
        .unwrap();

    for cmd in [
        json!({"type":"DeletePod","data":{"namespace":"default","name":"web"}}),
        json!({"type":"BulkDelete","data":{"items":[["fathom-dev","a"],["kube-system","b"]]}}),
        json!({"type":"DeletePersistentVolume","data":{"name":"pv"}}),
        json!({"type":"SetContext","data":{"context":"prod"}}),
        json!({"type":"StartMcpServer"}),
    ] {
        send_json(&mut ws, json!({"op":"send","cmd":cmd})).await;
        let reply = recv_json(&mut ws).await;
        assert_eq!(reply["op"], "error", "{cmd}");
        assert_eq!(reply["error"]["type"], "Forbidden", "{cmd}");
        assert!(
            reply["error"]["data"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
        );
    }
    assert_eq!(h.cmds.try_iter().count(), 0);
}

#[tokio::test]
async fn subscribe_replies_first_then_flushes_backlog_and_streams() {
    let h = start().await;
    h.bridge.hub.deliver(vec![envelope(1)]);

    let mut ws = connect(h.addr, Some(TOKEN), Some(GOOD_ORIGIN))
        .await
        .unwrap();
    send_json(&mut ws, json!({"op":"subscribe"})).await;

    let subscribed = recv_json(&mut ws).await;
    assert_eq!(subscribed["op"], "subscribed");
    assert_eq!(subscribed["info"]["backendStarted"], false);
    assert!(subscribed["info"]["protocolVersion"].is_number());

    let backlog = recv_json(&mut ws).await;
    assert_eq!(backlog["op"], "batch");
    assert_eq!(backlog["batch"]["dropped"], 0);
    assert_eq!(backlog["batch"]["envelopes"][0]["seq"], 1);

    h.bridge.hub.deliver(vec![envelope(2), envelope(3)]);
    let live = recv_json(&mut ws).await;
    assert_eq!(live["op"], "batch");
    let seqs: Vec<u64> = live["batch"]["envelopes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["seq"].as_u64().unwrap())
        .collect();
    assert_eq!(seqs, vec![2, 3]);
}

#[tokio::test]
async fn malformed_message_closes_with_1007() {
    let h = start().await;
    let mut ws = connect(h.addr, Some(TOKEN), Some(GOOD_ORIGIN))
        .await
        .unwrap();
    send_json(&mut ws, json!({"op":"launch-missiles"})).await;
    let frame = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    match frame {
        Message::Close(Some(close)) => assert_eq!(u16::from(close.code), 1007),
        other => panic!("expected a close frame, got {other:?}"),
    }
}
