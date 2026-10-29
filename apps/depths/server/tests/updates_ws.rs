//! Spec 062 US1 (T033): the update commands' gates over the socket. `ListUpdates` is admin only,
//! `CheckUpdatesNow` and a settings change need a trusted network, a user sees update states only
//! in `StackDetail` of their own stacks and never gets `UpdatesBadge`, and `UpdatesChanged` goes
//! out at most once a second.
//!
//! No registry is reached: the scheduler is not running here, so the states are seeded into the
//! store directly.

mod support;

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use depths_protocol::{
    AuditAction, EffectivePolicy, PolicySource, ServiceUpdate, UpdatePolicy, UpdateState,
};
use depths_server::updates::registry::Unavailable;
use depths_server::updates::{SystemClock, Updates};
use futures_util::SinkExt as _;
use serde_json::{Value, json};
use support::ws::*;
use support::{App, OWNER, app, app_with_env};
use tokio_tungstenite::tungstenite::Message;

const USER: &str = "2002";
const CAFE: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 9));

/// An app with image updates on and stacks `web` and `db`.
fn updates_app() -> App {
    let a = app_with_env(&[("DEPTHS_UPDATES", "on")], |s| {
        let u = Updates::open(&s.cfg, Arc::new(SystemClock), || {
            Arc::new(Unavailable("not in tests".into()))
        })
        .unwrap();
        assert!(u.is_some());
        *s = s.clone().with_updates(u);
    });
    a.add_stack("web", COMPOSE, None);
    a.add_stack("db", COMPOSE, None);
    a
}

fn updates(a: &App) -> &Arc<Updates> {
    a.state.updates.as_ref().unwrap()
}

fn service(stack: &str, state: UpdateState) -> ServiceUpdate {
    ServiceUpdate {
        stack: stack.into(),
        service: "app".into(),
        reference: "nginx".into(),
        running: None,
        available: None,
        state,
        reason: None,
        effective: EffectivePolicy {
            policy: UpdatePolicy::Notify,
            source: PolicySource::Default,
            excluded: None,
            labels: Vec::new(),
        },
        last_checked: None,
        last_outcome: None,
        replicas: 1,
    }
}

/// `web` has an update, `db` is up to date.
fn seed(a: &App) {
    updates(a)
        .store()
        .update_observed(|o| {
            o.services
                .insert("web/app".into(), service("web", UpdateState::Available));
            o.services
                .insert("db/app".into(), service("db", UpdateState::UpToDate));
        })
        .unwrap();
}

fn checks_on(a: &App) {
    updates(a)
        .store()
        .update_decisions(|d| d.settings.checks_enabled = true)
        .unwrap();
}

fn cmd(ty: &str, data: Value) -> Value {
    json!({ "type": ty, "data": data })
}

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

#[tokio::test]
async fn with_updates_off_every_update_command_is_feature_disabled() {
    let a = app();
    a.add_stack("web", COMPOSE, None);
    let (mut ws, server, _) = open(&a, OWNER).await;
    for (i, c) in [
        json!({ "type": "ListUpdates" }),
        cmd("CheckUpdatesNow", json!({})),
    ]
    .into_iter()
    .enumerate()
    {
        let r = ask(&mut ws, i as u64 + 1, c).await;
        assert_eq!(r["event"]["type"], "Denied", "{r}");
        assert_eq!(r["event"]["data"]["reason"], "feature_disabled", "{r}");
    }
    // And the stack page carries no update states.
    let r = ask(&mut ws, 9, cmd("GetStack", json!({ "name": "web" }))).await;
    assert_eq!(r["event"]["type"], "StackDetail", "{r}");
    assert!(r["event"]["data"]["updates"].is_null(), "{r}");
    server.abort();
}

#[tokio::test]
async fn list_updates_is_admin_only() {
    let a = updates_app();
    seed(&a);
    grant(&a, USER, &["web"]);
    let (mut ws, server, _) = open(&a, USER).await;
    let r = ask(&mut ws, 1, json!({ "type": "ListUpdates" })).await;
    assert_eq!(r["event"]["type"], "Denied", "{r}");
    assert_eq!(r["event"]["data"]["reason"], "not_permitted", "{r}");
    let r = ask(&mut ws, 2, cmd("CheckUpdatesNow", json!({}))).await;
    assert_eq!(r["event"]["data"]["reason"], "not_permitted", "{r}");
    server.abort();

    let (mut ws, server, _) = open(&a, OWNER).await;
    let r = ask(&mut ws, 1, json!({ "type": "ListUpdates" })).await;
    assert_eq!(r["event"]["type"], "Updates", "{r}");
    assert_eq!(
        r["event"]["data"]["services"].as_array().unwrap().len(),
        2,
        "{r}"
    );
    server.abort();
}

#[tokio::test]
async fn check_now_needs_a_trusted_network_and_the_list_does_not() {
    let a = updates_app();
    checks_on(&a);
    let (mut ws, server, _) = open_untrusted(&a, OWNER, CAFE).await;
    let r = ask(&mut ws, 1, json!({ "type": "ListUpdates" })).await;
    assert_eq!(r["event"]["type"], "Updates", "{r}");
    let r = ask(&mut ws, 2, cmd("CheckUpdatesNow", json!({}))).await;
    assert_eq!(r["event"]["type"], "Denied", "{r}");
    assert_eq!(r["event"]["data"]["reason"], "untrusted_network", "{r}");
    server.abort();
}

#[tokio::test]
async fn check_now_is_invalid_while_checks_are_off_then_busy_within_the_gap() {
    let a = updates_app();
    let (mut ws, server, _) = open(&a, OWNER).await;
    let r = ask(&mut ws, 1, cmd("CheckUpdatesNow", json!({}))).await;
    assert_eq!(r["event"]["type"], "Invalid", "{r}");
    assert!(r.to_string().contains("checks are off"), "{r}");

    checks_on(&a);
    let r = ask(&mut ws, 2, cmd("CheckUpdatesNow", json!({}))).await;
    assert_eq!(r["event"]["type"], "CheckStarted", "{r}");
    let r = ask(
        &mut ws,
        3,
        cmd("CheckUpdatesNow", json!({ "stacks": ["web"] })),
    )
    .await;
    assert_eq!(r["event"]["type"], "CheckBusy", "{r}");
    assert!(r["event"]["data"]["retry_after_secs"].as_u64().unwrap() > 0);
    server.abort();
}

#[tokio::test]
async fn a_user_sees_states_only_for_their_stacks_and_gets_no_badge() {
    let a = updates_app();
    seed(&a);
    grant(&a, USER, &["web"]);
    let (mut ws, server, _) = open(&a, USER).await;
    ws.send(Message::text(
        json!({ "id": 1, "cmd": cmd("GetStack", json!({ "name": "web" })) }).to_string(),
    ))
    .await
    .unwrap();
    let r = loop {
        let e = next(&mut ws).await;
        assert_ne!(
            e["event"]["type"], "UpdatesBadge",
            "a user got the badge: {e}"
        );
        if e["reply_to"] == 1 {
            break e;
        }
    };
    assert_eq!(r["event"]["type"], "StackDetail", "{r}");
    let states = r["event"]["data"]["updates"].as_array().unwrap();
    assert_eq!(states.len(), 1, "{r}");
    assert_eq!(states[0]["stack"], "web");
    assert_eq!(states[0]["state"], "available");

    let r = ask(&mut ws, 2, cmd("GetStack", json!({ "name": "db" }))).await;
    assert_eq!(r["event"]["type"], "Denied", "{r}");
    assert!(!r.to_string().contains("up_to_date"), "{r}");
    server.abort();
}

#[tokio::test]
async fn an_admin_gets_the_badge_on_sign_in_and_when_the_count_moves() {
    let a = updates_app();
    seed(&a);
    let (mut ws, server, _) = open(&a, OWNER).await;
    let e = next_of(&mut ws, "UpdatesBadge").await;
    assert_eq!(e["event"]["data"]["stacks_with_updates"], 1, "{e}");

    updates(&a)
        .store()
        .update_observed(|o| {
            o.services.get_mut("db/app").unwrap().state = UpdateState::Available;
        })
        .unwrap();
    updates(&a).notify_changed();
    let e = next_of(&mut ws, "UpdatesBadge").await;
    assert_eq!(e["event"]["data"]["stacks_with_updates"], 2, "{e}");
    server.abort();
}

#[tokio::test]
async fn updates_changed_is_coalesced_to_one_a_second() {
    let a = updates_app();
    seed(&a);
    let (mut ws, server, _) = open(&a, OWNER).await;
    let r = ask(&mut ws, 1, json!({ "type": "ListUpdates" })).await;
    assert_eq!(r["event"]["type"], "Updates", "{r}");

    let start = tokio::time::Instant::now();
    // Read while the changes arrive, so the times are when they were sent.
    let u = updates(&a).clone();
    let burst = tokio::spawn(async move {
        for _ in 0..20 {
            u.notify_changed();
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let mut at = Vec::new();
    while start.elapsed() < Duration::from_millis(1800) {
        let left = Duration::from_millis(1800).saturating_sub(start.elapsed());
        match tokio::time::timeout(left, next_of(&mut ws, "UpdatesChanged")).await {
            Ok(_) => at.push(start.elapsed()),
            Err(_) => break,
        }
    }
    assert!(
        (1..=2).contains(&at.len()),
        "20 changes in 200 ms gave {} UpdatesChanged: {at:?}",
        at.len()
    );
    if let [first, second] = at[..] {
        assert!(second - first >= Duration::from_millis(900), "{at:?}");
    }
    burst.await.unwrap();
    server.abort();
}

#[tokio::test]
async fn a_settings_change_is_previewed_confirmed_and_audited_with_before_and_after() {
    let a = updates_app();
    let (mut ws, server, _) = open(&a, OWNER).await;
    let r = ask(&mut ws, 1, json!({ "type": "ListUpdates" })).await;
    let mut settings = r["event"]["data"]["settings"].clone();
    assert_eq!(settings["checks_enabled"], false);

    // Settings other than the two US1 takes are refused.
    let mut other = settings.clone();
    other["cleanup"] = json!(true);
    let r = ask(
        &mut ws,
        2,
        cmd(
            "PreviewUpdateChange",
            json!({ "kind": "settings", "data": other }),
        ),
    )
    .await;
    assert_eq!(r["event"]["type"], "Invalid", "{r}");

    settings["checks_enabled"] = json!(true);
    settings["interval_secs"] = json!(7200);
    let r = ask(
        &mut ws,
        3,
        cmd(
            "PreviewUpdateChange",
            json!({ "kind": "settings", "data": settings }),
        ),
    )
    .await;
    assert_eq!(r["event"]["type"], "Preview", "{r}");
    let diff = r["event"]["data"]["file_diff"].as_str().unwrap();
    assert!(diff.contains("update checks: off -> on"), "{diff}");
    let preview_id = r["event"]["data"]["preview_id"].clone();
    assert!(
        !updates(&a).checks_enabled(),
        "nothing changes before Confirm"
    );

    let r = ask(
        &mut ws,
        4,
        cmd("Confirm", json!({ "preview_id": preview_id })),
    )
    .await;
    assert_eq!(r["event"]["type"], "UpdatesChanged", "{r}");
    assert_eq!(
        r["event"]["data"]["settings"]["checks_enabled"], true,
        "{r}"
    );
    assert!(updates(&a).checks_enabled());
    assert_eq!(updates(&a).store().settings().interval_secs, 7200);

    let entries = a.audit_entries();
    let e = entries
        .iter()
        .find(|e| e.action == AuditAction::UpdateSettings)
        .unwrap_or_else(|| panic!("{entries:?}"));
    let change = e.change.as_ref().unwrap();
    assert_eq!(change.before["checks_enabled"], false);
    assert_eq!(change.after["checks_enabled"], true);
    assert_eq!(change.after["interval_secs"], 7200);

    // Used once.
    let r = ask(
        &mut ws,
        5,
        cmd("Confirm", json!({ "preview_id": preview_id })),
    )
    .await;
    assert_eq!(r["event"]["type"], "PreviewVoid", "{r}");
    server.abort();
}

#[tokio::test]
async fn a_settings_change_needs_a_trusted_network() {
    let a = updates_app();
    let (mut ws, server, _) = open_untrusted(&a, OWNER, CAFE).await;
    let mut settings = serde_json::to_value(updates(&a).store().settings()).unwrap();
    settings["checks_enabled"] = json!(true);
    let r = ask(
        &mut ws,
        1,
        cmd(
            "PreviewUpdateChange",
            json!({ "kind": "settings", "data": settings }),
        ),
    )
    .await;
    assert_eq!(r["event"]["data"]["reason"], "untrusted_network", "{r}");
    server.abort();
}

/// Spec 054: `ManageUpdates` always steps up. A stale session's preview carries the check, a
/// confirm without (or with a bad) answer changes nothing, and the answered confirm applies.
#[tokio::test]
async fn a_settings_change_steps_up_on_a_stale_session() {
    use support::ORIGIN;
    use support::passkey::*;

    let a = updates_app();
    let (mut ws, server, cookie) = open_unverified(&a, OWNER).await;
    let mut auth = authenticator();
    enrol(&mut ws, &mut auth, "Laptop").await;
    stale(&a, &cookie);

    let mut settings = serde_json::to_value(updates(&a).store().settings()).unwrap();
    settings["checks_enabled"] = json!(true);
    let r = ask(
        &mut ws,
        1,
        cmd(
            "PreviewUpdateChange",
            json!({ "kind": "settings", "data": settings }),
        ),
    )
    .await;
    assert_eq!(r["event"]["type"], "Preview", "{r}");
    let preview_id = r["event"]["data"]["preview_id"].clone();
    let request = r["event"]["data"]["step_up"].clone();
    assert!(request.is_object(), "the preview carries the check: {r}");

    // No answer: the check is asked for and nothing is claimed or changed.
    let r = ask(
        &mut ws,
        2,
        cmd("Confirm", json!({ "preview_id": preview_id })),
    )
    .await;
    assert_eq!(r["event"]["type"], "StepUpRequired", "{r}");
    assert!(!updates(&a).checks_enabled());

    // An answer made elsewhere is refused, and still nothing changes.
    let bad = answer(&mut auth, ELSEWHERE, &request_of(&r));
    let r = ask(
        &mut ws,
        3,
        cmd(
            "Confirm",
            json!({ "preview_id": preview_id, "assertion": bad }),
        ),
    )
    .await;
    assert_eq!(r["event"]["type"], "PasskeyRefused", "{r}");
    assert!(!updates(&a).checks_enabled());

    // The preview's own check, answered, applies the change.
    let good = answer(&mut auth, ORIGIN, &request);
    let r = ask(
        &mut ws,
        4,
        cmd(
            "Confirm",
            json!({ "preview_id": preview_id, "assertion": good }),
        ),
    )
    .await;
    assert_eq!(r["event"]["type"], "UpdatesChanged", "{r}");
    assert!(updates(&a).checks_enabled());
    server.abort();
}

/// A fresh session's settings preview asks for no check.
#[tokio::test]
async fn a_fresh_session_previews_a_settings_change_without_a_check() {
    let a = updates_app();
    let (mut ws, server, _) = open(&a, OWNER).await;
    let mut settings = serde_json::to_value(updates(&a).store().settings()).unwrap();
    settings["checks_enabled"] = json!(true);
    let r = ask(
        &mut ws,
        1,
        cmd(
            "PreviewUpdateChange",
            json!({ "kind": "settings", "data": settings }),
        ),
    )
    .await;
    assert_eq!(r["event"]["type"], "Preview", "{r}");
    assert!(r["event"]["data"]["step_up"].is_null(), "{r}");
    server.abort();
}
