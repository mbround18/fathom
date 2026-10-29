//! Helpers for the spec 062 real-Docker tests that drive Depths over its WebSocket as an
//! administrator (`update_walkthrough`, `update_watchtower`).

use std::time::Duration;

use depths_protocol::{
    RunTrigger, ServiceUpdate, StackOutcome, UpdateRun, UpdateState, UpdatesView,
};
use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

use super::OWNER;
use super::docker::{Lab, image_id, ok};
use super::passkey::{Authenticator, answer, authenticator, enrol, open_unverified};
use super::ws::{Socket, ask};

/// A check pass against the local registry, or an update run with its health wait.
pub const RUN: Duration = Duration::from_secs(120);

pub fn cmd(ty: &str, data: Value) -> Value {
    json!({ "type": ty, "data": data })
}

pub fn compose(image: &str) -> String {
    format!("services:\n  app:\n    image: {image}\n")
}

/// Pushes a new `app:1` built from `dockerfile` and points the local tag back at the image the
/// stack runs, as on a host that has not pulled yet. Returns the new image's id.
pub fn repush(lab: &Lab, dockerfile: &str) -> String {
    let reference = lab.reference("app", "1");
    let old = image_id(&reference).expect("app:1 was pushed before");
    let new = lab.push("app", "1", dockerfile);
    ok(&["tag", &old, &reference]);
    new
}

/// The window from an hour ago to an hour from now, every day, in host local time (the zone
/// `auto::HostZone` reads).
pub fn window_around_now() -> Value {
    let now = chrono::Local::now();
    let at = |t: chrono::DateTime<chrono::Local>| t.format("%H:%M").to_string();
    json!({
        "days": ["mon", "tue", "wed", "thu", "fri", "sat", "sun"],
        "start": at(now - chrono::Duration::hours(1)),
        "end": at(now + chrono::Duration::hours(1)),
    })
}

/// `docker inspect` of a container: `(Config.Image, Image)`.
pub fn running_image(container: &str) -> (String, String) {
    let out = ok(&[
        "inspect",
        "--format",
        "{{.Config.Image}} {{.Image}}",
        container,
    ]);
    let (reference, id) = out.split_once(' ').expect("two fields");
    (reference.to_owned(), id.to_owned())
}

pub fn service<'a>(v: &'a UpdatesView, stack: &str, name: &str) -> Option<&'a ServiceUpdate> {
    v.services
        .iter()
        .find(|s| s.stack == stack && s.service == name)
}

pub fn state(v: &UpdatesView, stack: &str) -> Option<UpdateState> {
    service(v, stack, "app").map(|s| s.state)
}

/// The latest run that names `stack`, and that stack's part of it.
pub fn run_of<'a>(v: &'a UpdatesView, stack: &str) -> Option<(&'a UpdateRun, StackOutcome)> {
    v.recent_runs.iter().rev().find_map(|r| {
        r.stacks
            .iter()
            .find(|s| s.stack == stack)
            .and_then(|s| s.outcome.clone())
            .map(|o| (r, o))
    })
}

pub fn view_of(e: &Value) -> Option<UpdatesView> {
    matches!(
        e["event"]["type"].as_str(),
        Some("Updates" | "UpdatesChanged")
    )
    .then(|| serde_json::from_value(e["event"]["data"].clone()).expect("an UpdatesView"))
}

/// The next text event within `within`, or `None`.
pub async fn recv(ws: &mut Socket, within: Duration) -> Option<Value> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        match tokio::time::timeout_at(deadline, ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                return Some(serde_json::from_str(t.as_str()).unwrap());
            }
            Ok(Some(Ok(_))) => {}
            Ok(other) => panic!("the socket closed: {other:?}"),
            Err(_) => return None,
        }
    }
}

/// Request ids for `ListUpdates` asked while waiting, clear of the ones tests pick.
pub static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(70_000);

/// Reads update views until one satisfies `pred`. When the socket is quiet for two seconds it asks
/// `ListUpdates` again, so a coalesced `UpdatesChanged` cannot be missed.
pub async fn view_where(
    ws: &mut Socket,
    within: Duration,
    what: &str,
    pred: impl Fn(&UpdatesView) -> bool,
) -> UpdatesView {
    let deadline = tokio::time::Instant::now() + within;
    let mut last = None;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(!left.is_zero(), "timed out: {what}; last view: {last:#?}");
        match recv(ws, left.min(Duration::from_secs(2))).await {
            Some(e) => {
                if let Some(v) = view_of(&e) {
                    if pred(&v) {
                        return v;
                    }
                    last = Some(v);
                }
            }
            None => {
                let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let ask = json!({ "id": id, "cmd": { "type": "ListUpdates" } });
                ws.send(Message::text(ask.to_string())).await.unwrap();
            }
        }
    }
}

/// An administrator's socket, signed in with a passkey enrolled now (so a fresh check).
pub async fn admin(lab: &Lab) -> (Socket, tokio::task::JoinHandle<()>, String, Authenticator) {
    let (mut ws, server, cookie) = open_unverified(&lab.app, OWNER).await;
    let mut auth = authenticator();
    enrol(&mut ws, &mut auth, "Laptop").await;
    (ws, server, cookie, auth)
}

/// `ListUpdates`: the view, and live changes from now on.
pub async fn subscribe(ws: &mut Socket) -> UpdatesView {
    let r = ask(ws, 1, json!({ "type": "ListUpdates" })).await;
    view_of(&r).unwrap_or_else(|| panic!("no view: {r}"))
}

pub async fn check_now(ws: &mut Socket, id: u64, stacks: Option<&[&str]>) {
    // `stacks` is left out for "every stack": the protocol does not take `null`.
    let body = match stacks {
        Some(stacks) => json!({ "stacks": stacks }),
        None => json!({}),
    };
    let r = ask(ws, id, cmd("CheckUpdatesNow", body)).await;
    assert_eq!(r["event"]["type"], "CheckStarted", "{r}");
}

/// Confirms the preview in `r`, answering its passkey check when it asks for one.
pub async fn confirm(ws: &mut Socket, id: u64, auth: &mut Authenticator, r: &Value) -> Value {
    let data = &r["event"]["data"];
    let mut confirm = json!({ "preview_id": data["preview_id"] });
    if data["step_up"].is_object() {
        confirm["assertion"] = answer(auth, super::ORIGIN, &data["step_up"]);
    }
    if data["stacks"]
        .as_array()
        .is_some_and(|s| s.iter().any(|p| p["ack_required"] == true))
    {
        confirm["acknowledge_findings"] = json!(true);
    }
    ask(ws, id, cmd("Confirm", confirm)).await
}

/// Previews and confirms an update change (`PreviewUpdateChange`).
pub async fn change(ws: &mut Socket, id: u64, auth: &mut Authenticator, change: Value) {
    let r = ask(ws, id, cmd("PreviewUpdateChange", change)).await;
    assert_eq!(r["event"]["type"], "Preview", "{r}");
    let r = confirm(ws, id + 1, auth, &r).await;
    assert_eq!(r["event"]["type"], "UpdatesChanged", "{r}");
}

/// Sets `stack` to Auto in the window around now.
pub async fn auto_now(ws: &mut Socket, id: u64, auth: &mut Authenticator, stack: &str) {
    let policy = json!({ "stack": stack, "policy": "auto", "window": window_around_now() });
    change(ws, id, auth, json!({ "kind": "policy", "data": policy })).await;
}

pub fn scheduled_for_owner(trigger: &RunTrigger, stack: &str) -> bool {
    matches!(trigger, RunTrigger::Scheduled { on_behalf_of }
        if on_behalf_of.get(stack).is_some_and(|a| a.subject == OWNER))
}
