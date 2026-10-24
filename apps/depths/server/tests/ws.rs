//! The WebSocket over real TCP, through the accept loop: the Hello and its close codes, the
//! stack commands and their authorisation, operations, revocation, and the T090/T092 limits.

mod support;

use std::time::Duration;

use axum::body::Body;
use depths_protocol::{Role, StackName, UserId};
use futures_util::{SinkExt as _, StreamExt as _};
use http::{Request, StatusCode, header};
use serde_json::{Value, json};
use support::{App, ORIGIN, OWNER, app, app_with, container, json as body_json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tower::ServiceExt as _;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const WAIT: Duration = Duration::from_secs(5);

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
    body_json(a.get("/api/v1/me", Some(session)).await).await["csrf_token"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn hello(csrf: &str) -> String {
    json!({ "protocol_version": 1, "csrf_token": csrf }).to_string()
}

/// Signs `subject` in and opens a socket past the Hello.
async fn open(a: &App, subject: &str) -> (Socket, tokio::task::JoinHandle<()>, String) {
    let session = a.sign_in(subject).await;
    let csrf = csrf_of(a, &session).await;
    let (mut ws, server) = connect(a, Some(&session)).await;
    ws.send(Message::text(hello(&csrf))).await.unwrap();
    (ws, server, session)
}

/// The next event, whatever it is.
async fn next(ws: &mut Socket) -> Value {
    loop {
        match tokio::time::timeout(WAIT, ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => return serde_json::from_str(t.as_str()).unwrap(),
            Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => {}
            other => panic!("no event: {other:?}"),
        }
    }
}

/// The next event of type `ty`, skipping others.
async fn next_of(ws: &mut Socket, ty: &str) -> Value {
    loop {
        let e = next(ws).await;
        if e["event"]["type"] == ty {
            return e;
        }
    }
}

/// Sends `cmd` as request `id`; returns the reply to it.
async fn ask(ws: &mut Socket, id: u64, cmd: Value) -> Value {
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

fn grant(a: &App, subject: &str, stacks: &[&str]) {
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

fn names(stacks: &Value) -> Vec<String> {
    let mut v: Vec<String> = stacks["event"]["data"]["stacks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_owned())
        .collect();
    v.sort();
    v
}

const COMPOSE: &str = "services:\n  app:\n    image: nginx\n    restart: unless-stopped\n";

#[tokio::test]
async fn signed_out_closes_4401() {
    let a = app();
    let (mut ws, server) = connect(&a, None).await;
    assert_eq!(close_code(&mut ws, WAIT).await, 4401);
    server.abort();
}

#[tokio::test]
async fn a_wrong_csrf_token_closes_4403() {
    let a = app();
    let session = a.sign_in(OWNER).await;
    let (mut ws, server) = connect(&a, Some(&session)).await;
    ws.send(Message::text(hello("d3JvbmctdG9rZW4")))
        .await
        .unwrap();
    assert_eq!(close_code(&mut ws, WAIT).await, 4403);
    server.abort();
}

#[tokio::test]
async fn a_pending_person_closes_4403() {
    let a = app();
    let (mut ws, server, _) = open(&a, "777").await;
    assert_eq!(close_code(&mut ws, WAIT).await, 4403);
    server.abort();
}

#[tokio::test]
async fn no_hello_within_the_timeout_closes_4401() {
    let a = app_with(|s| s.ws.hello_timeout = Duration::from_millis(200));
    let session = a.sign_in(OWNER).await;
    let (mut ws, server) = connect(&a, Some(&session)).await;
    assert_eq!(close_code(&mut ws, WAIT).await, 4401);
    server.abort();
}

#[tokio::test]
async fn an_admin_socket_answers_and_closes_4401_on_logout() {
    let a = app();
    a.add_stack("web", COMPOSE, None);
    let session = a.sign_in(OWNER).await;
    let csrf = csrf_of(&a, &session).await;
    let (mut ws, server) = connect(&a, Some(&session)).await;
    ws.send(Message::text(hello(&csrf))).await.unwrap();
    let reply = ask(&mut ws, 1, json!({ "type": "ListStacks" })).await;
    assert_eq!(reply["event"]["type"], "Stacks", "{reply}");
    assert_eq!(names(&reply), ["web"]);

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
    assert_eq!(close_code(&mut ws, WAIT).await, 4401);
    server.abort();
}

#[tokio::test]
async fn a_user_sees_only_assigned_stacks_and_no_foreign_projects() {
    let a = app();
    a.add_stack("web", COMPOSE, None);
    a.add_stack("db", COMPOSE, None);
    a.set_containers(vec![
        container("web", "app", "running", "Up 1 hour"),
        container("elsewhere", "app", "running", "Up 1 hour"),
    ]);
    grant(&a, "7", &["web"]);

    let (mut user, s1, _) = open(&a, "7").await;
    let r = ask(&mut user, 1, json!({ "type": "ListStacks" })).await;
    assert_eq!(r["event"]["type"], "Stacks", "{r}");
    assert_eq!(names(&r), ["web"]);
    assert_eq!(r["event"]["data"]["foreign"], json!([]));
    assert_eq!(r["event"]["data"]["standalone"], json!([]));

    // A stack the user was not given is refused, with the request's id, and audited.
    let r = ask(
        &mut user,
        2,
        json!({ "type": "GetStack", "data": { "name": "db" } }),
    )
    .await;
    assert_eq!(r["event"]["type"], "Denied", "{r}");
    assert!(
        a.audit_entries()
            .iter()
            .any(|e| e.stack.as_deref() == Some("db")),
        "the refusal is audited"
    );

    let (mut admin, s2, _) = open(&a, OWNER).await;
    let r = ask(&mut admin, 1, json!({ "type": "ListStacks" })).await;
    assert_eq!(names(&r), ["db", "web"]);
    let foreign = r["event"]["data"]["foreign"].as_array().unwrap();
    assert_eq!(foreign.len(), 1, "{r}");
    s1.abort();
    s2.abort();
}

#[tokio::test]
async fn a_one_shot_job_that_exited_zero_does_not_make_the_stack_partial() {
    let a = app();
    a.add_stack(
        "web",
        "services:\n  app:\n    image: nginx\n    restart: unless-stopped\n  migrate:\n    image: \
         nginx\n    restart: \"no\"\n",
        None,
    );
    a.set_containers(vec![
        container("web", "app", "running", "Up 1 hour"),
        container("web", "migrate", "exited", "Exited (0) 2 minutes ago"),
    ]);
    let (mut ws, server, _) = open(&a, OWNER).await;
    let r = ask(&mut ws, 1, json!({ "type": "ListStacks" })).await;
    let web = &r["event"]["data"]["stacks"][0];
    assert_eq!(web["status"], "running", "{r}");
    assert_eq!(web["total_services"], 1, "{r}");
    assert_eq!(web["running_services"], 1, "{r}");
    server.abort();
}

#[tokio::test]
async fn env_values_are_masked_revealed_only_on_request_and_never_audited() {
    let a = app();
    a.add_stack("web", COMPOSE, Some("DB_PASSWORD=hunter2\nPORT=8080\n"));
    let (mut ws, server, _) = open(&a, OWNER).await;
    let r = ask(
        &mut ws,
        1,
        json!({ "type": "GetStack", "data": { "name": "web" } }),
    )
    .await;
    assert_eq!(r["event"]["type"], "StackDetail", "{r}");
    assert!(!r.to_string().contains("hunter2"), "{r}");

    let r = ask(
        &mut ws,
        2,
        json!({ "type": "RevealEnv", "data": { "name": "web" } }),
    )
    .await;
    assert_eq!(r["event"]["type"], "EnvRevealed", "{r}");
    assert!(
        r["event"]["data"]["env_text"]
            .as_str()
            .unwrap()
            .contains("hunter2")
    );

    let entries = a.audit_entries();
    assert!(
        entries
            .iter()
            .any(|e| e.action == depths_protocol::AuditAction::RevealEnv),
        "{entries:?}"
    );
    let bytes = a.audit_bytes();
    assert!(!bytes.is_empty());
    assert!(!String::from_utf8_lossy(&bytes).contains("hunter2"));
    server.abort();
}

#[tokio::test]
async fn a_user_cannot_operate_an_unassigned_stack() {
    let a = app();
    a.add_stack("web", COMPOSE, None);
    a.add_stack("db", COMPOSE, None);
    grant(&a, "7", &["web"]);
    let (mut ws, server, _) = open(&a, "7").await;
    let r = ask(
        &mut ws,
        5,
        json!({ "type": "PreviewOp", "data": { "stack": "db", "op": { "kind": "restart" } } }),
    )
    .await;
    assert_eq!(r["event"]["type"], "Denied", "{r}");
    assert_eq!(r["reply_to"], 5);
    let e = a.audit_entries();
    assert!(
        e.iter()
            .any(|e| e.action == depths_protocol::AuditAction::Denied
                && e.stack.as_deref() == Some("db"))
    );
    assert!(a.runtime.ran.lock().unwrap().is_empty());
    server.abort();
}

#[tokio::test]
async fn a_confirmed_operation_streams_and_reports_dockers_stderr_verbatim() {
    let a = app();
    a.add_stack("web", COMPOSE, None);
    *a.runtime.fail_with.lock().unwrap() = Some("Error response from daemon: boom\n".into());
    let (mut ws, server, _) = open(&a, OWNER).await;
    let p = ask(
        &mut ws,
        1,
        json!({ "type": "PreviewOp", "data": { "stack": "web", "op": { "kind": "restart" } } }),
    )
    .await;
    assert_eq!(p["event"]["type"], "Preview", "{p}");
    let preview_id = p["event"]["data"]["preview_id"].clone();
    let argv = p["event"]["data"]["steps"][0]["argv"].clone();

    let s = ask(
        &mut ws,
        2,
        json!({ "type": "Confirm", "data": { "preview_id": preview_id } }),
    )
    .await;
    assert_eq!(s["event"]["type"], "OpStarted", "{s}");
    let op = s["event"]["data"]["operation_id"].clone();
    let out = next_of(&mut ws, "OpOutput").await;
    assert_eq!(out["event"]["data"]["operation_id"], op);
    let done = next_of(&mut ws, "OpFinished").await;
    assert_eq!(done["event"]["data"]["operation_id"], op);
    assert!(
        done.to_string()
            .contains("Error response from daemon: boom"),
        "{done}"
    );
    // What ran is what the preview showed.
    let ran = a.runtime.ran.lock().unwrap().clone();
    assert_eq!(json!(ran[0]), argv);

    // A preview is single-use.
    let again = ask(
        &mut ws,
        3,
        json!({ "type": "Confirm", "data": { "preview_id": preview_id } }),
    )
    .await;
    assert_eq!(again["event"]["type"], "PreviewVoid", "{again}");
    server.abort();
}

#[tokio::test]
async fn removing_a_user_closes_their_socket_4403() {
    let a = app();
    a.add_stack("web", COMPOSE, None);
    grant(&a, "7", &["web"]);
    let (mut user, s1, _) = open(&a, "7").await;
    let r = ask(&mut user, 1, json!({ "type": "ListStacks" })).await;
    assert_eq!(r["event"]["type"], "Stacks");

    let (mut admin, s2, _) = open(&a, OWNER).await;
    let r = ask(
        &mut admin,
        1,
        json!({ "type": "RemoveUser", "data": { "id": "discord:7" } }),
    )
    .await;
    assert_eq!(r["event"]["type"], "Users", "{r}");
    assert_eq!(close_code(&mut user, WAIT).await, 4403);
    assert!(
        a.audit_entries()
            .iter()
            .any(|e| e.action == depths_protocol::AuditAction::UserRemoved)
    );
    s1.abort();
    s2.abort();
}

#[tokio::test]
async fn docker_going_away_keeps_a_stale_list_and_says_so() {
    let a = app();
    a.add_stack("web", COMPOSE, None);
    let (mut ws, server, _) = open(&a, OWNER).await;
    let r = ask(&mut ws, 1, json!({ "type": "ListStacks" })).await;
    assert_eq!(r["event"]["data"]["stale"], false, "{r}");

    *a.runtime.unavailable.lock().unwrap() =
        Some("Cannot connect to the Docker daemon at unix:///var/run/docker.sock".into());
    let r = ask(&mut ws, 2, json!({ "type": "ListStacks" })).await;
    assert_eq!(r["event"]["type"], "Stacks");
    assert_eq!(names(&r), ["web"]);
    assert_eq!(r["event"]["data"]["stale"], true, "{r}");
    let u = next_of(&mut ws, "RuntimeUnavailable").await;
    assert!(
        u["event"]["data"]["reason"]
            .as_str()
            .unwrap()
            .contains("Cannot connect to the Docker daemon")
    );
    server.abort();
}

#[tokio::test]
async fn a_malformed_command_is_invalid_with_its_id() {
    let a = app();
    let (mut ws, server, _) = open(&a, OWNER).await;
    let r = ask(
        &mut ws,
        9,
        json!({ "type": "GetStack", "data": { "name": "../etc" } }),
    )
    .await;
    assert_eq!(r["event"]["type"], "Invalid", "{r}");
    assert!(
        a.audit_entries()
            .iter()
            .any(|e| e.action == depths_protocol::AuditAction::Denied)
    );
    server.abort();
}

#[tokio::test]
async fn an_assigned_user_saves_over_the_socket_and_a_stale_hash_conflicts() {
    let a = app();
    a.add_stack("web", COMPOSE, Some("API_TOKEN=s3cr3t\n"));
    grant(&a, "7", &["web"]);
    let (mut ws, server, _) = open(&a, "7").await;
    let d = ask(
        &mut ws,
        1,
        json!({ "type": "GetStack", "data": { "name": "web" } }),
    )
    .await;
    let base = d["event"]["data"]["content_hash"].clone();
    let edited = COMPOSE.replace("nginx", "caddy");

    let p = ask(
        &mut ws,
        2,
        json!({ "type": "PreviewSave", "data": {
            "draft": { "stack": "web", "base_hash": base, "compose": edited },
            "deploy": false,
        } }),
    )
    .await;
    assert_eq!(p["event"]["type"], "Preview", "{p}");
    assert_eq!(p["event"]["data"]["kind"], "save");
    assert_eq!(p["event"]["data"]["steps"][0]["stdin"], "draft");
    let s = ask(
        &mut ws,
        3,
        json!({ "type": "Confirm", "data": { "preview_id": p["event"]["data"]["preview_id"] } }),
    )
    .await;
    assert_eq!(s["event"]["type"], "OpStarted", "{s}");
    let done = next_of(&mut ws, "OpFinished").await;
    assert_eq!(
        done["event"]["data"]["outcome"]["kind"], "succeeded",
        "{done}"
    );
    let dir = a.stacks.join("web");
    assert_eq!(
        std::fs::read_to_string(dir.join("compose.yaml")).unwrap(),
        edited
    );
    assert_eq!(
        std::fs::read_to_string(dir.join(".env")).unwrap(),
        "API_TOKEN=s3cr3t\n"
    );

    // The same base again: the files moved on, so it conflicts, `.env` masked.
    let c = ask(
        &mut ws,
        4,
        json!({ "type": "PreviewSave", "data": {
            "draft": { "stack": "web", "base_hash": base, "compose": COMPOSE },
            "deploy": false,
        } }),
    )
    .await;
    assert_eq!(c["event"]["type"], "Conflict", "{c}");
    assert_eq!(c["event"]["data"]["current_text"], edited);
    assert!(!c.to_string().contains("s3cr3t"), "{c}");

    // A create is admin-only.
    let r = ask(
        &mut ws,
        5,
        json!({ "type": "PreviewSave", "data": {
            "draft": { "stack": "new", "compose": COMPOSE },
            "deploy": false,
        } }),
    )
    .await;
    assert_eq!(r["event"]["data"]["reason"], "not_permitted", "{r}");
    server.abort();
}

#[tokio::test]
async fn saving_is_refused_when_editing_is_off() {
    let a = app_with(|s| {
        let mut cfg = (*s.cfg).clone();
        cfg.features.edit = false;
        s.cfg = std::sync::Arc::new(cfg);
    });
    a.add_stack("web", COMPOSE, None);
    let (mut ws, server, _) = open(&a, OWNER).await;
    let r = ask(
        &mut ws,
        1,
        json!({ "type": "PreviewSave", "data": {
            "draft": { "stack": "web", "compose": COMPOSE },
            "deploy": false,
        } }),
    )
    .await;
    assert_eq!(r["event"]["type"], "Denied", "{r}");
    assert_eq!(r["event"]["data"]["reason"], "feature_disabled", "{r}");
    server.abort();
}

#[tokio::test]
async fn the_shell_is_refused_when_its_flag_is_off() {
    let a = app_with(|s| {
        let mut cfg = (*s.cfg).clone();
        cfg.features.shell = false;
        s.cfg = std::sync::Arc::new(cfg);
    });
    a.add_stack("web", COMPOSE, None);
    a.set_containers(vec![container("web", "app", "running", "Up 1 hour")]);
    let (mut ws, server, _) = open(&a, OWNER).await;
    let r = ask(
        &mut ws,
        1,
        json!({ "type": "OpenShell", "data": { "container": "web-app-1", "cols": 80, "rows": 24 } }),
    )
    .await;
    assert_eq!(r["event"]["type"], "Denied", "{r}");
    assert_eq!(r["event"]["data"]["reason"], "feature_disabled", "{r}");
    assert!(a.runtime.shells.lock().unwrap().is_empty());
    server.abort();
}

#[tokio::test]
async fn a_binary_frame_closes_1003() {
    let a = app();
    let (mut ws, server, _) = open(&a, OWNER).await;
    ws.send(Message::binary(vec![1, 2, 3])).await.unwrap();
    assert_eq!(close_code(&mut ws, WAIT).await, 1003);
    server.abort();
}

#[tokio::test]
async fn an_oversized_message_ends_the_socket() {
    let a = app();
    let (mut ws, server, _) = open(&a, OWNER).await;
    let max = depths_server::ws::max_message_bytes(a.state.cfg.max_file_bytes);
    let _ = ws.send(Message::text("x".repeat(max + 1))).await;
    match tokio::time::timeout(WAIT, async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Close(f))) => return f.map(|f| u16::from(f.code)),
                Some(Ok(_)) => {}
                Some(Err(_)) | None => return None,
            }
        }
    })
    .await
    {
        Ok(Some(code)) => assert_ne!(code, 1000, "closed with an error code"),
        Ok(None) => {}
        Err(_) => panic!("the socket stayed open"),
    }
    server.abort();
}

#[tokio::test]
async fn unanswered_pings_close_1001() {
    let a = app_with(|s| s.ws.ping_every = Duration::from_millis(100));
    let (mut ws, server, _) = open(&a, OWNER).await;
    // Not reading means not answering pings.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(close_code(&mut ws, WAIT).await, 1001);
    server.abort();
}

#[tokio::test]
async fn the_ninth_socket_of_a_session_closes_4429() {
    let a = app();
    let session = a.sign_in(OWNER).await;
    let csrf = csrf_of(&a, &session).await;
    let mut held = Vec::new();
    for i in 0..8u64 {
        let (mut ws, server) = connect(&a, Some(&session)).await;
        ws.send(Message::text(hello(&csrf))).await.unwrap();
        let r = ask(
            &mut ws,
            i,
            json!({ "type": "CloseStream", "data": { "stream_id": 999 } }),
        )
        .await;
        assert_eq!(r["event"]["type"], "Invalid", "socket {i} is open: {r}");
        held.push((ws, server));
    }
    let (mut ws, server) = connect(&a, Some(&session)).await;
    ws.send(Message::text(hello(&csrf))).await.unwrap();
    assert_eq!(close_code(&mut ws, WAIT).await, 4429);
    server.abort();
    // Closing one frees its slot.
    let (mut first, s) = held.remove(0);
    first.close(None).await.unwrap();
    s.abort();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (mut ws, server) = connect(&a, Some(&session)).await;
    ws.send(Message::text(hello(&csrf))).await.unwrap();
    let r = ask(
        &mut ws,
        99,
        json!({ "type": "CloseStream", "data": { "stream_id": 999 } }),
    )
    .await;
    assert_eq!(r["event"]["type"], "Invalid");
    server.abort();
    for (_, s) in held {
        s.abort();
    }
}

#[tokio::test]
async fn a_command_flood_closes_4429() {
    let a = app();
    let (mut ws, server, _) = open(&a, OWNER).await;
    for i in 0..80u64 {
        let m = json!({ "id": i, "cmd": { "type": "CloseStream", "data": { "stream_id": 999 } } })
            .to_string();
        if ws.send(Message::text(m)).await.is_err() {
            break;
        }
    }
    assert_eq!(close_code(&mut ws, WAIT).await, 4429);
    server.abort();
}

// ---------------------------------------------------------------------------
// Streams (T066): logs, shells, stats, and the per-socket cap.
// ---------------------------------------------------------------------------

/// Waits until `done` holds, polling.
async fn until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !done() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn b64(s: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(s)
}

fn unb64(v: &Value) -> String {
    use base64::Engine as _;
    String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(v.as_str().unwrap())
            .unwrap(),
    )
    .unwrap()
}

fn count(n: &std::sync::atomic::AtomicUsize) -> usize {
    n.load(std::sync::atomic::Ordering::SeqCst)
}

/// Stacks `web` and `db` (service `app`) running, and container `stray-x-1` outside every stack.
fn streams_app() -> App {
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

fn stream_of(r: &Value) -> u64 {
    assert_eq!(r["event"]["type"], "StreamOpened", "{r}");
    r["event"]["data"]["stream_id"].as_u64().unwrap()
}

fn open_logs(target: Value) -> Value {
    json!({ "type": "OpenLogs", "data": { "target": target, "tail": 10 } })
}

fn open_shell(container: &str) -> Value {
    json!({ "type": "OpenShell", "data": { "container": container, "cols": 80, "rows": 24 } })
}

/// Sends `cmd` without waiting for a reply (the stream commands that have none).
async fn tell(ws: &mut Socket, cmd: Value) {
    ws.send(Message::text(json!({ "id": 900, "cmd": cmd }).to_string()))
        .await
        .unwrap();
}

#[tokio::test]
async fn logs_stream_and_close_stops_the_follow() {
    let a = streams_app();
    *a.runtime.log_lines.lock().unwrap() = vec!["2026-10-23T00:00:00Z hello".into()];
    let (mut ws, server, _) = open(&a, OWNER).await;
    let r = ask(
        &mut ws,
        1,
        json!({ "type": "OpenLogs", "data": {
            "target": { "kind": "stack", "stack": "web", "service": "app" },
            "tail": 1000,
            "since": "2026-10-23T00:00:00Z",
        } }),
    )
    .await;
    let id = stream_of(&r);
    let chunk = next_of(&mut ws, "LogChunk").await;
    assert_eq!(chunk["event"]["data"]["stream_id"], id);
    assert_eq!(
        chunk["event"]["data"]["lines"],
        json!(["2026-10-23T00:00:00Z hello"])
    );
    let argv = a.runtime.follows.lock().unwrap()[0].join(" ");
    assert!(argv.starts_with("docker compose"), "{argv}");
    assert!(
        argv.ends_with(
            "logs --no-color --timestamps --follow --tail 1000 --since=2026-10-23T00:00:00Z app"
        ),
        "{argv}"
    );

    let close = json!({ "type": "CloseStream", "data": { "stream_id": id } });
    let r = ask(&mut ws, 2, close.clone()).await;
    assert_eq!(r["event"]["type"], "StreamClosed", "{r}");
    assert_eq!(r["event"]["data"]["reason"]["kind"], "closed", "{r}");
    until("the follow stops", || {
        count(&a.runtime.follows_stopped) == 1
    })
    .await;
    let r = ask(&mut ws, 3, close).await;
    assert_eq!(
        r["event"]["type"], "Invalid",
        "a closed stream is unknown: {r}"
    );
    server.abort();
}

#[tokio::test]
async fn a_follow_that_ends_closes_its_stream_after_its_lines() {
    let a = streams_app();
    *a.runtime.log_lines.lock().unwrap() = vec!["last words".into()];
    a.runtime
        .logs_end
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let (mut ws, server, _) = open(&a, OWNER).await;
    let r = ask(
        &mut ws,
        1,
        json!({ "type": "OpenLogs", "data": {
            "target": { "kind": "container", "container": "web-app-1" },
            "tail": 5,
        } }),
    )
    .await;
    stream_of(&r);
    assert_eq!(next(&mut ws).await["event"]["type"], "LogChunk");
    let closed = next_of(&mut ws, "StreamClosed").await;
    assert_eq!(
        closed["event"]["data"]["reason"]["kind"], "ended",
        "{closed}"
    );
    assert_eq!(
        a.runtime.follows.lock().unwrap()[0].join(" "),
        "docker logs --timestamps --follow --tail 5 web-app-id"
    );
    server.abort();
}

#[tokio::test]
async fn logs_refuse_unknown_services_and_containers_a_user_may_not_see() {
    let a = streams_app();
    grant(&a, "7", &["web"]);
    let (mut ws, server, _) = open(&a, "7").await;
    let r = ask(
        &mut ws,
        1,
        open_logs(json!({ "kind": "stack", "stack": "web", "service": "nope" })),
    )
    .await;
    assert_eq!(r["event"]["type"], "Invalid", "{r}");
    let r = ask(
        &mut ws,
        2,
        open_logs(json!({ "kind": "stack", "stack": "db" })),
    )
    .await;
    assert_eq!(r["event"]["type"], "Denied", "{r}");
    for (i, c) in ["stray-x-1", "db-app-1", "nonexistent"].iter().enumerate() {
        let r = ask(
            &mut ws,
            10 + i as u64,
            open_logs(json!({ "kind": "container", "container": c })),
        )
        .await;
        assert_eq!(r["event"]["type"], "Denied", "{c}: {r}");
    }
    stream_of(
        &ask(
            &mut ws,
            5,
            open_logs(json!({ "kind": "container", "container": "web-app-1" })),
        )
        .await,
    );
    assert_eq!(a.runtime.follows.lock().unwrap().len(), 1);
    server.abort();

    // An admin may follow the stray, and is told plainly when a container does not exist.
    let (mut ws, server, _) = open(&a, OWNER).await;
    stream_of(
        &ask(
            &mut ws,
            1,
            open_logs(json!({ "kind": "container", "container": "stray-x-1" })),
        )
        .await,
    );
    let r = ask(
        &mut ws,
        2,
        open_logs(json!({ "kind": "container", "container": "nonexistent" })),
    )
    .await;
    assert_eq!(r["event"]["type"], "Invalid", "{r}");
    server.abort();
}

#[tokio::test]
async fn a_shell_is_refused_outside_the_users_stacks() {
    let a = streams_app();
    grant(&a, "7", &["web"]);
    let (mut ws, server, _) = open(&a, "7").await;
    for (i, c) in ["db-app-1", "stray-x-1"].iter().enumerate() {
        let r = ask(&mut ws, i as u64, open_shell(c)).await;
        assert_eq!(r["event"]["type"], "Denied", "{c}: {r}");
        assert_eq!(r["event"]["data"]["reason"], "not_permitted", "{r}");
    }
    assert!(a.runtime.shells.lock().unwrap().is_empty());
    assert!(
        !a.audit_entries()
            .iter()
            .any(|e| e.action == depths_protocol::AuditAction::ShellOpen)
    );
    server.abort();
}

#[tokio::test]
async fn a_shell_is_audited_echoes_input_resizes_and_ends() {
    let a = streams_app();
    grant(&a, "7", &["web"]);
    let (mut ws, server, _) = open(&a, "7").await;
    let r = ask(
        &mut ws,
        1,
        json!({ "type": "OpenShell", "data": {
            "container": "web-app-1", "shell": "bash", "cols": 80, "rows": 24,
        } }),
    )
    .await;
    let id = stream_of(&r);
    let audited = a.audit_entries();
    let shell = audited
        .iter()
        .find(|e| e.action == depths_protocol::AuditAction::ShellOpen)
        .expect("a shell_open record before StreamOpened");
    assert_eq!(shell.stack.as_deref(), Some("web"));
    assert_eq!(
        a.runtime.shells.lock().unwrap()[0],
        ("web-app-id".to_owned(), "/bin/bash".to_owned(), (80, 24))
    );

    let input =
        |s: &str| json!({ "type": "ShellInput", "data": { "stream_id": id, "bytes_b64": b64(s) } });
    tell(&mut ws, input("ls\n")).await;
    let out = next_of(&mut ws, "ShellOutput").await;
    assert_eq!(out["event"]["data"]["stream_id"], id);
    assert_eq!(unb64(&out["event"]["data"]["bytes_b64"]), "echo:ls\n");

    tell(
        &mut ws,
        json!({ "type": "ShellResize", "data": { "stream_id": id, "cols": 120, "rows": 40 } }),
    )
    .await;
    until("the resize", || {
        a.runtime.shell_log.resizes.lock().unwrap().as_slice() == [(120, 40)]
    })
    .await;
    let r = ask(
        &mut ws,
        2,
        json!({ "type": "ShellResize", "data": { "stream_id": id, "cols": 0, "rows": 40 } }),
    )
    .await;
    assert_eq!(r["event"]["type"], "Invalid", "{r}");
    let r = ask(
        &mut ws,
        3,
        json!({ "type": "ShellInput", "data": { "stream_id": id + 100, "bytes_b64": b64("x") } }),
    )
    .await;
    assert_eq!(r["event"]["type"], "Invalid", "{r}");

    tell(&mut ws, input("exit\n")).await;
    let closed = next_of(&mut ws, "StreamClosed").await;
    assert_eq!(closed["event"]["data"]["stream_id"], id);
    assert_eq!(
        closed["event"]["data"]["reason"]["kind"], "ended",
        "{closed}"
    );
    until("the session closes", || {
        count(&a.runtime.shell_log.closed) == 1
    })
    .await;
    server.abort();
}

#[tokio::test]
async fn closing_the_socket_stops_its_shells_and_follows() {
    let a = streams_app();
    let (mut ws, server, _) = open(&a, OWNER).await;
    stream_of(&ask(&mut ws, 1, open_shell("web-app-1")).await);
    stream_of(
        &ask(
            &mut ws,
            2,
            open_logs(json!({ "kind": "stack", "stack": "web" })),
        )
        .await,
    );
    ws.close(None).await.unwrap();
    drop(ws);
    until("the follow stops", || {
        count(&a.runtime.follows_stopped) == 1
    })
    .await;
    until("the shell closes", || {
        count(&a.runtime.shell_log.closed) == 1
    })
    .await;
    server.abort();
}

/// How many stats readings happen over a while, once whatever was in flight has settled.
async fn ticks_after_settling(a: &App) -> usize {
    tokio::time::sleep(Duration::from_millis(200)).await;
    let before = a.runtime.stats_calls.lock().unwrap().len();
    tokio::time::sleep(Duration::from_millis(400)).await;
    a.runtime.stats_calls.lock().unwrap().len() - before
}

#[tokio::test]
async fn stats_tick_with_full_ids_and_stop_on_unwatch() {
    let a = streams_app();
    grant(&a, "7", &["web"]);
    let (mut ws, server, _) = open(&a, "7").await;
    let r = ask(
        &mut ws,
        1,
        json!({ "type": "WatchStats", "data": { "targets": [{ "kind": "stack", "stack": "db" }] } }),
    )
    .await;
    assert_eq!(r["event"]["type"], "Denied", "{r}");

    tell(
        &mut ws,
        json!({ "type": "WatchStats", "data": { "targets": [
            { "kind": "stack", "stack": "web" },
            { "kind": "container", "container": "web-app-1" },
        ] } }),
    )
    .await;
    let s = next_of(&mut ws, "StatsSample").await;
    assert_eq!(
        s["event"]["data"]["samples"][0]["container_id"],
        "web-app-id"
    );
    assert_eq!(
        a.runtime.stats_calls.lock().unwrap()[0],
        ["web-app-id".to_owned()],
        "one reading per container, however it was named"
    );
    next_of(&mut ws, "StatsSample").await;

    tell(&mut ws, json!({ "type": "UnwatchStats" })).await;
    assert_eq!(
        ticks_after_settling(&a).await,
        0,
        "no readings after unwatch"
    );
    server.abort();
}

#[tokio::test]
async fn stats_stop_when_the_socket_closes() {
    let a = streams_app();
    let (mut ws, server, _) = open(&a, OWNER).await;
    tell(
        &mut ws,
        json!({ "type": "WatchStats", "data": { "targets": [
            { "kind": "container", "container": "stray-x-1" },
        ] } }),
    )
    .await;
    let s = next_of(&mut ws, "StatsSample").await;
    assert_eq!(
        s["event"]["data"]["samples"][0]["container_id"],
        "stray-x-id"
    );
    ws.close(None).await.unwrap();
    drop(ws);
    assert_eq!(ticks_after_settling(&a).await, 0, "no readings after close");
    server.abort();
}

#[tokio::test]
async fn a_socket_holds_at_most_32_streams() {
    let a = streams_app();
    let (mut ws, server, _) = open(&a, OWNER).await;
    let logs = open_logs(json!({ "kind": "stack", "stack": "web" }));
    let mut ids = Vec::new();
    for i in 0..depths_protocol::MAX_STREAMS_PER_SOCKET as u64 {
        ids.push(stream_of(&ask(&mut ws, i + 1, logs.clone()).await));
    }
    let r = ask(&mut ws, 100, logs.clone()).await;
    assert_eq!(r["event"]["type"], "Invalid", "{r}");
    let r = ask(&mut ws, 101, open_shell("web-app-1")).await;
    assert_eq!(r["event"]["type"], "Invalid", "{r}");
    assert!(a.runtime.shells.lock().unwrap().is_empty());
    assert!(
        !a.audit_entries()
            .iter()
            .any(|e| e.action == depths_protocol::AuditAction::ShellOpen),
        "a refused shell is not audited as opened"
    );
    // Closing one makes room again.
    ask(
        &mut ws,
        102,
        json!({ "type": "CloseStream", "data": { "stream_id": ids[0] } }),
    )
    .await;
    stream_of(&ask(&mut ws, 103, logs).await);
    server.abort();
}

#[tokio::test]
async fn losing_a_stack_closes_its_streams() {
    let a = streams_app();
    grant(&a, "7", &["web"]);
    let (mut ws, server, _) = open(&a, "7").await;
    let logs = open_logs(json!({ "kind": "stack", "stack": "web" }));
    let id = stream_of(&ask(&mut ws, 1, logs).await);
    grant(&a, "7", &["db"]);
    let _ = a.state.users_changed.send(());
    let closed = next_of(&mut ws, "StreamClosed").await;
    assert_eq!(closed["event"]["data"]["stream_id"], id);
    assert_eq!(
        closed["event"]["data"]["reason"]["kind"], "failed",
        "{closed}"
    );
    until("the follow stops", || {
        count(&a.runtime.follows_stopped) == 1
    })
    .await;
    server.abort();
}
