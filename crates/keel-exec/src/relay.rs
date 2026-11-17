//! The Keel ↔ relay link (spec 067, contracts/relay.md).
//!
//! Keel dials a relay over SSH as user [`LINK_USER`], opens one session channel with the
//! [`RELAY_SUBSYSTEM`] control subsystem, and accepts `forwarded-tcpip` channels to
//! [`RUNNER_CHANNEL_ADDR`] from the relay, one per runner TCP connection. Inside each of those it
//! runs the runner listener's own SSH server, so the relay only ever sees ciphertext.
//!
//! This module holds what both ends share: the control [`Frame`]s and their codec (1 MiB cap),
//! the SSH configurations, and the per-connection policies ([`RelayServerPolicy`] on the relay,
//! [`LinkClientPolicy`] on Keel).

use std::time::Duration;

use russh::keys::PrivateKey;
use russh::{MethodKind, MethodSet};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::client_config::client_preferred;
use crate::codec::{self, CodecError};
use crate::server_config::server_preferred;

/// The control subsystem Keel opens on the link's one session channel.
pub const RELAY_SUBSYSTEM: &str = "keel-relay-v1";
/// The only user name Keel logs in to a relay as.
pub const LINK_USER: &str = "keel";
/// The connected address and port of every runner channel the relay opens.
pub const RUNNER_CHANNEL_ADDR: (&str, u32) = ("keel-runner", 0);
/// The largest control frame body accepted or sent.
pub const MAX_CONTROL_FRAME_LEN: usize = 1024 * 1024;
/// The largest raw `CacheChunk.bytes`.
pub const MAX_CACHE_CHUNK: usize = 512 * 1024;
/// SSH keepalive on the link, from both ends.
pub const LINK_KEEPALIVE: Duration = Duration::from_secs(15);
/// Missed keepalives that close the link.
pub const LINK_KEEPALIVE_MAX: usize = 3;
/// The relay's `Hello` must arrive within this after the control channel opens.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// A runner channel Keel has not accepted within this is closed by the relay.
pub const RUNNER_CHANNEL_TIMEOUT: Duration = Duration::from_secs(20);

/// A control frame on `keel-relay-v1`. JSON tagged `"t"`; unknown tags and fields are errors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", deny_unknown_fields)]
pub enum Frame {
    /// R → K, the relay's first frame.
    Hello {
        version: String,
        relay_key_fingerprint: String,
        max_streams: u32,
        http_cache: bool,
    },
    /// K → R, every health interval.
    Ping { seq: u64 },
    /// R → K, the answer to `Ping`.
    Pong {
        seq: u64,
        version: String,
        streams: u32,
        uptime_s: u64,
    },
    /// R → K, a runner release file for the relay's cache.
    CacheGet {
        req: u32,
        release: String,
        path: String,
    },
    /// K → R, part of a `CacheGet` answer. `bytes` is base64 on the wire, at most
    /// [`MAX_CACHE_CHUNK`] raw.
    CacheChunk {
        req: u32,
        offset: u64,
        #[serde(with = "crate::frames::b64")]
        bytes: Vec<u8>,
        last: bool,
    },
    /// K → R, a `CacheGet` Keel will not answer.
    CacheMiss { req: u32, reason: CacheMissReason },
    /// K → R, revocation or shutdown: the relay closes every runner stream.
    Close { reason: String },
}

/// Why Keel did not serve a `CacheGet`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheMissReason {
    /// Keel does not serve runner releases (yet).
    Unavailable,
    /// Not a file of that release's manifest.
    NotFound,
    /// Over the per-release budget.
    TooLarge,
}

impl Frame {
    fn check(&self) -> Result<(), CodecError> {
        match self {
            Frame::CacheChunk { bytes, .. } if bytes.len() > MAX_CACHE_CHUNK => {
                Err(CodecError::TooLarge {
                    len: bytes.len(),
                    max: MAX_CACHE_CHUNK,
                })
            }
            _ => Ok(()),
        }
    }
}

/// Serialise a control frame with its length prefix.
pub fn encode(frame: &Frame) -> Result<Vec<u8>, CodecError> {
    frame.check()?;
    codec::encode_capped(frame, MAX_CONTROL_FRAME_LEN)
}

/// Parse a control frame body (no prefix).
pub fn decode_body(body: &[u8]) -> Result<Frame, CodecError> {
    let frame: Frame = codec::decode_body_capped(body, MAX_CONTROL_FRAME_LEN)?;
    frame.check()?;
    Ok(frame)
}

/// Write one control frame and flush.
pub async fn write_frame<W: AsyncWrite + Unpin>(
    w: &mut W,
    frame: &Frame,
) -> Result<(), CodecError> {
    frame.check()?;
    codec::write_capped(w, frame, MAX_CONTROL_FRAME_LEN).await
}

/// Read one control frame; `Ok(None)` is a clean end of stream.
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Option<Frame>, CodecError> {
    let frame: Option<Frame> = codec::read_capped(r, MAX_CONTROL_FRAME_LEN).await?;
    if let Some(f) = &frame {
        f.check()?;
    }
    Ok(frame)
}

/// The relay's SSH server configuration: its plain Ed25519 host key (no certificate), public-key
/// authentication only, keepalive every 15 s.
pub fn relay_server_config(host_key: PrivateKey) -> russh::server::Config {
    let mut methods = MethodSet::empty();
    methods.push(MethodKind::PublicKey);
    russh::server::Config {
        methods,
        auth_rejection_time: Duration::from_millis(250),
        auth_rejection_time_initial: Some(Duration::from_millis(0)),
        keys: vec![host_key],
        preferred: server_preferred(),
        max_auth_attempts: 4,
        inactivity_timeout: None,
        keepalive_interval: Some(LINK_KEEPALIVE),
        keepalive_max: LINK_KEEPALIVE_MAX,
        nodelay: true,
        ..russh::server::Config::default()
    }
}

/// Keel's SSH client configuration for dialling relays. It expects a plain host key, which
/// [`LinkClientPolicy`] pins.
pub fn link_client_config() -> russh::client::Config {
    let mut preferred = client_preferred();
    // A relay has no host certificate: ask for the plain key only.
    preferred.host_key_certificates = std::borrow::Cow::Borrowed(&[]);
    russh::client::Config {
        preferred,
        inactivity_timeout: None,
        keepalive_interval: Some(LINK_KEEPALIVE),
        keepalive_max: LINK_KEEPALIVE_MAX,
        nodelay: true,
        ..russh::client::Config::default()
    }
}
