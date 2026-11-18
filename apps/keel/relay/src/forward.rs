//! The runner listener: each runner TCP connection becomes one `forwarded-tcpip` channel to
//! Keel (spec 067 US1).
//!
//! With no link up, a connection is accepted and closed at once. Otherwise the relay opens a
//! channel to `keel-runner:0`, with the runner's TCP peer as originator, and copies bytes both
//! ways until either side closes or the link goes. Nothing is read from the runner before Keel
//! accepts the channel, and nothing is buffered beyond the copy buffers and the channel window.
//!
//! Limits: at most `max_streams` streams (pending ones included), at most `max_pending_accepts`
//! channels waiting for Keel, and Keel has `channel_accept_timeout` to accept one. A stream that
//! passes no bytes either way for `stream_idle_timeout` is closed. Logs carry the peer and byte
//! counts, never contents.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use keel_exec::relay::RUNNER_CHANNEL_ADDR;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::Instrument;

use crate::Limits;
use crate::link::{Link, Shared};

/// How long to back off after an `accept` error such as running out of file descriptors.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

/// Handles an `accept` error: per-connection errors and resource exhaustion are waited out;
/// anything else ends the listener.
pub(crate) async fn accept_error(e: &io::Error) -> io::Result<()> {
    use io::ErrorKind::*;
    match e.kind() {
        ConnectionAborted | ConnectionReset | Interrupted | WouldBlock => Ok(()),
        _ if matches!(e.raw_os_error(), Some(23 | 24 | 105 | 12)) => {
            // ENFILE, EMFILE, ENOBUFS, ENOMEM.
            tracing::warn!(error = %e, "accept failed; backing off");
            tokio::time::sleep(ACCEPT_BACKOFF).await;
            Ok(())
        }
        _ => Err(io::Error::new(e.kind(), e.to_string())),
    }
}

/// Accepts runner connections until the listener fails.
pub(crate) async fn serve(
    listener: TcpListener,
    shared: Arc<Shared>,
    limits: Limits,
) -> io::Result<()> {
    let streams = Arc::new(Semaphore::new(limits.max_streams as usize));
    let pending = Arc::new(Semaphore::new(limits.max_pending_accepts));
    loop {
        let (tcp, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                accept_error(&e).await?;
                continue;
            }
        };
        let Some(link) = shared.current() else {
            tracing::debug!(%peer, "runner connection closed: no link to Keel");
            continue;
        };
        let Ok(stream) = streams.clone().try_acquire_owned() else {
            tracing::warn!(%peer, max_streams = limits.max_streams, "runner connection refused: max_streams");
            continue;
        };
        let Ok(waiting) = pending.clone().try_acquire_owned() else {
            tracing::warn!(
                %peer,
                max_pending_accepts = limits.max_pending_accepts,
                "runner connection refused: too many waiting for Keel"
            );
            continue;
        };
        let _ = tcp.set_nodelay(true);
        let shared = shared.clone();
        let timeouts = Timeouts {
            accept: limits.channel_accept_timeout,
            idle: limits.stream_idle_timeout,
        };
        tokio::spawn(async move {
            carry(tcp, peer, link, shared, timeouts, stream, waiting).await;
        });
    }
}

/// A carried stream's timeouts: Keel's accept, and inactivity once carried.
#[derive(Clone, Copy)]
struct Timeouts {
    accept: Duration,
    idle: Duration,
}

/// When a stream last moved bytes either way, as milliseconds since it opened.
struct Activity {
    opened: Instant,
    last_ms: AtomicU64,
}

impl Activity {
    fn new() -> Self {
        Self {
            opened: Instant::now(),
            last_ms: AtomicU64::new(0),
        }
    }

    fn touch(&self) {
        let ms = u64::try_from(self.opened.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.last_ms.store(ms, Ordering::Relaxed);
    }

    fn last(&self) -> Instant {
        self.opened + Duration::from_millis(self.last_ms.load(Ordering::Relaxed))
    }

    /// Returns once no bytes have moved for `idle`.
    async fn idle(&self, idle: Duration) {
        loop {
            let due = self.last() + idle;
            if Instant::now() >= due {
                return;
            }
            tokio::time::sleep_until(due.into()).await;
        }
    }
}

/// Counts the bytes read through it and notes the activity.
struct Counted<'a, S> {
    inner: S,
    read: u64,
    activity: &'a Activity,
}

impl<S: AsyncRead + Unpin> AsyncRead for Counted<'_, S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
        let n = buf.filled().len() - before;
        if n > 0 {
            self.read += n as u64;
            self.activity.touch();
        }
        poll
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Counted<'_, S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// Decrements `Pong.streams` when the stream ends, however it ends.
struct Active<'a>(&'a Shared);

impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.streams.fetch_sub(1, Ordering::Relaxed);
    }
}

async fn carry(
    tcp: TcpStream,
    peer: SocketAddr,
    link: Arc<Link>,
    shared: Arc<Shared>,
    timeouts: Timeouts,
    _stream: OwnedSemaphorePermit,
    waiting: OwnedSemaphorePermit,
) {
    let mut closed = link.closed();
    let (address, port) = RUNNER_CHANNEL_ADDR;
    let open = link.handle.channel_open_forwarded_tcpip(
        address,
        port,
        peer.ip().to_string(),
        u32::from(peer.port()),
    );
    let opened = tokio::select! {
        opened = tokio::time::timeout(timeouts.accept, open) => opened,
        _ = closed.wait_for(|c| *c) => {
            tracing::info!(%peer, link = link.id, "runner connection closed: link went down");
            return;
        }
    };
    drop(waiting);
    let channel = match opened {
        Ok(Ok(channel)) => channel,
        Ok(Err(e)) => {
            tracing::info!(%peer, link = link.id, error = %e, "Keel refused the runner channel");
            return;
        }
        Err(_) => {
            tracing::warn!(
                %peer,
                link = link.id,
                timeout_s = timeouts.accept.as_secs_f32(),
                "Keel did not accept the runner channel in time"
            );
            return;
        }
    };

    shared.streams.fetch_add(1, Ordering::Relaxed);
    let _active = Active(&shared);
    let started = Instant::now();
    // `relay.stream`: the relay's key fingerprint is its id here (Keel's row id is Keel's).
    let span = tracing::info_span!(
        "relay.stream",
        relay.id = shared.fingerprint(),
        relay.link = link.id,
        relay.stream.bytes_in = tracing::field::Empty,
        relay.stream.bytes_out = tracing::field::Empty,
        relay.stream.end = tracing::field::Empty,
    );
    tracing::info!(%peer, link = link.id, "stream open");
    let activity = Activity::new();
    let mut runner = Counted {
        inner: tcp,
        read: 0,
        activity: &activity,
    };
    let mut keel = Counted {
        inner: channel.into_stream(),
        read: 0,
        activity: &activity,
    };
    let end = async {
        tokio::select! {
            r = tokio::io::copy_bidirectional(&mut runner, &mut keel) => match r {
                Ok(_) => "closed".to_owned(),
                Err(e) => format!("error: {e}"),
            },
            _ = closed.wait_for(|c| *c) => "link went down".to_owned(),
            () = activity.idle(timeouts.idle) => "idle".to_owned(),
        }
    }
    .instrument(span.clone())
    .await;
    // Bytes in = from the runner, bytes out = to it, as on Keel's span.
    span.record("relay.stream.bytes_in", runner.read)
        .record("relay.stream.bytes_out", keel.read)
        .record("relay.stream.end", end.as_str());
    tracing::info!(
        %peer,
        link = link.id,
        bytes_up = runner.read,
        bytes_down = keel.read,
        secs = started.elapsed().as_secs(),
        end = %end,
        "stream closed"
    );
}
