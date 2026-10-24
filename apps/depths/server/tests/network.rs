//! Spec 052: the network gate end to end. Who is trusted where (T011), what an untrusted network
//! may and may not do over the socket and over HTTP (T011), ending sessions (T015), and the admin
//! whitelist (T019).
//!
//! Test sockets always come from loopback. To put a socket on an untrusted network, the person
//! signs in from another address first, so that one becomes their first (trusted) network and
//! loopback is unknown.

mod support;

use std::net::IpAddr;
use std::time::Duration;

use axum::body::Body;
use chrono::{TimeDelta, TimeZone as _, Utc};
use depths_core::network::{Answer, NetworkKey, TrustReason};
use futures_util::SinkExt as _;
use http::{Request, Response, StatusCode, header};
use serde_json::{Value, json};
use support::ws::*;
use support::{App, LOOPBACK, ORIGIN, OWNER, app, app_with_env, from, json as body_json};
use tokio_tungstenite::tungstenite::Message;

const AWAY: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(198, 51, 100, 7));
const CAFE: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 9));
const USER: &str = "2002";

/// A mutation from `ip` with the session's CSRF token.
async fn mutate(
    a: &App,
    ip: IpAddr,
    method: http::Method,
    path: &str,
    session: &str,
    body: Option<Value>,
) -> Response<Body> {
    let csrf = csrf_of(a, session).await;
    let b = Request::builder()
        .method(method)
        .uri(path)
        .header(header::COOKIE, session)
        .header(header::ORIGIN, ORIGIN)
        .header("x-csrf-token", csrf);
    let req = match body {
        Some(v) => b
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    a.send(from(ip, req)).await
}

async fn me_from(a: &App, ip: IpAddr, session: &str) -> Value {
    let r = a.get_from(ip, "/api/v1/me", Some(session)).await;
    assert_eq!(r.status(), StatusCode::OK);
    body_json(r).await
}

fn count_action(a: &App, action: &str) -> usize {
    a.audit_entries()
        .iter()
        .filter(|e| serde_json::to_value(e.action).unwrap() == action)
        .count()
}

/// Signs `subject` in from `first`, then opens a socket from loopback (an unknown network).
async fn open_untrusted(
    a: &App,
    subject: &str,
    first: IpAddr,
) -> (Socket, tokio::task::JoinHandle<()>, String) {
    let session = a.sign_in_from(first, subject).await;
    let csrf = csrf_of(a, &session).await;
    let (mut ws, server) = connect(a, Some(&session)).await;
    ws.send(Message::text(hello(&csrf))).await.unwrap();
    (ws, server, session)
}

fn cmd(ty: &str, data: Value) -> Value {
    json!({ "type": ty, "data": data })
}

// ---- who is trusted where (T011) ----------------------------------------------------------

#[tokio::test]
async fn the_first_network_is_trusted_and_any_other_is_unknown_and_asked() {
    let a = app();
    let session = a.sign_in_from(CAFE, OWNER).await;

    let first = me_from(&a, CAFE, &session).await;
    assert_eq!(first["network"]["trusted"], true, "{first}");
    assert_eq!(first["network"]["reason"], "first");
    assert_eq!(first["network"]["network"], "203.0.113.9/32");
    assert_eq!(first["needs_answer"], false);

    let other = me_from(&a, AWAY, &session).await;
    assert_eq!(other["network"]["trusted"], false, "{other}");
    assert_eq!(other["network"]["reason"], "unknown");
    assert_eq!(other["network"]["network"], "198.51.100.7/32");
    assert_eq!(other["needs_answer"], true);
    assert_eq!(count_action(&a, "network_first"), 1);
    assert_eq!(count_action(&a, "network_seen"), 1);
}

#[tokio::test]
async fn an_answer_never_trusts_at_once_and_public_never_matures() {
    let a = app();
    let session = a.sign_in_from(CAFE, OWNER).await;
    let _ = me_from(&a, AWAY, &session).await;

    let answer = |answer: &'static str| json!({ "network": "198.51.100.7/32", "answer": answer });
    let r = mutate(
        &a,
        AWAY,
        http::Method::POST,
        "/api/v1/networks/answer",
        &session,
        Some(answer("home")),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    let d = body_json(r).await;
    assert_eq!(d["trusted"], false, "{d}");
    assert_eq!(d["reason"], "maturing");
    assert!(d["matures_at"].is_string(), "{d}");
    assert_eq!(me_from(&a, AWAY, &session).await["needs_answer"], false);

    let r = mutate(
        &a,
        AWAY,
        http::Method::POST,
        "/api/v1/networks/answer",
        &session,
        Some(answer("public")),
    )
    .await;
    let d = body_json(r).await;
    assert_eq!(d["trusted"], false, "{d}");
    assert_eq!(d["reason"], "public");
    assert_eq!(count_action(&a, "network_answer"), 2);

    // Not a network this person used; and `none` is not an answer.
    let r = mutate(
        &a,
        AWAY,
        http::Method::POST,
        "/api/v1/networks/answer",
        &session,
        Some(json!({ "network": "192.0.2.0/24", "answer": "home" })),
    )
    .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let r = mutate(
        &a,
        AWAY,
        http::Method::POST,
        "/api/v1/networks/answer",
        &session,
        Some(answer("none")),
    )
    .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn home_matures_after_the_window_and_two_distinct_days() {
    let a = app();
    let nets = &a.state.trust.networks;
    let user = "discord:3003";
    let t0 = Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap();
    nets.record(user, CAFE, t0).unwrap();
    nets.record(user, AWAY, t0).unwrap();
    let key = nets.key_of(AWAY);
    nets.answer(user, &key, Answer::Home).unwrap();

    let at = |days: i64| nets.trust(user, AWAY, t0 + TimeDelta::days(days));
    assert_eq!(at(0).reason, TrustReason::Maturing);
    // The window passed, but it was used on one day only.
    assert_eq!(at(30).reason, TrustReason::Maturing);
    nets.record(user, AWAY, t0 + TimeDelta::days(1)).unwrap();
    assert_eq!(at(6).reason, TrustReason::Maturing);
    let d = at(7);
    assert!(d.trusted, "{d:?}");
    assert_eq!(d.reason, TrustReason::Matured);

    nets.answer(user, &key, Answer::Public).unwrap();
    assert_eq!(at(365).reason, TrustReason::Public);
}

#[tokio::test]
async fn always_trusted_networks_and_the_gate_off_switch() {
    let a = app_with_env(&[("DEPTHS_TRUSTED_NETWORKS", "198.51.100.0/24")], |_| {});
    let session = a.sign_in_from(CAFE, OWNER).await;
    let d = me_from(&a, AWAY, &session).await;
    assert_eq!(d["network"]["trusted"], true, "{d}");
    assert_eq!(d["network"]["reason"], "always");

    let a = app_with_env(&[("DEPTHS_NETWORK_GATE", "false")], |_| {});
    let session = a.sign_in_from(CAFE, OWNER).await;
    let d = me_from(&a, AWAY, &session).await;
    assert_eq!(d["network"]["trusted"], true, "{d}");
    assert_eq!(d["network"]["reason"], "gate_off");
    assert_eq!(d["needs_answer"], false);
}

// ---- what an untrusted network may do (T011) ----------------------------------------------

#[tokio::test]
async fn an_untrusted_socket_is_refused_every_dangerous_command_and_allowed_the_rest() {
    let a = streams_app();
    let (mut ws, server, _) = open_untrusted(&a, OWNER, CAFE).await;

    let refused = [
        cmd("RevealEnv", json!({ "name": "web" })),
        cmd(
            "PreviewOp",
            json!({ "stack": "web", "op": { "kind": "down", "options": {} } }),
        ),
        cmd(
            "PreviewOp",
            json!({ "stack": "web", "op": { "kind": "pull" } }),
        ),
        cmd(
            "PreviewOp",
            json!({ "stack": "web", "op": { "kind": "update" } }),
        ),
        cmd(
            "PreviewOp",
            json!({ "stack": "web", "op": { "kind": "delete", "options": {} } }),
        ),
        cmd(
            "PreviewSave",
            json!({ "draft": { "stack": "fresh", "compose": COMPOSE }, "deploy": false }),
        ),
        open_shell("web-app-1"),
        cmd("GetAudit", json!({ "filter": {} })),
        cmd(
            "PutUser",
            json!({ "id": "discord:555", "role": "user", "stacks": ["web"] }),
        ),
        cmd("RemoveUser", json!({ "id": "discord:555" })),
    ];
    for (i, c) in refused.iter().enumerate() {
        let r = ask(&mut ws, 10 + i as u64, c.clone()).await;
        assert_eq!(r["event"]["type"], "Denied", "{c}: {r}");
        assert_eq!(
            r["event"]["data"]["reason"], "untrusted_network",
            "{c}: {r}"
        );
    }
    assert_eq!(count_action(&a, "network_refused"), refused.len());
    let mut cmds: Vec<String> = a
        .audit_entries()
        .into_iter()
        .filter(|e| serde_json::to_value(e.action).unwrap() == "network_refused")
        .map(|e| {
            let n = e.network.unwrap();
            assert_eq!(n.network.as_deref(), Some("127.0.0.1/32"));
            n.cmd.unwrap()
        })
        .collect();
    cmds.sort();
    let mut want = [
        "GetAudit",
        "OpenShell",
        "PreviewOp",
        "PreviewOp",
        "PreviewOp",
        "PreviewOp",
        "PreviewSave",
        "PutUser",
        "RemoveUser",
        "RevealEnv",
    ];
    want.sort_unstable();
    assert_eq!(cmds, want);

    // The restricted set still works: view, logs, and start/stop/restart.
    // Listing users is a view, so an admin can still find a session to end.
    let r = ask(&mut ws, 49, json!({ "type": "GetUsers" })).await;
    assert_eq!(r["event"]["type"], "Users", "{r}");
    let r = ask(&mut ws, 50, json!({ "type": "ListStacks" })).await;
    assert_ne!(r["event"]["type"], "Denied", "{r}");
    // The stack, but not its compose file or `.env`.
    let r = ask(&mut ws, 51, cmd("GetStack", json!({ "name": "web" }))).await;
    assert_eq!(r["event"]["type"], "StackDetail", "{r}");
    assert!(r["event"]["data"]["compose_text"].is_null(), "{r}");
    assert!(r["event"]["data"]["env"].is_null(), "{r}");
    let r = ask(
        &mut ws,
        52,
        open_logs(json!({ "kind": "stack", "stack": "web" })),
    )
    .await;
    stream_of(&r);
    for (i, kind) in ["up", "stop"].into_iter().enumerate() {
        let r = ask(
            &mut ws,
            53 + i as u64,
            cmd(
                "PreviewOp",
                json!({ "stack": "web", "op": { "kind": kind } }),
            ),
        )
        .await;
        assert_eq!(r["event"]["type"], "Preview", "{kind}: {r}");
    }
    let p = ask(
        &mut ws,
        60,
        cmd(
            "PreviewOp",
            json!({ "stack": "web", "op": { "kind": "restart" } }),
        ),
    )
    .await;
    assert_eq!(p["event"]["type"], "Preview", "{p}");
    let s = ask(
        &mut ws,
        61,
        cmd(
            "Confirm",
            json!({ "preview_id": p["event"]["data"]["preview_id"] }),
        ),
    )
    .await;
    assert_eq!(s["event"]["type"], "OpStarted", "{s}");
    assert_eq!(count_action(&a, "network_refused"), refused.len());
    server.abort();
}

#[tokio::test]
async fn the_compose_file_and_env_are_sent_only_to_a_trusted_network() {
    let get = json!({ "type": "GetStack", "data": { "name": "game" } });
    let game = || {
        let a = streams_app();
        a.add_stack("game", COMPOSE, Some("TOKEN=hunter2\nPORT=2456\n"));
        a
    };

    let a = game();
    let session = a.sign_in(OWNER).await;
    let csrf = csrf_of(&a, &session).await;
    let (mut ws, _server) = connect(&a, Some(&session)).await;
    ws.send(Message::text(hello(&csrf))).await.unwrap();
    let r = ask(&mut ws, 1, get.clone()).await;
    assert_eq!(r["event"]["data"]["compose_text"], COMPOSE, "{r}");
    assert!(!r["event"]["data"]["env"].is_null(), "{r}");

    let a = game();
    let (mut ws, _server, _) = open_untrusted(&a, OWNER, CAFE).await;
    let r = ask(&mut ws, 1, get).await;
    assert_eq!(r["event"]["type"], "StackDetail", "{r}");
    assert!(r["event"]["data"]["compose_text"].is_null(), "{r}");
    assert!(r["event"]["data"]["env"].is_null(), "{r}");
    // Withholding them is not a refusal: nothing is audited.
    assert_eq!(count_action(&a, "network_refused"), 0);
}

#[tokio::test]
async fn the_role_check_comes_first_so_a_user_still_sees_not_permitted() {
    let a = streams_app();
    grant(&a, USER, &["web"]);
    let (mut ws, server, _) = open_untrusted(&a, USER, CAFE).await;
    let r = ask(&mut ws, 1, json!({ "type": "GetUsers" })).await;
    assert_eq!(r["event"]["data"]["reason"], "not_permitted", "{r}");
    assert_eq!(count_action(&a, "network_refused"), 0);
    server.abort();
}

#[tokio::test]
async fn crafted_http_requests_from_an_untrusted_network_are_refused() {
    let a = app();
    let session = a.sign_in_from(CAFE, OWNER).await;

    let r = a
        .get_from(AWAY, "/api/v1/audit/export", Some(&session))
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let b = body_json(r).await;
    assert_eq!(b["error"], "untrusted_network", "{b}");
    assert_eq!(b["network"], "198.51.100.7/32");
    assert_eq!(b["reason"], "unknown");

    // An admin may not whitelist their own way in.
    let r = mutate(
        &a,
        AWAY,
        http::Method::POST,
        &format!("/api/v1/users/discord:{OWNER}/networks/whitelist"),
        &session,
        Some(json!({ "cidr": "198.51.100.0/24" })),
    )
    .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(r).await["error"], "untrusted_network");
    assert!(
        !me_from(&a, AWAY, &session).await["network"]["trusted"]
            .as_bool()
            .unwrap()
    );

    // From the trusted network, the same requests work.
    let r = a
        .get_from(CAFE, "/api/v1/audit/export", Some(&session))
        .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(count_action(&a, "network_refused"), 2);
}

#[tokio::test]
async fn using_a_session_from_an_untrusted_network_ends_its_shells() {
    let a = streams_app();
    let (mut ws, server, session) = open(&a, OWNER).await;
    let r = ask(&mut ws, 1, open_shell("web-app-1")).await;
    let shell = stream_of(&r);

    let _ = a.get_from(AWAY, "/api/v1/me", Some(&session)).await;
    let closed = next_of(&mut ws, "StreamClosed").await;
    assert_eq!(closed["event"]["data"]["stream_id"], shell, "{closed}");
    assert!(closed.to_string().contains("untrusted network"), "{closed}");
    server.abort();
}

// ---- ending sessions (T015) ---------------------------------------------------------------

#[tokio::test]
async fn ending_other_sessions_closes_their_sockets_and_signs_them_out() {
    let a = app();
    let (mut ws, server, old) = open(&a, OWNER).await;
    let current = a.sign_in(OWNER).await;

    let list = body_json(a.get("/api/v1/sessions", Some(&current)).await).await;
    let rows = list.as_array().unwrap();
    assert_eq!(rows.len(), 2, "{list}");
    assert_eq!(rows.iter().filter(|r| r["current"] == true).count(), 1);
    assert!(rows.iter().all(|r| r["address"] == "127.0.0.1"), "{list}");

    let r = mutate(
        &a,
        LOOPBACK,
        http::Method::POST,
        "/api/v1/sessions/end-others",
        &current,
        None,
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(body_json(r).await["ended"], 1);

    assert_eq!(close_code(&mut ws, Duration::from_secs(5)).await, 4401);
    assert_eq!(
        a.get("/api/v1/me", Some(&old)).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        a.get("/api/v1/me", Some(&current)).await.status(),
        StatusCode::OK
    );
    assert_eq!(count_action(&a, "session_end"), 1);
    server.abort();
}

#[tokio::test]
async fn a_session_can_be_ended_from_an_untrusted_network() {
    let a = app();
    let lost = a.sign_in_from(CAFE, OWNER).await;
    let here = a.sign_in_from(CAFE, OWNER).await;
    let list = body_json(a.get_from(AWAY, "/api/v1/sessions", Some(&here)).await).await;
    let hint = list
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["current"] == false)
        .unwrap()["id_hint"]
        .as_str()
        .unwrap()
        .to_owned();

    let r = mutate(
        &a,
        AWAY,
        http::Method::DELETE,
        &format!("/api/v1/sessions/{hint}"),
        &here,
        None,
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(
        a.get("/api/v1/me", Some(&lost)).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let r = mutate(
        &a,
        AWAY,
        http::Method::DELETE,
        &format!("/api/v1/sessions/{hint}"),
        &here,
        None,
    )
    .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}

// ---- the admin whitelist (T019) -----------------------------------------------------------

#[tokio::test]
async fn an_admin_whitelists_from_a_trusted_network_and_removal_reverts_to_unknown() {
    let a = app();
    grant(&a, USER, &["web"]);
    let admin = a.sign_in(OWNER).await;
    let user = a.sign_in_from(CAFE, USER).await;
    assert_eq!(
        me_from(&a, AWAY, &user).await["network"]["reason"],
        "unknown"
    );

    let id = format!("discord:{USER}");
    let r = mutate(
        &a,
        LOOPBACK,
        http::Method::POST,
        &format!("/api/v1/users/{id}/networks/whitelist"),
        &admin,
        Some(json!({ "cidr": "198.51.100.0/24", "label": "office" })),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    let rows = body_json(r).await;
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["key"] == "198.51.100.0/24")
        .unwrap()
        .clone();
    assert_eq!(row["source"], "whitelist", "{row}");
    assert_eq!(row["label"], "office");

    let d = me_from(&a, AWAY, &user).await;
    assert_eq!(d["network"]["trusted"], true, "{d}");
    assert_eq!(d["network"]["reason"], "whitelist");

    let r = mutate(
        &a,
        LOOPBACK,
        http::Method::DELETE,
        &format!("/api/v1/users/{id}/networks/198.51.100.0/24"),
        &admin,
        None,
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    let d = me_from(&a, AWAY, &user).await;
    assert_eq!(d["network"]["trusted"], false, "{d}");
    assert_eq!(d["network"]["reason"], "unknown");
    assert_eq!(count_action(&a, "network_whitelist_add"), 1);
    assert_eq!(count_action(&a, "network_whitelist_remove"), 1);

    // Forgetting a used network: the next use is a new one again.
    let r = mutate(
        &a,
        LOOPBACK,
        http::Method::DELETE,
        &format!("/api/v1/users/{id}/networks/198.51.100.7/32"),
        &admin,
        None,
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(count_action(&a, "network_forget"), 1);
}

#[tokio::test]
async fn whitelisting_needs_an_admin_on_a_trusted_network() {
    let a = app();
    grant(&a, USER, &["web"]);
    let admin = a.sign_in_from(CAFE, OWNER).await;
    let user = a.sign_in(USER).await;
    let path = format!("/api/v1/users/discord:{USER}/networks/whitelist");
    let body = json!({ "cidr": "192.0.2.0/24" });

    // From an untrusted network: refused, audited, nothing changes.
    let r = mutate(
        &a,
        AWAY,
        http::Method::POST,
        &path,
        &admin,
        Some(body.clone()),
    )
    .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(r).await["error"], "untrusted_network");
    assert_eq!(count_action(&a, "network_refused"), 1);
    let key = NetworkKey::parse("192.0.2.0/24").unwrap();
    assert!(
        a.state
            .trust
            .networks
            .list(&format!("discord:{USER}"))
            .iter()
            .all(|n| n.key != key)
    );

    // Not an admin: forbidden before the network is considered.
    let r = mutate(
        &a,
        LOOPBACK,
        http::Method::POST,
        &path,
        &user,
        Some(body.clone()),
    )
    .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(r).await["error"], "forbidden");

    // An unknown person, and a CIDR that does not parse.
    let r = mutate(
        &a,
        CAFE,
        http::Method::POST,
        "/api/v1/users/discord:999/networks/whitelist",
        &admin,
        Some(body),
    )
    .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let r = mutate(
        &a,
        CAFE,
        http::Method::POST,
        &path,
        &admin,
        Some(json!({ "cidr": "home" })),
    )
    .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
}
