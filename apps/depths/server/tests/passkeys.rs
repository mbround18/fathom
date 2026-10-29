//! Spec 054 US4 and US5 over the real socket: passkey ceremonies (T055), the verification gate
//! (T056), owner recovery (T057), step-up on risky actions (T069) and the window (T070). The
//! software passkey answers as a browser would; time moves on the passkey clock, never by sleep.

mod support;

use std::net::IpAddr;

use axum::body::Body;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};
use support::passkey::*;
use support::ws::*;
use support::{App, LOOPBACK, ORIGIN, OWNER, app_with, app_with_env, from, session_of};

const CAFE: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 9));
const PRIVILEGED: &str =
    "services:\n  app:\n    image: nginx\n    user: \"1000\"\n    privileged: true\n";

fn ty(r: &Value) -> &str {
    r["event"]["type"].as_str().unwrap_or_default()
}

fn refused(r: &Value) -> &str {
    assert_eq!(ty(r), "PasskeyRefused", "{r}");
    r["event"]["data"]["reason"].as_str().unwrap()
}

fn put_user() -> Value {
    json!({ "type": "PutUser", "data": { "id": "discord:555", "role": "user", "stacks": ["web"] } })
}

fn get_audit() -> Value {
    json!({ "type": "GetAudit", "data": { "filter": {} } })
}

fn preview_up() -> Value {
    json!({ "type": "PreviewOp", "data": { "stack": "web", "op": { "kind": "up" } } })
}

fn confirm(p: &Value, assertion: Option<Value>) -> Value {
    let mut data = json!({ "preview_id": p["event"]["data"]["preview_id"] });
    if let Some(a) = assertion {
        data["assertion"] = a;
    }
    json!({ "type": "Confirm", "data": data })
}

/// Stack `web` with `compose` under `warn`, so an escape is previewed rather than refused.
fn deploy_app(compose: &str) -> App {
    use depths_core::compose_lint::ComposePolicy::Warn;
    let a = app_with(|s| {
        let mut cfg = (*s.cfg).clone();
        cfg.compose_policy = Warn;
        s.cfg = std::sync::Arc::new(cfg);
        s.ops.set_compose_policy(Warn);
    });
    a.add_stack("web", compose, None);
    a
}

fn web_app() -> App {
    let a = app_with(|_| {});
    a.add_stack("web", COMPOSE, None);
    a
}

// ---- T055: ceremonies ----------------------------------------------------------------------

#[tokio::test]
async fn registering_then_verifying_marks_the_session_verified_and_is_audited() {
    let a = web_app();
    let (mut ws, server, _) = open_unverified(&a, OWNER).await;
    let v = verification(&mut ws).await;
    assert_eq!(v["required"], true, "{v}");
    assert_eq!(v["verified"], false, "{v}");
    assert_eq!(v["enrolled"], false, "{v}");
    assert_eq!(v["window_secs"], 300, "{v}");

    let mut auth = authenticator();
    let r = enrol(&mut ws, &mut auth, "Laptop").await;
    assert_eq!(r["event"]["data"]["name"], "Laptop", "{r}");
    let v = verification(&mut ws).await;
    assert_eq!(v["verified"], true, "{v}");
    assert_eq!(v["enrolled"], true, "{v}");
    let first = v["at"].clone();

    // Later, a verify ceremony refreshes it.
    a.state.passkeys.clock.advance(600);
    let r = verify_now(&mut ws, &mut auth).await;
    assert_eq!(ty(&r), "Verification", "{r}");
    assert_eq!(r["event"]["data"]["verified"], true, "{r}");
    assert_ne!(r["event"]["data"]["at"], first, "{r}");

    let reg = audited(&a, "passkey_registered");
    assert_eq!(reg.len(), 1);
    assert_eq!(
        reg[0].passkey.as_ref().unwrap().name.as_deref(),
        Some("Laptop")
    );
    assert_eq!(audited(&a, "passkey_verified").len(), 1);
    let listed = ask(&mut ws, 1, json!({ "type": "ListPasskeys" })).await;
    assert_eq!(ty(&listed), "Passkeys", "{listed}");
    assert_eq!(listed["event"]["data"]["items"][0]["name"], "Laptop");
    server.abort();
}

#[tokio::test]
async fn replayed_foreign_late_and_misdirected_answers_are_refused_and_audited() {
    let a = web_app();
    let (mut ws, server, _) = open_unverified(&a, OWNER).await;
    let mut auth = authenticator();
    enrol(&mut ws, &mut auth, "Laptop").await;
    let mut challenges = Vec::new();

    // An answer is single-use: sent again, it is a replay.
    let req = request_of(&begin(&mut ws, verify()).await);
    challenges.push(req["options"]["challenge"].as_str().unwrap().to_owned());
    let cred = assert_with(&mut auth, ORIGIN, &req["options"]);
    let r = finish(&mut ws, &req["ceremony"], cred.clone()).await;
    assert_eq!(ty(&r), "Verification", "{r}");
    assert_eq!(
        refused(&finish(&mut ws, &req["ceremony"], cred).await),
        "replayed"
    );

    // Made for another origin.
    let req = request_of(&begin(&mut ws, verify()).await);
    challenges.push(req["options"]["challenge"].as_str().unwrap().to_owned());
    let cred = assert_with(&mut auth, ELSEWHERE, &req["options"]);
    assert_eq!(
        refused(&finish(&mut ws, &req["ceremony"], cred).await),
        "wrong_origin"
    );

    // Another browser's ceremony is refused here and left alone for its owner.
    let (mut other, other_server, _) = open_unverified(&a, OWNER).await;
    let req = request_of(&begin(&mut other, verify()).await);
    let cred = assert_with(&mut auth, ORIGIN, &req["options"]);
    assert_eq!(
        refused(&finish(&mut ws, &req["ceremony"], cred.clone()).await),
        "expired"
    );
    let r = finish(&mut other, &req["ceremony"], cred).await;
    assert_eq!(ty(&r), "Verification", "{r}");

    // Older than two minutes.
    let req = request_of(&begin(&mut ws, verify()).await);
    challenges.push(req["options"]["challenge"].as_str().unwrap().to_owned());
    let cred = assert_with(&mut auth, ORIGIN, &req["options"]);
    a.state.passkeys.clock.advance(121);
    assert_eq!(
        refused(&finish(&mut ws, &req["ceremony"], cred).await),
        "expired"
    );

    // The log reads newest first.
    let failed: Vec<String> = audited(&a, "passkey_failed")
        .iter()
        .rev()
        .map(|e| {
            serde_json::to_value(e.passkey.as_ref().unwrap().reason.unwrap())
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(failed, ["replayed", "wrong_origin", "expired", "expired"]);
    let log = String::from_utf8(a.audit_bytes()).unwrap();
    for c in &challenges {
        assert!(
            !log.contains(c.as_str()),
            "a challenge reached the audit log"
        );
    }
    other_server.abort();
    server.abort();
}

// ---- T056: the verification gate -----------------------------------------------------------

#[tokio::test]
async fn an_admin_without_a_passkey_can_only_enrol() {
    let a = web_app();
    let (mut ws, server, _) = open_unverified(&a, OWNER).await;
    for (i, c) in [put_user(), get_audit()].into_iter().enumerate() {
        let r = ask(&mut ws, 10 + i as u64, c.clone()).await;
        assert_eq!(ty(&r), "PasskeyEnrolmentRequired", "{c}: {r}");
    }
    // The restricted view still works.
    let r = ask(&mut ws, 20, json!({ "type": "ListStacks" })).await;
    assert_eq!(ty(&r), "Stacks", "{r}");
    // Enrolling is how out.
    let r = begin(&mut ws, register("Laptop")).await;
    assert_eq!(ty(&r), "PasskeyChallenge", "{r}");
    assert!(
        a.audit_entries().iter().any(
            |e| matches!(&e.outcome, depths_protocol::AuditOutcome::Denied { reason }
                if reason.contains("passkey_enrolment_required"))
        ),
        "the refusal is audited"
    );
    server.abort();
}

#[tokio::test]
async fn an_unverified_session_gets_the_restricted_set_until_it_verifies() {
    let a = web_app();
    let (mut first, s1, _) = open_unverified(&a, OWNER).await;
    let mut auth = authenticator();
    enrol(&mut first, &mut auth, "Laptop").await;
    let r = ask(&mut first, 1, get_audit()).await;
    assert_ne!(ty(&r), "Denied", "a verified admin has admin access: {r}");

    // Another browser of the same admin is unverified.
    let (mut second, s2, _) = open_unverified(&a, OWNER).await;
    let v = verification(&mut second).await;
    assert_eq!(v["verified"], false, "{v}");
    assert_eq!(v["enrolled"], true, "{v}");
    let r = ask(&mut second, 1, get_audit()).await;
    assert_eq!(ty(&r), "Denied", "{r}");
    assert_eq!(r["event"]["data"]["reason"], "not_permitted", "{r}");
    let r = ask(&mut second, 2, put_user()).await;
    assert_eq!(ty(&r), "Denied", "{r}");
    let r = ask(&mut second, 3, json!({ "type": "ListStacks" })).await;
    assert_eq!(ty(&r), "Stacks", "{r}");

    let r = verify_now(&mut second, &mut auth).await;
    assert_eq!(r["event"]["data"]["verified"], true, "{r}");
    let r = ask(&mut second, 4, get_audit()).await;
    assert_ne!(ty(&r), "Denied", "{r}");
    s1.abort();
    s2.abort();
}

/// FR-022: verifying does not make an untrusted network trusted.
#[tokio::test]
async fn verifying_on_an_untrusted_network_still_gives_only_the_restricted_view() {
    let a = web_app();
    // Signed in at the cafe (its first network); the socket comes from loopback (unknown).
    let cookie = a.sign_in_unverified_from(CAFE, OWNER).await;
    let session = session_of(&cookie);
    let user = format!("discord:{OWNER}");
    let mut auth = authenticator();
    // Enrolled at the cafe, where the first passkey may be added.
    let req = a
        .state
        .passkeys
        .begin_register(&session, &user, "owner", "Laptop", true)
        .unwrap();
    let req = serde_json::to_value(req).unwrap();
    let response = serde_json::from_value(json!({
        "ceremony": req["ceremony"],
        "credential": attest(&mut auth, ORIGIN, &req["options"]),
    }))
    .unwrap();
    a.state
        .passkeys
        .finish(&session, &user, &response, true)
        .await
        .unwrap();

    let csrf = csrf_of(&a, &cookie).await;
    let (mut ws, server) = connect(&a, Some(&cookie)).await;
    use futures_util::SinkExt as _;
    ws.send(tokio_tungstenite::tungstenite::Message::text(hello(&csrf)))
        .await
        .unwrap();
    let r = verify_now(&mut ws, &mut auth).await;
    assert_eq!(r["event"]["data"]["verified"], true, "{r}");
    let r = ask(&mut ws, 1, put_user()).await;
    assert_eq!(ty(&r), "Denied", "{r}");
    assert_eq!(r["event"]["data"]["reason"], "untrusted_network", "{r}");
    server.abort();
}

#[tokio::test]
async fn a_user_promoted_to_admin_must_enrol_before_their_next_admin_action() {
    let a = web_app();
    grant(&a, "7", &["web"]);
    let (mut ws, server, _) = open_unverified(&a, "7").await;
    let v = verification(&mut ws).await;
    assert_eq!(v["required"], false, "{v}");
    let r = ask(&mut ws, 1, json!({ "type": "ListStacks" })).await;
    assert_eq!(ty(&r), "Stacks", "{r}");

    a.state
        .users
        .put(
            &depths_protocol::UserId::new("discord:7".to_owned()).unwrap(),
            depths_protocol::Role::Admin,
            &[],
        )
        .unwrap();
    let r = ask(&mut ws, 2, put_user()).await;
    assert_eq!(ty(&r), "PasskeyEnrolmentRequired", "{r}");
    assert_eq!(verification(&mut ws).await["required"], true);
    server.abort();
}

// ---- T057: recovery ------------------------------------------------------------------------

async fn issue(a: &App) -> String {
    depths_server::passkeys::recovery_command(
        &a.state.cfg.data_dir,
        1 << 20,
        &format!("discord:{OWNER}"),
        chrono::Utc::now(),
    )
    .await
    .unwrap()
    .code
}

fn recover(code: &str) -> Value {
    json!({ "type": "RecoverPasskeys", "data": { "code": code } })
}

#[tokio::test]
async fn a_recovery_code_removes_the_passkeys_once_and_lets_the_session_enrol() {
    let a = web_app();
    let (mut ws, server, _) = open_unverified(&a, OWNER).await;
    let mut lost = authenticator();
    enrol(&mut ws, &mut lost, "Lost phone").await;

    // A new browser without the passkey.
    let (mut fresh, s2, _) = open_unverified(&a, OWNER).await;
    assert_eq!(
        refused(&ask(&mut fresh, 1, recover("AAAAAAAAAA")).await),
        "recovery_invalid"
    );

    let old = issue(&a).await;
    let code = issue(&a).await;
    let file = std::fs::read_to_string(depths_server::passkeys::recovery_path(
        &a.state.cfg.data_dir,
    ))
    .unwrap();
    assert!(!file.contains(&code), "only the hash is stored");
    assert_eq!(
        refused(&ask(&mut fresh, 2, recover(&old)).await),
        "recovery_invalid",
        "replaced"
    );

    let r = ask(&mut fresh, 3, recover(&code)).await;
    assert_eq!(ty(&r), "Verification", "{r}");
    assert_eq!(r["event"]["data"]["recovering"], true, "{r}");
    assert_eq!(r["event"]["data"]["enrolled"], false, "{r}");
    let list = ask(&mut fresh, 4, json!({ "type": "ListPasskeys" })).await;
    assert_eq!(list["event"]["data"]["items"], json!([]), "{list}");

    let mut new = authenticator();
    enrol(&mut fresh, &mut new, "New phone").await;
    assert_eq!(verification(&mut fresh).await["verified"], true);
    assert_eq!(
        refused(&ask(&mut fresh, 5, recover(&code)).await),
        "recovery_invalid",
        "used"
    );

    // A code works for 15 minutes.
    let late = issue(&a).await;
    a.state.passkeys.clock.advance(15 * 60 + 1);
    assert_eq!(
        refused(&ask(&mut fresh, 6, recover(&late)).await),
        "recovery_invalid",
        "expired"
    );

    assert_eq!(audited(&a, "passkey_recovery_issued").len(), 3);
    assert_eq!(audited(&a, "passkey_recovered").len(), 1);
    let log = String::from_utf8(a.audit_bytes()).unwrap();
    for c in [&old, &code, &late] {
        assert!(
            !log.contains(c.as_str()),
            "a recovery code reached the audit log"
        );
    }
    server.abort();
    s2.abort();
}

// ---- T069: step-up -------------------------------------------------------------------------

#[tokio::test]
async fn a_risky_confirm_steps_up_and_a_failed_check_changes_nothing() {
    let a = deploy_app(PRIVILEGED);
    let (mut ws, server, cookie) = open_unverified(&a, OWNER).await;
    let mut auth = authenticator();
    enrol(&mut ws, &mut auth, "Laptop").await;
    stale(&a, &cookie);

    let p = ask(&mut ws, 1, preview_up()).await;
    assert_eq!(ty(&p), "Preview", "{p}");
    assert!(p["event"]["data"]["step_up"]["ceremony"].is_string(), "{p}");

    let r = ask(&mut ws, 2, confirm(&p, None)).await;
    assert_eq!(ty(&r), "StepUpRequired", "{r}");
    assert!(a.runtime.ran.lock().unwrap().is_empty(), "nothing ran");
    assert_eq!(audited(&a, "step_up_required").len(), 1);

    let wrong = answer(&mut auth, ELSEWHERE, &request_of(&r));
    let r = ask(&mut ws, 3, confirm(&p, Some(wrong))).await;
    assert_eq!(refused(&r), "wrong_origin");
    assert!(a.runtime.ran.lock().unwrap().is_empty(), "nothing ran");
    assert_eq!(audited(&a, "passkey_failed").len(), 1);

    // The preview survives a refused check; a good one applies it and refreshes the window.
    let good = answer(&mut auth, ORIGIN, &p["event"]["data"]["step_up"]);
    let r = ask(&mut ws, 4, confirm(&p, Some(good))).await;
    assert_eq!(ty(&r), "OpStarted", "{r}");
    next_of(&mut ws, "OpFinished").await;
    let at = verification(&mut ws).await["at"].clone();
    assert!(at.is_string());

    // Inside the window, the next one needs none.
    let p = ask(&mut ws, 5, preview_up()).await;
    assert!(p["event"]["data"]["step_up"].is_null(), "{p}");
    let r = ask(&mut ws, 6, confirm(&p, None)).await;
    assert_eq!(ty(&r), "OpStarted", "{r}");
    next_of(&mut ws, "OpFinished").await;
    server.abort();
}

#[tokio::test]
async fn a_deploy_without_escape_findings_needs_no_step_up() {
    let a = deploy_app(COMPOSE);
    let (mut ws, server, cookie) = open_unverified(&a, OWNER).await;
    enrol(&mut ws, &mut authenticator(), "Laptop").await;
    stale(&a, &cookie);
    let p = ask(&mut ws, 1, preview_up()).await;
    assert_eq!(ty(&p), "Preview", "{p}");
    assert!(p["event"]["data"]["step_up"].is_null(), "{p}");
    let r = ask(&mut ws, 2, confirm(&p, None)).await;
    assert_eq!(ty(&r), "OpStarted", "{r}");
    server.abort();
}

#[tokio::test]
async fn a_shell_steps_up_and_the_retry_with_the_assertion_opens_it() {
    let a = streams_app();
    let (mut ws, server, cookie) = open_unverified(&a, OWNER).await;
    let mut auth = authenticator();
    enrol(&mut ws, &mut auth, "Laptop").await;
    stale(&a, &cookie);
    let shell = |assertion: Option<Value>| {
        let mut data = json!({ "container": "web-app-1", "cols": 80, "rows": 24 });
        if let Some(a) = assertion {
            data["assertion"] = a;
        }
        json!({ "type": "OpenShell", "data": data })
    };
    let r = ask(&mut ws, 1, shell(None)).await;
    assert_eq!(ty(&r), "StepUpRequired", "{r}");
    assert!(a.runtime.shells.lock().unwrap().is_empty());
    let good = answer(&mut auth, ORIGIN, &request_of(&r));
    let r = ask(&mut ws, 2, shell(Some(good))).await;
    stream_of(&r);
    server.abort();
}

async fn whitelist(a: &App, cookie: &str) -> http::Response<Body> {
    let csrf = csrf_of(a, cookie).await;
    let req = Request::post("/api/v1/users/discord:555/networks/whitelist")
        .header(header::COOKIE, cookie)
        .header(header::ORIGIN, ORIGIN)
        .header("x-csrf-token", csrf)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({ "cidr": "198.51.100.0/24", "label": "office" }).to_string(),
        ))
        .unwrap();
    a.send(from(LOOPBACK, req)).await
}

#[tokio::test]
async fn user_changes_and_the_whitelist_need_a_fresh_check() {
    let a = web_app();
    grant(&a, "555", &["web"]);
    let (mut ws, server, cookie) = open_unverified(&a, OWNER).await;
    let mut auth = authenticator();
    enrol(&mut ws, &mut auth, "Laptop").await;
    stale(&a, &cookie);

    let r = ask(&mut ws, 1, put_user()).await;
    assert_eq!(ty(&r), "StepUpRequired", "{r}");
    let r = ask(
        &mut ws,
        2,
        json!({ "type": "RemoveUser", "data": { "id": "discord:555" } }),
    )
    .await;
    assert_eq!(ty(&r), "StepUpRequired", "{r}");
    let r = whitelist(&a, &cookie).await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(support::json(r).await["error"], "step_up_required");

    // Commands without an assertion field: check, then retry.
    let r = verify_now(&mut ws, &mut auth).await;
    assert_eq!(ty(&r), "Verification", "{r}");
    let r = ask(&mut ws, 3, put_user()).await;
    assert_eq!(ty(&r), "Users", "{r}");
    let r = whitelist(&a, &cookie).await;
    assert_eq!(r.status(), StatusCode::OK);
    server.abort();
}

/// Q2 = A: step-up applies to everyone, so a plain user must enrol for a risky action.
#[tokio::test]
async fn a_plain_user_without_a_passkey_must_enrol_for_a_risky_action() {
    let a = deploy_app(PRIVILEGED);
    grant(&a, "7", &["web"]);
    let (mut ws, server, _) = open_unverified(&a, "7").await;
    let p = ask(&mut ws, 1, preview_up()).await;
    assert_eq!(ty(&p), "Preview", "{p}");
    let r = ask(&mut ws, 2, confirm(&p, None)).await;
    assert_eq!(ty(&r), "PasskeyEnrolmentRequired", "{r}");
    assert!(a.runtime.ran.lock().unwrap().is_empty());
    server.abort();
}

// ---- T070: the window ----------------------------------------------------------------------

#[tokio::test]
async fn a_thirty_second_window_is_honoured() {
    let a = app_with_env(&[("DEPTHS_STEP_UP_WINDOW", "30")], |_| {});
    a.add_stack("web", COMPOSE, None);
    let (mut ws, server, _) = open_unverified(&a, OWNER).await;
    enrol(&mut ws, &mut authenticator(), "Laptop").await;
    assert_eq!(verification(&mut ws).await["window_secs"], 30);

    a.state.passkeys.clock.advance(29);
    let r = ask(&mut ws, 1, put_user()).await;
    assert_eq!(ty(&r), "Users", "inside the window: {r}");
    a.state.passkeys.clock.advance(2);
    let r = ask(&mut ws, 2, put_user()).await;
    assert_eq!(ty(&r), "StepUpRequired", "past the window: {r}");
    server.abort();
}
