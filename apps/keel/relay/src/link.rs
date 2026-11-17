//! The SSH server Keel dials and the `keel-relay-v1` control channel (spec 067 US1).
//!
//! Every connection to `link_listen` gets a [`RelayServerPolicy`]: user `keel`, a key from
//! `hub_keys`, one session channel carrying the control subsystem, nothing else. A connection that
//! has not opened its control channel within [`Limits::link_setup_timeout`] is dropped.
//!
//! One link at a time: a newly set-up link replaces the current one, which is disconnected with
//! every runner stream it carried. On the control channel the relay sends `Hello` first, then a
//! `Pong` for each `Ping`. `Close` from Keel ends the link and its streams. Any other frame, or a
//! malformed or oversized one, is a violation: logged, and the link is closed.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use keel_exec::relay::{
    self, Frame, LinkRefusal, LinkViolation, RefusalSink, RelayServerPolicy, ViolationSink,
};
use russh::keys::PrivateKey;
use russh::keys::ssh_key::public::KeyData;
use russh::{ChannelStream, Disconnect};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, oneshot, watch};

use crate::Limits;

/// The longest `Close.reason` from Keel that is logged.
const MAX_LOGGED_REASON: usize = 200;

/// State shared by the link server and the runner listener.
#[derive(Debug)]
pub(crate) struct Shared {
    current: Mutex<Option<Arc<Link>>>,
    next_id: AtomicU64,
    shutting_down: AtomicBool,
    /// Runner streams Keel accepted and the relay is copying (`Pong.streams`).
    pub(crate) streams: AtomicU32,
    started: Instant,
    fingerprint: String,
    max_streams: u32,
}

impl Shared {
    pub(crate) fn new(host_key: &PrivateKey, limits: &Limits) -> Self {
        let [_, fingerprint] = crate::key::public_lines(host_key);
        Self {
            current: Mutex::new(None),
            next_id: AtomicU64::new(1),
            shutting_down: AtomicBool::new(false),
            streams: AtomicU32::new(0),
            started: Instant::now(),
            fingerprint,
            max_streams: limits.max_streams,
        }
    }

    /// The link runner streams go to, if one is up.
    pub(crate) fn current(&self) -> Option<Arc<Link>> {
        self.lock().clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Arc<Link>>> {
        // The lock is never held across a panic-prone call; recover the value if it was.
        self.current.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Makes `link` current. Returns `false` (and installs nothing) while shutting down.
    fn install(&self, link: &Arc<Link>) -> bool {
        let old = {
            let mut current = self.lock();
            if self.shutting_down.load(Ordering::SeqCst) {
                return false;
            }
            current.replace(link.clone())
        };
        if let Some(old) = old {
            tracing::info!(
                old = old.id,
                new = link.id,
                peer = %old.peer,
                "link replaced by a newer one"
            );
            old.shut("replaced by a newer link");
        }
        true
    }

    /// Clears the current link if it is still `id`.
    fn remove(&self, id: u64) {
        let mut current = self.lock();
        if current.as_ref().is_some_and(|l| l.id == id) {
            *current = None;
        }
    }

    /// Stops installing links and closes the current one.
    pub(crate) fn shutdown(&self) {
        let old = {
            let mut current = self.lock();
            self.shutting_down.store(true, Ordering::SeqCst);
            current.take()
        };
        if let Some(old) = old {
            old.shut("relay shutting down");
        }
    }
}

/// One authenticated link with its control channel up.
#[derive(Debug)]
pub(crate) struct Link {
    pub(crate) id: u64,
    pub(crate) peer: SocketAddr,
    pub(crate) handle: russh::server::Handle,
    /// `true` once the link is closing; every runner stream on it watches this.
    closed: watch::Sender<bool>,
}

impl Link {
    /// A receiver that turns `true` when the link closes.
    pub(crate) fn closed(&self) -> watch::Receiver<bool> {
        self.closed.subscribe()
    }

    /// Closes every stream on the link and disconnects it. Idempotent.
    fn shut(&self, why: &str) {
        if self.closed.send_replace(true) {
            return;
        }
        let handle = self.handle.clone();
        let why = why.to_owned();
        tokio::spawn(async move {
            // An error means the session already ended.
            let _ = handle
                .disconnect(Disconnect::ByApplication, why, String::new())
                .await;
        });
    }
}

/// Accepts link connections until the listener fails.
pub(crate) async fn serve(
    listener: TcpListener,
    shared: Arc<Shared>,
    config: Arc<russh::server::Config>,
    hub_keys: Arc<Vec<KeyData>>,
    limits: Limits,
) -> std::io::Result<()> {
    let setups = Arc::new(Semaphore::new(limits.max_link_setups));
    loop {
        let (tcp, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                crate::forward::accept_error(&e).await?;
                continue;
            }
        };
        let Ok(permit) = setups.clone().try_acquire_owned() else {
            tracing::warn!(%peer, "link connection refused: too many in setup");
            continue;
        };
        let _ = tcp.set_nodelay(true);
        let (shared, config, hub_keys) = (shared.clone(), config.clone(), hub_keys.clone());
        let setup_timeout = limits.link_setup_timeout;
        tokio::spawn(async move {
            connection(tcp, peer, shared, config, hub_keys, setup_timeout, permit).await;
        });
    }
}

fn refusal_sink(peer: SocketAddr) -> RefusalSink {
    Arc::new(move |refusal: LinkRefusal| {
        let reason = refusal.reason();
        match refusal {
            LinkRefusal::Auth { user, fingerprint } => tracing::warn!(
                %peer,
                reason,
                user = %user.escape_debug(),
                %fingerprint,
                "link login refused"
            ),
        }
    })
}

fn violation_sink(peer: SocketAddr) -> ViolationSink {
    Arc::new(move |violation: LinkViolation| log_violation(peer, &violation))
}

fn log_violation(peer: SocketAddr, violation: &LinkViolation) {
    tracing::warn!(%peer, reason = violation.reason(), detail = ?violation, "link refused");
}

async fn connection(
    tcp: TcpStream,
    peer: SocketAddr,
    shared: Arc<Shared>,
    config: Arc<russh::server::Config>,
    hub_keys: Arc<Vec<KeyData>>,
    setup_timeout: std::time::Duration,
    setup_permit: tokio::sync::OwnedSemaphorePermit,
) {
    let deadline = tokio::time::Instant::now() + setup_timeout;
    let (control_tx, control_rx) = oneshot::channel();
    let policy = RelayServerPolicy::new(
        hub_keys.to_vec(),
        refusal_sink(peer),
        violation_sink(peer),
        control_tx,
    );
    let mut session =
        match tokio::time::timeout_at(deadline, russh::server::run_stream(config, tcp, policy))
            .await
        {
            Ok(Ok(session)) => session,
            Ok(Err(e)) => {
                tracing::info!(%peer, error = %e, "link handshake failed");
                return;
            }
            Err(_) => {
                tracing::info!(%peer, "link handshake timed out");
                return;
            }
        };
    let handle = session.handle();
    let control = match tokio::time::timeout_at(deadline, control_rx).await {
        Ok(Ok(control)) => control,
        outcome => {
            if outcome.is_err() {
                tracing::info!(%peer, "link dropped: no control channel in time");
            }
            let _ = handle
                .disconnect(
                    Disconnect::ByApplication,
                    "no control channel".into(),
                    String::new(),
                )
                .await;
            return;
        }
    };
    drop(setup_permit);

    let (closed, _) = watch::channel(false);
    let link = Arc::new(Link {
        id: shared.next_id.fetch_add(1, Ordering::Relaxed),
        peer,
        handle,
        closed,
    });
    if !shared.install(&link) {
        link.shut("relay shutting down");
        return;
    }
    tracing::info!(%peer, link = link.id, "link up");
    let mut closed = link.closed();
    let reason = tokio::select! {
        reason = control_loop(control, &shared, peer) => reason,
        _ = &mut session => "session ended".to_owned(),
        _ = closed.wait_for(|c| *c) => "closed".to_owned(),
    };
    shared.remove(link.id);
    link.shut(&reason);
    tracing::info!(%peer, link = link.id, %reason, "link down");
}

/// Runs the control channel; returns why it ended.
async fn control_loop(
    mut control: ChannelStream<russh::server::Msg>,
    shared: &Shared,
    peer: SocketAddr,
) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let hello = Frame::Hello {
        version: version.to_owned(),
        relay_key_fingerprint: shared.fingerprint.clone(),
        max_streams: shared.max_streams,
        // The release cache (`cache`) is not served yet.
        http_cache: false,
    };
    if let Err(e) = relay::write_frame(&mut control, &hello).await {
        return format!("control write failed: {e}");
    }
    loop {
        match relay::read_frame(&mut control).await {
            Ok(None) => return "control channel closed".to_owned(),
            Ok(Some(Frame::Ping { seq })) => {
                let pong = Frame::Pong {
                    seq,
                    version: version.to_owned(),
                    streams: shared.streams.load(Ordering::Relaxed),
                    uptime_s: shared.started.elapsed().as_secs(),
                };
                if let Err(e) = relay::write_frame(&mut control, &pong).await {
                    return format!("control write failed: {e}");
                }
            }
            Ok(Some(Frame::Close { reason })) => {
                let reason: String = reason.chars().take(MAX_LOGGED_REASON).collect();
                return format!("closed by Keel: {}", reason.escape_debug());
            }
            Ok(Some(other)) => {
                let violation = LinkViolation::out_of_order(frame_name(&other));
                log_violation(peer, &violation);
                return format!("violation: {}", violation.reason());
            }
            Err(e) => {
                return match LinkViolation::frame(&e) {
                    Some(violation) => {
                        log_violation(peer, &violation);
                        format!("violation: {}", violation.reason())
                    }
                    None => format!("control channel lost: {e}"),
                };
            }
        }
    }
}

/// A frame's tag, for logs; never its contents.
const fn frame_name(frame: &Frame) -> &'static str {
    match frame {
        Frame::Hello { .. } => "Hello",
        Frame::Ping { .. } => "Ping",
        Frame::Pong { .. } => "Pong",
        Frame::CacheGet { .. } => "CacheGet",
        Frame::CacheChunk { .. } => "CacheChunk before any CacheGet",
        Frame::CacheMiss { .. } => "CacheMiss before any CacheGet",
        Frame::Close { .. } => "Close",
    }
}
