//! Quickstart scenarios 6 to 10 of spec 061, end to end on one server (T086).
//!
//! Unlike `updates.rs`, which sets the status by hand, this drives the real checker against
//! `oci_registry::testing::FakeRegistry` and watches what owners get over `/api/v1/ws`:
//!
//! - 6: checks on, the tag at the running digest: the scheduled first check (60–120 s after
//!   start) says up to date, with HEAD requests only (SC-005).
//! - 7: a newer digest and Check now: the status is pushed without a reload, the new image is read
//!   once (one manifest GET pair and one config blob GET), and pressing again at once is refused
//!   with the time it is allowed (FR-007).
//! - 8: the dismissed dot survives a reconnect and the card stays; a different digest brings the
//!   dot back (FR-014).
//! - 9: a 429 keeps the last result and shows `rate_limited` and the next attempt (FR-010).
//! - 10: taking `keel-server` out of the allowed profiles gives the manual path naming
//!   `profile_not_allowed` and the `mbround18/keel@sha256:…` reference (FR-021).
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`); skips without it. Scenario 6 waits for the
//! real scheduled check, so this file takes about two minutes.

mod common;
mod repos;
mod update_fixture;

use std::sync::Arc;
use std::time::Duration;

use axum::http::header;
use futures_util::{SinkExt, StreamExt};
use keel_core::ports::TestClock;
use keel_protocol::{
    Cmd, Event, EventEnvelope, HostProfiles, ProfileName, UpdateCheckState, UpdateOutcome,
    UpdatePath, UpdateStatus,
};
use keel_server::build_info::BuildInfo;
use keel_server::config::UpdateConfig;
use keel_server::serve;
use keel_server::signin::Established;
use keel_server::updates::Updates;
use oci_registry::testing::FakeRegistry;
use oci_registry::{Client, Platform};
use repos::{Harness, ORIGIN};
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use update_fixture::{NEW, Setup, offer, running, system};

const REPO: &str = "mbround18/keel";
const TAG_PATH: &str = "/v2/mbround18/keel/manifests/latest";

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

async fn send(s: &mut Socket, id: u64, cmd: &Cmd) {
    let env = json!({ "id": id, "cmd": cmd });
    s.send(Message::Text(env.to_string().into())).await.unwrap();
}

/// Opens a socket as `est` and returns it with the `Me` it got (a page load).
async fn open_socket(addr: std::net::SocketAddr, est: &Established) -> (Socket, keel_protocol::Me) {
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
    send(&mut s, 1, &Cmd::GetMe).await;
    let me = match until(&mut s, Duration::from_secs(5), |e| {
        matches!(e, Event::Me(_))
    })
    .await
    {
        Some(Event::Me(me)) => me,
        other => panic!("expected Me, got {other:?}"),
    };
    (s, me)
}

/// The next event matching `want` within `within`, skipping others.
async fn until(s: &mut Socket, within: Duration, want: impl Fn(&Event) -> bool) -> Option<Event> {
    let end = tokio::time::Instant::now() + within;
    loop {
        let left = end.checked_duration_since(tokio::time::Instant::now())?;
        let m = tokio::time::timeout(left, s.next()).await.ok()?;
        match m {
            Some(Ok(Message::Text(t))) => {
                let env: EventEnvelope = serde_json::from_str(&t).expect("event");
                if want(&env.event) {
                    return Some(env.event);
                }
            }
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return None,
            Some(Ok(_)) => {}
        }
    }
}

/// The next pushed `UpdateStatus` that has finished checking.
async fn checked(s: &mut Socket, within: Duration) -> UpdateStatus {
    match until(
        s,
        within,
        |e| matches!(e, Event::UpdateStatus(st) if st.state == UpdateCheckState::Checked),
    )
    .await
    {
        Some(Event::UpdateStatus(st)) => st,
        other => panic!("expected a checked UpdateStatus, got {other:?}"),
    }
}

fn count(fake: &FakeRegistry, method: &str, part: &str) -> usize {
    fake.requests()
        .iter()
        .filter(|r| r.method == method && r.path.contains(part))
        .count()
}

fn authorised(fake: &FakeRegistry, method: &str, part: &str) -> usize {
    fake.requests()
        .iter()
        .filter(|r| r.method == method && r.path.contains(part) && r.had_authorization)
        .count()
}

/// The dot rule the web app applies (`features/settings/updates.ts`).
fn dot(s: &UpdateStatus, me: &keel_protocol::Me) -> bool {
    s.outcome == Some(UpdateOutcome::UpdateAvailable)
        && s.available.as_ref().map(|a| a.digest.as_str()) != me.update_dot_dismissed.as_deref()
}

#[tokio::test]
async fn scenarios_6_to_9_against_the_fake_registry() {
    let Some(mut h) = Harness::new("upd_walk").await else {
        return;
    };
    let fake = FakeRegistry::start().await;
    let platform = Platform::current();
    let running_image = fake.multi_arch(
        REPO,
        "latest",
        &[("org.opencontainers.image.revision", "abc1234")],
    );
    let u = fake.url();
    let image = format!("{}:{}/{REPO}", u.host_str().unwrap(), u.port().unwrap());
    let config = UpdateConfig::watching(true, Duration::from_secs(6 * 3600), &image).unwrap();
    // Deployed pinned to the index digest, like the keel_server role does.
    let build = BuildInfo {
        version: None,
        revision: Some("abc1234".into()),
        built_at: Some(time::OffsetDateTime::from_unix_timestamp(1_790_000_000).unwrap()),
        source: None,
        digest: Some(running_image.index.to_string()),
        platform,
    };
    let clock = Arc::new(TestClock::new(time::OffsetDateTime::now_utc()));
    h.app.updates = Arc::new(
        Updates::new(build, config)
            .with_clock(clock.clone())
            .with_client(Client::with_base_url("keel/test", fake.url())),
    );
    let (addr, stop) = spawn(&h).await;
    let (mut owner, me) = open_socket(addr, &h.owner).await;
    assert_eq!(me.update_dot_dismissed, None);

    // ---- 6: the scheduled first check, within 2 minutes, HEAD only ----
    let checker = h.app.updates.spawn_checker().expect("checks are on");
    let s = checked(&mut owner, Duration::from_secs(125)).await;
    assert_eq!(s.outcome, Some(UpdateOutcome::UpToDate), "{s:?}");
    assert!(s.checked_at.is_some());
    assert!(s.next_check_at.is_some(), "{s:?}");
    let reqs = fake.requests();
    eprintln!("scenario 6 requests: {reqs:?}");
    assert!(
        reqs.iter()
            .filter(|r| r.path.starts_with("/v2/"))
            .all(|r| r.method == "HEAD"),
        "{reqs:?}"
    );
    // The anonymous HEAD only fetches the token challenge; one authorised HEAD does the check.
    assert_eq!(authorised(&fake, "HEAD", "/manifests/"), 1);

    // ---- 7: a newer digest, Check now; pushed without a reload ----
    let newer = fake.multi_arch(
        REPO,
        "latest",
        &[("org.opencontainers.image.revision", "def5678")],
    );
    fake.clear_requests();
    send(&mut owner, 2, &Cmd::CheckForUpdates).await;
    let s = checked(&mut owner, Duration::from_secs(30)).await;
    assert_eq!(s.outcome, Some(UpdateOutcome::UpdateAvailable), "{s:?}");
    let available = s.available.clone().expect("available");
    assert_eq!(available.digest, newer.index.to_string());
    assert_eq!(available.revision.as_deref(), Some("def5678"));
    assert!(dot(&s, &me), "the dot shows");
    eprintln!("scenario 7 requests: {:?}", fake.requests());
    assert_eq!(authorised(&fake, "HEAD", "/manifests/"), 1);
    assert_eq!(
        count(&fake, "GET", "/manifests/"),
        2,
        "{:?}",
        fake.requests()
    );
    assert_eq!(count(&fake, "GET", "/blobs/"), 1, "{:?}", fake.requests());
    // Pressed again at once: refused with the time it's allowed.
    send(&mut owner, 3, &Cmd::CheckForUpdates).await;
    match until(&mut owner, Duration::from_secs(5), |e| {
        matches!(e, Event::OpError { .. })
    })
    .await
    {
        Some(Event::OpError { code, message, .. }) => {
            assert_eq!(code, "update_check_too_soon");
            assert_eq!(Some(message), s.manual_after.clone());
        }
        other => panic!("expected update_check_too_soon, got {other:?}"),
    }

    // ---- 8: dismiss, reload, a different digest brings the dot back ----
    send(
        &mut owner,
        4,
        &Cmd::DismissUpdateDot {
            digest: available.digest.clone(),
        },
    )
    .await;
    let after = match until(&mut owner, Duration::from_secs(5), |e| {
        matches!(e, Event::Me(_))
    })
    .await
    {
        Some(Event::Me(me)) => me,
        other => panic!("expected Me, got {other:?}"),
    };
    assert!(!dot(&s, &after));
    drop(owner);
    let (mut owner, me) = open_socket(addr, &h.owner).await;
    assert_eq!(
        me.update_dot_dismissed.as_deref(),
        Some(available.digest.as_str())
    );
    send(&mut owner, 5, &Cmd::GetUpdateStatus).await;
    let s = match until(&mut owner, Duration::from_secs(5), |e| {
        matches!(e, Event::UpdateStatus(_))
    })
    .await
    {
        Some(Event::UpdateStatus(s)) => s,
        other => panic!("{other:?}"),
    };
    assert!(s.available.is_some(), "the card stays");
    assert!(!dot(&s, &me), "the dot stays gone after the reload");

    let third = fake.multi_arch(
        REPO,
        "latest",
        &[("org.opencontainers.image.revision", "0123abc")],
    );
    clock.advance(time::Duration::seconds(61));
    send(&mut owner, 6, &Cmd::CheckForUpdates).await;
    let s = checked(&mut owner, Duration::from_secs(30)).await;
    assert_eq!(
        s.available.as_ref().map(|a| a.digest.clone()),
        Some(third.index.to_string())
    );
    assert!(dot(&s, &me), "the dot comes back for the new digest");

    // ---- 9: 429 keeps the last result, shows rate_limited and the next attempt ----
    fake.set_status(TAG_PATH, 429);
    clock.advance(time::Duration::seconds(61));
    send(&mut owner, 7, &Cmd::CheckForUpdates).await;
    let failed = checked(&mut owner, Duration::from_secs(30)).await;
    assert_eq!(failed.outcome, Some(UpdateOutcome::UpdateAvailable));
    assert_eq!(failed.available, s.available, "the last result stays");
    assert_eq!(failed.checked_at, s.checked_at);
    assert_eq!(
        failed.last_error.as_ref().map(|e| e.code.as_str()),
        Some("rate_limited")
    );
    assert!(failed.next_check_at.is_some(), "{failed:?}");
    eprintln!(
        "scenario 9: last_error {:?}, next_check_at {:?}",
        failed.last_error, failed.next_check_at
    );

    checker.abort();
    let _ = stop.send(());
    h.finish().await;
}

fn profiles(names: &[&str]) -> HostProfiles {
    HostProfiles(
        names
            .iter()
            .map(|n| ProfileName::new(*n).expect("profile"))
            .collect(),
    )
}

#[tokio::test]
async fn scenario_10_a_disallowed_profile_gives_the_manual_path() {
    let Some(mut h) = Harness::new("upd_walk_profile").await else {
        return;
    };
    running(&mut h, update_fixture::build(), false);
    offer(&h, UpdateOutcome::UpdateAvailable, NEW);
    system(&h, Setup::FULL).await;
    let path = || async {
        match h.cmd(Cmd::GetUpdateStatus).await.as_slice() {
            [Event::UpdateStatus(s)] => s.update_path.clone(),
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(path().await, UpdatePath::Run);
    // Through the System tab's own command, as an owner would.
    let (_, out) = h
        .confirm(Cmd::PreviewSystemConfigure {
            allowed_profiles: profiles(&["debian"]),
            version: 1,
        })
        .await;
    assert!(
        matches!(out.first(), Some(Event::Confirmed { .. })),
        "{out:?}"
    );
    match path().await {
        UpdatePath::Manual {
            missing,
            digest_ref,
            ..
        } => {
            assert_eq!(missing, "profile_not_allowed");
            assert_eq!(digest_ref, format!("mbround18/keel@{NEW}"));
        }
        UpdatePath::Run => panic!("expected the manual path"),
    }
    h.finish().await;
}
