//! The WebSocket over real TCP, through the accept loop: the Hello and its close codes, the
//! stack commands and their authorisation, operations, revocation, and the T090/T092 limits.

mod support;

use std::time::Duration;

use axum::body::Body;
use futures_util::{SinkExt as _, StreamExt as _};
use http::{Request, StatusCode, header};
use serde_json::json;
use support::ws::*;
use support::{App, ORIGIN, OWNER, app, app_with, container};
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt as _;

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

    // Audited: who, which stack, and the secret key names; never a value.
    let entries = a.audit_entries();
    let reveal = entries
        .iter()
        .find(|e| e.action == depths_protocol::AuditAction::RevealEnv)
        .unwrap_or_else(|| panic!("{entries:?}"));
    assert_eq!(reveal.stack.as_deref(), Some("web"));
    assert_eq!(reveal.actor.subject, OWNER);
    assert_eq!(
        reveal.diff_summary.as_deref(),
        Some("revealed .env keys: DB_PASSWORD")
    );
    let bytes = a.audit_bytes();
    assert!(!bytes.is_empty());
    let log = String::from_utf8_lossy(&bytes);
    assert!(!log.contains("hunter2") && !log.contains("8080"), "{log}");
    server.abort();
}

#[tokio::test]
async fn a_user_cannot_reveal_env_even_on_an_assigned_stack() {
    let a = app();
    a.add_stack("web", COMPOSE, Some("DB_PASSWORD=hunter2\nPORT=8080\n"));
    grant(&a, "7", &["web"]);
    let (mut ws, server, _) = open(&a, "7").await;
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
    assert_eq!(r["event"]["type"], "Denied", "{r}");
    assert!(!r.to_string().contains("hunter2"), "{r}");
    let entries = a.audit_entries();
    assert!(
        !entries
            .iter()
            .any(|e| e.action == depths_protocol::AuditAction::RevealEnv),
        "{entries:?}"
    );
    assert!(
        entries
            .iter()
            .any(|e| e.action == depths_protocol::AuditAction::Denied
                && e.stack.as_deref() == Some("web")),
        "the refusal is audited: {entries:?}"
    );
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

// ---------------------------------------------------------------------------
// 051: the container gate on shells, and shells ending with their container
// ---------------------------------------------------------------------------

/// Stack `web` (`web-app-1`) under a shell policy, its container inspecting as `level`.
fn gated_app(policy: depths_core::security::ShellPolicy, level: &str) -> App {
    let a = app_with(|s| {
        let mut cfg = (*s.cfg).clone();
        cfg.shell_policy = policy;
        s.cfg = std::sync::Arc::new(cfg);
    });
    a.add_stack("web", COMPOSE, None);
    a.set_containers(vec![container("web", "app", "running", "Up 1 hour")]);
    match level {
        "ok" => a.set_inspect("web-app-id", json!({ "Config": { "User": "1000:1000" } })),
        "root" => a.set_inspect("web-app-id", json!({ "Config": { "User": "" } })),
        "escape" => a.set_inspect(
            "web-app-id",
            json!({ "Config": { "User": "1000" }, "HostConfig": { "Privileged": true } }),
        ),
        "unknown" => a
            .runtime
            .inspect_fails
            .store(true, std::sync::atomic::Ordering::SeqCst),
        other => panic!("no level {other}"),
    }
    a
}

/// 051 FR-008 to FR-012, every policy x role x level: what `OpenShell` answers, that a refused
/// shell never reaches docker, and that both outcomes are audited with the level and reasons.
#[tokio::test]
async fn a_shell_is_gated_by_policy_role_and_container_level() {
    use depths_core::security::ShellPolicy::{Relaxed, Strict};
    use depths_protocol::{AuditAction, SecurityLevel};
    let allow = "open";
    let table = [
        (Strict, "ok", allow, allow),
        (Strict, "root", allow, "shell_root_admin_only"),
        (Strict, "escape", "shell_escape", "shell_escape"),
        (
            Strict,
            "unknown",
            "shell_unverifiable",
            "shell_unverifiable",
        ),
        (Relaxed, "ok", allow, allow),
        (Relaxed, "root", allow, allow),
        (Relaxed, "escape", allow, "shell_escape"),
        (
            Relaxed,
            "unknown",
            "shell_unverifiable",
            "shell_unverifiable",
        ),
    ];
    for (policy, level, admin, user) in table {
        for (who, want) in [(OWNER, admin), ("7", user)] {
            let a = gated_app(policy, level);
            grant(&a, "7", &["web"]);
            let (mut ws, server, _) = open(&a, who).await;
            let r = ask(&mut ws, 1, open_shell("web-app-1")).await;
            let case = format!("{policy:?} {level} {who}: {r}");
            let audited = a.audit_entries();
            if want == allow {
                stream_of(&r);
                assert_eq!(a.runtime.shells.lock().unwrap().len(), 1, "{case}");
                let e = audited
                    .iter()
                    .find(|e| e.action == AuditAction::ShellOpen)
                    .expect("audited open");
                let sec = e.security.as_ref().expect("the open carries its rating");
                let expect = match level {
                    "ok" => SecurityLevel::Ok,
                    "root" => SecurityLevel::Root,
                    _ => SecurityLevel::Escape,
                };
                assert_eq!(sec.level, expect, "{case}");
            } else {
                assert_eq!(r["event"]["type"], "Denied", "{case}");
                assert_eq!(r["event"]["data"]["reason"], want, "{case}");
                assert!(
                    !r["event"]["data"]["detail"]
                        .as_str()
                        .unwrap_or("")
                        .is_empty(),
                    "{case}"
                );
                assert!(a.runtime.shells.lock().unwrap().is_empty(), "{case}");
                assert!(!audited.iter().any(|e| e.action == AuditAction::ShellOpen));
                let d = audited
                    .iter()
                    .find(|e| e.action == AuditAction::Denied)
                    .expect("audited refusal");
                if level == "unknown" {
                    assert!(d.security.is_none(), "{case}");
                } else {
                    let sec = d.security.as_ref().expect("the refusal carries its rating");
                    assert!(!sec.reasons.is_empty(), "{case}");
                }
            }
            // The gate inspected the container at the moment of the request.
            assert!(
                a.runtime
                    .inspects
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|ids| ids == &["web-app-id".to_owned()]),
                "{case}"
            );
            server.abort();
        }
    }
}

/// 051 FR-008: the gate reads the container now, not the inventory's cached rating: a container
/// rated ok when the list was read and privileged by the time of the request is refused.
#[tokio::test]
async fn the_shell_gate_reads_the_container_at_the_moment_of_the_request() {
    let a = gated_app(depths_core::security::ShellPolicy::Strict, "ok");
    let (mut ws, server, _) = open(&a, OWNER).await;
    let list = ask(&mut ws, 1, json!({ "type": "ListStacks" })).await;
    assert_eq!(
        list["event"]["data"]["stacks"][0]["security"], "ok",
        "{list}"
    );
    a.set_inspect(
        "web-app-id",
        json!({ "HostConfig": { "Privileged": true } }),
    );
    let r = ask(&mut ws, 2, open_shell("web-app-1")).await;
    assert_eq!(r["event"]["data"]["reason"], "shell_escape", "{r}");
    server.abort();
}

/// 051 FR-014: a shell ends when its container is recreated under its name, or removed.
#[tokio::test]
async fn a_shell_ends_when_its_container_is_recreated_or_removed() {
    for (gone, kind) in [(false, "container_recreated"), (true, "container_removed")] {
        let a = gated_app(depths_core::security::ShellPolicy::Strict, "ok");
        let (mut ws, server, _) = open(&a, OWNER).await;
        let id = stream_of(&ask(&mut ws, 1, open_shell("web-app-1")).await);
        if gone {
            a.set_containers(Vec::new());
        } else {
            let mut c = container("web", "app", "running", "Up 1 second");
            c["id"] = json!("web-app-new-id");
            a.set_containers(vec![c]);
        }
        // As a `docker events` would; the shell's own follow keeps the watch running.
        assert_eq!(
            a.state.watch.subscribers(),
            1,
            "the shell follows the inventory"
        );
        a.state.watch.kick();
        until("the reading", || {
            a.state.watch.last().is_some_and(|l| {
                !l.stacks[0].services[0]
                    .containers
                    .iter()
                    .any(|c| c.container.id == "web-app-id")
            })
        })
        .await;
        let closed = next_of(&mut ws, "StreamClosed").await;
        assert_eq!(closed["event"]["data"]["stream_id"], id, "{closed}");
        assert_eq!(closed["event"]["data"]["reason"]["kind"], kind, "{closed}");
        until("the session closes", || {
            count(&a.runtime.shell_log.closed) == 1
        })
        .await;
        server.abort();
    }
}

/// 051 FR-009: a shell open only because its person is an admin closes when they stop being one.
#[tokio::test]
async fn an_admin_only_shell_closes_when_its_person_stops_being_an_admin() {
    use depths_protocol::{Role, StackName, UserId};
    let a = gated_app(depths_core::security::ShellPolicy::Strict, "root");
    let who = UserId::new("discord:8").unwrap();
    let web = [StackName::new("web").unwrap()];
    a.state.users.put(&who, Role::Admin, &web).unwrap();
    let (mut ws, server, _) = open(&a, "8").await;
    let id = stream_of(&ask(&mut ws, 1, open_shell("web-app-1")).await);
    a.state.users.put(&who, Role::User, &web).unwrap();
    let _ = a.state.users_changed.send(());
    let closed = next_of(&mut ws, "StreamClosed").await;
    assert_eq!(closed["event"]["data"]["stream_id"], id, "{closed}");
    assert_eq!(
        closed["event"]["data"]["reason"]["kind"], "failed",
        "{closed}"
    );
    server.abort();
}

// ---------------------------------------------------------------------------
// 051: compose lint on previews, and `DEPTHS_COMPOSE_POLICY=enforce`
// ---------------------------------------------------------------------------

const PRIVILEGED: &str =
    "services:\n  app:\n    image: nginx\n    user: \"1000\"\n    privileged: true\n";

/// Stack `web` with `compose`, user `7` granted it, under a compose policy.
fn lint_app(policy: depths_core::compose_lint::ComposePolicy, compose: &str) -> App {
    let a = app_with(|s| {
        let mut cfg = (*s.cfg).clone();
        cfg.compose_policy = policy;
        s.cfg = std::sync::Arc::new(cfg);
        s.ops.set_compose_policy(policy);
    });
    a.add_stack("web", compose, None);
    grant(&a, "7", &["web"]);
    a
}

fn preview_up() -> serde_json::Value {
    json!({ "type": "PreviewOp", "data": { "stack": "web", "op": { "kind": "up" } } })
}

fn confirm(p: &serde_json::Value, ack: bool) -> serde_json::Value {
    json!({ "type": "Confirm", "data": {
        "preview_id": p["event"]["data"]["preview_id"], "acknowledge_findings": ack,
    } })
}

fn op_entry(a: &App) -> depths_protocol::AuditEntry {
    a.audit_entries()
        .into_iter()
        .find(|e| e.operation_id.is_some())
        .expect("the operation is audited")
}

/// 051 FR-016/FR-017: under `warn` the preview carries the findings and nothing blocks; the
/// audit entry records what the deploy went ahead with.
#[tokio::test]
async fn under_warn_an_up_shows_its_findings_and_runs() {
    use depths_core::compose_lint::ComposePolicy::Warn;
    let a = lint_app(Warn, PRIVILEGED);
    let (mut ws, server, _) = open(&a, "7").await;
    let p = ask(&mut ws, 1, preview_up()).await;
    assert_eq!(p["event"]["type"], "Preview", "{p}");
    let warnings = p["event"]["data"]["warnings"].as_array().unwrap().clone();
    assert!(
        warnings
            .iter()
            .any(|w| w["severity"] == "escape" && w["service"] == "app"),
        "{p}"
    );
    assert_eq!(p["event"]["data"]["ack_required"], false, "{p}");
    let s = ask(&mut ws, 2, confirm(&p, false)).await;
    assert_eq!(s["event"]["type"], "OpStarted", "{s}");
    next_of(&mut ws, "OpFinished").await;
    let sec = op_entry(&a).security.expect("the findings are audited");
    assert_eq!(sec.level, depths_protocol::SecurityLevel::Escape);
    assert!(!sec.acknowledged);
    assert!(sec.reasons.iter().any(|r| r.starts_with("app:")), "{sec:?}");

    // A stop does not deploy the file, so it is not linted.
    let p = ask(
        &mut ws,
        3,
        json!({ "type": "PreviewOp", "data": { "stack": "web", "op": { "kind": "stop" } } }),
    )
    .await;
    assert_eq!(p["event"]["data"]["warnings"], json!([]), "{p}");
    server.abort();
}

/// 051 FR-018: under `enforce` a user's up or save of a file with escape findings is refused
/// with `compose_blocked`, audited with the findings, and nothing runs or is written.
#[tokio::test]
async fn under_enforce_a_user_cannot_deploy_or_save_an_escape() {
    use depths_core::compose_lint::ComposePolicy::Enforce;
    let a = lint_app(Enforce, PRIVILEGED);
    let (mut ws, server, _) = open(&a, "7").await;
    let r = ask(&mut ws, 1, preview_up()).await;
    assert_eq!(r["event"]["type"], "Denied", "{r}");
    assert_eq!(r["event"]["data"]["reason"], "compose_blocked", "{r}");
    assert!(
        r["event"]["data"]["detail"]
            .as_str()
            .unwrap()
            .contains("app:"),
        "{r}"
    );
    let denied = a
        .audit_entries()
        .into_iter()
        .find(|e| e.action == depths_protocol::AuditAction::Denied)
        .expect("audited");
    assert_eq!(
        denied.security.as_ref().map(|s| s.level),
        Some(depths_protocol::SecurityLevel::Escape)
    );

    // Saving a clean file over it is fine; saving an escape is not.
    let d = ask(
        &mut ws,
        2,
        json!({ "type": "GetStack", "data": { "name": "web" } }),
    )
    .await;
    let base = d["event"]["data"]["content_hash"].clone();
    let save = |compose: &str| {
        json!({ "type": "PreviewSave", "data": {
            "draft": { "stack": "web", "base_hash": base, "compose": compose },
            "deploy": true,
        } })
    };
    let r = ask(&mut ws, 3, save(&PRIVILEGED.replace("app:", "web:"))).await;
    assert_eq!(r["event"]["data"]["reason"], "compose_blocked", "{r}");
    let clean = PRIVILEGED.replace("    privileged: true\n", "");
    let r = ask(&mut ws, 4, save(&clean)).await;
    assert_eq!(r["event"]["type"], "Preview", "{r}");
    assert_eq!(r["event"]["data"]["ack_required"], false, "{r}");
    assert!(a.runtime.ran.lock().unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(a.stacks.join("web/compose.yaml")).unwrap(),
        PRIVILEGED,
        "nothing is written or rewritten (FR-019)"
    );
    server.abort();
}

/// 051 FR-018: an admin's preview of an escape needs `acknowledge_findings`; without it the
/// confirm is refused and the preview stays usable, with it the run is audited as acknowledged.
#[tokio::test]
async fn under_enforce_an_admin_deploys_an_escape_only_by_acknowledging_it() {
    use depths_core::compose_lint::ComposePolicy::Enforce;
    let a = lint_app(Enforce, PRIVILEGED);
    let (mut ws, server, _) = open(&a, OWNER).await;
    let p = ask(&mut ws, 1, preview_up()).await;
    assert_eq!(p["event"]["type"], "Preview", "{p}");
    assert_eq!(p["event"]["data"]["ack_required"], true, "{p}");
    let r = ask(&mut ws, 2, confirm(&p, false)).await;
    assert_eq!(r["event"]["data"]["reason"], "compose_blocked", "{r}");
    assert!(a.runtime.ran.lock().unwrap().is_empty());
    let s = ask(&mut ws, 3, confirm(&p, true)).await;
    assert_eq!(s["event"]["type"], "OpStarted", "{s}");
    next_of(&mut ws, "OpFinished").await;
    let sec = op_entry(&a)
        .security
        .expect("the acknowledgement is audited");
    assert_eq!(sec.level, depths_protocol::SecurityLevel::Escape);
    assert!(sec.acknowledged);
    server.abort();
}

/// 051 FR-018: root-level findings never block, even under `enforce`.
#[tokio::test]
async fn under_enforce_root_findings_never_block() {
    use depths_core::compose_lint::ComposePolicy::Enforce;
    let a = lint_app(Enforce, COMPOSE);
    let (mut ws, server, _) = open(&a, "7").await;
    let p = ask(&mut ws, 1, preview_up()).await;
    assert_eq!(p["event"]["type"], "Preview", "{p}");
    assert_eq!(p["event"]["data"]["ack_required"], false, "{p}");
    assert!(
        p["event"]["data"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["severity"] == "root"),
        "{p}"
    );
    let s = ask(&mut ws, 2, confirm(&p, false)).await;
    assert_eq!(s["event"]["type"], "OpStarted", "{s}");
    next_of(&mut ws, "OpFinished").await;
    let sec = op_entry(&a).security.expect("root findings are audited");
    assert_eq!(sec.level, depths_protocol::SecurityLevel::Root);
    server.abort();
}
