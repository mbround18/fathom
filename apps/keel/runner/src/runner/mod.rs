//! `keel-runner`, the network half (spec 055 T084, contracts/runner.md).
//!
//! - Reads `policy.toml` and `host_ca.pub` from the trust directory.
//! - Creates its Ed25519 key (`id_ed25519`, `0600`) on first start; it never leaves the host.
//! - Enrolls with `enroll-token` over `keel-enroll-v1`, after checking that the token pins a CA
//!   in `host_ca.pub`, and deletes the token once the certificate is saved.
//! - With an expired certificate, authenticates with the bare key and sends only `RenewCert`.
//! - Otherwise holds a `keel-runner-v1` session ([`session`]): `Hello`, `Ping`/`Pong`, renewal
//!   at half the certificate's lifetime, the KRL, and run frames relayed unchanged to and from
//!   one executor socket connection per run.
//!
//! It never interprets work: the executor checks everything itself.

pub mod outside;
pub(crate) mod session;

use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use keel_exec::ca::{HOST_PRINCIPAL_PREFIX, HostCertVerifier, RUNNER_KEY_ID_PREFIX, TrustedCa};
use keel_exec::client_config::{
    EnrollToken, KeelSshClient, ServerKeyRefusal, SubsystemError, TrustFileError, client_config,
    open_subsystem, parse_host_ca,
};
use keel_exec::codec::{read_frame, write_frame};
use keel_exec::frames::{Base, Frame, Subsystem};
use russh::client::{AuthResult, Handle};
use russh::keys::ssh_key::LineEnding;
use russh::keys::ssh_key::certificate::CertType;
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, Certificate, PrivateKey, PrivateKeyWithHashAlg};
use tokio::net::{TcpStream, UnixStream};

use crate::policy::{Policy, PolicyError, read_trust_file};

/// Keel's runner listener port when `--server` names none.
pub const DEFAULT_PORT: u16 = 7831;
/// The SSH user name runners log in as.
pub const SSH_USER: &str = "runner";

pub const HOST_CA_FILE: &str = "host_ca.pub";
pub const KEY_FILE: &str = "id_ed25519";
pub const CERT_FILE: &str = "id_ed25519-cert.pub";
pub const HOST_ID_FILE: &str = "host_id";
pub const KRL_FILE: &str = "krl";
pub const TOKEN_FILE: &str = "enroll-token";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);

/// Where things are. Defaults match contracts/host-install.md.
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    /// `host:port` of Keel's runner listener (`keel_runner_server`). The host part is the name
    /// the server's host certificate must carry.
    pub server: String,
    pub trust_dir: PathBuf,
    pub state_dir: PathBuf,
    pub executor_socket: PathBuf,
    /// The Ansible venv, for the versions in `Hello`.
    pub venv: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error(transparent)]
    Policy(#[from] PolicyError),
    #[error("trust: {0}")]
    Trust(#[from] TrustFileError),
    #[error("state: {0}")]
    State(String),
    #[error("not enrolled: no certificate and no enroll-token")]
    NotEnrolled,
    #[error("the enrollment token pins no CA in host_ca.pub; refusing to connect")]
    TokenCaMismatch,
    #[error("refused the server: {0}")]
    ServerRefused(ServerKeyRefusal),
    #[error("connect: {0}")]
    Connect(String),
    #[error("the server refused this host's credentials")]
    AuthRefused,
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("refused the certificate the server issued: {0}")]
    BadCert(&'static str),
    #[error("executor: {0}")]
    Executor(String),
}

impl From<SubsystemError> for RunnerError {
    fn from(e: SubsystemError) -> Self {
        RunnerError::Protocol(e.to_string())
    }
}

fn state_err(what: &str, path: &Path, e: impl std::fmt::Display) -> RunnerError {
    RunnerError::State(format!("{what} {}: {e}", path.display()))
}

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Splits `host[:port]`. The host must be a name (IP literals in brackets are not supported).
pub fn split_server(server: &str) -> Result<(String, u16), RunnerError> {
    let bad = || RunnerError::Connect(format!("--server {server:?} is not host[:port]"));
    let (host, port) = match server.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().map_err(|_| bad())?),
        None => (server, DEFAULT_PORT),
    };
    if host.is_empty()
        || port == 0
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.'))
    {
        return Err(bad());
    }
    Ok((host.to_owned(), port))
}

/// Loads `id_ed25519`, or creates it (`0600`, exclusively) when absent. A key file that is not a
/// regular file, carries group or world permission bits, is encrypted or is not Ed25519 is
/// refused.
pub fn load_or_create_key(state_dir: &Path) -> Result<PrivateKey, RunnerError> {
    let path = state_dir.join(KEY_FILE);
    match std::fs::symlink_metadata(&path) {
        Ok(meta) => {
            if !meta.file_type().is_file() {
                return Err(state_err("not a regular file:", &path, "refused"));
            }
            if meta.permissions().mode() & 0o077 != 0 {
                return Err(state_err(
                    "key file is readable by others:",
                    &path,
                    "chmod 600",
                ));
            }
            let text = std::fs::read_to_string(&path).map_err(|e| state_err("read", &path, e))?;
            let key = PrivateKey::from_openssh(&text)
                .map_err(|_| state_err("parse", &path, "not an OpenSSH private key"))?;
            if key.is_encrypted() || key.algorithm() != Algorithm::Ed25519 {
                return Err(state_err(
                    "key",
                    &path,
                    "must be an unencrypted Ed25519 key",
                ));
            }
            Ok(key)
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
                .map_err(|e| state_err("generate", &path, e))?;
            let pem = key
                .to_openssh(LineEnding::LF)
                .map_err(|e| state_err("encode", &path, e))?;
            crate::write_private_new(&path, pem.as_bytes())
                .map_err(|e| state_err("write", &path, e))?;
            Ok(key)
        }
        Err(e) => Err(state_err("stat", &path, e)),
    }
}

/// Checks a certificate Keel issued to this host: our key, a user certificate, a `runner:` key
/// id, the `host:<inventory_name>` principal, and not already expired. The runner does not hold
/// the runner CA, so the server checks the signature when the certificate is used.
pub fn check_issued(
    line: &str,
    key: &PrivateKey,
    inventory_name: &str,
    now: u64,
) -> Result<Certificate, RunnerError> {
    let cert = Certificate::from_openssh(line.trim())
        .map_err(|_| RunnerError::BadCert("not an OpenSSH certificate"))?;
    if cert.public_key() != key.public_key().key_data() {
        return Err(RunnerError::BadCert("it is not for this host's key"));
    }
    if cert.cert_type() != CertType::User {
        return Err(RunnerError::BadCert("not a user certificate"));
    }
    if !cert.key_id().starts_with(RUNNER_KEY_ID_PREFIX) {
        return Err(RunnerError::BadCert("not a runner key id"));
    }
    let principal = format!("{HOST_PRINCIPAL_PREFIX}{inventory_name}");
    if !cert.valid_principals().contains(&principal) {
        return Err(RunnerError::BadCert("not issued for this inventory name"));
    }
    if cert.valid_before() <= now || cert.valid_after() >= cert.valid_before() {
        return Err(RunnerError::BadCert("already expired"));
    }
    Ok(cert)
}

/// When to renew: half way through the certificate's lifetime (Unix seconds).
pub fn renew_at(cert: &Certificate) -> u64 {
    let after = cert.valid_after();
    after + cert.valid_before().saturating_sub(after) / 2
}

/// The network half's state: configuration, trust, and the host key.
pub struct Runner {
    cfg: RunnerConfig,
    policy: Policy,
    host_cas: Vec<KeyData>,
    key: Arc<PrivateKey>,
    /// The record of runs outside Keel (spec 058 amendment A2).
    outside: outside::OutsideLog,
}

impl std::fmt::Debug for Runner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runner")
            .field("cfg", &self.cfg)
            .field("inventory_name", &self.policy.inventory_name)
            .finish_non_exhaustive()
    }
}

impl Runner {
    /// Reads the trust files and loads or creates the host key.
    pub fn load(cfg: RunnerConfig) -> Result<Self, RunnerError> {
        split_server(&cfg.server)?;
        let policy = Policy::load(&cfg.trust_dir)?;
        let host_cas = parse_host_ca(&read_trust_file(&cfg.trust_dir.join(HOST_CA_FILE))?)?;
        let key = Arc::new(load_or_create_key(&cfg.state_dir)?);
        let outside = outside::OutsideLog::new(outside::DEFAULT_OUTSIDE_LOG, &cfg.state_dir);
        Ok(Self {
            cfg,
            policy,
            host_cas,
            key,
            outside,
        })
    }

    /// Reads runs outside Keel from `path` instead of `/var/lib/keel-outside/runs.jsonl`.
    pub fn with_outside_log(mut self, path: impl Into<PathBuf>) -> Self {
        self.outside = outside::OutsideLog::new(path, &self.cfg.state_dir);
        self
    }

    pub fn config(&self) -> &RunnerConfig {
        &self.cfg
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    pub fn public_key(&self) -> &KeyData {
        self.key.public_key().key_data()
    }

    fn path(&self, name: &str) -> PathBuf {
        self.cfg.state_dir.join(name)
    }

    /// The saved certificate, if any. A file that does not parse is an error, not "absent".
    pub fn certificate(&self) -> Result<Option<Certificate>, RunnerError> {
        let path = self.path(CERT_FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => Certificate::from_openssh(text.trim())
                .map(Some)
                .map_err(|_| state_err("parse", &path, "not an OpenSSH certificate")),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(state_err("read", &path, e)),
        }
    }

    fn host_id(&self) -> Result<String, RunnerError> {
        let path = self.path(HOST_ID_FILE);
        let id = std::fs::read_to_string(&path).map_err(|e| state_err("read", &path, e))?;
        let id = id.trim().to_owned();
        valid_host_id(&id)
            .then_some(id)
            .ok_or_else(|| state_err("parse", &path, "invalid host id"))
    }

    fn save_cert(&self, cert: &Certificate) -> Result<(), RunnerError> {
        let path = self.path(CERT_FILE);
        let line = cert
            .to_openssh()
            .map_err(|e| state_err("encode", &path, e))?;
        crate::write_private_atomic(&path, format!("{line}\n").as_bytes())
            .map_err(|e| state_err("write", &path, e))
    }

    pub(crate) fn accept_cert(&self, line: &str) -> Result<Certificate, RunnerError> {
        let cert = check_issued(line, &self.key, &self.policy.inventory_name, unix_now())?;
        self.save_cert(&cert)?;
        Ok(cert)
    }

    pub(crate) fn save_krl(&self, bytes: &[u8]) -> Result<(), RunnerError> {
        keel_exec::krl::Krl::from_bytes(bytes)
            .map_err(|e| RunnerError::Protocol(format!("the server sent a bad KRL: {e}")))?;
        let path = self.path(KRL_FILE);
        crate::write_private_atomic(&path, bytes).map_err(|e| state_err("write", &path, e))
    }

    /// Connects and checks the server's host certificate against `cas`. Nothing is sent before
    /// that check passes.
    async fn connect(&self, cas: &[KeyData]) -> Result<Handle<KeelSshClient>, RunnerError> {
        let (host, port) = split_server(&self.cfg.server)?;
        let verifier = HostCertVerifier::new(
            cas.iter().cloned().map(TrustedCa::active).collect(),
            host.clone(),
        );
        let client = KeelSshClient::new(verifier);
        let refusal = client.refusal_slot();
        let stream =
            tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host.as_str(), port)))
                .await
                .map_err(|_| RunnerError::Connect("timed out".to_owned()))?
                .map_err(|e| RunnerError::Connect(e.to_string()))?;
        let result = tokio::time::timeout(
            CONNECT_TIMEOUT,
            russh::client::connect_stream(Arc::new(client_config()), stream, client),
        )
        .await;
        let refused = refusal.lock().ok().and_then(|mut slot| slot.take());
        match (result, refused) {
            (_, Some(why)) => Err(RunnerError::ServerRefused(why)),
            (Ok(Ok(handle)), None) => Ok(handle),
            (Ok(Err(e)), None) => Err(RunnerError::Connect(e.to_string())),
            (Err(_), None) => Err(RunnerError::Connect("handshake timed out".to_owned())),
        }
    }

    async fn auth_bare_key(&self, handle: &mut Handle<KeelSshClient>) -> Result<(), RunnerError> {
        let key = PrivateKeyWithHashAlg::new(Arc::clone(&self.key), None);
        match handle.authenticate_publickey(SSH_USER, key).await {
            Ok(AuthResult::Success) => Ok(()),
            Ok(_) => Err(RunnerError::AuthRefused),
            Err(e) => Err(RunnerError::Connect(e.to_string())),
        }
    }

    async fn auth_cert(
        &self,
        handle: &mut Handle<KeelSshClient>,
        cert: Certificate,
    ) -> Result<(), RunnerError> {
        match handle
            .authenticate_openssh_cert(SSH_USER, Arc::clone(&self.key), cert)
            .await
        {
            Ok(AuthResult::Success) => Ok(()),
            Ok(_) => Err(RunnerError::AuthRefused),
            Err(e) => Err(RunnerError::Connect(e.to_string())),
        }
    }

    /// Enrolls when there is no certificate yet. Returns `Ok(false)` when already enrolled.
    ///
    /// The token must pin one of the CAs in `host_ca.pub`, and the connection then trusts only
    /// that CA. The token file is deleted after the certificate and host id are saved.
    pub async fn enroll_if_needed(&self) -> Result<bool, RunnerError> {
        if self.certificate()?.is_some() {
            return Ok(false);
        }
        let token_path = self.path(TOKEN_FILE);
        let text = match std::fs::read_to_string(&token_path) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => return Err(RunnerError::NotEnrolled),
            Err(e) => return Err(state_err("read", &token_path, e)),
        };
        let token = EnrollToken::parse(&text)?;
        let ca = token
            .pinned_ca(&self.host_cas)
            .ok_or(RunnerError::TokenCaMismatch)?
            .clone();

        let mut handle = self.connect(std::slice::from_ref(&ca)).await?;
        self.auth_bare_key(&mut handle).await?;
        let mut stream = open_subsystem(&handle, Subsystem::Enroll).await?;
        let enroll = Frame::Enroll {
            token: token.secret().clone(),
            inventory_name: self.policy.inventory_name.clone(),
            allowed_profiles: self.policy.allowed_profiles.clone(),
            require_signed: self.policy.require_signed,
        };
        write_frame(&mut stream, &enroll)
            .await
            .map_err(|e| RunnerError::Protocol(e.to_string()))?;
        let answer = tokio::time::timeout(ANSWER_TIMEOUT, read_frame(&mut stream))
            .await
            .map_err(|_| RunnerError::Protocol("no answer to Enroll".to_owned()))?;
        let (host_id, cert) = match answer {
            Ok(Some(Frame::Enrolled { host_id, cert })) => (host_id, cert),
            Ok(Some(other)) => {
                return Err(RunnerError::Protocol(format!(
                    "expected Enrolled, got {}",
                    other.name()
                )));
            }
            Ok(None) | Err(_) => {
                return Err(RunnerError::Protocol(
                    "the server refused the enrollment".to_owned(),
                ));
            }
        };
        if !valid_host_id(&host_id) {
            return Err(RunnerError::Protocol("invalid host id".to_owned()));
        }
        let cert = check_issued(&cert, &self.key, &self.policy.inventory_name, unix_now())?;
        let id_path = self.path(HOST_ID_FILE);
        crate::write_private_atomic(&id_path, format!("{host_id}\n").as_bytes())
            .map_err(|e| state_err("write", &id_path, e))?;
        self.save_cert(&cert)?;
        std::fs::remove_file(&token_path).map_err(|e| state_err("delete", &token_path, e))?;
        let _ = handle
            .disconnect(russh::Disconnect::ByApplication, "", "")
            .await;
        Ok(true)
    }

    /// Renews an expired certificate: the bare key, the runner subsystem, and only `RenewCert`.
    pub async fn renew_expired(&self) -> Result<Certificate, RunnerError> {
        let mut handle = self.connect(&self.host_cas).await?;
        self.auth_bare_key(&mut handle).await?;
        let mut stream = open_subsystem(&handle, Subsystem::Runner).await?;
        write_frame(&mut stream, &Frame::RenewCert {})
            .await
            .map_err(|e| RunnerError::Protocol(e.to_string()))?;
        let answer = tokio::time::timeout(ANSWER_TIMEOUT, read_frame(&mut stream))
            .await
            .map_err(|_| RunnerError::Protocol("no answer to RenewCert".to_owned()))?;
        let cert = match answer {
            Ok(Some(Frame::Cert { cert })) => self.accept_cert(&cert)?,
            Ok(Some(other)) => {
                return Err(RunnerError::Protocol(format!(
                    "expected Cert, got {}",
                    other.name()
                )));
            }
            Ok(None) | Err(_) => {
                return Err(RunnerError::Protocol(
                    "the server refused the renewal".to_owned(),
                ));
            }
        };
        let _ = handle
            .disconnect(russh::Disconnect::ByApplication, "", "")
            .await;
        Ok(cert)
    }

    /// Asks the executor for the ledger head and bases (`ReadLedger` → `Ledger`).
    pub async fn read_ledger(&self) -> Result<(Option<String>, Vec<Base>), RunnerError> {
        let exec = |e: &dyn std::fmt::Display| RunnerError::Executor(e.to_string());
        let mut sock = tokio::time::timeout(
            ANSWER_TIMEOUT,
            UnixStream::connect(&self.cfg.executor_socket),
        )
        .await
        .map_err(|_| exec(&"connect timed out"))?
        .map_err(|e| exec(&e))?;
        write_frame(&mut sock, &Frame::ReadLedger)
            .await
            .map_err(|e| exec(&e))?;
        match tokio::time::timeout(ANSWER_TIMEOUT, read_frame(&mut sock)).await {
            Ok(Ok(Some(Frame::Ledger { ledger_head, bases }))) => Ok((ledger_head, bases)),
            _ => Err(exec(&"no Ledger answer to ReadLedger")),
        }
    }

    /// One pass: enroll if needed, renew an expired certificate, then hold a session until it
    /// ends.
    pub async fn run_once(&self) -> Result<(), RunnerError> {
        self.enroll_if_needed().await?;
        let cert = self.certificate()?.ok_or(RunnerError::NotEnrolled)?;
        let cert = if cert.valid_before() <= unix_now() {
            self.renew_expired().await?
        } else {
            cert
        };
        self.session(cert).await
    }

    /// Runs forever, reconnecting with backoff (1 s doubling to 60 s; reset after a session that
    /// lasted a minute).
    pub async fn run_forever(&self) -> ! {
        let mut backoff = Duration::from_secs(1);
        loop {
            let started = std::time::Instant::now();
            match self.run_once().await {
                Ok(()) => eprintln!("keel-runner: session ended"),
                Err(e) => eprintln!("keel-runner: {e}"),
            }
            if started.elapsed() > Duration::from_secs(60) {
                backoff = Duration::from_secs(1);
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(60));
        }
    }
}

fn valid_host_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.'))
}
