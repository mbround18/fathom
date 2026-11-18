//! T066 adversarial review of the Keel relay (spec 067): proofs of concept.
//!
//! Each test asserts the SECURE behaviour, so it fails against today's code and is
//! `#[ignore]`d to keep the gate green. Run one with
//! `cargo test -p keel-server --test t066_review -- --ignored <name>`.

mod common;
mod relay_kit;
mod repos;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use keel_exec::client_config::{KeelSshClient, open_subsystem};
use keel_exec::codec::{read_frame, write_frame};
use keel_exec::frames::{self, Subsystem};
use keel_exec::relay::{RelayServerPolicy, relay_server_config};
use keel_server::relays::{RelayState, RelaySupervisor, SupervisorOptions};
use keel_server::runner_listener::{EnrollRequest, RunnerHost, enroll_key};
use relay_kit::{
    RelaySide, TestRelay, connect_over, harness, key, mint, opts, register, wait_live,
};
use repos::Harness;
use russh::ChannelMsg;
use russh::client::{AuthResult, Handle};
use russh::keys::{Certificate, PrivateKeyWithHashAlg};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

const FAILS_TODAY: &str = "T066 finding: fails today";

async fn linked(
    h: &Harness,
    zone: Uuid,
    options: SupervisorOptions,
) -> (TestRelay, Uuid, RelaySupervisor, relay_kit::Answered) {
    let mut relay = TestRelay::start(h).await;
    let id = register(
        h,
        zone,
        "edge-1",
        &relay.addr.to_string(),
        &relay.host_key_data(),
    )
    .await;
    let sup = RelaySupervisor::new(h.app.clone(), RunnerHost::new().expect("host"), options);
    sup.sync().await.expect("sync");
    let side = relay.next_link().await.answer(relay.fingerprint());
    wait_live(h, id, |l| l.state == RelayState::Connected).await;
    (relay, id, sup, side)
}

async fn bare_auth(ssh: &mut Handle<KeelSshClient>) {
    let bare = PrivateKeyWithHashAlg::new(Arc::new(key()), None);
    let auth = ssh
        .authenticate_publickey("runner", bare)
        .await
        .expect("auth");
    assert!(matches!(auth, AuthResult::Success), "bare key refused");
}

// ---- F1: a control channel with no window stalls the serve loop; channels pile up ----------

/// A relay whose SSH server grants Keel no window on the control channel. Keel's first `Ping`
/// write then blocks the link's single select loop for `health_interval` (30 s in production).
/// Meanwhile `LinkClientPolicy` keeps accepting `forwarded-tcpip` channels into an unbounded
/// queue, each with a 256 KiB window Keel buffers, and `max_streams` / the rate are never
/// applied because `carry` cannot run.
///
/// Secure: with `max_streams = 4`, no more than 4 carried channels stay open (and buffered)
/// whatever the control channel is doing.
#[tokio::test]
async fn t066_f1_zero_window_control_stalls_caps() {
    let _ = FAILS_TODAY;
    let Some((h, zone)) = harness("t066_f1").await else {
        return;
    };
    // A relay like `TestRelay`, but advertising window 0.
    let hub = h.app.relay_link.get().expect("relay-link key");
    let hub_key = hub.key().public_key().key_data().clone();
    let host_key = key();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (links_tx, mut links) = mpsc::unbounded_channel::<RelaySide>();
    let server_key = host_key.clone();
    let accept = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let mut config = relay_server_config(server_key.clone());
            config.window_size = 0;
            let (control_tx, control_rx) = oneshot::channel();
            let policy = RelayServerPolicy::new(
                vec![hub_key.clone()],
                Arc::new(|_| {}),
                Arc::new(|_| {}),
                control_tx,
            );
            let links_tx = links_tx.clone();
            tokio::spawn(async move {
                let Ok(session) = russh::server::run_stream(Arc::new(config), socket, policy).await
                else {
                    return;
                };
                let handle = session.handle();
                let running = tokio::spawn(session);
                if let Ok(control) = control_rx.await {
                    let _ = links_tx.send(RelaySide { handle, control });
                }
                let _ = running.await;
            });
        }
    });
    let host_key_data = host_key.public_key().key_data().clone();
    let id = register(&h, zone, "edge-1", &addr.to_string(), &host_key_data).await;
    let options = SupervisorOptions {
        health_interval: Duration::from_secs(8),
        max_streams: 4,
        ..opts()
    };
    let sup = RelaySupervisor::new(h.app.clone(), RunnerHost::new().expect("host"), options);
    sup.sync().await.expect("sync");
    let side = tokio::time::timeout(relay_kit::WAIT, links.recv())
        .await
        .expect("never linked")
        .expect("relay stopped");
    let side = side.answer(keel_exec::ca::fingerprint(&host_key_data));
    wait_live(&h, id, |l| l.version.is_some()).await;
    // Let the first tick's Ping block on the empty window.
    tokio::time::sleep(Duration::from_millis(300)).await;

    const N: u32 = 64;
    let closed = Arc::new(AtomicU32::new(0));
    let confirmed = Arc::new(AtomicU32::new(0));
    let buffered = Arc::new(AtomicU32::new(0));
    for port in 0..N {
        let handle = side.handle.clone();
        let (closed, confirmed, buffered) = (
            Arc::clone(&closed),
            Arc::clone(&confirmed),
            Arc::clone(&buffered),
        );
        tokio::spawn(async move {
            let Ok(mut ch) = handle
                .channel_open_forwarded_tcpip("keel-runner", 0, "10.9.0.1", port)
                .await
            else {
                closed.fetch_add(1, Ordering::SeqCst);
                return;
            };
            confirmed.fetch_add(1, Ordering::SeqCst);
            // 128 KiB that Keel must hold for a stream it has not looked at yet.
            let chunk = vec![0x41_u8; 128 * 1024];
            if tokio::time::timeout(Duration::from_secs(1), ch.data(&chunk[..]))
                .await
                .is_ok_and(|r| r.is_ok())
            {
                buffered.fetch_add(128, Ordering::SeqCst);
            }
            loop {
                match ch.wait().await {
                    Some(ChannelMsg::Close | ChannelMsg::Eof) | None => break,
                    Some(_) => {}
                }
            }
            closed.fetch_add(1, Ordering::SeqCst);
        });
    }
    tokio::time::sleep(Duration::from_secs(3)).await;
    let open = confirmed.load(Ordering::SeqCst)
        - closed
            .load(Ordering::SeqCst)
            .min(confirmed.load(Ordering::SeqCst));
    let kib = buffered.load(Ordering::SeqCst);
    sup.shutdown();
    accept.abort();
    h.finish().await;
    assert!(
        open <= 4,
        "{open} carried channels open past max_streams=4 while the control write stalled; \
         Keel accepted {kib} KiB of data on them"
    );
}

// ---- F2: the join first frame is read before the per-relay rate limit ------------------------

/// Past the per-relay join budget, a fresh bare key still gets `keel-join-v1` and Keel reads its
/// first frame (up to the 16 MiB codec cap, with a 30 s timeout) before `join_key` consults
/// `JoinLimiter`. Times `max_streams` (256), that is gigabytes per relay.
///
/// Secure: over budget, the join stream is closed before Keel reads (or buffers) a frame.
#[tokio::test]
async fn t066_f2_join_frame_is_read_past_the_rate_limit() {
    let Some((h, zone)) = harness("t066_f2").await else {
        return;
    };
    let (_relay, _id, sup, side) = linked(&h, zone, opts()).await;
    let rate = h.app.cfg.join_rate;
    for port in 1..=rate {
        let mut ssh = connect_over(&h, &side.handle, port).await;
        bare_auth(&mut ssh).await;
        let mut stream = open_subsystem(&ssh, Subsystem::Join).await.expect("join");
        write_frame(
            &mut stream,
            &frames::Frame::JoinRequest {
                token: frames::SecretString::new("keel-join-v1-not-a-token".to_owned()),
                inventory_name: "web-1".to_owned(),
                allowed_profiles: vec!["debian".to_owned()],
                require_signed: false,
                hostname: "web-1".to_owned(),
                os: "Debian".to_owned(),
                arch: "x86_64".to_owned(),
                runner_version: "0.1.0".to_owned(),
            },
        )
        .await
        .expect("join request");
        let _ = tokio::time::timeout(relay_kit::WAIT, read_frame(&mut stream)).await;
    }
    // The budget is spent. A new join stream sends an 8 MiB frame.
    let mut ssh = connect_over(&h, &side.handle, 9000).await;
    bare_auth(&mut ssh).await;
    let mut stream = open_subsystem(&ssh, Subsystem::Join).await.expect("join");
    let body = vec![b' '; 8 * 1024 * 1024];
    let len = u32::try_from(body.len()).expect("len").to_be_bytes();
    let accepted = tokio::time::timeout(Duration::from_secs(8), async {
        stream.write_all(&len).await?;
        stream.write_all(&body).await?;
        stream.flush().await
    })
    .await;
    sup.shutdown();
    h.finish().await;
    assert!(
        !matches!(accepted, Ok(Ok(()))),
        "Keel accepted an 8 MiB join frame after the relay's join budget was spent"
    );
}

// ---- F3: the auth deadline ends at any successful auth, even a bare key ----------------------

/// Any key authenticates as enroll-only/join-only, which fires `started` in `serve_relayed`; the
/// stream then has no deadline: the join service waits 30 s for its first frame and the SSH
/// session lingers until its 600 s inactivity timeout (keepalives reset it). A relay can hold
/// every stream slot with sessions that never present a credential.
///
/// Secure: a stream that has not authenticated as a runner (or finished enroll/join) is closed
/// at `auth_deadline`.
#[tokio::test]
async fn t066_f3_bare_key_session_outlives_auth_deadline() {
    let Some((h, zone)) = harness("t066_f3").await else {
        return;
    };
    let options = SupervisorOptions {
        auth_deadline: Duration::from_millis(300),
        ..opts()
    };
    let (_relay, id, sup, side) = linked(&h, zone, options).await;
    let mut ssh = connect_over(&h, &side.handle, 1).await;
    bare_auth(&mut ssh).await;
    let _stream = open_subsystem(&ssh, Subsystem::Join).await.expect("join");
    wait_live(&h, id, |l| l.streams >= 1).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let streams = h.app.relay_live.get(id).map(|l| l.streams);
    let closed = ssh.is_closed();
    sup.shutdown();
    h.finish().await;
    assert!(
        closed && streams == Some(0),
        "a bare-key session idled 10x past auth_deadline: streams={streams:?} closed={closed}"
    );
}

// ---- F4: a relay carries runners of another workspace ----------------------------------------

/// Relays belong to a workspace and zone, but `serve_relayed` never checks that the host behind
/// a carried runner session belongs to that workspace. A relay registered in workspace B carries
/// workspace A's runner, and `hosts.last_path` records B's relay for A's host.
///
/// Secure: a runner session for a host outside the relay's workspace is refused.
#[tokio::test]
async fn t066_f4_relay_carries_another_workspaces_runner() {
    let Some((h, _zone)) = harness("t066_f4").await else {
        return;
    };
    let other = Uuid::now_v7();
    sqlx::query("insert into workspaces (id, slug, name) values ($1, 'other', 'Other')")
        .bind(other)
        .execute(&h.db.owner)
        .await
        .expect("workspace");
    let other_zone = Uuid::now_v7();
    sqlx::query("insert into zones (id, workspace_id, name) values ($1, $2, 'edge')")
        .bind(other_zone)
        .bind(other)
        .execute(&h.db.owner)
        .await
        .expect("zone");
    let mut relay = TestRelay::start(&h).await;
    let line = russh::keys::PublicKey::from(relay.host_key_data())
        .to_openssh()
        .expect("line");
    let mut tx = h.app.db.begin().await.expect("tx");
    let r = keel_server::relays::register_relay(
        &mut tx,
        &keel_server::audit::Actor::System,
        &keel_server::relays::RelayRegistration {
            workspace: other,
            zone: other_zone,
            name: "edge-b",
            dial_address: &relay.addr.to_string(),
            host_key: &line,
            host: None,
        },
    )
    .await
    .expect("register");
    tx.commit().await.expect("commit");
    let sup = RelaySupervisor::new(h.app.clone(), RunnerHost::new().expect("host"), opts());
    sup.sync().await.expect("sync");
    let side = relay.next_link().await.answer(relay.fingerprint());
    wait_live(&h, r.id, |l| l.state == RelayState::Connected).await;

    // A host of workspace A (h.workspace) over workspace B's relay.
    let (_, token) = mint(&h, "web-a").await;
    let runner = key();
    let allowed = vec!["debian".to_owned()];
    let (host_id, cert) = enroll_key(
        &h.app,
        runner.public_key().key_data(),
        &EnrollRequest {
            token: &token,
            inventory_name: "web-a",
            allowed_profiles: &allowed,
            require_signed: false,
        },
    )
    .await
    .expect("enrol");
    let mut ssh = connect_over(&h, &side.handle, 7).await;
    let cert = Certificate::from_openssh(&cert).expect("cert");
    let auth = ssh
        .authenticate_openssh_cert("runner", Arc::new(runner), cert)
        .await
        .expect("auth");
    if matches!(auth, AuthResult::Success)
        && let Ok(mut stream) = open_subsystem(&ssh, Subsystem::Runner).await
    {
        let _ = write_frame(
            &mut stream,
            &frames::Frame::Hello {
                host_id: host_id.to_string(),
                version: "0.0.0".into(),
                ansible_core: "2.21.0".into(),
                python: "3.12.3".into(),
                allowed_profiles: allowed.clone(),
                require_signed: false,
                override_quorum: 1,
                ledger_head: None,
                bases: Vec::new(),
                age_recipient: None,
                capabilities: Vec::new(),
            },
        )
        .await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
    let path = h.app.runs.path(host_id);
    let (last_path,): (Option<serde_json::Value>,) =
        sqlx::query_as("select last_path from hosts where id = $1")
            .bind(host_id)
            .fetch_one(&h.db.owner)
            .await
            .expect("host");

    // Refusals coalesce per relay and reason: close the runner's window first.
    sup.flush_refusals(true).await;

    // Nor does it enrol a host into workspace A, and the token is left as it was.
    let (enrollment, token) = mint(&h, "web-b").await;
    let mut ssh = connect_over(&h, &side.handle, 8).await;
    bare_auth(&mut ssh).await;
    let mut enrolled = None;
    if let Ok(mut stream) = open_subsystem(&ssh, Subsystem::Enroll).await {
        let enroll = frames::Frame::Enroll {
            token: frames::SecretString::new(token),
            inventory_name: "web-b".to_owned(),
            allowed_profiles: allowed.clone(),
            require_signed: false,
        };
        if write_frame(&mut stream, &enroll).await.is_ok() {
            enrolled = tokio::time::timeout(Duration::from_secs(10), read_frame(&mut stream))
                .await
                .ok()
                .and_then(Result::ok)
                .flatten();
        }
    }
    let (burned,): (Option<String>,) =
        sqlx::query_as("select burned_reason from host_enrollments where id = $1")
            .bind(enrollment)
            .fetch_one(&h.db.owner)
            .await
            .expect("enrollment");
    // The refusal is noted once the enroll subsystem returns, just after the stream closes.
    let mut refused = Vec::new();
    for _ in 0..50 {
        sup.flush_refusals(true).await;
        refused = relay_kit::refused(&h, "workspace_mismatch").await;
        if refused.iter().any(|r| r["detail"] == "enroll") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    sup.shutdown();
    h.finish().await;
    assert!(
        path.is_none(),
        "workspace B's relay carries workspace A's runner: path={path:?} last_path={last_path:?}"
    );
    assert!(
        enrolled.is_none(),
        "enrolled over another workspace's relay: {enrolled:?}"
    );
    assert_eq!(burned, None, "the token was burned");
    let details: Vec<_> = refused.iter().map(|r| r["detail"].clone()).collect();
    assert!(
        details.contains(&serde_json::json!(host_id.to_string()))
            && details.contains(&serde_json::json!("enroll")),
        "{refused:?}"
    );
}
