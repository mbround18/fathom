//! Spec 067 T007: the link policies over an in-process russh pair. The relay end is
//! `RelayServerPolicy` and the Keel end is `LinkClientPolicy`, joined by a duplex pipe.

use std::sync::Arc;
use std::time::Duration;

use keel_exec::relay::{
    self, LinkClientPolicy, LinkRefusal, LinkViolation, RELAY_SUBSYSTEM, RelayServerPolicy,
    RunnerChannel, link_client_config, relay_server_config,
};
use russh::client::{self, AuthResult, Handle};
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, PrivateKey, PrivateKeyWithHashAlg};
use russh::{ChannelMsg, ChannelStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

const WAIT: Duration = Duration::from_secs(10);

fn key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()
}

fn data(k: &PrivateKey) -> KeyData {
    k.public_key().key_data().clone()
}

type Sink<T> = Arc<dyn Fn(T) + Send + Sync>;

fn sink<T: Send + 'static>() -> (Sink<T>, mpsc::UnboundedReceiver<T>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let sink: Sink<T> = Arc::new(move |r| {
        let _ = tx.send(r);
    });
    (sink, rx)
}

struct Link {
    keel: Option<Handle<LinkClientPolicy>>,
    keel_err: Option<russh::Error>,
    relay: Option<russh::server::Handle>,
    relay_refused: mpsc::UnboundedReceiver<LinkRefusal>,
    relay_violations: mpsc::UnboundedReceiver<LinkViolation>,
    keel_violations: mpsc::UnboundedReceiver<LinkViolation>,
    control: oneshot::Receiver<ChannelStream<russh::server::Msg>>,
    runners: mpsc::UnboundedReceiver<RunnerChannel>,
}

impl Link {
    fn keel(&mut self) -> &mut Handle<LinkClientPolicy> {
        self.keel.as_mut().expect("handshake")
    }
}

/// A relay holding `host_key` and trusting `hub_keys`, dialled by Keel pinning `pinned`.
async fn link_with(host_key: PrivateKey, hub_keys: Vec<KeyData>, pinned: KeyData) -> Link {
    let (relay_io, keel_io) = tokio::io::duplex(256 * 1024);
    let (relay_sink, relay_refused) = sink();
    let (relay_violation_sink, relay_violations) = sink();
    let (keel_sink, keel_violations) = sink();
    let (control_tx, control) = oneshot::channel();
    let (runners_tx, runners) = mpsc::unbounded_channel();

    let mut config = relay_server_config(host_key);
    config.auth_rejection_time = Duration::from_millis(1);
    let policy = RelayServerPolicy::new(hub_keys, relay_sink, relay_violation_sink, control_tx);
    let (handle_tx, handle_rx) = oneshot::channel();
    tokio::spawn(async move {
        if let Ok(session) = russh::server::run_stream(Arc::new(config), relay_io, policy).await {
            let _ = handle_tx.send(session.handle());
            let _ = session.await;
        }
    });

    let policy = LinkClientPolicy::new(pinned, keel_sink, runners_tx);
    let connected = tokio::time::timeout(
        WAIT,
        client::connect_stream(Arc::new(link_client_config()), keel_io, policy),
    )
    .await
    .expect("handshake hung");
    let (keel, keel_err) = match connected {
        Ok(h) => (Some(h), None),
        Err(e) => (None, Some(e)),
    };
    // `None` when the handshake failed before the relay's session started.
    let relay = tokio::time::timeout(WAIT, handle_rx)
        .await
        .ok()
        .and_then(Result::ok);
    Link {
        keel,
        keel_err,
        relay,
        relay_refused,
        relay_violations,
        keel_violations,
        control,
        runners,
    }
}

/// A relay and Keel with matching keys, Keel logged in as `keel`.
async fn linked() -> Link {
    let host = key();
    let hub = key();
    let mut link = link_with(host.clone(), vec![data(&hub)], data(&host)).await;
    assert!(auth(link.keel(), relay::LINK_USER, &hub).await);
    link
}

async fn auth(handle: &mut Handle<LinkClientPolicy>, user: &str, key: &PrivateKey) -> bool {
    let key = PrivateKeyWithHashAlg::new(Arc::new(key.clone()), None);
    matches!(
        handle.authenticate_publickey(user, key).await.unwrap(),
        AuthResult::Success
    )
}

async fn reply(channel: &mut russh::Channel<client::Msg>) -> ChannelMsg {
    loop {
        match tokio::time::timeout(WAIT, channel.wait())
            .await
            .expect("no reply")
        {
            Some(ChannelMsg::Success) => return ChannelMsg::Success,
            Some(ChannelMsg::Failure) => return ChannelMsg::Failure,
            Some(ChannelMsg::WindowAdjusted { .. }) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}

async fn refused<T>(rx: &mut mpsc::UnboundedReceiver<T>) -> T {
    tokio::time::timeout(WAIT, rx.recv())
        .await
        .expect("no refusal reported")
        .expect("sink closed")
}

#[tokio::test]
async fn keel_opens_the_control_subsystem_and_frames_flow() {
    let mut link = linked().await;
    let mut channel = link.keel().channel_open_session().await.unwrap();
    channel
        .request_subsystem(true, RELAY_SUBSYSTEM)
        .await
        .unwrap();
    assert!(matches!(reply(&mut channel).await, ChannelMsg::Success));

    let mut relay_end = tokio::time::timeout(WAIT, link.control)
        .await
        .unwrap()
        .expect("control stream handed to the relay");
    let mut keel_end = channel.into_stream();
    let hello = relay::Frame::Hello {
        version: "0.1.0".into(),
        relay_key_fingerprint: "SHA256:x".into(),
        max_streams: 256,
        http_cache: false,
    };
    relay::write_frame(&mut relay_end, &hello).await.unwrap();
    assert_eq!(relay::read_frame(&mut keel_end).await.unwrap(), Some(hello));
    relay::write_frame(&mut keel_end, &relay::Frame::Ping { seq: 1 })
        .await
        .unwrap();
    assert_eq!(
        relay::read_frame(&mut relay_end).await.unwrap(),
        Some(relay::Frame::Ping { seq: 1 })
    );
}

#[tokio::test]
async fn only_user_keel_with_a_hub_key_logs_in() {
    let host = key();
    let hub = key();
    let mut link = link_with(host.clone(), vec![data(&hub)], data(&host)).await;

    assert!(!auth(link.keel(), "root", &hub).await, "another user");
    assert!(matches!(
        refused(&mut link.relay_refused).await,
        LinkRefusal::Auth { user, .. } if user == "root"
    ));
    let stranger = key();
    assert!(!auth(link.keel(), "keel", &stranger).await, "another key");
    let expected = keel_exec::ca::fingerprint(&data(&stranger));
    assert!(matches!(
        refused(&mut link.relay_refused).await,
        LinkRefusal::Auth { user, fingerprint } if user == "keel" && fingerprint == expected
    ));
    assert!(auth(link.keel(), "keel", &hub).await, "the hub key");
    assert!(
        link.relay_violations.try_recv().is_err(),
        "a failed login is an ordinary refusal, not a violation"
    );
}

#[tokio::test]
async fn an_empty_hub_key_list_refuses_everyone() {
    let host = key();
    let hub = key();
    let mut link = link_with(host.clone(), vec![], data(&host)).await;
    assert!(!auth(link.keel(), "keel", &hub).await);
}

#[tokio::test]
async fn exec_shell_pty_env_agent_x11_and_other_subsystems_are_refused() {
    let mut link = linked().await;
    let mut channel = link.keel().channel_open_session().await.unwrap();

    channel
        .request_pty(true, "xterm", 80, 24, 0, 0, &[])
        .await
        .unwrap();
    assert!(
        matches!(reply(&mut channel).await, ChannelMsg::Failure),
        "pty"
    );
    channel.request_shell(true).await.unwrap();
    assert!(
        matches!(reply(&mut channel).await, ChannelMsg::Failure),
        "shell"
    );
    channel.exec(true, "id").await.unwrap();
    assert!(
        matches!(reply(&mut channel).await, ChannelMsg::Failure),
        "exec"
    );
    channel
        .set_env(true, "LD_PRELOAD", "/tmp/x.so")
        .await
        .unwrap();
    assert!(
        matches!(reply(&mut channel).await, ChannelMsg::Failure),
        "env"
    );
    channel.agent_forward(true).await.unwrap();
    assert!(
        matches!(reply(&mut channel).await, ChannelMsg::Failure),
        "agent"
    );
    channel
        .request_x11(true, false, "MIT-MAGIC-COOKIE-1", "00", 0)
        .await
        .unwrap();
    assert!(
        matches!(reply(&mut channel).await, ChannelMsg::Failure),
        "x11"
    );
    channel.request_subsystem(true, "sftp").await.unwrap();
    assert!(
        matches!(reply(&mut channel).await, ChannelMsg::Failure),
        "sftp"
    );

    let mut kinds = Vec::new();
    for _ in 0..7 {
        match refused(&mut link.relay_violations).await {
            LinkViolation::Request { kind, detail } => {
                assert!(
                    !detail.contains("/tmp/x.so"),
                    "env values are never reported"
                );
                kinds.push(kind);
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(
        kinds,
        [
            "pty",
            "shell",
            "exec",
            "env",
            "auth-agent",
            "x11",
            "subsystem"
        ]
    );

    // The same channel still carries the control subsystem afterwards.
    channel
        .request_subsystem(true, RELAY_SUBSYSTEM)
        .await
        .unwrap();
    assert!(matches!(reply(&mut channel).await, ChannelMsg::Success));
}

#[tokio::test]
async fn forwarding_and_other_channel_types_from_keel_are_refused() {
    let mut link = linked().await;
    let keel = link.keel.as_ref().unwrap();
    assert!(
        keel.channel_open_direct_tcpip("10.0.0.5", 22, "127.0.0.1", 40000)
            .await
            .is_err(),
        "direct-tcpip"
    );
    assert!(
        keel.channel_open_direct_streamlocal("/run/docker.sock")
            .await
            .is_err(),
        "streamlocal"
    );
    assert!(
        keel.channel_open_x11("127.0.0.1", 6000).await.is_err(),
        "x11"
    );
    assert!(
        keel.tcpip_forward("0.0.0.0", 0).await.is_err(),
        "tcpip-forward"
    );
    assert!(
        keel.streamlocal_forward("/tmp/k.sock").await.is_err(),
        "streamlocal-forward"
    );
    let mut kinds = Vec::new();
    for _ in 0..5 {
        match refused(&mut link.relay_violations).await {
            LinkViolation::Channel { kind, .. } | LinkViolation::Request { kind, .. } => {
                kinds.push(kind)
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(
        kinds,
        [
            "direct-tcpip",
            "direct-streamlocal",
            "x11",
            "tcpip-forward",
            "streamlocal-forward"
        ]
    );
}

#[tokio::test]
async fn a_second_session_is_refused_and_closes_the_link() {
    let mut link = linked().await;
    let mut first = link.keel().channel_open_session().await.unwrap();
    first
        .request_subsystem(true, RELAY_SUBSYSTEM)
        .await
        .unwrap();
    assert!(matches!(reply(&mut first).await, ChannelMsg::Success));

    assert!(link.keel().channel_open_session().await.is_err());
    assert_eq!(
        refused(&mut link.relay_violations).await,
        LinkViolation::SecondSession
    );
    tokio::time::timeout(WAIT, async {
        while !link.keel().is_closed() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the link stays up after a second session");
}

#[tokio::test]
async fn keel_accepts_runner_channels_and_bytes_flow() {
    let mut link = linked().await;
    let mut relay_end = link
        .relay
        .clone()
        .unwrap()
        .channel_open_forwarded_tcpip("keel-runner", 0, "10.0.0.5", 40122)
        .await
        .expect("runner channel accepted")
        .into_stream();
    let runner = tokio::time::timeout(WAIT, link.runners.recv())
        .await
        .unwrap()
        .expect("runner channel handed to Keel");
    assert_eq!(runner.originator_address, "10.0.0.5");
    assert_eq!(runner.originator_port, 40122);
    let mut keel_end = runner.channel.into_stream();

    relay_end
        .write_all(b"SSH-2.0-keel-runner\r\n")
        .await
        .unwrap();
    let mut buf = [0u8; 21];
    keel_end.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"SSH-2.0-keel-runner\r\n");
    keel_end.write_all(b"pong").await.unwrap();
    let mut buf = [0u8; 4];
    relay_end.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"pong");
}

#[tokio::test]
async fn keel_refuses_every_other_channel_from_the_relay() {
    let mut link = linked().await;
    let relay = link.relay.clone().unwrap();
    assert!(
        relay
            .channel_open_forwarded_tcpip("10.0.0.9", 22, "10.0.0.5", 40122)
            .await
            .is_err(),
        "forwarded-tcpip to another address"
    );
    assert!(
        relay
            .channel_open_forwarded_tcpip("keel-runner", 22, "10.0.0.5", 40122)
            .await
            .is_err(),
        "forwarded-tcpip to another port"
    );
    assert!(relay.channel_open_session().await.is_err(), "session");
    assert!(
        relay
            .channel_open_direct_tcpip("127.0.0.1", 5432, "10.0.0.5", 1)
            .await
            .is_err(),
        "direct-tcpip"
    );
    assert!(
        relay
            .channel_open_direct_streamlocal("/run/postgresql/.s.PGSQL.5432")
            .await
            .is_err(),
        "direct-streamlocal"
    );
    assert!(
        relay
            .channel_open_forwarded_streamlocal("/tmp/x")
            .await
            .is_err(),
        "forwarded-streamlocal"
    );
    assert!(
        relay.channel_open_x11("10.0.0.5", 6000).await.is_err(),
        "x11"
    );
    assert!(relay.channel_open_agent().await.is_err(), "agent");

    let mut kinds = Vec::new();
    for _ in 0..8 {
        match refused(&mut link.keel_violations).await {
            LinkViolation::Channel { kind, .. } => kinds.push(kind),
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(
        kinds,
        [
            "forwarded-tcpip",
            "forwarded-tcpip",
            "session",
            "direct-tcpip",
            "direct-streamlocal",
            "forwarded-streamlocal",
            "x11",
            "auth-agent"
        ]
    );
    assert!(link.runners.try_recv().is_err(), "nothing reached Keel");
}

#[tokio::test]
async fn keel_refuses_a_relay_host_key_other_than_the_pinned_one() {
    let host = key();
    let other = key();
    let hub = key();
    let mut link = link_with(host.clone(), vec![data(&hub)], data(&other)).await;
    assert!(link.keel.is_none(), "handshake must fail");
    assert!(link.keel_err.is_some());
    assert_eq!(
        refused(&mut link.keel_violations).await,
        LinkViolation::HostKey {
            fingerprint: keel_exec::ca::fingerprint(&data(&host))
        }
    );
}

#[tokio::test]
async fn bad_control_frames_are_violations_but_a_hang_up_is_not() {
    // Over the cap: refused from the length prefix, before any body is read or allocated.
    let (mut a, mut b) = tokio::io::duplex(1024);
    a.write_all(&(relay::MAX_CONTROL_FRAME_LEN as u32 + 1).to_be_bytes())
        .await
        .unwrap();
    let err = relay::read_frame(&mut b).await.unwrap_err();
    assert!(matches!(
        LinkViolation::frame(&err),
        Some(LinkViolation::Frame { .. })
    ));

    let err = relay::decode_body(br#"{"t":"Exec","cmd":"id"}"#).unwrap_err();
    assert_eq!(
        LinkViolation::frame(&err).map(|v| v.reason()),
        Some("bad_frame")
    );

    let (mut a, mut b) = tokio::io::duplex(1024);
    a.write_all(&[0, 0]).await.unwrap();
    drop(a);
    let err = relay::read_frame(&mut b).await.unwrap_err();
    assert_eq!(LinkViolation::frame(&err), None, "truncated is a hang-up");

    assert_eq!(
        LinkViolation::out_of_order("Pong before Hello"),
        LinkViolation::Frame {
            detail: "out of order: Pong before Hello".into()
        }
    );
}
