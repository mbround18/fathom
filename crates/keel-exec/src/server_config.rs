//! The server half of the runner SSH channel (spec 055, T073; contracts/runner.md "SSH connection").
//!
//! [`server_config`] pins the algorithms (`ssh-ed25519` host keys and certificates,
//! `curve25519-sha256`, `chacha20-poly1305@openssh.com` or `aes256-gcm@openssh.com`, public-key
//! authentication only). [`KeelSshHandler`] is the per-connection policy: it authenticates the
//! user `runner` by certificate, bare enrolled key or unknown key (see [`Access`]), refuses every
//! channel and request except one session channel carrying the one subsystem its [`Access`]
//! allows, and hands that subsystem's byte stream to a [`SubsystemService`].
//!
//! The listener itself (accept loop, `--runner-max-offline`, the database behind
//! [`RunnerAuthority`]) lives in the server app; this module is transport policy only.

use std::borrow::Cow;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, Certificate, PrivateKey};
use russh::server::{
    Auth, ChannelOpenHandle, Config, Handler, Msg, PublicKeyOffer, Session, VerifiedPublicKey,
};
use russh::{
    Channel, ChannelId, ChannelOpenFailure, ChannelStream, MethodKind, MethodSet, Preferred, Pty,
    cipher, compression, kex,
};

use crate::ca::{RunnerCertVerifier, RunnerIdentity};
use crate::frames::{Frame, Peer, Subsystem};

/// The only SSH user name a runner may log in as.
pub const RUNNER_USER: &str = "runner";

/// The oldest `ansible-core` a runner may report in `Hello` (contracts/runner.md); older is
/// refused with `version_too_old`.
pub const MIN_ANSIBLE_CORE: (u64, u64, u64) = (2, 17, 0);

/// Whether `version` (`2.19.0`, `2.19.0rc1`, `2.20`) is at least [`MIN_ANSIBLE_CORE`]. A version
/// that does not parse is refused.
pub fn ansible_core_supported(version: &str) -> bool {
    let mut parts = [0u64; 3];
    let mut seen = 0;
    for (i, part) in version.trim().split('.').take(3).enumerate() {
        let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
        let Ok(n) = digits.parse::<u64>() else {
            return false;
        };
        parts[i] = n;
        seen = i + 1;
        if digits.len() != part.len() {
            break;
        }
    }
    seen >= 2 && (parts[0], parts[1], parts[2]) >= MIN_ANSIBLE_CORE
}

/// Ciphers both sides accept, most preferred first. Both are AEAD, so no MAC is negotiated.
pub const CIPHERS: &[cipher::Name] = &[cipher::CHACHA20_POLY1305, cipher::AES_256_GCM];

/// Host and user key algorithms (plain and certificate) both sides accept.
pub const KEY_ALGORITHMS: &[Algorithm] = &[Algorithm::Ed25519];

const SERVER_KEX: &[kex::Name] = &[
    kex::CURVE25519,
    kex::EXTENSION_SUPPORT_AS_SERVER,
    kex::EXTENSION_OPENSSH_STRICT_KEX_AS_SERVER,
];

/// The server's algorithm lists: anything a client offers outside them ends the handshake.
pub fn server_preferred() -> Preferred {
    Preferred {
        kex: Cow::Borrowed(SERVER_KEX),
        key: Cow::Borrowed(KEY_ALGORITHMS),
        // Only read by clients; a server advertises its `Config::certificates`.
        host_key_certificates: Cow::Borrowed(&[]),
        cipher: Cow::Borrowed(CIPHERS),
        mac: Cow::Borrowed(&[]),
        compression: Cow::Borrowed(&[compression::NONE]),
    }
}

/// The runner listener's russh configuration: the ed25519 host key, its host certificate (a
/// runner refuses a server without one), public-key authentication only.
pub fn server_config(host_key: PrivateKey, host_cert: Option<Certificate>) -> Config {
    let mut methods = MethodSet::empty();
    methods.push(MethodKind::PublicKey);
    Config {
        methods,
        auth_rejection_time: Duration::from_millis(250),
        auth_rejection_time_initial: Some(Duration::from_millis(0)),
        keys: vec![host_key],
        certificates: host_cert.into_iter().collect(),
        preferred: server_preferred(),
        max_auth_attempts: 4,
        inactivity_timeout: Some(Duration::from_secs(600)),
        keepalive_interval: Some(Duration::from_secs(30)),
        nodelay: true,
        ..Config::default()
    }
}

/// What an authenticated connection may do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    /// A valid runner certificate for an enrolled, non-revoked host: `keel-runner-v1`, all
    /// runner frames.
    Runner(RunnerIdentity),
    /// The bare enrolled key of a host whose certificate expired within the allowed offline
    /// window: `keel-runner-v1`, but only `RenewCert`.
    RenewOnly { host_id: String },
    /// An unknown key: `keel-enroll-v1` only.
    EnrollOnly,
}

impl Access {
    /// The one subsystem this connection may open.
    pub const fn subsystem(&self) -> Subsystem {
        match self {
            Access::Runner(_) | Access::RenewOnly { .. } => Subsystem::Runner,
            Access::EnrollOnly => Subsystem::Enroll,
        }
    }

    /// Whether the runner on this connection may send `frame`. A service drops the connection
    /// on the first frame this refuses.
    pub fn permits(&self, frame: &Frame) -> bool {
        if !frame.may_be_sent_by(Peer::Runner) || frame.subsystem() != Some(self.subsystem()) {
            return false;
        }
        match self {
            Access::Runner(_) => true,
            Access::RenewOnly { .. } => matches!(frame, Frame::RenewCert {}),
            Access::EnrollOnly => matches!(frame, Frame::Enroll { .. }),
        }
    }
}

/// Where a bare (certificate-less) key stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BareKey {
    /// Never enrolled: may enroll.
    Unknown,
    /// The enrolled key of a host whose certificate expired no longer than
    /// `--runner-max-offline` ago: may renew.
    Renewable { host_id: String },
    /// Enrolled but revoked, or offline for too long: refused outright.
    Refused,
}

/// The server's view of enrolled runners. Implemented by the server app over its database.
pub trait RunnerAuthority: Send + Sync + 'static {
    /// The runner CAs (active and retiring) and the current KRL.
    fn verifier(&self) -> Arc<RunnerCertVerifier>;

    /// Whether a certificate that passed [`RunnerAuthority::verifier`] belongs to the enrolled
    /// key of a host that is not revoked.
    fn is_enrolled(
        &self,
        identity: &RunnerIdentity,
        key: &KeyData,
    ) -> impl Future<Output = bool> + Send;

    /// Where a bare key stands. Keys revoked in the KRL are refused before this is asked.
    fn bare_key(&self, key: &KeyData, now: SystemTime) -> impl Future<Output = BareKey> + Send;
}

/// Serves one subsystem stream. The handler has already checked that `access` allows
/// `access.subsystem()`; the service must still refuse frames [`Access::permits`] refuses.
pub trait SubsystemService: Send + Sync + 'static {
    fn serve(&self, access: Access, stream: ChannelStream<Msg>) -> impl Future<Output = ()> + Send;

    /// [`SubsystemService::serve`], with the public key the connection authenticated with (the
    /// certificate's key for [`Access::Runner`]). Enrollment needs it: it is the key Keel
    /// records. The handler calls this one; by default it ignores the key.
    fn serve_with_key(
        &self,
        access: Access,
        key: KeyData,
        stream: ChannelStream<Msg>,
    ) -> impl Future<Output = ()> + Send {
        let _ = key;
        self.serve(access, stream)
    }
}

/// The per-connection SSH policy. Construct one per accepted connection.
pub struct KeelSshHandler<A, S> {
    authority: Arc<A>,
    service: Arc<S>,
    access: Option<Access>,
    /// The key the connection authenticated with.
    key: Option<KeyData>,
    /// The session channel, until a subsystem is bound to it.
    pending: Option<Channel<Msg>>,
    session_opened: bool,
}

impl<A, S> std::fmt::Debug for KeelSshHandler<A, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeelSshHandler")
            .field("access", &self.access)
            .field("session_opened", &self.session_opened)
            .finish_non_exhaustive()
    }
}

impl<A: RunnerAuthority, S: SubsystemService> KeelSshHandler<A, S> {
    pub fn new(authority: Arc<A>, service: Arc<S>) -> Self {
        Self {
            authority,
            service,
            access: None,
            key: None,
            pending: None,
            session_opened: false,
        }
    }

    /// What the connection was authenticated for (`None` before authentication).
    pub fn access(&self) -> Option<&Access> {
        self.access.as_ref()
    }

    fn refuse(channel: ChannelId, session: &mut Session) -> Result<(), russh::Error> {
        session.channel_failure(channel)
    }

    async fn refuse_open(reply: ChannelOpenHandle) -> Result<(), russh::Error> {
        reply
            .reject(ChannelOpenFailure::AdministrativelyProhibited)
            .await;
        Ok(())
    }
}

fn is_runner_user(user: &str) -> bool {
    user == RUNNER_USER
}

impl<A: RunnerAuthority, S: SubsystemService> Handler for KeelSshHandler<A, S> {
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
        offer: PublicKeyOffer<'_>,
    ) -> Result<Auth, Self::Error> {
        if is_runner_user(user) && offer.public_key.algorithm() == Algorithm::Ed25519 {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        let data = key.public_key.key_data();
        if !is_runner_user(user) || data.algorithm() != Algorithm::Ed25519 {
            return Ok(Auth::reject());
        }
        if self.authority.verifier().krl.is_key_revoked(data) {
            return Ok(Auth::reject());
        }
        let access = match self.authority.bare_key(data, SystemTime::now()).await {
            BareKey::Unknown => Access::EnrollOnly,
            BareKey::Renewable { host_id } => Access::RenewOnly { host_id },
            BareKey::Refused => return Ok(Auth::reject()),
        };
        self.access = Some(access);
        self.key = Some(data.clone());
        Ok(Auth::Accept)
    }

    async fn auth_openssh_certificate(
        &mut self,
        user: &str,
        cert: &Certificate,
    ) -> Result<Auth, Self::Error> {
        if !is_runner_user(user) {
            return Ok(Auth::reject());
        }
        // russh checked validity and the signature, not who signed it: the verifier checks the
        // CA, the KRL, the key id, principals, options and extensions.
        let Ok(identity) = self.authority.verifier().verify(cert, SystemTime::now()) else {
            return Ok(Auth::reject());
        };
        if !self
            .authority
            .is_enrolled(&identity, cert.public_key())
            .await
        {
            return Ok(Auth::reject());
        }
        self.access = Some(Access::Runner(identity));
        self.key = Some(cert.public_key().clone());
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.access.is_none() || self.session_opened {
            return Self::refuse_open(reply).await;
        }
        self.session_opened = true;
        self.pending = Some(channel);
        reply.accept().await;
        Ok(())
    }

    async fn channel_open_x11(
        &mut self,
        _channel: Channel<Msg>,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        Self::refuse_open(reply).await
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        _channel: Channel<Msg>,
        _host_to_connect: &str,
        _port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        Self::refuse_open(reply).await
    }

    async fn channel_open_forwarded_tcpip(
        &mut self,
        _channel: Channel<Msg>,
        _host_to_connect: &str,
        _port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        Self::refuse_open(reply).await
    }

    async fn channel_open_direct_streamlocal(
        &mut self,
        _channel: Channel<Msg>,
        _socket_path: &str,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        Self::refuse_open(reply).await
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        _col_width: u32,
        _row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        Self::refuse(channel, session)
    }

    async fn x11_request(
        &mut self,
        channel: ChannelId,
        _single_connection: bool,
        _x11_auth_protocol: &str,
        _x11_auth_cookie: &str,
        _x11_screen_number: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        Self::refuse(channel, session)
    }

    async fn env_request(
        &mut self,
        channel: ChannelId,
        _variable_name: &str,
        _variable_value: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        Self::refuse(channel, session)
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        Self::refuse(channel, session)
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        _data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        Self::refuse(channel, session)
    }

    async fn agent_request(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    async fn tcpip_forward(
        &mut self,
        _address: &str,
        _port: &mut u32,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    async fn cancel_tcpip_forward(
        &mut self,
        _address: &str,
        _port: u32,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    async fn streamlocal_forward(
        &mut self,
        _socket_path: &str,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    async fn cancel_streamlocal_forward(
        &mut self,
        _socket_path: &str,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let allowed = match (&self.access, Subsystem::from_name(name)) {
            (Some(access), Some(sub)) => access.subsystem() == sub,
            _ => false,
        };
        let bound = self.pending.as_ref().is_some_and(|c| c.id() == channel);
        let (true, true, Some(access), Some(key)) =
            (allowed, bound, self.access.clone(), self.key.clone())
        else {
            return Self::refuse(channel, session);
        };
        let Some(chan) = self.pending.take() else {
            return Self::refuse(channel, session);
        };
        session.channel_success(channel)?;
        let service = Arc::clone(&self.service);
        let stream = chan.into_stream();
        tokio::spawn(async move { service.serve_with_key(access, key, stream).await });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::ansible_core_supported;

    #[test]
    fn ansible_core_below_the_minimum_or_unparsable_is_refused() {
        for ok in ["2.17.0", "2.19.0", "2.19.0rc1", "2.20", "3.0.0", " 2.18.1 "] {
            assert!(ansible_core_supported(ok), "{ok}");
        }
        for bad in ["2.16.14", "2.9.27", "2", "", "two.19", "v2.19.0", "1.99.99"] {
            assert!(!ansible_core_supported(bad), "{bad}");
        }
    }
}
