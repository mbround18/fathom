//! Shared harness for the relay tests that need a misbehaving or scripted relay (spec 067 US2):
//! `relays.rs` (revoke and release) and `hostile_relay.rs` (the SC-002 matrix and Keel's own
//! limits). The relay is an in-test one built from `keel_exec::relay`'s server policy, so a test
//! can open any channel or send any bytes a compromised relay host could.

#![allow(dead_code)]

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
use keel_server::relays::{self, RelayLive, RelayRegistration, RelayState, SupervisorOptions};
use keel_server::runner_listener::{EnrollRequest, enroll_key};
use russh::client::{self, AuthResult, Handle};
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, Certificate, PrivateKey};
use russh::{ChannelMsg, ChannelStream};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::repos::Harness;

pub const WAIT: Duration = Duration::from_secs(10);

pub fn key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("key")
}

pub fn opts() -> SupervisorOptions {
    SupervisorOptions {
        health_interval: Duration::from_millis(200),
        hello_timeout: Duration::from_secs(5),
        ..SupervisorOptions::default()
    }
}

/// A harness with the SSH CAs, the relay-link key and a zone `edge`.
pub async fn harness(test: &str) -> Option<(Harness, Uuid)> {
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
pub async fn register(h: &Harness, zone: Uuid, name: &str, addr: &str, host_key: &KeyData) -> Uuid {
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
pub struct RelaySide {
    pub handle: russh::server::Handle,
    pub control: ChannelStream<russh::server::Msg>,
}

/// A relay answering on its control channel ([`RelaySide::answer`]).
pub struct Answered {
    pub handle: russh::server::Handle,
    /// Raw bytes written to the control channel as they are: a hostile relay's frames.
    pub inject: mpsc::UnboundedSender<Vec<u8>>,
    /// Every frame Keel sent other than `Ping`.
    pub seen: mpsc::UnboundedReceiver<Frame>,
    pub task: JoinHandle<()>,
}

impl RelaySide {
    /// Sends `Hello` (naming `fingerprint`), answers every `Ping` with a `Pong`, writes what
    /// arrives on `inject`, and reports Keel's other frames on `seen`.
    pub fn answer(self, fingerprint: String) -> Answered {
        let (mut rd, mut wr) = tokio::io::split(self.control);
        let (inject, mut injected) = mpsc::unbounded_channel::<Vec<u8>>();
        let (seen_tx, seen) = mpsc::unbounded_channel();
        let out = inject.clone();
        let task = tokio::spawn(async move {
            let writer = tokio::spawn(async move {
                while let Some(bytes) = injected.recv().await {
                    if wr.write_all(&bytes).await.is_err() || wr.flush().await.is_err() {
                        return;
                    }
                }
            });
            let hello = Frame::Hello {
                version: "0.9.0-test".into(),
                relay_key_fingerprint: fingerprint,
                max_streams: 256,
                http_cache: false,
            };
            let _ = out.send(relay::encode(&hello).expect("hello"));
            while let Ok(Some(frame)) = relay::read_frame(&mut rd).await {
                if let Frame::Ping { seq } = frame {
                    let pong = Frame::Pong {
                        seq,
                        version: "0.9.0-test".into(),
                        streams: 0,
                        uptime_s: 1,
                    };
                    let _ = out.send(relay::encode(&pong).expect("pong"));
                } else {
                    let _ = seen_tx.send(frame);
                }
            }
            writer.abort();
        });
        Answered {
            handle: self.handle,
            inject,
            seen,
            task,
        }
    }
}

/// A relay listening on TCP, trusting Keel's relay-link key. Each link whose control channel
/// opens arrives on `links`.
pub struct TestRelay {
    pub addr: SocketAddr,
    pub host_key: PrivateKey,
    pub links: mpsc::UnboundedReceiver<RelaySide>,
    task: JoinHandle<()>,
}

impl TestRelay {
    pub async fn start(h: &Harness) -> Self {
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

    pub fn host_key_data(&self) -> KeyData {
        self.host_key.public_key().key_data().clone()
    }

    pub fn fingerprint(&self) -> String {
        keel_exec::ca::fingerprint(&self.host_key_data())
    }

    pub async fn next_link(&mut self) -> RelaySide {
        tokio::time::timeout(WAIT, self.links.recv())
            .await
            .expect("Keel never linked")
            .expect("relay stopped")
    }

    /// Asserts Keel does not link again within a short grace period.
    pub async fn no_link(&mut self) {
        let again = tokio::time::timeout(Duration::from_millis(600), self.links.recv()).await;
        assert!(again.is_err(), "Keel dialled the relay again");
    }
}

impl Drop for TestRelay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Waits until relay `id`'s live state satisfies `ok`.
pub async fn wait_live(h: &Harness, id: Uuid, ok: impl Fn(&RelayLive) -> bool) -> RelayLive {
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

/// Waits until the relay's side of a channel sees it closed.
pub async fn closed(channel: &mut russh::Channel<russh::server::Msg>) {
    tokio::time::timeout(WAIT, async {
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Close) | None => return,
                Some(_) => {}
            }
        }
    })
    .await
    .expect("Keel kept the channel open");
}

/// The `relay.refused` contexts with `reason`.
pub async fn refused(h: &Harness, reason: &str) -> Vec<serde_json::Value> {
    let rows: Vec<(serde_json::Value,)> = sqlx::query_as(
        "select context from audit_event where action = 'relay.refused' and reason = $1",
    )
    .bind(reason)
    .fetch_all(&h.db.owner)
    .await
    .expect("audit");
    rows.into_iter().map(|r| r.0).collect()
}

/// The `relay.quarantine` rows: `(reason, context)`.
pub async fn quarantines(h: &Harness) -> Vec<(String, serde_json::Value)> {
    sqlx::query_as(
        "select reason, context from audit_event where action = 'relay.quarantine' order by seq",
    )
    .fetch_all(&h.db.owner)
    .await
    .expect("audit")
}

/// The relay row's quarantine reason.
pub async fn quarantine_reason(h: &Harness, id: Uuid) -> Option<String> {
    let (reason,): (Option<String>,) =
        sqlx::query_as("select quarantine_reason from relays where id = $1")
            .bind(id)
            .fetch_one(&h.db.owner)
            .await
            .expect("relay");
    reason
}

/// No run was started and no host has a runner session.
pub async fn no_run(h: &Harness) {
    let (runs,): (i64,) = sqlx::query_as("select count(*) from runs")
        .fetch_one(&h.db.owner)
        .await
        .expect("runs");
    assert_eq!(runs, 0, "a run was started");
    let hosts: Vec<(Uuid,)> = sqlx::query_as("select id from hosts")
        .fetch_all(&h.db.owner)
        .await
        .expect("hosts");
    for (host,) in hosts {
        assert!(h.app.runs.path(host).is_none(), "host {host} has a session");
    }
}

/// Mints an enrolment token for `name` (profile `debian`).
pub async fn mint(h: &Harness, name: &str) -> (Uuid, String) {
    sqlx::query(
        "insert into profiles (id, workspace_id, name) select $1, $2, 'debian' \
         where not exists (select 1 from profiles where workspace_id = $2 and name = 'debian')",
    )
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

/// A client verifying Keel's host certificate.
pub fn runner_client(h: &Harness) -> KeelSshClient {
    let cas = h.app.ssh.get().expect("cas");
    KeelSshClient::new(HostCertVerifier::new(
        vec![TrustedCa::active(cas.host.key_data().clone())],
        "keel.example.com",
    ))
}

/// The SSH handshake with Keel over a `forwarded-tcpip` channel the relay opens.
pub async fn connect_over(
    h: &Harness,
    handle: &russh::server::Handle,
    port: u32,
) -> Handle<KeelSshClient> {
    let channel = handle
        .channel_open_forwarded_tcpip("keel-runner", 0, "10.0.0.5", port)
        .await
        .expect("open runner channel");
    tokio::time::timeout(
        WAIT,
        client::connect_stream(
            Arc::new(client_config()),
            channel.into_stream(),
            runner_client(h),
        ),
    )
    .await
    .expect("handshake hung")
    .expect("handshake over the relay")
}

/// An enrolled runner's session with Keel over the relay: enrolled (`name`), authenticated with
/// its certificate, `Hello` sent, attached. Returns its host id, SSH session and runner stream.
pub async fn runner_session(
    h: &Harness,
    handle: &russh::server::Handle,
    name: &str,
    port: u32,
) -> (
    Uuid,
    Handle<KeelSshClient>,
    ChannelStream<russh::client::Msg>,
) {
    runner_session_with(h, handle, name, port, Vec::new()).await
}

/// [`runner_session`] whose `Hello` advertises `capabilities`.
pub async fn runner_session_with(
    h: &Harness,
    handle: &russh::server::Handle,
    name: &str,
    port: u32,
    capabilities: Vec<String>,
) -> (
    Uuid,
    Handle<KeelSshClient>,
    ChannelStream<russh::client::Msg>,
) {
    let (_, token) = mint(h, name).await;
    let runner = key();
    let allowed = vec!["debian".to_owned()];
    let (host_id, cert) = enroll_key(
        &h.app,
        runner.public_key().key_data(),
        &EnrollRequest {
            token: &token,
            inventory_name: name,
            allowed_profiles: &allowed,
            require_signed: false,
        },
    )
    .await
    .expect("enrol");
    let mut ssh = connect_over(h, handle, port).await;
    let cert = Certificate::from_openssh(&cert).expect("cert");
    assert!(matches!(
        ssh.authenticate_openssh_cert("runner", Arc::new(runner), cert)
            .await
            .expect("auth"),
        AuthResult::Success
    ));
    let stream = runner_hello_with(h, &ssh, host_id, capabilities).await;
    (host_id, ssh, stream)
}

/// Opens the runner subsystem, sends `Hello` and waits until Keel attached the session.
pub async fn runner_hello(
    h: &Harness,
    ssh: &Handle<KeelSshClient>,
    host_id: Uuid,
) -> ChannelStream<russh::client::Msg> {
    runner_hello_with(h, ssh, host_id, Vec::new()).await
}

/// [`runner_hello`] advertising `capabilities`.
pub async fn runner_hello_with(
    h: &Harness,
    ssh: &Handle<KeelSshClient>,
    host_id: Uuid,
    capabilities: Vec<String>,
) -> ChannelStream<russh::client::Msg> {
    let mut stream = open_subsystem(ssh, Subsystem::Runner)
        .await
        .expect("runner subsystem");
    write_frame(
        &mut stream,
        &frames::Frame::Hello {
            host_id: host_id.to_string(),
            version: "0.0.0".into(),
            ansible_core: "2.21.0".into(),
            python: "3.12.3".into(),
            allowed_profiles: vec!["debian".to_owned()],
            require_signed: false,
            override_quorum: 1,
            ledger_head: None,
            bases: Vec::new(),
            age_recipient: None,
            capabilities,
        },
    )
    .await
    .expect("hello");
    let deadline = tokio::time::Instant::now() + WAIT;
    while h.app.runs.path(host_id).is_none() {
        assert!(tokio::time::Instant::now() < deadline, "never attached");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    stream
}

/// Waits until `host` has no runner session.
pub async fn detached(h: &Harness, host: Uuid) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while h.app.runs.path(host).is_some() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the runner session outlived its relay"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Whether relay `id` is connected (live state).
pub fn connected(h: &Harness, id: Uuid) -> bool {
    h.app
        .relay_live
        .get(id)
        .is_some_and(|l| l.state == RelayState::Connected)
}
