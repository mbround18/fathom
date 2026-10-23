//! The WebSocket stub over real TCP, through the accept loop: close codes 4401 and 4403, and a
//! signed-in socket that ends when its session does.

mod support;

use std::time::Duration;

use axum::body::Body;
use futures_util::{SinkExt as _, StreamExt as _};
use http::{Request, StatusCode, header};
use support::{App, ORIGIN, OWNER, app, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tower::ServiceExt as _;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(a: &App, cookie: Option<&str>) -> (Socket, tokio::task::JoinHandle<()>) {
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

async fn close_code(ws: &mut Socket, within: Duration) -> u16 {
    loop {
        match tokio::time::timeout(within, ws.next()).await {
            Ok(Some(Ok(Message::Close(Some(f))))) => return u16::from(f.code),
            Ok(Some(Ok(_))) => {}
            other => panic!("no close frame: {other:?}"),
        }
    }
}

async fn csrf_of(a: &App, session: &str) -> String {
    json(a.get("/api/v1/me", Some(session)).await).await["csrf_token"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn hello(csrf: &str) -> String {
    serde_json::json!({ "protocol_version": 1, "csrf_token": csrf }).to_string()
}

#[tokio::test]
async fn signed_out_closes_4401() {
    let a = app();
    let (mut ws, server) = connect(&a, None).await;
    assert_eq!(close_code(&mut ws, Duration::from_secs(5)).await, 4401);
    server.abort();
}

#[tokio::test]
async fn a_wrong_csrf_token_closes_4401() {
    let a = app();
    let session = a.sign_in(OWNER).await;
    let (mut ws, server) = connect(&a, Some(&session)).await;
    ws.send(Message::text(hello("d3JvbmctdG9rZW4")))
        .await
        .unwrap();
    assert_eq!(close_code(&mut ws, Duration::from_secs(5)).await, 4401);
    server.abort();
}

#[tokio::test]
async fn a_pending_person_closes_4403() {
    let a = app();
    let session = a.sign_in("777").await;
    let csrf = csrf_of(&a, &session).await;
    let (mut ws, server) = connect(&a, Some(&session)).await;
    ws.send(Message::text(hello(&csrf))).await.unwrap();
    assert_eq!(close_code(&mut ws, Duration::from_secs(5)).await, 4403);
    server.abort();
}

#[tokio::test]
async fn an_admin_socket_answers_and_closes_4401_on_logout() {
    let a = app();
    let session = a.sign_in(OWNER).await;
    let csrf = csrf_of(&a, &session).await;
    let (mut ws, server) = connect(&a, Some(&session)).await;
    ws.send(Message::text(hello(&csrf))).await.unwrap();
    ws.send(Message::text(r#"{"id":1,"cmd":{"type":"ListStacks"}}"#))
        .await
        .unwrap();
    let Some(Ok(Message::Text(reply))) = ws.next().await else {
        panic!("no reply");
    };
    let reply: serde_json::Value = serde_json::from_str(reply.as_str()).unwrap();
    assert_eq!(reply["reply_to"], 1);
    assert_eq!(reply["event"]["type"], "Invalid");

    let r = a
        .router
        .clone()
        .oneshot(
            Request::post("/auth/logout")
                .header(header::COOKIE, &session)
                .header(header::ORIGIN, ORIGIN)
                .header("x-csrf-token", &csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(close_code(&mut ws, Duration::from_secs(5)).await, 4401);
    server.abort();
}
