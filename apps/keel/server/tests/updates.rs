//! The update notice on the server (spec 061 US1 and US3; T022, T049).
//!
//! - `GetKeelBuild` is shaped by role; `Me.keel_version` is set for every role.
//! - `record_build_seen` fills `keel_build_seen` silently the first time, audits `keel_updated`
//!   once per change, and writes nothing on the same build.
//! - A status change pushes `UpdateStatus` to owners, `UpdateStatusLine` to admins and nothing
//!   to users; `UpdateStatusLine` carries only `update_available` and `checked_at`.
//! - `GetUpdateStatus` and `CheckForUpdates` are shaped and limited by role; Check now is
//!   rate-limited once a minute; `DismissUpdateDot` stores a well-formed digest for owners.
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`); skips without it. No registry is called:
//! the status is set directly.

mod common;
mod repos;
mod update_fixture;

use std::time::Duration;

use axum::http::header;
use futures_util::{SinkExt, StreamExt};
use keel_protocol::{Cmd, Event, EventEnvelope, UpdateOutcome};
use keel_server::serve;
use keel_server::signin::Established;
use repos::{Harness, ORIGIN, op_error, sign_in};
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use update_fixture::{NEW, RUNNING, audited, build, offer, running};

async fn with_role(h: &Harness, sub: &str, name: &str, role: &str) -> Established {
    let est = sign_in(&h.app, sub, name).await;
    sqlx::query("update users set instance_role = $2 where id = $1")
        .bind(est.user.id)
        .bind(role)
        .execute(&h.db.owner)
        .await
        .expect("role");
    est
}

fn denied(events: &[Event]) -> String {
    match events {
        [Event::Denied { reason, .. }] => reason.clone(),
        other => panic!("expected Denied, got {other:?}"),
    }
}

fn me(events: &[Event]) -> keel_protocol::Me {
    match events {
        [Event::Me(me)] => me.clone(),
        other => panic!("expected Me, got {other:?}"),
    }
}

#[tokio::test]
async fn the_running_build_is_shaped_by_role() {
    let Some(mut h) = Harness::new("upd_build").await else {
        return;
    };
    running(&mut h, build(), false);
    let admin = with_role(&h, "2", "admin", "admin").await;
    let user = with_role(&h, "3", "user", "user").await;

    for est in [&h.owner, &admin] {
        match h.cmd_as(est, Cmd::GetKeelBuild).await.as_slice() {
            [Event::KeelBuild(b)] => {
                assert_eq!(b.version.as_deref(), Some("0.2.0"));
                assert_eq!(b.revision.as_deref(), Some("abcdef1"));
                assert!(b.built_at.is_some());
                assert_eq!(b.digest.as_deref(), Some(RUNNING));
                assert_eq!(b.display, "0.2.0");
            }
            other => panic!("{other:?}"),
        }
    }
    match h.cmd_as(&user, Cmd::GetKeelBuild).await.as_slice() {
        [Event::KeelBuild(b)] => {
            assert_eq!(b.version.as_deref(), Some("0.2.0"));
            assert_eq!(b.display, "0.2.0");
            assert_eq!((&b.revision, &b.built_at, &b.digest), (&None, &None, &None));
        }
        other => panic!("{other:?}"),
    }
    for est in [&h.owner, &admin, &user] {
        let m = me(&h.cmd_as(est, Cmd::GetMe).await);
        assert_eq!(m.keel_version.as_deref(), Some("0.2.0"));
    }
    h.finish().await;
}

#[tokio::test]
async fn a_start_on_a_new_build_is_audited_once() {
    let Some(h) = Harness::new("upd_build_seen").await else {
        return;
    };
    let record = keel_server::updates::record_build_seen;
    let first = build();
    // First start: the row is filled, nothing audited.
    assert!(!record(&h.app.db, &first).await.expect("record"));
    let row: (Option<String>, Option<String>, Option<String>) =
        sqlx::query_as("select digest, revision, version from keel_build_seen")
            .fetch_one(&h.db.owner)
            .await
            .expect("row");
    assert_eq!(
        row,
        (
            Some(RUNNING.into()),
            Some("abcdef1".into()),
            Some("0.2.0".into())
        )
    );
    assert!(audited(&h, "keel_updated").await.is_empty());
    // Same build again: nothing.
    assert!(!record(&h.app.db, &first).await.expect("record"));
    assert!(audited(&h, "keel_updated").await.is_empty());
    // A new digest and revision: one keel_updated, actor system.
    let mut next = build();
    next.digest = Some(NEW.into());
    next.revision = Some("fedcba9".into());
    next.version = Some("0.3.0".into());
    assert!(record(&h.app.db, &next).await.expect("record"));
    assert!(!record(&h.app.db, &next).await.expect("record"));
    let rows = audited(&h, "keel_updated").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["actor"], "system");
    assert_eq!(
        rows[0]["context"],
        json!({
            "old_digest": RUNNING,
            "new_digest": NEW,
            "old_revision": "abcdef1",
            "new_revision": "fedcba9",
            "old_version": "0.2.0",
            "new_version": "0.3.0",
        })
    );
    h.finish().await;
}

#[tokio::test]
async fn status_and_check_now_are_shaped_and_limited_by_role() {
    let Some(mut h) = Harness::new("upd_status").await else {
        return;
    };
    running(&mut h, build(), false);
    let admin = with_role(&h, "2", "admin", "admin").await;
    let user = with_role(&h, "3", "user", "user").await;
    offer(&h, UpdateOutcome::UpdateAvailable, NEW);

    match h.cmd(Cmd::GetUpdateStatus).await.as_slice() {
        [Event::UpdateStatus(s)] => {
            assert_eq!(s.outcome, Some(UpdateOutcome::UpdateAvailable));
            let a = s.available.as_ref().expect("available");
            assert_eq!(a.digest, NEW);
            assert_eq!(a.release_notes_note.as_deref(), Some("no source label"));
            assert_eq!(s.update_run, None);
            assert_eq!(s.last_update_run, None);
        }
        other => panic!("{other:?}"),
    }
    let line = h.cmd_as(&admin, Cmd::GetUpdateStatus).await;
    match line.as_slice() {
        [Event::UpdateStatusLine(l)] => {
            assert!(l.update_available);
            // SC-006: nothing about the image.
            let v = serde_json::to_value(l).expect("json");
            let mut keys: Vec<_> = v.as_object().expect("object").keys().cloned().collect();
            keys.sort();
            assert_eq!(keys, ["checked_at", "update_available"]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        denied(&h.cmd_as(&user, Cmd::GetUpdateStatus).await),
        "admin_only"
    );

    // Check now: owners only, and not with checks off.
    assert_eq!(
        denied(&h.cmd_as(&admin, Cmd::CheckForUpdates).await),
        "owner_only"
    );
    assert_eq!(
        op_error(&h.cmd(Cmd::CheckForUpdates).await),
        "update_check_off"
    );

    // With checks on, a manual check within the last minute gives the retry time. (The last
    // check is set directly, so nothing calls the registry.)
    running(&mut h, build(), true);
    let last = time::OffsetDateTime::now_utc();
    *h.app.updates.last_manual.lock().expect("lock") = Some(last);
    match h.cmd(Cmd::CheckForUpdates).await.as_slice() {
        [Event::OpError { code, message }] => {
            assert_eq!(code, "update_check_too_soon");
            let retry = time::OffsetDateTime::parse(
                message,
                &time::format_description::well_known::Rfc3339,
            )
            .expect("an RFC 3339 retry time");
            let wait = retry - last;
            assert!(
                wait > time::Duration::ZERO && wait <= time::Duration::minutes(1),
                "{wait}"
            );
        }
        other => panic!("{other:?}"),
    }
    h.finish().await;
}

#[tokio::test]
async fn dismissing_the_dot_is_owner_only_and_checked() {
    let Some(mut h) = Harness::new("upd_dismiss").await else {
        return;
    };
    running(&mut h, build(), false);
    let admin = with_role(&h, "2", "admin", "admin").await;

    assert_eq!(me(&h.cmd(Cmd::GetMe).await).update_dot_dismissed, None);
    let bad = h
        .cmd(Cmd::DismissUpdateDot {
            digest: "sha256:XYZ".into(),
        })
        .await;
    assert_eq!(op_error(&bad), "invalid_digest");
    let m = me(&h.cmd(Cmd::DismissUpdateDot { digest: NEW.into() }).await);
    assert_eq!(m.update_dot_dismissed.as_deref(), Some(NEW));
    assert_eq!(
        me(&h.cmd(Cmd::GetMe).await).update_dot_dismissed.as_deref(),
        Some(NEW)
    );

    let out = h
        .cmd_as(&admin, Cmd::DismissUpdateDot { digest: NEW.into() })
        .await;
    assert_eq!(denied(&out), "owner_only");
    // Even with a value stored, non-owners never get one back.
    sqlx::query("update users set update_dot_dismissed = $2 where id = $1")
        .bind(admin.user.id)
        .bind(NEW)
        .execute(&h.db.owner)
        .await
        .expect("dismissed");
    assert_eq!(
        me(&h.cmd_as(&admin, Cmd::GetMe).await).update_dot_dismissed,
        None
    );
    h.finish().await;
}

// ---- WebSocket ------------------------------------------------------------------------------

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn cookie_pair(e: &Established) -> String {
    let set = e.cookies[0].to_str().expect("ascii");
    set.split(';').next().expect("pair").to_owned()
}

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

async fn open_socket(addr: std::net::SocketAddr, est: &Established) -> Socket {
    let mut req = format!("ws://{addr}/api/v1/ws")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert(header::ORIGIN, ORIGIN.parse().unwrap());
    req.headers_mut()
        .insert(header::COOKIE, cookie_pair(est).parse().unwrap());
    let (mut s, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let hello = json!({
        "protocol_version": keel_protocol::PROTOCOL_VERSION,
        "client": "test/1",
        "csrf_token": est.csrf,
    });
    s.send(Message::Text(hello.to_string().into()))
        .await
        .unwrap();
    s.send(Message::Text(r#"{"id":1,"cmd":{"type":"GetMe"}}"#.into()))
        .await
        .unwrap();
    assert!(matches!(
        next(&mut s, Duration::from_secs(5)).await,
        Some(EventEnvelope {
            event: Event::Me(_),
            ..
        })
    ));
    s
}

/// The next event within `within`, or `None`.
async fn next(s: &mut Socket, within: Duration) -> Option<EventEnvelope> {
    loop {
        let m = tokio::time::timeout(within, s.next()).await.ok()?;
        match m {
            Some(Ok(Message::Text(t))) => return Some(serde_json::from_str(&t).expect("event")),
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return None,
            Some(Ok(_)) => {}
        }
    }
}

#[tokio::test]
async fn a_status_change_is_pushed_by_role() {
    let Some(mut h) = Harness::new("upd_push").await else {
        return;
    };
    running(&mut h, build(), false);
    let admin = with_role(&h, "2", "admin", "admin").await;
    let user = with_role(&h, "3", "user", "user").await;
    let (addr, stop) = spawn(&h).await;
    let mut owner_s = open_socket(addr, &h.owner).await;
    let mut admin_s = open_socket(addr, &admin).await;
    let mut user_s = open_socket(addr, &user).await;

    offer(&h, UpdateOutcome::UpdateAvailable, NEW);

    match next(&mut owner_s, Duration::from_secs(5)).await {
        Some(EventEnvelope {
            reply_to: None,
            event: Event::UpdateStatus(s),
        }) => assert_eq!(s.available.expect("available").digest, NEW),
        other => panic!("owner: {other:?}"),
    }
    match next(&mut admin_s, Duration::from_secs(5)).await {
        Some(EventEnvelope {
            reply_to: None,
            event: Event::UpdateStatusLine(l),
        }) => assert!(l.update_available),
        other => panic!("admin: {other:?}"),
    }
    let nothing = next(&mut user_s, Duration::from_millis(500)).await;
    assert!(nothing.is_none(), "user got {nothing:?}");
    let _ = stop.send(());
    h.finish().await;
}
