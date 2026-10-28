//! The runner half of the runner SSH channel (spec 055, T073; contracts/runner.md "SSH connection").
//!
//! [`client_config`] pins the same algorithms as the server and asks for the server's ed25519
//! host certificate. [`KeelSshClient`] accepts the server only if that certificate verifies
//! against the trusted host CAs ([`HostCertVerifier`]); a bare host key, another CA's
//! certificate or an expired one ends the connection before anything is sent.
//! [`open_subsystem`] opens the one session channel and its subsystem.
//!
//! [`parse_host_ca`] reads the runner's pinned `host_ca.pub`, and [`EnrollToken`] the one-time
//! enrollment token, whose fingerprint must name one of those CAs before the runner connects.

use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use russh::client::{Config, Handle, Handler, Msg};
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, PublicKey, PublicKeyOrCertificate};
use russh::{ChannelMsg, ChannelStream, Preferred, compression, kex};

use crate::ca::{CertRefusal, HostCertVerifier};
use crate::frames::{SecretString, Subsystem};
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

/// The prefix of an enrollment token.
pub const ENROLL_TOKEN_PREFIX: &str = "keel-enroll-v1.";

/// A parsed enrollment token, `keel-enroll-v1.<secret>.<host CA SHA-256 fingerprint>`
/// (contracts/runner.md "Enrollment"). `Debug` never shows the token.
#[derive(Clone, PartialEq, Eq)]
pub struct EnrollToken {
    token: SecretString,
    ca_fingerprint: String,
}

impl std::fmt::Debug for EnrollToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnrollToken")
            .field("token", &self.token)
            .field("ca_fingerprint", &self.ca_fingerprint)
            .finish()
    }
}

/// Why an enrollment token or `host_ca.pub` was refused. Never contains the token.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TrustFileError {
    #[error("the enrollment token is not keel-enroll-v1.<secret>.<SHA256 fingerprint>")]
    MalformedToken,
    #[error("host_ca.pub line {0}: not an ssh-ed25519 key")]
    HostCaLine(usize),
    #[error("host_ca.pub holds no host CA")]
    NoHostCa,
}

impl EnrollToken {
    /// Parses the token file's contents (surrounding whitespace allowed).
    pub fn parse(text: &str) -> Result<Self, TrustFileError> {
        let token = text.trim();
        let rest = token
            .strip_prefix(ENROLL_TOKEN_PREFIX)
            .ok_or(TrustFileError::MalformedToken)?;
        let (secret, fingerprint) = rest
            .rsplit_once('.')
            .ok_or(TrustFileError::MalformedToken)?;
        let b64 = fingerprint
            .strip_prefix("SHA256:")
            .ok_or(TrustFileError::MalformedToken)?;
        let printable = |s: &str| s.bytes().all(|b| b.is_ascii_graphic());
        if secret.is_empty()
            || b64.is_empty()
            || !printable(secret)
            || !b64
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=')
        {
            return Err(TrustFileError::MalformedToken);
        }
        Ok(Self {
            token: SecretString::new(token),
            ca_fingerprint: fingerprint.to_owned(),
        })
    }

    /// The whole token, for `Enroll.token`.
    pub fn secret(&self) -> &SecretString {
        &self.token
    }

    /// The `SHA256:…` fingerprint of the host CA the token pins.
    pub fn ca_fingerprint(&self) -> &str {
        &self.ca_fingerprint
    }

    /// The one CA in `cas` the token pins, or `None`: then the runner must not connect.
    pub fn pinned_ca<'a>(&self, cas: &'a [KeyData]) -> Option<&'a KeyData> {
        cas.iter()
            .find(|ca| crate::ca::fingerprint(ca) == self.ca_fingerprint)
    }
}

/// Parses `/etc/keel-runner/trust/host_ca.pub`: one key per line, either
/// `@cert-authority <hosts> ssh-ed25519 <base64> [comment]` or `ssh-ed25519 <base64> [comment]`.
/// Blank lines and `#` comments are skipped. Anything else, or any key that is not Ed25519, fails
/// the whole file (fail closed).
pub fn parse_host_ca(text: &str) -> Result<Vec<KeyData>, TrustFileError> {
    let mut cas = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad = || TrustFileError::HostCaLine(i + 1);
        let mut fields = line.split_whitespace();
        let mut first = fields.next().ok_or_else(bad)?;
        if first == "@cert-authority" {
            fields.next().ok_or_else(bad)?;
            first = fields.next().ok_or_else(bad)?;
        }
        if first != "ssh-ed25519" {
            return Err(bad());
        }
        let b64 = fields.next().ok_or_else(bad)?;
        let key = PublicKey::from_openssh(&format!("ssh-ed25519 {b64}")).map_err(|_| bad())?;
        if key.algorithm() != Algorithm::Ed25519 {
            return Err(bad());
        }
        cas.push(key.key_data().clone());
    }
    if cas.is_empty() {
        return Err(TrustFileError::NoHostCa);
    }
    Ok(cas)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ca::{CaKind, SshCa};

    #[test]
    fn a_token_pins_one_host_ca_by_fingerprint() {
        let ca = SshCa::generate(CaKind::Host).unwrap();
        let other = SshCa::generate(CaKind::Host).unwrap();
        let text = format!("keel-enroll-v1.s3cr.et.{}\n", ca.fingerprint());
        let token = EnrollToken::parse(&text).unwrap();
        assert_eq!(token.ca_fingerprint(), ca.fingerprint());
        assert_eq!(token.secret().expose(), text.trim());
        assert!(!format!("{token:?}").contains("s3cr"));
        let both = [other.key_data().clone(), ca.key_data().clone()];
        assert_eq!(token.pinned_ca(&both), Some(ca.key_data()));
        assert_eq!(token.pinned_ca(&both[..1]), None);
    }

    #[test]
    fn malformed_tokens_are_refused() {
        for t in [
            "",
            "keel-enroll-v1.",
            "keel-enroll-v1.secret",
            "keel-enroll-v1..SHA256:abc",
            "keel-enroll-v1.secret.MD5:abc",
            "keel-enroll-v1.secret.SHA256:",
            "keel-enroll-v2.secret.SHA256:abc",
            "keel-enroll-v1.sec ret.SHA256:abc",
        ] {
            assert_eq!(
                EnrollToken::parse(t),
                Err(TrustFileError::MalformedToken),
                "{t}"
            );
        }
    }

    #[test]
    fn host_ca_pub_takes_cert_authority_and_plain_lines_only() {
        let a = SshCa::generate(CaKind::Host).unwrap();
        let b = SshCa::generate(CaKind::Host).unwrap();
        let text = format!(
            "# Keel host CA\n\n{}\n{}\n",
            a.cert_authority_line("*"),
            b.public_line()
        );
        let cas = parse_host_ca(&text).unwrap();
        assert_eq!(cas, vec![a.key_data().clone(), b.key_data().clone()]);
        assert_eq!(parse_host_ca("# nothing\n"), Err(TrustFileError::NoHostCa));
        assert_eq!(
            parse_host_ca("ssh-rsa AAAAB3NzaC1yc2E= x\n"),
            Err(TrustFileError::HostCaLine(1))
        );
        assert_eq!(
            parse_host_ca(&format!("{}\ngarbage\n", a.public_line())),
            Err(TrustFileError::HostCaLine(2))
        );
    }
}
