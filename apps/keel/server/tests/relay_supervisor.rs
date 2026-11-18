//! The relay supervisor (spec 067 T020, T021, T023): Keel dials a registered relay, pins its host
//! key, logs in with the relay-link key, waits for `Hello`, pings, and serves the runner
//! streams the relay forwards as ordinary runner connections with a relay path.
//!
//! The relay here is a minimal in-test one built from `keel_exec::relay`'s server policy; the
//! real `keel-relay` binary is exercised by `relay_link` (T014).
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`); skips without it.

mod common;
mod repos;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use keel_exec::ca::{HostCertVerifier, TrustedCa};
use keel_exec::client_config::{KeelSshClient, client_config, open_subsystem};
use keel_exec::codec::write_frame;
use keel_exec::frames::{self, Subsystem};
use keel_exec::relay::{self, Frame, RelayServerPolicy, relay_server_config};
use keel_protocol::{Cmd, Event, HostName, HostProfiles, ProfileName};
use keel_server::audit::Actor;
use keel_server::hosts;
use keel_server::relays::{
    self, RegisterError, RelayLive, RelayRegistration, RelayState, RelaySupervisor,
    SupervisorOptions,
};
use keel_server::runner_listener::{EnrollRequest, RunnerHost, enroll_key};
use repos::Harness;
use russh::ChannelStream;
use russh::client::{self, AuthResult};
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, Certificate, PrivateKey};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use uuid::Uuid;

const WAIT: Duration = Duration::from_secs(10);

fn key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("key")
}

fn opts() -> SupervisorOptions {
    SupervisorOptions {
        health_interval: Duration::from_millis(200),
        hello_timeout: Duration::from_secs(5),
        ..SupervisorOptions::default()
    }
}

/// A harness with the SSH CAs, the relay-link key and a zone `edge`.
async fn harness(test: &str) -> Option<(Harness, Uuid)> {
    let h = Harness::new(test).await?;
    hosts::bootstrap(&h.app).await.expect("ssh bootstrap");
    relays::bootstrap(&h.app).await.expect("relay bootstrap");
    let zone = Uuid::now_v7();
    sqlx::query("insert into zones (id, workspace_id, name) values ($1, $2, 'edge')")
        .bind(zone)
        .bind(h.workspace)
        .execute(&h.db.owner)
        .await
        .expect("zone");
    Some((h, zone))
}

/// Registers a relay at `addr` pinning `host_key`.
async fn register(h: &Harness, zone: Uuid, name: &str, addr: &str, host_key: &KeyData) -> Uuid {
    let line = russh::keys::PublicKey::from(host_key.clone())
        .to_openssh()
        .expect("line");
    let mut tx = h.app.db.begin().await.expect("tx");
    let r = relays::register_relay(
        &mut tx,
        &Actor::System,
        &RelayRegistration {
            workspace: h.workspace,
            zone,
            name,
            dial_address: addr,
            host_key: &line,
            host: None,
        },
    )
    .await
    .expect("register");
    tx.commit().await.expect("commit");
    r.id
}

/// One link the test relay accepted: its session handle and its control stream.
struct RelaySide {
    handle: russh::server::Handle,
    control: ChannelStream<russh::server::Msg>,
}

impl RelaySide {
    /// Sends `Hello` (naming `fingerprint`), then answers every `Ping` with a `Pong`.
    fn answer(self, fingerprint: String) -> (russh::server::Handle, JoinHandle<()>) {
        let (mut rd, mut wr) = tokio::io::split(self.control);
        let task = tokio::spawn(async move {
            let hello = Frame::Hello {
                version: "0.9.0-test".into(),
                relay_key_fingerprint: fingerprint,
                max_streams: 256,
                http_cache: false,
            };
            if relay::write_frame(&mut wr, &hello).await.is_err() {
                return;
            }
            while let Ok(Some(frame)) = relay::read_frame(&mut rd).await {
                if let Frame::Ping { seq } = frame {
                    let pong = Frame::Pong {
                        seq,
                        version: "0.9.0-test".into(),
                        streams: 0,
                        uptime_s: 1,
                    };
                    if relay::write_frame(&mut wr, &pong).await.is_err() {
                        return;
                    }
                }
            }
        });
        (self.handle, task)
    }
}

/// A relay listening on TCP, trusting Keel's relay-link key. Each link whose control channel
/// opens arrives on `links`.
struct TestRelay {
    addr: SocketAddr,
    host_key: PrivateKey,
    links: mpsc::UnboundedReceiver<RelaySide>,
    task: JoinHandle<()>,
}

impl TestRelay {
    async fn start(h: &Harness) -> Self {
        let hub = h.app.relay_link.get().expect("relay-link key");
        let hub_key = hub.key().public_key().key_data().clone();
        let host_key = key();
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (links_tx, links) = mpsc::unbounded_channel();
        let server_key = host_key.clone();
        let task = tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let mut config = relay_server_config(server_key.clone());
                config.auth_rejection_time = Duration::from_millis(1);
                let (control_tx, control_rx) = oneshot::channel();
                let policy = RelayServerPolicy::new(
                    vec![hub_key.clone()],
                    Arc::new(|_| {}),
                    Arc::new(|_| {}),
                    control_tx,
                );
                let links_tx = links_tx.clone();
                tokio::spawn(async move {
                    let Ok(session) =
                        russh::server::run_stream(Arc::new(config), socket, policy).await
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
        Self {
            addr,
            host_key,
            links,
            task,
        }
    }

    fn host_key_data(&self) -> KeyData {
        self.host_key.public_key().key_data().clone()
    }

    fn fingerprint(&self) -> String {
        keel_exec::ca::fingerprint(&self.host_key_data())
    }

    async fn next_link(&mut self) -> RelaySide {
        tokio::time::timeout(WAIT, self.links.recv())
            .await
            .expect("Keel never linked")
            .expect("relay stopped")
    }
}

impl Drop for TestRelay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Waits until relay `id`'s live state satisfies `ok`.
async fn wait_live(h: &Harness, id: Uuid, ok: impl Fn(&RelayLive) -> bool) -> RelayLive {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let live = h.app.relay_live.get(id);
        if let Some(l) = &live
            && ok(l)
        {
            return l.clone();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "relay state never matched; last {live:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn refused(h: &Harness, reason: &str) -> Vec<serde_json::Value> {
    let rows: Vec<(serde_json::Value,)> = sqlx::query_as(
        "select context from audit_event where action = 'relay.refused' and reason = $1",
    )
    .bind(reason)
    .fetch_all(&h.db.owner)
    .await
    .expect("audit");
    rows.into_iter().map(|r| r.0).collect()
}

#[tokio::test]
async fn registration_validates_and_rejects_duplicates() {
    let Some((h, zone)) = harness("relay_register").await else {
        return;
    };
    let line = key().public_key().to_openssh().expect("line");
    let attempt = |name: &'static str, addr: &'static str, key: String, zone: Uuid| {
        let h = &h;
        async move {
            let mut tx = h.app.db.begin().await.expect("tx");
            let r = relays::register_relay(
                &mut tx,
                &Actor::System,
                &RelayRegistration {
                    workspace: h.workspace,
                    zone,
                    name,
                    dial_address: addr,
                    host_key: &key,
                    host: None,
                },
            )
            .await;
            tx.commit().await.expect("commit");
            r
        }
    };
    let first = attempt("edge-1", "relay.example.com:2222", line.clone(), zone)
        .await
        .expect("register");
    assert!(first.fingerprint.starts_with("SHA256:"));
    assert!(matches!(
        attempt("direct", "relay.example.com:2222", line.clone(), zone).await,
        Err(RegisterError::Name)
    ));
    assert!(matches!(
        attempt("edge-2", "https://relay:22", line.clone(), zone).await,
        Err(RegisterError::DialAddress(_))
    ));
    assert!(matches!(
        attempt("edge-2", "relay:22", "ssh-rsa AAAA x".into(), zone).await,
        Err(RegisterError::HostKey(_))
    ));
    assert!(matches!(
        attempt("edge-2", "relay:22", line.clone(), Uuid::now_v7()).await,
        Err(RegisterError::Zone)
    ));
    assert!(matches!(
        attempt(
            "edge-1",
            "relay:22",
            key().public_key().to_openssh().unwrap(),
            zone
        )
        .await,
        Err(RegisterError::NameTaken)
    ));
    assert!(matches!(
        attempt("edge-2", "relay:22", line, zone).await,
        Err(RegisterError::HostKeyTaken)
    ));
    let actions = h.audit_actions().await;
    assert_eq!(
        actions
            .iter()
            .filter(|(a, _)| a == relays::ACTION_REGISTER)
            .count(),
        1
    );
    h.finish().await;
}

#[tokio::test]
async fn keel_links_pings_and_carries_a_runner_over_the_relay() {
    let Some((h, zone)) = harness("relay_supervisor_link").await else {
        return;
    };
    let mut relay = TestRelay::start(&h).await;
    let id = register(
        &h,
        zone,
        "edge-1",
        &relay.addr.to_string(),
        &relay.host_key_data(),
    )
    .await;
    let host = RunnerHost::new().expect("host");
    let sup = RelaySupervisor::new(h.app.clone(), host, opts());
    let mut changes = h.app.relay_live.subscribe();
    sup.sync().await.expect("sync");

    // Linked but no Hello yet: connecting, and a stream now is refused.
    let side = relay.next_link().await;
    assert_eq!(
        h.app.relay_live.get(id).map(|l| l.state),
        Some(RelayState::Connecting)
    );
    // Refused at open (T066 F1): Keel never holds a channel it will not carry.
    let early = side
        .handle
        .channel_open_forwarded_tcpip("keel-runner", 0, "10.0.0.5", 40121)
        .await;
    assert!(early.is_err(), "a stream before Hello was accepted");

    // Hello, then Pongs: connected, with an RTT and the relay's version.
    let (handle, _responder) = side.answer(relay.fingerprint());
    let live = wait_live(&h, id, |l| l.state == RelayState::Connected).await;
    assert!(live.rtt_ms.is_some(), "{live:?}");
    assert_eq!(live.version.as_deref(), Some("0.9.0-test"));
    let first = changes.try_recv().expect("a change");
    assert_eq!(first.relay, id);
    assert_eq!(first.workspace, h.workspace);

    // A real runner session over a forwarded channel: authenticated with its certificate, its
    // Hello records the relay path with the advisory peer.
    let (_, token) = mint(&h, "web-1").await;
    let runner = key();
    let allowed = vec!["debian".to_owned()];
    let (host_id, cert) = enroll_key(
        &h.app,
        runner.public_key().key_data(),
        &EnrollRequest {
            token: &token,
            inventory_name: "web-1",
            allowed_profiles: &allowed,
            require_signed: false,
        },
    )
    .await
    .expect("enrol");
    let channel = handle
        .channel_open_forwarded_tcpip("keel-runner", 0, "10.0.0.5", 40122)
        .await
        .expect("open runner channel");
    let cas = h.app.ssh.get().expect("cas");
    let client = KeelSshClient::new(HostCertVerifier::new(
        vec![TrustedCa::active(cas.host.key_data().clone())],
        "keel.example.com",
    ));
    let mut runner_ssh = tokio::time::timeout(
        WAIT,
        client::connect_stream(Arc::new(client_config()), channel.into_stream(), client),
    )
    .await
    .expect("handshake hung")
    .expect("handshake over the relay");
    let cert = Certificate::from_openssh(&cert).expect("cert");
    assert!(matches!(
        runner_ssh
            .authenticate_openssh_cert("runner", Arc::new(runner), cert)
            .await
            .expect("auth"),
        AuthResult::Success
    ));
    let mut stream = open_subsystem(&runner_ssh, Subsystem::Runner)
        .await
        .expect("runner subsystem");
    write_frame(
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
    .await
    .expect("hello");
    let deadline = tokio::time::Instant::now() + WAIT;
    let path = loop {
        let (path,): (Option<serde_json::Value>,) =
            sqlx::query_as("select last_path from hosts where id = $1")
                .bind(host_id)
                .fetch_one(&h.db.owner)
                .await
                .expect("host");
        if let Some(path) = path
            && h.app.runs.path(host_id).is_some()
        {
            break path;
        }
        assert!(tokio::time::Instant::now() < deadline, "no last_path");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(path["kind"], "relay", "{path}");
    assert_eq!(path["relay"], id.to_string(), "{path}");
    assert_eq!(path["peer"], "10.0.0.5:40122", "{path}");
    assert_eq!(h.app.runs.path(host_id).and_then(|p| p.relay()), Some(id));
    let live = wait_live(&h, id, |l| l.streams == 1).await;
    assert_eq!(live.state, RelayState::Connected);

    sup.flush_refusals(true).await;
    let early = refused(&h, "stream_before_control").await;
    assert_eq!(early.len(), 1, "{early:?}");
    assert_eq!(early[0]["count"], 1);
    assert_eq!(early[0]["detail"], "10.0.0.5:40121");
    let states: Vec<(serde_json::Value,)> =
        sqlx::query_as("select context from audit_event where action = 'relay.state' order by seq")
            .fetch_all(&h.db.owner)
            .await
            .expect("audit");
    assert_eq!(states.len(), 1, "{states:?}");
    assert_eq!(states[0].0["state"], "connected");

    // Stopping the link sends Close and ends the runner stream with it.
    drop(stream);
    sup.shutdown();
    drop(runner_ssh);
    h.finish().await;
}

/// Mints an enrolment token for `name` (profile `debian`).
async fn mint(h: &Harness, name: &str) -> (Uuid, String) {
    sqlx::query("insert into profiles (id, workspace_id, name) values ($1, $2, 'debian')")
        .bind(Uuid::now_v7())
        .bind(h.workspace)
        .execute(&h.db.owner)
        .await
        .expect("profile");
    let cmd = Cmd::PreviewHostEnroll {
        workspace: h.workspace,
        inventory_name: HostName::new(name).expect("name"),
        profiles: HostProfiles(vec![
            ProfileName::new("debian".to_owned()).expect("profile"),
        ]),
        auto_converge: None,
    };
    let (_, out) = h.confirm(cmd).await;
    match out.as_slice() {
        [
            Event::Confirmed { .. },
            Event::HostEnrollment { id, token, .. },
        ] => (*id, token.0.clone()),
        other => panic!("enrol: {other:?}"),
    }
}

#[tokio::test]
async fn streams_over_max_streams_are_refused() {
    let Some((h, zone)) = harness("relay_supervisor_limits").await else {
        return;
    };
    let mut relay = TestRelay::start(&h).await;
    let id = register(
        &h,
        zone,
        "edge-1",
        &relay.addr.to_string(),
        &relay.host_key_data(),
    )
    .await;
    let sup = RelaySupervisor::new(
        h.app.clone(),
        RunnerHost::new().expect("host"),
        SupervisorOptions {
            max_streams: 1,
            ..opts()
        },
    );
    sup.sync().await.expect("sync");
    let (handle, _responder) = relay.next_link().await.answer(relay.fingerprint());
    wait_live(&h, id, |l| l.state == RelayState::Connected).await;

    // The first is carried (Keel's SSH server waits for the runner's handshake) …
    let _held = handle
        .channel_open_forwarded_tcpip("keel-runner", 0, "10.0.0.5", 1)
        .await
        .expect("open");
    wait_live(&h, id, |l| l.streams == 1).await;
    // … the second is over the cap and refused at open (T066 F1).
    let over = handle
        .channel_open_forwarded_tcpip("keel-runner", 0, "10.0.0.5", 2)
        .await;
    assert!(over.is_err(), "a stream over max_streams was accepted");
    sup.flush_refusals(true).await;
    let rows = refused(&h, "max_streams").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["detail"], "10.0.0.5:2");
    // Not a violation: the relay stays in service.
    assert_eq!(
        h.app.relay_live.get(id).map(|l| l.state),
        Some(RelayState::Connected)
    );
    sup.shutdown();
    h.finish().await;
}

/// A mismatch after the pinned key has authenticated (T066 F7: quarantine after auth only; a
/// pre-auth mismatch is a lost link, see `hostile_relay.rs` and `t066_review.rs`).
#[tokio::test]
async fn a_host_key_mismatch_after_auth_quarantines_the_relay_and_carries_nothing() {
    let Some((h, zone)) = harness("relay_supervisor_hostkey").await else {
        return;
    };
    let mut relay = TestRelay::start(&h).await;
    // Pinned to a key the relay does not hold, which once authenticated.
    let wrong = key().public_key().key_data().clone();
    let id = register(&h, zone, "edge-1", &relay.addr.to_string(), &wrong).await;
    sqlx::query("update relays set host_key_verified_at = now() where id = $1")
        .bind(id)
        .execute(&h.db.owner)
        .await
        .expect("verified");
    let sup = RelaySupervisor::new(h.app.clone(), RunnerHost::new().expect("host"), opts());
    sup.sync().await.expect("sync");

    let live = wait_live(&h, id, |l| l.state == RelayState::Quarantined).await;
    assert!(
        live.last_error
            .as_deref()
            .is_some_and(|e| e.starts_with("host_key_mismatch")),
        "{live:?}"
    );
    let (reason, at): (Option<String>, Option<time::OffsetDateTime>) =
        sqlx::query_as("select quarantine_reason, quarantined_at from relays where id = $1")
            .bind(id)
            .fetch_one(&h.db.owner)
            .await
            .expect("relay");
    assert_eq!(reason.as_deref(), Some("host_key_mismatch"));
    assert!(at.is_some());
    let audit = h.audit_text().await;
    assert!(audit.contains("relay.quarantine"), "{audit}");
    assert!(
        audit.contains(&relay.fingerprint()),
        "presented key in the detail"
    );

    // No link came up, and a re-read does not dial a quarantined relay again.
    sup.sync().await.expect("sync");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        relay.links.try_recv().is_err(),
        "a link reached the control channel"
    );
    assert_eq!(
        h.app.relay_live.get(id).map(|l| l.state),
        Some(RelayState::Quarantined)
    );
    sup.shutdown();
    h.finish().await;
}

#[tokio::test]
async fn an_unreachable_relay_is_redialled_and_a_dropped_link_recovers() {
    let Some((h, zone)) = harness("relay_supervisor_redial").await else {
        return;
    };
    // Nothing listens here.
    let dead = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let dead_addr = dead.local_addr().expect("addr");
    drop(dead);
    let ghost = register(
        &h,
        zone,
        "ghost",
        &dead_addr.to_string(),
        &key().public_key().key_data().clone(),
    )
    .await;

    let mut relay = TestRelay::start(&h).await;
    let id = register(
        &h,
        zone,
        "edge-1",
        &relay.addr.to_string(),
        &relay.host_key_data(),
    )
    .await;
    let sup = RelaySupervisor::new(h.app.clone(), RunnerHost::new().expect("host"), opts());
    sup.sync().await.expect("sync");

    let live = wait_live(&h, ghost, |l| l.state == RelayState::Unreachable).await;
    assert!(
        live.last_error
            .as_deref()
            .is_some_and(|e| e.starts_with("dial ")),
        "{live:?}"
    );

    let (handle, responder) = relay.next_link().await.answer(relay.fingerprint());
    wait_live(&h, id, |l| l.state == RelayState::Connected).await;

    // The relay goes away: unreachable, then Keel redials and it is connected again.
    responder.abort();
    let _ = handle
        .disconnect(
            russh::Disconnect::ByApplication,
            "bye".into(),
            String::new(),
        )
        .await;
    wait_live(&h, id, |l| l.state == RelayState::Unreachable).await;
    let (_handle, _responder) = relay.next_link().await.answer(relay.fingerprint());
    wait_live(&h, id, |l| l.state == RelayState::Connected).await;

    // Revoking stops supervision.
    sqlx::query("update relays set revoked_at = now() where id = $1")
        .bind(ghost)
        .execute(&h.db.owner)
        .await
        .expect("revoke");
    sup.sync().await.expect("sync");
    assert!(h.app.relay_live.get(ghost).is_none());

    let states: Vec<(serde_json::Value,)> = sqlx::query_as(
        "select context from audit_event where action = 'relay.state' \
         and target->>'relay' = $1 order by seq",
    )
    .bind(id.to_string())
    .fetch_all(&h.db.owner)
    .await
    .expect("audit");
    let states: Vec<&str> = states
        .iter()
        .map(|s| s.0["state"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(states, ["connected", "unreachable", "connected"]);
    sup.shutdown();
    h.finish().await;
}
