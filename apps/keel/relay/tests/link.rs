//! Spec 067 T016: the relay in-process, with ephemeral ports. The test plays Keel (an SSH client
//! with keel-exec's `LinkClientPolicy`, or one that never answers a channel open) and the runners
//! (plain TCP clients).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use keel_exec::relay::{
    self, Frame, LINK_USER, LinkClientPolicy, RELAY_SUBSYSTEM, RunnerChannel, link_client_config,
};
use keel_relay::config::Config;
use keel_relay::{Limits, Relay};
use russh::client::{self, AuthResult, Handle};
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, PrivateKey, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{Channel, ChannelStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

const WAIT: Duration = Duration::from_secs(10);

fn key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()
}

fn data(k: &PrivateKey) -> KeyData {
    k.public_key().key_data().clone()
}

fn config(max_streams: u32) -> Config {
    Config {
        link_listen: "127.0.0.1:0".parse().unwrap(),
        runner_listen: "127.0.0.1:0".parse().unwrap(),
        hub_keys: "/unused/hub_keys".into(),
        http_listen: None,
        max_streams,
    }
}

struct Running {
    link: SocketAddr,
    runner: SocketAddr,
    host: PrivateKey,
    hub: PrivateKey,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn start(limits: Limits) -> Running {
    let host = key();
    let hub = key();
    let relay = Relay::bind(&config(limits.max_streams), host.clone(), vec![data(&hub)])
        .await
        .unwrap()
        .with_limits(limits);
    let (link, runner) = (relay.link_addr(), relay.runner_addr());
    assert_ne!(link.port(), 0);
    assert_ne!(runner.port(), 0);
    let task = tokio::spawn(relay.run());
    Running {
        link,
        runner,
        host,
        hub,
        task,
    }
}

fn limits() -> Limits {
    Limits::default()
}

/// Keel's end of a link: the SSH handle and the control stream, `Hello` already read.
struct Keel<H: client::Handler> {
    /// Held so the link stays up: dropping the handle disconnects.
    _handle: Handle<H>,
    control: ChannelStream<client::Msg>,
    hello: Frame,
}

async fn dial<H>(relay: &Running, handler: H) -> Keel<H>
where
    H: client::Handler<Error = russh::Error> + Send + 'static,
{
    let mut handle = tokio::time::timeout(
        WAIT,
        client::connect(Arc::new(link_client_config()), relay.link, handler),
    )
    .await
    .expect("handshake hung")
    .expect("handshake");
    let key = PrivateKeyWithHashAlg::new(Arc::new(relay.hub.clone()), None);
    assert!(matches!(
        handle.authenticate_publickey(LINK_USER, key).await.unwrap(),
        AuthResult::Success
    ));
    let channel = handle.channel_open_session().await.unwrap();
    channel
        .request_subsystem(true, RELAY_SUBSYSTEM)
        .await
        .unwrap();
    let mut control = channel.into_stream();
    let hello = tokio::time::timeout(WAIT, relay::read_frame(&mut control))
        .await
        .expect("no Hello")
        .unwrap()
        .expect("control closed before Hello");
    Keel {
        _handle: handle,
        control,
        hello,
    }
}

/// Keel with keel-exec's real policy; carried runner channels arrive on the receiver.
async fn keel(
    relay: &Running,
) -> (
    Keel<LinkClientPolicy>,
    mpsc::UnboundedReceiver<RunnerChannel>,
) {
    let (runners_tx, runners) = mpsc::unbounded_channel();
    let violation: relay::ViolationSink = Arc::new(|v| panic!("violation {v:?}"));
    let policy = LinkClientPolicy::new(data(&relay.host), violation, runners_tx);
    (dial(relay, policy).await, runners)
}

/// A Keel that never answers a runner channel open (it holds the reply handle).
#[derive(Default)]
struct Stalling {
    held: Vec<client::ChannelOpenHandle>,
}

impl client::Handler for Stalling {
    type Error = russh::Error;

    async fn check_server_key(&mut self, _: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        _channel: Channel<client::Msg>,
        _connected_address: &str,
        _connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        self.held.push(reply);
        Ok(())
    }
}

async fn next_runner(runners: &mut mpsc::UnboundedReceiver<RunnerChannel>) -> RunnerChannel {
    tokio::time::timeout(WAIT, runners.recv())
        .await
        .expect("no runner channel")
        .expect("Keel stopped")
}

async fn no_runner(runners: &mut mpsc::UnboundedReceiver<RunnerChannel>) {
    if let Ok(Some(r)) = tokio::time::timeout(Duration::from_millis(500), runners.recv()).await {
        panic!("unexpected runner channel from {}", r.originator_address);
    }
}

/// The relay closes `tcp`: a read sees end of stream or a reset.
async fn closed(tcp: &mut TcpStream) {
    let mut buf = [0u8; 64];
    match tokio::time::timeout(WAIT, tcp.read(&mut buf))
        .await
        .expect("the relay kept the runner connection open")
    {
        Ok(0) | Err(_) => {}
        Ok(n) => panic!("relay sent {n} bytes to a runner"),
    }
}

async fn ping(control: &mut ChannelStream<client::Msg>, seq: u64) -> Frame {
    relay::write_frame(control, &Frame::Ping { seq })
        .await
        .unwrap();
    tokio::time::timeout(WAIT, relay::read_frame(control))
        .await
        .expect("no Pong")
        .unwrap()
        .expect("control closed")
}

fn pattern(len: usize, seed: u32) -> Vec<u8> {
    let mut x = seed;
    (0..len)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 24) as u8
        })
        .collect()
}

#[tokio::test]
async fn without_a_link_a_runner_is_accepted_and_closed_at_once() {
    let relay = start(limits()).await;
    let mut tcp = TcpStream::connect(relay.runner).await.unwrap();
    let started = Instant::now();
    closed(&mut tcp).await;
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn hello_comes_first_then_a_pong_for_each_ping() {
    let relay = start(Limits {
        max_streams: 7,
        ..limits()
    })
    .await;
    let (mut keel, _runners) = keel(&relay).await;
    let Frame::Hello {
        version,
        relay_key_fingerprint,
        max_streams,
        http_cache,
    } = keel.hello.clone()
    else {
        panic!("first frame {:?}", keel.hello);
    };
    assert_eq!(version, env!("CARGO_PKG_VERSION"));
    assert_eq!(
        relay_key_fingerprint,
        keel_relay::key::public_lines(&relay.host)[1]
    );
    assert_eq!(max_streams, 7);
    assert!(!http_cache);
    for seq in [1, 2, u64::MAX] {
        let Frame::Pong {
            seq: got, streams, ..
        } = ping(&mut keel.control, seq).await
        else {
            panic!("not a Pong");
        };
        assert_eq!((got, streams), (seq, 0));
    }
}

#[tokio::test]
async fn with_a_link_bytes_are_copied_both_ways_unchanged() {
    let relay = start(limits()).await;
    let (mut keel, mut runners) = keel(&relay).await;
    let mut tcp = TcpStream::connect(relay.runner).await.unwrap();
    let local = tcp.local_addr().unwrap();
    let carried = next_runner(&mut runners).await;
    assert_eq!(carried.originator_address, local.ip().to_string());
    assert_eq!(carried.originator_port, u32::from(local.port()));
    let Frame::Pong { streams, .. } = ping(&mut keel.control, 1).await else {
        panic!("not a Pong");
    };
    assert_eq!(streams, 1);

    let up = pattern(300 * 1024, 1);
    let down = pattern(200 * 1024, 2);
    let mut chan = carried.channel.into_stream();
    let (mut tcp_r, mut tcp_w) = tcp.split();
    let (mut chan_r, mut chan_w) = tokio::io::split(&mut chan);
    let (up2, down2) = (up.clone(), down.clone());
    let runner_side = async {
        tcp_w.write_all(&up2).await.unwrap();
        tcp_w.shutdown().await.unwrap();
        let mut got = Vec::new();
        tcp_r.read_to_end(&mut got).await.unwrap();
        got
    };
    let keel_side = async {
        chan_w.write_all(&down2).await.unwrap();
        chan_w.shutdown().await.unwrap();
        let mut got = Vec::new();
        chan_r.read_to_end(&mut got).await.unwrap();
        got
    };
    let (at_runner, at_keel) =
        tokio::time::timeout(WAIT, async { tokio::join!(runner_side, keel_side) })
            .await
            .expect("copy hung");
    assert!(at_keel == up, "runner → Keel bytes changed");
    assert!(at_runner == down, "Keel → runner bytes changed");
    drop(chan);
    drop(tcp);

    // The stream is gone once both ends closed.
    let deadline = Instant::now() + WAIT;
    loop {
        let Frame::Pong { streams, .. } = ping(&mut keel.control, 2).await else {
            panic!("not a Pong");
        };
        if streams == 0 {
            break;
        }
        assert!(Instant::now() < deadline, "stream still counted");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_channel_keel_does_not_accept_in_time_closes_the_runner() {
    let relay = start(Limits {
        channel_accept_timeout: Duration::from_millis(400),
        ..limits()
    })
    .await;
    let _keel = dial(&relay, Stalling::default()).await;
    let mut tcp = TcpStream::connect(relay.runner).await.unwrap();
    let started = Instant::now();
    closed(&mut tcp).await;
    assert!(
        started.elapsed() >= Duration::from_millis(350),
        "closed after {:?}, before the accept timeout",
        started.elapsed()
    );
}

#[test]
fn the_default_limits_are_the_contracts() {
    let l = Limits::default();
    assert_eq!(l.max_streams, 256);
    assert_eq!(l.max_pending_accepts, 64);
    assert_eq!(l.channel_accept_timeout, Duration::from_secs(20));
    assert_eq!(Limits::from_config(&config(9)).max_streams, 9);
}

#[tokio::test]
async fn stream_max_streams_plus_one_is_refused() {
    let relay = start(Limits {
        max_streams: 2,
        ..limits()
    })
    .await;
    let (_keel, mut runners) = keel(&relay).await;
    let first = TcpStream::connect(relay.runner).await.unwrap();
    let first_chan = next_runner(&mut runners).await;
    let _second = TcpStream::connect(relay.runner).await.unwrap();
    let _second_chan = next_runner(&mut runners).await;

    let mut third = TcpStream::connect(relay.runner).await.unwrap();
    closed(&mut third).await;
    no_runner(&mut runners).await;

    // A finished stream frees its slot. (Closing only the runner side half-closes the stream: the
    // relay waits for Keel's end too.)
    drop(first);
    first_chan.channel.close().await.unwrap();
    let deadline = Instant::now() + WAIT;
    loop {
        let _next = TcpStream::connect(relay.runner).await.unwrap();
        if let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(300), runners.recv()).await
        {
            break;
        }
        assert!(Instant::now() < deadline, "the slot was never freed");
    }
}

#[tokio::test]
async fn pending_accepts_are_capped() {
    let relay = start(Limits {
        max_pending_accepts: 1,
        channel_accept_timeout: Duration::from_secs(30),
        ..limits()
    })
    .await;
    let _keel = dial(&relay, Stalling::default()).await;
    let _pending = TcpStream::connect(relay.runner).await.unwrap();
    // Let the relay start opening the first channel.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut refused = TcpStream::connect(relay.runner).await.unwrap();
    let started = Instant::now();
    closed(&mut refused).await;
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn a_new_link_replaces_the_old_one_and_closes_its_streams() {
    let relay = start(limits()).await;
    let (mut old, mut old_runners) = keel(&relay).await;
    let mut tcp = TcpStream::connect(relay.runner).await.unwrap();
    let _carried = next_runner(&mut old_runners).await;

    let (_new, mut new_runners) = keel(&relay).await;
    closed(&mut tcp).await;
    // The old control channel ends.
    let ended = tokio::time::timeout(WAIT, relay::read_frame(&mut old.control))
        .await
        .expect("old link kept open");
    assert!(matches!(ended, Ok(None) | Err(_)), "{ended:?}");

    let _next = TcpStream::connect(relay.runner).await.unwrap();
    let _carried = next_runner(&mut new_runners).await;
    no_runner(&mut old_runners).await;
}

#[tokio::test]
async fn close_from_keel_closes_every_stream() {
    let relay = start(limits()).await;
    let (mut keel, mut runners) = keel(&relay).await;
    let mut a = TcpStream::connect(relay.runner).await.unwrap();
    let _a = next_runner(&mut runners).await;
    let mut b = TcpStream::connect(relay.runner).await.unwrap();
    let _b = next_runner(&mut runners).await;
    relay::write_frame(
        &mut keel.control,
        &Frame::Close {
            reason: "revoked".into(),
        },
    )
    .await
    .unwrap();
    closed(&mut a).await;
    closed(&mut b).await;
    // With no link, new runners are turned away again.
    let mut c = TcpStream::connect(relay.runner).await.unwrap();
    closed(&mut c).await;
}

#[tokio::test]
async fn a_frame_keel_never_sends_ends_the_link() {
    let relay = start(limits()).await;
    let (mut keel, _runners) = keel(&relay).await;
    let mut tcp = TcpStream::connect(relay.runner).await.unwrap();
    // `Pong` only ever goes relay → Keel.
    relay::write_frame(
        &mut keel.control,
        &Frame::Pong {
            seq: 1,
            version: "x".into(),
            streams: 0,
            uptime_s: 0,
        },
    )
    .await
    .unwrap();
    let ended = tokio::time::timeout(WAIT, relay::read_frame(&mut keel.control))
        .await
        .expect("link kept open");
    assert!(matches!(ended, Ok(None) | Err(_)), "{ended:?}");
    closed(&mut tcp).await;
}

#[tokio::test]
async fn a_link_that_never_opens_the_control_channel_is_dropped() {
    let relay = start(Limits {
        link_setup_timeout: Duration::from_millis(300),
        ..limits()
    })
    .await;
    let handle = client::connect(
        Arc::new(link_client_config()),
        relay.link,
        Stalling::default(),
    )
    .await
    .unwrap();
    let deadline = Instant::now() + WAIT;
    while !handle.is_closed() {
        assert!(Instant::now() < deadline, "unauthenticated link kept open");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_key_outside_hub_keys_is_refused() {
    let relay = start(limits()).await;
    let mut handle = client::connect(
        Arc::new(link_client_config()),
        relay.link,
        Stalling::default(),
    )
    .await
    .unwrap();
    let stranger = PrivateKeyWithHashAlg::new(Arc::new(key()), None);
    assert!(!matches!(
        handle
            .authenticate_publickey(LINK_USER, stranger)
            .await
            .unwrap(),
        AuthResult::Success
    ));
}
