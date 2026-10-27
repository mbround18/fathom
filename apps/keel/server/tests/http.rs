//! The HTTP and WebSocket surface (spec 055 T025, T026, T029): sign-in, sessions, CSRF, the
//! command endpoint, the socket and static serving. Needs the test cluster
//! (`KEEL_TEST_DATABASE_URL`); skips without it.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::TestDb;
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use keel_core::ports::SystemClock;
use keel_protocol::{Cmd, CmdEnvelope, Event, EventEnvelope, PreviewId, RequestId};
use keel_server::config::{Config, Flags};
use keel_server::serve::{self, AppState};
use keel_server::signin::{self, Established};
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tower::ServiceExt;
use uuid::Uuid;
use web_auth::session::{Identity, Provider, Tokens};

const ORIGIN: &str = "https://keel.example.com";

struct Harness {
    db: TestDb,
    app: AppState,
    _static: tempfile::TempDir,
}

impl Harness {
    async fn new(test: &str) -> Option<Self> {
        let db = TestDb::new(test).await?;
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("index.html"),
            "<!doctype html><p>keel spa</p>",
        )
        .expect("index");
        std::fs::create_dir(dir.path().join("assets")).expect("assets");
        std::fs::write(dir.path().join("assets/app.js"), "console.log(1)").expect("asset");
        let env: HashMap<&str, String> = HashMap::from([
            ("KEEL_PUBLIC_URL", ORIGIN.to_owned()),
            ("KEEL_DATABASE_URL", "postgres://unused/keel".to_owned()),
            ("KEEL_MASTER_KEY_FILE", "/nonexistent/master.key".to_owned()),
            ("KEEL_GITHUB_CLIENT_ID", "gh-id".to_owned()),
            ("KEEL_GITHUB_CLIENT_SECRET", "gh-secret".to_owned()),
            ("KEEL_STATIC_DIR", dir.path().display().to_string()),
        ]);
        let cfg = Config::resolve(Flags::default(), &|k| env.get(k).cloned()).expect("config");
        let providers = serve::build_providers(&cfg).await.expect("providers");
        let app = AppState::new(
            Arc::new(cfg),
            providers,
            db.app.clone(),
            Arc::new(SystemClock),
        );
        Some(Self {
            db,
            app,
            _static: dir,
        })
    }

    async fn sign_in(&self, sub: &str, name: &str) -> Established {
        let who = Identity {
            provider: Provider::Github,
            issuer: "https://github.com".into(),
            sub: sub.into(),
            username: name.into(),
            groups: vec![],
            email: Some(format!("{name}@example.com")),
        };
        let tokens = Tokens {
            id_token: SecretString::from(String::new()),
            access_token: SecretString::from("provider-access-token".to_owned()),
            refresh_token: None,
            expires_at: tokio::time::Instant::now() + Duration::from_secs(3600),
        };
        signin::establish(&self.app, who, tokens, json!({ "provider": "github" }))
            .await
            .unwrap_or_else(|e| panic!("establish: {e:?}"))
    }

    async fn call(&self, req: Request<Body>) -> (StatusCode, Vec<u8>, header::HeaderMap) {
        let r = serve::router(self.app.clone())
            .oneshot(req)
            .await
            .expect("infallible");
        let status = r.status();
        let headers = r.headers().clone();
        let body = r.into_body().collect().await.expect("body").to_bytes();
        (status, body.to_vec(), headers)
    }

    async fn cmd(&self, est: Option<&Established>, cmd: Cmd) -> (StatusCode, Value) {
        let mut b = Request::post("/api/v1/cmd")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, ORIGIN);
        if let Some(e) = est {
            b = b
                .header(header::COOKIE, cookie_pair(e))
                .header("x-csrf-token", &e.csrf);
        }
        let body = serde_json::to_vec(&CmdEnvelope {
            id: RequestId(7),
            cmd,
        })
        .expect("json");
        let (status, body, _) = self.call(b.body(Body::from(body)).expect("req")).await;
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    async fn audit_rows(&self) -> Vec<Value> {
        let rows: Vec<(Value,)> = sqlx::query_as(
            "select to_jsonb(a) - 'hash' - 'prev_hash' from audit_event a order by seq",
        )
        .fetch_all(&self.db.owner)
        .await
        .expect("audit rows");
        rows.into_iter().map(|r| r.0).collect()
    }
}

/// `name=value` from the session cookie's `Set-Cookie`.
fn cookie_pair(e: &Established) -> String {
    let set = e.cookies[0].to_str().expect("ascii");
    set.split(';').next().expect("pair").to_owned()
}

fn events(v: &Value) -> Vec<EventEnvelope> {
    serde_json::from_value(v.clone()).expect("event envelopes")
}

#[tokio::test]
async fn unauthenticated_cmd_is_401_and_unknown_api_is_json_404() {
    let Some(h) = Harness::new("unauthenticated").await else {
        return;
    };
    let (status, body) = h.cmd(None, Cmd::GetMe).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "unauthenticated");
    assert_eq!(body["loginUrl"], "/auth/login");

    let (status, body, _) = h
        .call(Request::get("/api/v1/nope").body(Body::empty()).unwrap())
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["error"],
        "not_found"
    );
    let (status, _, _) = h
        .call(Request::get("/auth/nope").body(Body::empty()).unwrap())
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    h.db.drop().await;
}

#[tokio::test]
async fn static_files_and_spa_fallback_and_health() {
    let Some(h) = Harness::new("static").await else {
        return;
    };
    let (status, body, _) = h
        .call(Request::get("/assets/app.js").body(Body::empty()).unwrap())
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"console.log(1)");
    let (status, body, _) = h
        .call(
            Request::get("/workspaces/infra/runs")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&body).contains("keel spa"));
    let (status, _, _) = h
        .call(
            Request::get("/assets/missing.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for path in ["/healthz", "/readyz"] {
        let (status, _, _) = h
            .call(Request::get(path).body(Body::empty()).unwrap())
            .await;
        assert_eq!(status, StatusCode::OK, "{path}");
    }
    h.db.drop().await;
}

#[tokio::test]
async fn first_user_is_owner_and_secrets_stay_out_of_the_audit_log() {
    let Some(h) = Harness::new("first_owner").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let bob = h.sign_in("2", "bob").await;
    assert_eq!(alice.user.instance_role.as_str(), "owner");
    assert_eq!(bob.user.instance_role.as_str(), "user");
    // Signing in again keeps the role and the id.
    let again = h.sign_in("1", "alice").await;
    assert_eq!(again.user.id, alice.user.id);
    assert_eq!(again.user.instance_role.as_str(), "owner");

    let cookie = alice.cookies[0].to_str().unwrap();
    assert!(cookie.starts_with("__Host-keel_session="));
    assert!(cookie.contains("HttpOnly"));
    let csrf = alice.cookies[1].to_str().unwrap();
    assert!(csrf.starts_with("__Host-keel_csrf="));
    assert!(csrf.contains("Secure") && csrf.contains("SameSite=Strict"));
    assert!(!csrf.contains("HttpOnly"));

    let rows = h.audit_rows().await;
    let actions: Vec<&str> = rows.iter().map(|r| r["action"].as_str().unwrap()).collect();
    assert_eq!(
        actions,
        [
            "user_create",
            "sign_in",
            "user_create",
            "sign_in",
            "sign_in"
        ]
    );
    assert_eq!(rows[0]["reason"], "first_user_is_owner");
    assert_eq!(rows[2]["reason"], Value::Null);
    let dump = serde_json::to_string(&rows).unwrap();
    for e in [&alice, &bob, &again] {
        let value = cookie_pair(e);
        let value = value.split_once('=').unwrap().1;
        assert!(!dump.contains(value), "session cookie in the audit log");
        assert!(
            !dump.contains(&e.session),
            "session handle in the audit log"
        );
        assert!(!dump.contains(&e.csrf), "csrf token in the audit log");
    }
    assert!(!dump.contains("provider-access-token"));
    h.db.drop().await;
}

#[tokio::test]
async fn get_me_over_cmd_with_session_and_csrf() {
    let Some(h) = Harness::new("get_me").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let (status, body) = h.cmd(Some(&alice), Cmd::GetMe).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let evs = events(&body);
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].reply_to, Some(RequestId(7)));
    let Event::Me(me) = &evs[0].event else {
        panic!("not Me: {body}");
    };
    assert_eq!(me.id, alice.user.id);
    assert_eq!(me.display_name, "alice");
    assert_eq!(serde_json::to_value(me.instance_role).unwrap(), "owner");
    assert!(me.grants.is_empty());
    assert!(!me.verification.verified && !me.verification.enrolled);
    h.db.drop().await;
}

#[tokio::test]
async fn cmd_without_csrf_or_from_another_origin_is_403() {
    let Some(h) = Harness::new("csrf").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let body = || Body::from(r#"{"id":1,"cmd":{"type":"GetMe"}}"#);
    let cases = [
        // No token.
        Request::post("/api/v1/cmd")
            .header(header::ORIGIN, ORIGIN)
            .header(header::COOKIE, cookie_pair(&alice))
            .body(body())
            .unwrap(),
        // Wrong token.
        Request::post("/api/v1/cmd")
            .header(header::ORIGIN, ORIGIN)
            .header(header::COOKIE, cookie_pair(&alice))
            .header("x-csrf-token", "not-the-token")
            .body(body())
            .unwrap(),
        // Another origin.
        Request::post("/api/v1/cmd")
            .header(header::ORIGIN, "https://evil.example.com")
            .header(header::COOKIE, cookie_pair(&alice))
            .header("x-csrf-token", &alice.csrf)
            .body(body())
            .unwrap(),
        // Cross-site fetch.
        Request::post("/api/v1/cmd")
            .header(header::ORIGIN, ORIGIN)
            .header("sec-fetch-site", "cross-site")
            .header(header::COOKIE, cookie_pair(&alice))
            .header("x-csrf-token", &alice.csrf)
            .body(body())
            .unwrap(),
    ];
    for req in cases {
        let (status, body, _) = h.call(req).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["error"],
            "csrf"
        );
    }
    // Logout is behind the same checks.
    let (status, _, _) = h
        .call(
            Request::post("/auth/logout")
                .header(header::ORIGIN, ORIGIN)
                .header(header::COOKIE, cookie_pair(&alice))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    h.db.drop().await;
}

#[tokio::test]
async fn an_epoch_bump_ends_the_session_at_the_next_command() {
    let Some(h) = Harness::new("epoch").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    assert_eq!(h.cmd(Some(&alice), Cmd::GetMe).await.0, StatusCode::OK);
    let mut conn = h.db.app.acquire().await.unwrap();
    keel_server::identity::bump_epoch(&mut conn, alice.user.id)
        .await
        .unwrap();
    drop(conn);
    let (status, body) = h.cmd(Some(&alice), Cmd::GetMe).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    // The in-memory session is gone too, and a fresh sign-in works at the new epoch.
    assert!(h.app.auth.sessions.peek(&alice.session).is_none());
    let again = h.sign_in("1", "alice").await;
    assert_eq!(h.cmd(Some(&again), Cmd::GetMe).await.0, StatusCode::OK);
    h.db.drop().await;
}

#[tokio::test]
async fn a_bearer_token_is_scoped_and_refusals_are_audited() {
    let Some(h) = Harness::new("token").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let ws = Uuid::now_v7();
    sqlx::query("insert into workspaces (id, slug, name) values ($1, 'infra', 'Infra')")
        .bind(ws)
        .execute(&h.db.owner)
        .await
        .unwrap();
    let insert = |token: &'static str, actions: Vec<&'static str>| {
        let pool = h.db.owner.clone();
        let user = alice.user.id;
        async move {
            let id = Uuid::now_v7();
            sqlx::query(
                "insert into api_tokens (id, name, owner_user_id, workspace_id, actions, hash, \
                 expires_at) values ($1, 'ci', $2, $3, $4, $5, now() + interval '1 hour')",
            )
            .bind(id)
            .bind(user)
            .bind(ws)
            .bind(actions)
            .bind(keel_server::identity::token_hash(token).as_slice())
            .execute(&pool)
            .await
            .unwrap();
            id
        }
    };
    let narrow = insert("kt_narrow", vec!["run_create"]).await;
    insert("kt_me", vec!["get_me"]).await;

    let call = |token: &'static str| {
        Request::post("/api/v1/cmd")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(r#"{"id":3,"cmd":{"type":"GetMe"}}"#))
            .unwrap()
    };
    let (status, body, _) = h.call(call("kt_narrow")).await;
    assert_eq!(status, StatusCode::OK);
    let evs: Vec<EventEnvelope> = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        evs[0].event,
        Event::Denied {
            action: "get_me".into(),
            reason: "token_scope".into()
        }
    );
    let rows = h.audit_rows().await;
    let denied = rows.last().unwrap();
    assert_eq!(denied["action"], "get_me");
    assert_eq!(denied["decision"], "denied");
    assert_eq!(denied["reason"], "token_scope");
    assert_eq!(denied["workspace_id"], json!(ws));
    assert_eq!(denied["context"]["token"], json!(narrow));
    assert_eq!(denied["context"]["channel"], "api");
    assert!(!serde_json::to_string(&rows).unwrap().contains("kt_narrow"));

    let (status, body, _) = h.call(call("kt_me")).await;
    assert_eq!(status, StatusCode::OK);
    let evs: Vec<EventEnvelope> = serde_json::from_slice(&body).unwrap();
    assert!(matches!(&evs[0].event, Event::Me(me) if me.id == alice.user.id));

    let (status, _, _) = h.call(call("kt_unknown")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    h.db.drop().await;
}

#[tokio::test]
async fn confirm_of_an_unknown_preview_is_an_op_error_and_bad_bodies_are_400() {
    let Some(h) = Harness::new("confirm").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let (status, body) = h
        .cmd(
            Some(&alice),
            Cmd::Confirm {
                preview_id: PreviewId(Uuid::now_v7()),
                step_up: None,
            },
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let evs = events(&body);
    assert!(
        matches!(&evs[0].event, Event::OpError { code, .. } if code == "preview_unknown"),
        "{body}"
    );
    let (status, body, _) = h
        .call(
            Request::post("/api/v1/cmd")
                .header(header::ORIGIN, ORIGIN)
                .header(header::COOKIE, cookie_pair(&alice))
                .header("x-csrf-token", &alice.csrf)
                .body(Body::from(r#"{"id":1,"cmd":{"type":"Nope"}}"#))
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["error"],
        "bad_request"
    );
    h.db.drop().await;
}

#[tokio::test]
async fn logout_ends_the_session_and_is_audited() {
    let Some(h) = Harness::new("logout").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let (status, body, headers) = h
        .call(
            Request::post("/auth/logout")
                .header(header::ORIGIN, ORIGIN)
                .header(header::COOKIE, cookie_pair(&alice))
                .header("x-csrf-token", &alice.csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["endSessionUrl"],
        Value::Null
    );
    let cleared: Vec<&str> = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap())
        .collect();
    assert!(
        cleared
            .iter()
            .any(|c| c.starts_with("__Host-keel_session=;"))
    );
    assert!(cleared.iter().any(|c| c.starts_with("__Host-keel_csrf=;")));
    let (n,): (i64,) = sqlx::query_as("select count(*) from sessions")
        .fetch_one(&h.db.owner)
        .await
        .unwrap();
    assert_eq!(n, 0);
    assert_eq!(
        h.cmd(Some(&alice), Cmd::GetMe).await.0,
        StatusCode::UNAUTHORIZED
    );
    let rows = h.audit_rows().await;
    assert_eq!(rows.last().unwrap()["action"], "sign_out");
    h.db.drop().await;
}

// ---- WebSocket ------------------------------------------------------------------------------

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn spawn(h: &Harness) -> (std::net::SocketAddr, tokio::sync::oneshot::Sender<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let router = serve::router(h.app.clone());
    tokio::spawn(async move {
        let _ = serve::serve_until(listener, router, async {
            let _ = rx.await;
        })
        .await;
    });
    (addr, tx)
}

async fn connect(
    addr: std::net::SocketAddr,
    origin: &str,
    cookie: Option<&str>,
) -> Result<Socket, tokio_tungstenite::tungstenite::Error> {
    let mut req = format!("ws://{addr}/api/v1/ws").into_client_request()?;
    req.headers_mut()
        .insert(header::ORIGIN, origin.parse().unwrap());
    if let Some(c) = cookie {
        req.headers_mut().insert(header::COOKIE, c.parse().unwrap());
    }
    tokio_tungstenite::connect_async(req).await.map(|(s, _)| s)
}

async fn hello(s: &mut Socket, csrf: &str, version: u32) {
    let frame = json!({ "protocol_version": version, "client": "test/1", "csrf_token": csrf });
    s.send(Message::Text(frame.to_string().into()))
        .await
        .unwrap();
}

/// The next text event, or the close code.
async fn next(s: &mut Socket) -> Result<EventEnvelope, u16> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(5), s.next())
            .await
            .expect("socket answered in time");
        match m {
            Some(Ok(Message::Text(t))) => return Ok(serde_json::from_str(&t).expect("event")),
            Some(Ok(Message::Close(Some(f)))) => return Err(u16::from(f.code)),
            Some(Ok(Message::Close(None))) | None | Some(Err(_)) => return Err(0),
            Some(Ok(_)) => {}
        }
    }
}

#[tokio::test]
async fn websocket_handshake_checks() {
    let Some(h) = Harness::new("ws_handshake").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let (addr, stop) = spawn(&h).await;
    let cookie = cookie_pair(&alice);

    // Another origin: refused before the upgrade.
    let err = connect(addr, "https://evil.example.com", Some(&cookie))
        .await
        .expect_err("refused");
    match err {
        tokio_tungstenite::tungstenite::Error::Http(r) => {
            assert_eq!(r.status(), StatusCode::FORBIDDEN);
        }
        other => panic!("unexpected: {other}"),
    }
    // No session.
    let mut s = connect(addr, ORIGIN, None).await.unwrap();
    assert_eq!(next(&mut s).await.err(), Some(4401));
    // Wrong CSRF token.
    let mut s = connect(addr, ORIGIN, Some(&cookie)).await.unwrap();
    hello(&mut s, "not-the-token", keel_protocol::PROTOCOL_VERSION).await;
    assert_eq!(next(&mut s).await.err(), Some(4403));
    // Wrong protocol version.
    let mut s = connect(addr, ORIGIN, Some(&cookie)).await.unwrap();
    hello(&mut s, &alice.csrf, keel_protocol::PROTOCOL_VERSION + 1).await;
    assert_eq!(next(&mut s).await.err(), Some(4400));
    // Not a Hello.
    let mut s = connect(addr, ORIGIN, Some(&cookie)).await.unwrap();
    s.send(Message::Text(r#"{"id":1,"cmd":{"type":"GetMe"}}"#.into()))
        .await
        .unwrap();
    assert_eq!(next(&mut s).await.err(), Some(4401));
    let _ = stop.send(());
    h.db.drop().await;
}

#[tokio::test]
async fn websocket_commands_and_revocation_on_epoch_change() {
    let Some(h) = Harness::new("ws_revoke").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let (addr, stop) = spawn(&h).await;
    let mut s = connect(addr, ORIGIN, Some(&cookie_pair(&alice)))
        .await
        .unwrap();
    hello(&mut s, &alice.csrf, keel_protocol::PROTOCOL_VERSION).await;
    s.send(Message::Text(r#"{"id":5,"cmd":{"type":"GetMe"}}"#.into()))
        .await
        .unwrap();
    let ev = next(&mut s).await.expect("Me");
    assert_eq!(ev.reply_to, Some(RequestId(5)));
    assert!(matches!(ev.event, Event::Me(ref me) if me.id == alice.user.id));

    // A narrowing change: the socket is told straight away, without sending a command.
    let mut tx = h.db.app.begin().await.unwrap();
    keel_server::identity::bump_epoch(&mut tx, alice.user.id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    h.app.epoch_changed(alice.user.id);
    let ev = next(&mut s).await.expect("Revoked");
    assert_eq!(ev.event, Event::Revoked);
    assert_eq!(next(&mut s).await.err(), Some(4401));
    assert!(h.app.auth.sessions.peek(&alice.session).is_none());
    let _ = stop.send(());
    h.db.drop().await;
}
