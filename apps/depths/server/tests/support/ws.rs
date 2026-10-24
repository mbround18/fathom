//! The WebSocket client side of the harness: a real TCP socket through the accept loop, the
//! Hello, and request/reply helpers.

use std::time::Duration;

use depths_protocol::{Role, StackName, UserId};
use futures_util::{SinkExt as _, StreamExt as _};
use http::header;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

use super::{App, ORIGIN, app_with, container, json as body_json};

pub type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub const WAIT: Duration = Duration::from_secs(5);

pub async fn connect(a: &App, cookie: Option<&str>) -> (Socket, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = a.router.clone();
    let server = tokio::spawn(async move {
        let _ = depths_server::serve::serve(listener, router).await;
    });
    let mut req = format!("ws://{addr}/api/v1/ws")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert(header::ORIGIN, ORIGIN.parse().unwrap());
    if let Some(c) = cookie {
        req.headers_mut().insert(header::COOKIE, c.parse().unwrap());
    }
    let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    (ws, server)
}

pub async fn close_code(ws: &mut Socket, within: Duration) -> u16 {
    loop {
        match tokio::time::timeout(within, ws.next()).await {
            Ok(Some(Ok(Message::Close(Some(f))))) => return u16::from(f.code),
            Ok(Some(Ok(_))) => {}
            other => panic!("no close frame: {other:?}"),
        }
    }
}

pub async fn csrf_of(a: &App, session: &str) -> String {
    body_json(a.get("/api/v1/me", Some(session)).await).await["csrf_token"]
        .as_str()
        .unwrap()
        .to_owned()
}

pub fn hello(csrf: &str) -> String {
    json!({ "protocol_version": 1, "csrf_token": csrf }).to_string()
}

/// Signs `subject` in and opens a socket past the Hello.
pub async fn open(a: &App, subject: &str) -> (Socket, tokio::task::JoinHandle<()>, String) {
    let session = a.sign_in(subject).await;
    let csrf = csrf_of(a, &session).await;
    let (mut ws, server) = connect(a, Some(&session)).await;
    ws.send(Message::text(hello(&csrf))).await.unwrap();
    (ws, server, session)
}

/// The next event, whatever it is.
pub async fn next(ws: &mut Socket) -> Value {
    loop {
        match tokio::time::timeout(WAIT, ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => return serde_json::from_str(t.as_str()).unwrap(),
            Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => {}
            other => panic!("no event: {other:?}"),
        }
    }
}

/// The next event of type `ty`, skipping others.
pub async fn next_of(ws: &mut Socket, ty: &str) -> Value {
    loop {
        let e = next(ws).await;
        if e["event"]["type"] == ty {
            return e;
        }
    }
}

/// Sends `cmd` as request `id`; returns the reply to it.
pub async fn ask(ws: &mut Socket, id: u64, cmd: Value) -> Value {
    ws.send(Message::text(json!({ "id": id, "cmd": cmd }).to_string()))
        .await
        .unwrap();
    loop {
        let e = next(ws).await;
        if e["reply_to"] == id {
            return e;
        }
    }
}

pub fn grant(a: &App, subject: &str, stacks: &[&str]) {
    let stacks: Vec<StackName> = stacks.iter().map(|s| StackName::new(*s).unwrap()).collect();
    a.state
        .users
        .put(
            &UserId::new(format!("discord:{subject}")).unwrap(),
            Role::User,
            &stacks,
        )
        .unwrap();
}

pub fn names(stacks: &Value) -> Vec<String> {
    let mut v: Vec<String> = stacks["event"]["data"]["stacks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_owned())
        .collect();
    v.sort();
    v
}

pub const COMPOSE: &str = "services:\n  app:\n    image: nginx\n    restart: unless-stopped\n";

/// Waits until `done` holds, polling.
pub async fn until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !done() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub fn b64(s: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(s)
}

pub fn unb64(v: &Value) -> String {
    use base64::Engine as _;
    String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(v.as_str().unwrap())
            .unwrap(),
    )
    .unwrap()
}

pub fn count(n: &std::sync::atomic::AtomicUsize) -> usize {
    n.load(std::sync::atomic::Ordering::SeqCst)
}

/// Stacks `web` and `db` (service `app`) running, and container `stray-x-1` outside every stack.
pub fn streams_app() -> App {
    let a = app_with(|s| s.ws.stats_every = Duration::from_millis(50));
    a.add_stack("web", COMPOSE, None);
    a.add_stack("db", COMPOSE, None);
    a.set_containers(vec![
        container("web", "app", "running", "Up 1 hour"),
        container("db", "app", "running", "Up 1 hour"),
        container("stray", "x", "running", "Up 1 hour"),
    ]);
    a
}

pub fn stream_of(r: &Value) -> u64 {
    assert_eq!(r["event"]["type"], "StreamOpened", "{r}");
    r["event"]["data"]["stream_id"].as_u64().unwrap()
}

pub fn open_logs(target: Value) -> Value {
    json!({ "type": "OpenLogs", "data": { "target": target, "tail": 10 } })
}

pub fn open_shell(container: &str) -> Value {
    json!({ "type": "OpenShell", "data": { "container": container, "cols": 80, "rows": 24 } })
}

/// Sends `cmd` without waiting for a reply (the stream commands that have none).
pub async fn tell(ws: &mut Socket, cmd: Value) {
    ws.send(Message::text(json!({ "id": 900, "cmd": cmd }).to_string()))
        .await
        .unwrap();
}
