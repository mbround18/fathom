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

use std::sync::Arc;
use std::time::Duration;

use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, Certificate, PrivateKey, PublicKeyOrCertificate};
use russh::server::Auth;
use russh::{
    Channel, ChannelId, ChannelOpenFailure, ChannelStream, Disconnect, MethodKind, MethodSet, Pty,
    client, server,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};

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
    read_frame_capped(r, MAX_CONTROL_FRAME_LEN).await
}

/// [`read_frame`] with a lower cap on the body (Keel's `KEEL_RELAY_MAX_CONTROL_FRAME`); `max`
/// never raises it above [`MAX_CONTROL_FRAME_LEN`].
pub async fn read_frame_capped<R: AsyncRead + Unpin>(
    r: &mut R,
    max: usize,
) -> Result<Option<Frame>, CodecError> {
    let frame: Option<Frame> = codec::read_capped(r, max.min(MAX_CONTROL_FRAME_LEN)).await?;
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

/// The SSH window Keel grants each relay-link channel: what a relay can make Keel buffer per
/// stream before Keel reads it (russh's default is 2 MiB).
pub const LINK_WINDOW_SIZE: u32 = 256 << 10;
/// The largest SSH packet Keel accepts on a relay link.
pub const LINK_MAX_PACKET: u32 = 32 << 10;
/// Messages queued per relay-link channel before the link stops reading (russh's default 100).
pub const LINK_CHANNEL_BUFFER: usize = 32;
/// A relay link with no traffic for this long is dropped; keepalives count as traffic.
pub const LINK_INACTIVITY: Duration = Duration::from_secs(120);

/// Keel's SSH client configuration for dialling relays. It expects a plain host key, which
/// [`LinkClientPolicy`] pins, and bounds what a relay can make Keel hold (spec 067 T060): the
/// channel window, packet size and per-channel queue, an inactivity timeout and keepalives.
/// Keel may tighten these from its own settings.
pub fn link_client_config() -> russh::client::Config {
    let mut preferred = client_preferred();
    // A relay has no host certificate: ask for the plain key only.
    preferred.host_key_certificates = std::borrow::Cow::Borrowed(&[]);
    russh::client::Config {
        preferred,
        window_size: LINK_WINDOW_SIZE,
        maximum_packet_size: LINK_MAX_PACKET,
        channel_buffer_size: LINK_CHANNEL_BUFFER,
        inactivity_timeout: Some(LINK_INACTIVITY),
        keepalive_interval: Some(LINK_KEEPALIVE),
        keepalive_max: LINK_KEEPALIVE_MAX,
        nodelay: true,
        ..russh::client::Config::default()
    }
}

/// An ordinary refusal: someone knocked without the right credentials. Logged and audited
/// (`relay.refused`), but not held against the peer, because anyone who can reach the port can
/// cause one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkRefusal {
    /// Relay: a login that is not user `keel` with a key from `hub_keys`.
    Auth { user: String, fingerprint: String },
}

impl LinkRefusal {
    /// The audit `reason` code.
    pub const fn reason(&self) -> &'static str {
        match self {
            LinkRefusal::Auth { .. } => "auth_refused",
        }
    }
}

/// A protocol violation by an authenticated (or, for the host key, pinned) peer: something the
/// other end of a correct link never does. Keel quarantines a relay on one
/// (`relays.quarantined_at`) and the relay closes the link. Never carries secrets: an `env`
/// request reports the variable name, not its value, and an `exec` request only its length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkViolation {
    /// Keel: the relay presented a host key (or certificate) other than the pinned key.
    HostKey { fingerprint: String },
    /// Either end: a channel type it never accepts, or a `forwarded-tcpip` channel to anything
    /// but [`RUNNER_CHANNEL_ADDR`].
    Channel { kind: &'static str, detail: String },
    /// Relay: a channel or global request it never grants.
    Request { kind: &'static str, detail: String },
    /// Relay: a second session (control) channel. The link is closed.
    SecondSession,
    /// Either end: a control frame that is unknown, malformed, over the cap, or out of order.
    Frame { detail: String },
}

impl LinkViolation {
    /// The audit `reason` code, also stored as `relays.quarantine_reason`.
    pub const fn reason(&self) -> &'static str {
        match self {
            LinkViolation::HostKey { .. } => "host_key_mismatch",
            LinkViolation::Channel { .. } => "forbidden_channel",
            LinkViolation::Request { .. } => "forbidden_request",
            LinkViolation::SecondSession => "second_control_channel",
            LinkViolation::Frame { .. } => "bad_frame",
        }
    }

    /// The violation a control-channel read error amounts to. An oversized, unknown or
    /// malformed frame is one; the peer hanging up (`Truncated`, I/O) is not.
    pub fn frame(err: &CodecError) -> Option<Self> {
        match err {
            CodecError::TooLarge { .. } | CodecError::Malformed(_) => Some(LinkViolation::Frame {
                detail: err.to_string(),
            }),
            CodecError::Truncated | CodecError::Io(_) => None,
        }
    }

    /// A well-formed frame at the wrong time, such as anything before `Hello`.
    pub fn out_of_order(what: impl std::fmt::Display) -> Self {
        LinkViolation::Frame {
            detail: format!("out of order: {what}"),
        }
    }
}

/// Where a policy reports ordinary refusals. Called inline from the SSH handler, so it must not
/// block.
pub type RefusalSink = Arc<dyn Fn(LinkRefusal) + Send + Sync>;

/// Where a policy reports violations. Called inline from the SSH handler, so it must not block.
pub type ViolationSink = Arc<dyn Fn(LinkViolation) + Send + Sync>;

fn channel_violation(kind: &'static str, detail: impl Into<String>) -> LinkViolation {
    LinkViolation::Channel {
        kind,
        detail: detail.into(),
    }
}

fn request_violation(kind: &'static str, detail: impl Into<String>) -> LinkViolation {
    LinkViolation::Request {
        kind,
        detail: detail.into(),
    }
}

/// The relay's per-connection SSH policy. Accepts only user [`LINK_USER`] with a key from
/// `hub_keys`, and only one session channel, carrying [`RELAY_SUBSYSTEM`]: its byte stream is
/// sent on `control`. A second session channel closes the link; everything else is refused.
pub struct RelayServerPolicy {
    hub_keys: Vec<KeyData>,
    on_refusal: RefusalSink,
    on_violation: ViolationSink,
    control: Option<oneshot::Sender<ChannelStream<server::Msg>>>,
    authenticated: bool,
    session_opened: bool,
    pending: Option<Channel<server::Msg>>,
    last_auth_refusal: Option<(String, String)>,
}

impl std::fmt::Debug for RelayServerPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayServerPolicy")
            .field("hub_keys", &self.hub_keys.len())
            .field("authenticated", &self.authenticated)
            .field("session_opened", &self.session_opened)
            .finish_non_exhaustive()
    }
}

impl RelayServerPolicy {
    pub fn new(
        hub_keys: Vec<KeyData>,
        on_refusal: RefusalSink,
        on_violation: ViolationSink,
        control: oneshot::Sender<ChannelStream<server::Msg>>,
    ) -> Self {
        Self {
            hub_keys,
            on_refusal,
            on_violation,
            control: Some(control),
            authenticated: false,
            session_opened: false,
            pending: None,
            last_auth_refusal: None,
        }
    }

    fn violation(&self, violation: LinkViolation) {
        (self.on_violation)(violation);
    }

    /// Checks a login, reporting a refusal once per (user, key) although russh asks twice.
    fn check_login(&mut self, user: &str, key: &KeyData) -> Auth {
        if user == LINK_USER
            && key.algorithm() == Algorithm::Ed25519
            && self.hub_keys.iter().any(|k| k == key)
        {
            return Auth::Accept;
        }
        let seen = (user.to_owned(), crate::ca::fingerprint(key));
        if self.last_auth_refusal.as_ref() != Some(&seen) {
            (self.on_refusal)(LinkRefusal::Auth {
                user: seen.0.clone(),
                fingerprint: seen.1.clone(),
            });
            self.last_auth_refusal = Some(seen);
        }
        Auth::reject()
    }

    fn refuse_request(
        &self,
        channel: ChannelId,
        session: &mut server::Session,
        kind: &'static str,
        detail: impl Into<String>,
    ) -> Result<(), russh::Error> {
        self.violation(request_violation(kind, detail));
        session.channel_failure(channel)
    }

    async fn refuse_open(
        &self,
        reply: server::ChannelOpenHandle,
        kind: &'static str,
        detail: impl Into<String>,
    ) -> Result<(), russh::Error> {
        self.violation(channel_violation(kind, detail));
        reply
            .reject(ChannelOpenFailure::AdministrativelyProhibited)
            .await;
        Ok(())
    }
}

impl server::Handler for RelayServerPolicy {
    type Error = russh::Error;

    async fn auth_none(&mut self, _user: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::reject())
    }

    async fn auth_password(&mut self, _user: &str, _password: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::reject())
    }

    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        offer: server::PublicKeyOffer<'_>,
    ) -> Result<Auth, Self::Error> {
        Ok(self.check_login(user, offer.public_key.key_data()))
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: server::VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        let auth = self.check_login(user, key.public_key.key_data());
        if matches!(auth, Auth::Accept) {
            self.authenticated = true;
        }
        Ok(auth)
    }

    async fn auth_openssh_certificate(
        &mut self,
        user: &str,
        cert: &Certificate,
    ) -> Result<Auth, Self::Error> {
        // Hub keys are plain keys; a certificate is never accepted.
        (self.on_refusal)(LinkRefusal::Auth {
            user: user.to_owned(),
            fingerprint: crate::ca::fingerprint(cert.public_key()),
        });
        Ok(Auth::reject())
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if !self.authenticated {
            return self
                .refuse_open(reply, "session", "not authenticated")
                .await;
        }
        if self.session_opened {
            self.violation(LinkViolation::SecondSession);
            reply
                .reject(ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return session.disconnect(Disconnect::ByApplication, "second session channel", "");
        }
        self.session_opened = true;
        self.pending = Some(channel);
        reply.accept().await;
        Ok(())
    }

    async fn channel_open_x11(
        &mut self,
        _channel: Channel<server::Msg>,
        originator_address: &str,
        originator_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.refuse_open(
            reply,
            "x11",
            format!("{originator_address}:{originator_port}"),
        )
        .await
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        _channel: Channel<server::Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.refuse_open(
            reply,
            "direct-tcpip",
            format!("{host_to_connect}:{port_to_connect}"),
        )
        .await
    }

    async fn channel_open_forwarded_tcpip(
        &mut self,
        _channel: Channel<server::Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.refuse_open(
            reply,
            "forwarded-tcpip",
            format!("{host_to_connect}:{port_to_connect}"),
        )
        .await
    }

    async fn channel_open_direct_streamlocal(
        &mut self,
        _channel: Channel<server::Msg>,
        socket_path: &str,
        reply: server::ChannelOpenHandle,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.refuse_open(reply, "direct-streamlocal", socket_path)
            .await
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        _col_width: u32,
        _row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.refuse_request(channel, session, "pty", term)
    }

    async fn x11_request(
        &mut self,
        channel: ChannelId,
        _single_connection: bool,
        x11_auth_protocol: &str,
        _x11_auth_cookie: &str,
        _x11_screen_number: u32,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.refuse_request(channel, session, "x11", x11_auth_protocol)
    }

    async fn env_request(
        &mut self,
        channel: ChannelId,
        variable_name: &str,
        _variable_value: &str,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.refuse_request(channel, session, "env", variable_name)
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.refuse_request(channel, session, "shell", "")
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        // The command line could carry anything; report only its length.
        self.refuse_request(channel, session, "exec", format!("{} bytes", data.len()))
    }

    async fn agent_request(
        &mut self,
        _channel: ChannelId,
        _session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        self.violation(request_violation("auth-agent", ""));
        Ok(false)
    }

    async fn tcpip_forward(
        &mut self,
        address: &str,
        port: &mut u32,
        _session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        self.violation(request_violation(
            "tcpip-forward",
            format!("{address}:{port}"),
        ));
        Ok(false)
    }

    async fn cancel_tcpip_forward(
        &mut self,
        address: &str,
        port: u32,
        _session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        self.violation(request_violation(
            "cancel-tcpip-forward",
            format!("{address}:{port}"),
        ));
        Ok(false)
    }

    async fn streamlocal_forward(
        &mut self,
        socket_path: &str,
        _session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        self.violation(request_violation("streamlocal-forward", socket_path));
        Ok(false)
    }

    async fn cancel_streamlocal_forward(
        &mut self,
        socket_path: &str,
        _session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        self.violation(request_violation("cancel-streamlocal-forward", socket_path));
        Ok(false)
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let bound = self.pending.as_ref().is_some_and(|c| c.id() == channel);
        if name != RELAY_SUBSYSTEM || !bound || self.control.is_none() {
            return self.refuse_request(channel, session, "subsystem", name);
        }
        let (Some(chan), Some(control)) = (self.pending.take(), self.control.take()) else {
            return self.refuse_request(channel, session, "subsystem", name);
        };
        session.channel_success(channel)?;
        // The receiver gone means the relay is shutting down; the channel then just closes.
        let _ = control.send(chan.into_stream());
        Ok(())
    }
}

/// A runner channel the relay opened, accepted by [`LinkClientPolicy`]. The originator is the
/// runner's TCP peer as the relay claims it: advisory, never used for a decision.
#[derive(Debug)]
pub struct RunnerChannel {
    pub channel: Channel<client::Msg>,
    pub originator_address: String,
    pub originator_port: u32,
}

/// Asked before a runner channel is accepted, with the advisory originator address and port:
/// `true` takes the channel, `false` refuses it (`resource-shortage`, not a violation). Keel
/// counts streams and applies its rate here, so a relay cannot make it hold (and buffer) more
/// channels than its limits allow, whatever its own loop is doing (spec 067 T066).
pub type StreamAdmission = Arc<dyn Fn(&str, u32) -> bool + Send + Sync>;

/// Keel's per-link SSH client policy. Accepts only the pinned relay host key, and only
/// `forwarded-tcpip` channels to [`RUNNER_CHANNEL_ADDR`] that [`StreamAdmission`] admits, which
/// go to `runners`. Every other channel is refused.
pub struct LinkClientPolicy {
    pinned: KeyData,
    on_violation: ViolationSink,
    runners: mpsc::UnboundedSender<RunnerChannel>,
    admit: Option<StreamAdmission>,
}

impl std::fmt::Debug for LinkClientPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkClientPolicy")
            .field("pinned", &crate::ca::fingerprint(&self.pinned))
            .finish_non_exhaustive()
    }
}

impl LinkClientPolicy {
    pub fn new(
        pinned: KeyData,
        on_violation: ViolationSink,
        runners: mpsc::UnboundedSender<RunnerChannel>,
    ) -> Self {
        Self {
            pinned,
            on_violation,
            runners,
            admit: None,
        }
    }

    /// Asks `admit` before accepting each runner channel. Without it every runner channel is
    /// accepted, and `runners` is the only bound.
    #[must_use]
    pub fn with_admission(mut self, admit: StreamAdmission) -> Self {
        self.admit = Some(admit);
        self
    }

    /// The host-key decision, separate from the russh callback so it can be tested alone.
    pub fn check(&self, key: &PublicKeyOrCertificate) -> Result<(), LinkViolation> {
        let presented = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => {
                if key.key_data() == &self.pinned {
                    return Ok(());
                }
                key.key_data()
            }
            PublicKeyOrCertificate::Certificate(cert) => cert.public_key(),
        };
        Err(LinkViolation::HostKey {
            fingerprint: crate::ca::fingerprint(presented),
        })
    }

    async fn refuse_open(
        &self,
        reply: client::ChannelOpenHandle,
        kind: &'static str,
        detail: impl Into<String>,
    ) -> Result<(), russh::Error> {
        (self.on_violation)(channel_violation(kind, detail));
        reply
            .reject(ChannelOpenFailure::AdministrativelyProhibited)
            .await;
        Ok(())
    }
}

impl client::Handler for LinkClientPolicy {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        match self.check(key) {
            Ok(()) => Ok(true),
            Err(violation) => {
                (self.on_violation)(violation);
                Ok(false)
            }
        }
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<client::Msg>,
        connected_address: &str,
        connected_port: u32,
        originator_address: &str,
        originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        if (connected_address, connected_port) != RUNNER_CHANNEL_ADDR || self.runners.is_closed() {
            return self
                .refuse_open(
                    reply,
                    "forwarded-tcpip",
                    format!("{connected_address}:{connected_port}"),
                )
                .await;
        }
        if let Some(admit) = &self.admit
            && !admit(originator_address, originator_port)
        {
            reply.reject(ChannelOpenFailure::ResourceShortage).await;
            return Ok(());
        }
        reply.accept().await;
        // If Keel stopped listening in between, dropping the channel closes it.
        let _ = self.runners.send(RunnerChannel {
            channel,
            originator_address: originator_address.to_owned(),
            originator_port,
        });
        Ok(())
    }

    async fn server_channel_open_forwarded_streamlocal(
        &mut self,
        _channel: Channel<client::Msg>,
        socket_path: &str,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        self.refuse_open(reply, "forwarded-streamlocal", socket_path)
            .await
    }

    async fn server_channel_open_agent_forward(
        &mut self,
        _channel: Channel<client::Msg>,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        self.refuse_open(reply, "auth-agent", "").await
    }

    async fn server_channel_open_session(
        &mut self,
        _channel: Channel<client::Msg>,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        self.refuse_open(reply, "session", "").await
    }

    async fn server_channel_open_direct_tcpip(
        &mut self,
        _channel: Channel<client::Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        self.refuse_open(
            reply,
            "direct-tcpip",
            format!("{host_to_connect}:{port_to_connect}"),
        )
        .await
    }

    async fn server_channel_open_direct_streamlocal(
        &mut self,
        _channel: Channel<client::Msg>,
        socket_path: &str,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        self.refuse_open(reply, "direct-streamlocal", socket_path)
            .await
    }

    async fn server_channel_open_x11(
        &mut self,
        _channel: Channel<client::Msg>,
        originator_address: &str,
        originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        self.refuse_open(
            reply,
            "x11",
            format!("{originator_address}:{originator_port}"),
        )
        .await
    }

    async fn should_accept_unknown_server_channel(
        &mut self,
        _id: ChannelId,
        channel_type: &str,
    ) -> bool {
        let shown: String = channel_type.chars().take(64).collect();
        (self.on_violation)(channel_violation("unknown", shown));
        false
    }

    // Keel never opens a shell or exec, so a relay has nothing to send these about. russh
    // refuses other channel requests and every global request (`tcpip-forward`, ...) itself
    // without a hook, so those are refused but cannot be reported here.

    async fn xon_xoff(
        &mut self,
        _channel: ChannelId,
        client_can_do: bool,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        (self.on_violation)(request_violation("xon-xoff", client_can_do.to_string()));
        Ok(())
    }

    async fn exit_status(
        &mut self,
        _channel: ChannelId,
        exit_status: u32,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        (self.on_violation)(request_violation("exit-status", exit_status.to_string()));
        Ok(())
    }

    async fn exit_signal(
        &mut self,
        _channel: ChannelId,
        signal_name: russh::Sig,
        _core_dumped: bool,
        _error_message: &str,
        _lang_tag: &str,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        (self.on_violation)(request_violation("exit-signal", format!("{signal_name:?}")));
        Ok(())
    }
}
