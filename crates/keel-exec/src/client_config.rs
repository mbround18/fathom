//! The runner half of the runner SSH channel (spec 055, T073; contracts/runner.md "SSH connection").
//!
//! [`client_config`] pins the same algorithms as the server and asks for the server's ed25519
//! host certificate. [`KeelSshClient`] accepts the server only if that certificate verifies
//! against the trusted host CAs ([`HostCertVerifier`]); a bare host key, another CA's
//! certificate or an expired one ends the connection before anything is sent.
//! [`open_subsystem`] opens the one session channel and its subsystem.

use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use russh::client::{Config, Handle, Handler, Msg};
use russh::keys::PublicKeyOrCertificate;
use russh::{ChannelMsg, ChannelStream, Preferred, compression, kex};

use crate::ca::{CertRefusal, HostCertVerifier};
use crate::frames::Subsystem;
use crate::server_config::{CIPHERS, KEY_ALGORITHMS};

const CLIENT_KEX: &[kex::Name] = &[
    kex::CURVE25519,
    kex::EXTENSION_SUPPORT_AS_CLIENT,
    kex::EXTENSION_OPENSSH_STRICT_KEX_AS_CLIENT,
];

/// The runner's algorithm lists. It asks for an ed25519 host certificate.
pub fn client_preferred() -> Preferred {
    Preferred {
        kex: Cow::Borrowed(CLIENT_KEX),
        key: Cow::Borrowed(KEY_ALGORITHMS),
        host_key_certificates: Cow::Borrowed(KEY_ALGORITHMS),
        cipher: Cow::Borrowed(CIPHERS),
        mac: Cow::Borrowed(&[]),
        compression: Cow::Borrowed(&[compression::NONE]),
    }
}

/// The runner's russh configuration.
pub fn client_config() -> Config {
    Config {
        preferred: client_preferred(),
        inactivity_timeout: Some(Duration::from_secs(600)),
        keepalive_interval: Some(Duration::from_secs(30)),
        nodelay: true,
        ..Config::default()
    }
}

/// Why the runner refused the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ServerKeyRefusal {
    #[error("the server presented a bare host key, not a host certificate")]
    BareKey,
    #[error("the server's host certificate was refused: {0}")]
    Certificate(CertRefusal),
}

/// Where [`KeelSshClient`] records why it refused the server, for the runner's log.
pub type RefusalSlot = Arc<Mutex<Option<ServerKeyRefusal>>>;

/// The runner's russh client handler: host-certificate verification, nothing else.
#[derive(Debug)]
pub struct KeelSshClient {
    verifier: HostCertVerifier,
    refusal: RefusalSlot,
}

impl KeelSshClient {
    pub fn new(verifier: HostCertVerifier) -> Self {
        Self {
            verifier,
            refusal: RefusalSlot::default(),
        }
    }

    /// A handle on the refusal reason, readable after the connection failed.
    pub fn refusal_slot(&self) -> RefusalSlot {
        Arc::clone(&self.refusal)
    }

    /// The server-key decision, separate from the russh callback so it can be tested alone.
    pub fn check(
        &self,
        key: &PublicKeyOrCertificate,
        now: SystemTime,
    ) -> Result<(), ServerKeyRefusal> {
        match key {
            PublicKeyOrCertificate::PublicKey { .. } => Err(ServerKeyRefusal::BareKey),
            PublicKeyOrCertificate::Certificate(cert) => self
                .verifier
                .verify(cert, now)
                .map_err(ServerKeyRefusal::Certificate),
        }
    }
}

impl Handler for KeelSshClient {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        match self.check(key, SystemTime::now()) {
            Ok(()) => Ok(true),
            Err(refusal) => {
                if let Ok(mut slot) = self.refusal.lock() {
                    *slot = Some(refusal);
                }
                Ok(false)
            }
        }
    }
}

/// Failure to open a subsystem.
#[derive(Debug, thiserror::Error)]
pub enum SubsystemError {
    #[error("the server refused the {0} subsystem")]
    Refused(&'static str),
    #[error("the channel closed before the server answered")]
    Closed,
    #[error(transparent)]
    Ssh(#[from] russh::Error),
}

/// Opens a session channel on an authenticated connection and requests `subsystem` on it,
/// waiting for the server's answer. Returns the frame stream.
pub async fn open_subsystem<H: Handler>(
    handle: &Handle<H>,
    subsystem: Subsystem,
) -> Result<ChannelStream<Msg>, SubsystemError> {
    let mut channel = handle.channel_open_session().await?;
    channel.request_subsystem(true, subsystem.name()).await?;
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Success) => return Ok(channel.into_stream()),
            Some(ChannelMsg::Failure) => return Err(SubsystemError::Refused(subsystem.name())),
            Some(ChannelMsg::Close | ChannelMsg::Eof) | None => return Err(SubsystemError::Closed),
            Some(_) => {}
        }
    }
}
