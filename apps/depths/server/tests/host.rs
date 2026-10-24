//! Host metrics over the socket (`WatchHost`), and the per-stack stats the overview builds on:
//! admin only, off unless `DEPTHS_HOST_PROC` is set, and stopped when access goes.

mod support;

use std::sync::Arc;
use std::time::Duration;

use depths_protocol::{Role, UserId};
use futures_util::StreamExt as _;
use serde_json::{Value, json};
use support::ws::*;
use support::{App, OWNER, app_with, container, json as body_json};
use tokio_tungstenite::tungstenite::Message;

/// An admin who is not an owner.
const ADMIN: &str = "9";
/// A user with stack `web`.
const USER: &str = "7";

/// `streams_app` with host metrics on (a fake `/proc`) or off.
fn host_app(on: bool) -> App {
    let a = app_with(|s| {
        s.ws.stats_every = Duration::from_millis(50);
        let mut cfg = (*s.cfg).clone();
        cfg.host_proc = on.then(|| support::fake_host_proc(&cfg));
        s.cfg = Arc::new(cfg);
    });
    a.add_stack("web", COMPOSE, None);
    a.add_stack("db", COMPOSE, None);
    a.set_containers(vec![
        container("web", "app", "running", "Up 1 hour"),
        container("db", "app", "exited", "Exited (0) 1 minute ago"),
        container("stray", "x", "running", "Up 1 hour"),
    ]);
    grant(&a, USER, &["web"]);
    a.state
        .users
        .put(
            &UserId::new(format!("discord:{ADMIN}")).unwrap(),
            Role::Admin,
            &[],
        )
        .unwrap();
    a
}

/// `HostSample`s seen within `window`, skipping everything else.
async fn host_samples_within(ws: &mut Socket, window: Duration) -> usize {
    let deadline = tokio::time::Instant::now() + window;
    let mut n = 0;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(left, ws.next()).await {
            Err(_) => return n,
            Ok(Some(Ok(Message::Text(t)))) => {
                let e: Value = serde_json::from_str(t.as_str()).unwrap();
                if e["event"]["type"] == "HostSample" {
                    n += 1;
                }
            }
            Ok(Some(Ok(_))) => {}
            Ok(other) => panic!("socket ended: {other:?}"),
        }
    }
}

/// Waits for the samples already queued to drain, then counts what arrives in the next 300 ms.
async fn host_samples_after_settling(ws: &mut Socket) -> usize {
    host_samples_within(ws, Duration::from_millis(200)).await;
    host_samples_within(ws, Duration::from_millis(300)).await
}

async fn features(a: &App, subject: &str) -> Value {
    let session = a.sign_in(subject).await;
    let me = body_json(a.get("/api/v1/me", Some(&session)).await).await;
    me["features"].clone()
}

#[tokio::test]
async fn an_admin_watches_the_host_until_unwatching() {
    let a = host_app(true);
    let (mut ws, server, _) = open(&a, ADMIN).await;
    tell(&mut ws, json!({ "type": "WatchHost" })).await;
    let s = next_of(&mut ws, "HostSample").await;
    assert_eq!(s.get("reply_to"), None, "a live update: {s}");
    let d = &s["event"]["data"];
    assert_eq!(d["cpu_cores"], 4);
    assert_eq!(
        d["cpu_percent"],
        Value::Null,
        "a static /proc/stat has no ticks to share"
    );
    assert_eq!(
        d["load"],
        json!({ "one": 0.5, "five": 0.25, "fifteen": 0.1 })
    );
    assert_eq!(
        d["memory"],
        json!({ "total_bytes": 8_u64 << 30, "available_bytes": 6_u64 << 30, "used_bytes": 2_u64 << 30 })
    );
    assert_eq!(
        d["swap"],
        json!({ "total_bytes": 1_u64 << 30, "free_bytes": 1_u64 << 30, "used_bytes": 0 })
    );
    assert_eq!(d["uptime_secs"], 86_400);
    assert!(
        d["disk"]["total_bytes"].as_u64().unwrap() > 0,
        "statvfs of the stacks dir: {d}"
    );
    assert_eq!(
        (
            d["containers_running"].as_u64(),
            d["containers_stopped"].as_u64()
        ),
        (Some(2), Some(1))
    );
    assert!(
        !d.as_object().unwrap().contains_key("path"),
        "no host paths leave the server"
    );

    // A second watch keeps the first: still one sample per tick.
    tell(&mut ws, json!({ "type": "WatchHost" })).await;
    next_of(&mut ws, "HostSample").await;

    tell(&mut ws, json!({ "type": "UnwatchHost" })).await;
    assert_eq!(
        host_samples_after_settling(&mut ws).await,
        0,
        "no samples after UnwatchHost"
    );
    server.abort();
}

#[tokio::test]
async fn a_user_is_denied_the_host_whatever_the_setting() {
    for on in [true, false] {
        let a = host_app(on);
        let (mut ws, server, _) = open(&a, USER).await;
        let r = ask(&mut ws, 1, json!({ "type": "WatchHost" })).await;
        assert_eq!(r["event"]["type"], "Denied", "{r}");
        assert_eq!(
            r["event"]["data"]["reason"], "not_permitted",
            "on={on}: {r}"
        );
        assert_eq!(
            host_samples_within(&mut ws, Duration::from_millis(200)).await,
            0
        );
        server.abort();
    }
}

#[tokio::test]
async fn an_admin_is_told_host_metrics_are_off_when_unset() {
    let a = host_app(false);
    let (mut ws, server, _) = open(&a, ADMIN).await;
    let r = ask(&mut ws, 1, json!({ "type": "WatchHost" })).await;
    assert_eq!(r["event"]["data"]["reason"], "feature_disabled", "{r}");
    server.abort();
}

#[tokio::test]
async fn me_offers_host_metrics_to_admins_only_and_only_when_set() {
    // One sign-in per app: the auth routes are rate limited.
    for (on, subject, want) in [
        (true, ADMIN, true),
        (true, OWNER, true),
        (true, USER, false),
        (false, ADMIN, false),
    ] {
        let a = host_app(on);
        assert_eq!(
            features(&a, subject).await["host_metrics"],
            want,
            "on={on} subject={subject}"
        );
    }
}

#[tokio::test]
async fn demoting_an_admin_stops_their_host_watch() {
    let a = host_app(true);
    let (mut ws, server, _) = open(&a, ADMIN).await;
    tell(&mut ws, json!({ "type": "WatchHost" })).await;
    next_of(&mut ws, "HostSample").await;

    grant(&a, ADMIN, &["web"]);
    assert_eq!(
        host_samples_after_settling(&mut ws).await,
        0,
        "a user gets no host samples"
    );
    // The socket itself stays: they still have a stack.
    let r = ask(&mut ws, 2, json!({ "type": "ListStacks" })).await;
    assert_eq!(names(&r), ["web"]);
    server.abort();
}

#[tokio::test]
async fn the_host_watch_counts_as_one_stream() {
    let a = host_app(true);
    let (mut ws, server, _) = open(&a, ADMIN).await;
    let logs = open_logs(json!({ "kind": "stack", "stack": "web" }));
    for i in 0..depths_protocol::MAX_STREAMS_PER_SOCKET as u64 - 1 {
        stream_of(&ask(&mut ws, 10 + i, logs.clone()).await);
    }
    tell(&mut ws, json!({ "type": "WatchHost" })).await;
    next_of(&mut ws, "HostSample").await;
    let r = ask(&mut ws, 99, logs).await;
    assert_eq!(r["event"]["type"], "Invalid", "the 33rd stream: {r}");
    server.abort();
}

/// The overview's per-stack table: `WatchStats` on stack targets carries only the stacks the
/// person may see, re-checked every tick.
#[tokio::test]
async fn stack_stats_follow_the_stacks_a_user_may_view() {
    let a = host_app(false);
    grant(&a, USER, &["web", "db"]);
    let (mut ws, server, _) = open(&a, USER).await;
    tell(
        &mut ws,
        json!({ "type": "WatchStats", "data": { "targets": [
            { "kind": "stack", "stack": "web" },
            { "kind": "stack", "stack": "db" },
        ] } }),
    )
    .await;
    let ids = |s: &Value| -> Vec<String> {
        let mut v: Vec<String> = s["event"]["data"]["samples"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["container_id"].as_str().unwrap().to_owned())
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        ids(&next_of(&mut ws, "StatsSample").await),
        ["db-app-id", "web-app-id"]
    );

    grant(&a, USER, &["web"]);
    until("db's samples stop", || {
        a.runtime
            .stats_calls
            .lock()
            .unwrap()
            .last()
            .is_some_and(|c| c == &["web-app-id".to_owned()])
    })
    .await;
    // Drain whatever was in flight, then every sample is web's alone.
    host_samples_within(&mut ws, Duration::from_millis(150)).await;
    let s = next_of(&mut ws, "StatsSample").await;
    assert_eq!(ids(&s), ["web-app-id"]);

    // A stack outside the list is refused outright, and a new watch replaces the old.
    let r = ask(
        &mut ws,
        5,
        json!({ "type": "WatchStats", "data": { "targets": [{ "kind": "stack", "stack": "db" }] } }),
    )
    .await;
    assert_eq!(r["event"]["data"]["reason"], "not_permitted", "{r}");
    server.abort();
}
