//! Keel's two SSH certificate authorities and the certificates they issue (T081).
//!
//! - The **runner CA** signs runner (user) certificates:
//!   - key id `runner:<id>`;
//!   - principals `host:<inventory name>`, plus `profile:<p>` for each profile both assigned and
//!     allowed;
//!   - a random non-zero serial;
//!   - validity `[now, now + ttl)`;
//!   - no critical options and no extensions.
//! - The **host CA** signs the server's host certificate. Runners trust it through
//!   `host_ca.pub`.
//!
//! [`RunnerCertVerifier`] is the server's check of a runner certificate:
//! - the CA is trusted, active or retiring (a retiring CA stays trusted until its certificates
//!   expire);
//! - the signature and validity window are good;
//! - the certificate has the exact shape above;
//! - the KRL does not revoke it.
//!
//! [`HostCertVerifier`] is the runner's check of the server.

use std::collections::BTreeSet;
use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rand::RngExt;
use russh::keys::ssh_key::LineEnding;
use russh::keys::ssh_key::certificate::{Builder, CertType};
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, Certificate, HashAlg, PrivateKey, PublicKey};

use crate::frames::SecretString;
use crate::krl::Krl;

/// Key id prefix of runner certificates.
pub const RUNNER_KEY_ID_PREFIX: &str = "runner:";
/// Key id prefix of host certificates.
pub const HOST_KEY_ID_PREFIX: &str = "keel-host:";
/// Principal prefixes in runner certificates.
pub const HOST_PRINCIPAL_PREFIX: &str = "host:";
pub const PROFILE_PRINCIPAL_PREFIX: &str = "profile:";
/// Host certificates are backdated this much to tolerate clock skew on runners.
pub const HOST_CERT_BACKDATE: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaKind {
    Host,
    Runner,
}

impl CaKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            CaKind::Host => "host",
            CaKind::Runner => "runner",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CaError {
    #[error("ssh key: {0}")]
    Key(#[from] russh::keys::ssh_key::Error),
    #[error("only Ed25519 keys are accepted, got {0}")]
    NotEd25519(String),
    #[error("an encrypted CA key cannot be used")]
    Encrypted,
    #[error("the {have} CA cannot issue {want} certificates")]
    WrongKind {
        have: &'static str,
        want: &'static str,
    },
    #[error("invalid certificate request: {0}")]
    Request(&'static str),
}

/// A CA private key. `Debug` shows only the kind and fingerprint.
pub struct SshCa {
    kind: CaKind,
    key: PrivateKey,
}

impl fmt::Debug for SshCa {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SshCa")
            .field("kind", &self.kind)
            .field("fingerprint", &self.fingerprint())
            .finish()
    }
}

/// A runner certificate to issue.
#[derive(Debug, Clone)]
pub struct RunnerCertRequest<'a> {
    pub runner_id: &'a str,
    pub inventory_name: &'a str,
    /// Profiles the inventory assigns to the host.
    pub assigned_profiles: &'a [String],
    /// Profiles the host's local policy allows (`Hello.allowed_profiles`).
    pub allowed_profiles: &'a [String],
    pub public_key: &'a KeyData,
    /// `--runner-cert-ttl` (24 h by default).
    pub ttl: Duration,
    pub now: SystemTime,
}

/// A host certificate to issue.
#[derive(Debug, Clone)]
pub struct HostCertRequest<'a> {
    /// The names runners connect to; the first is used in the key id.
    pub host_names: &'a [String],
    pub public_key: &'a KeyData,
    pub ttl: Duration,
    pub now: SystemTime,
}

fn unix(t: SystemTime) -> Result<u64, CaError> {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| CaError::Request("time before 1970"))
}

fn require_ed25519(key: &KeyData) -> Result<(), CaError> {
    match key.algorithm() {
        Algorithm::Ed25519 => Ok(()),
        other => Err(CaError::NotEd25519(other.to_string())),
    }
}

/// `SHA256:…` of a public key.
pub fn fingerprint(key: &KeyData) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

impl SshCa {
    /// A fresh Ed25519 CA.
    pub fn generate(kind: CaKind) -> Result<Self, CaError> {
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)?;
        Ok(SshCa { kind, key })
    }

    /// Load a CA from its OpenSSH private key (as unsealed by the server).
    pub fn from_openssh(kind: CaKind, pem: &str) -> Result<Self, CaError> {
        let key = PrivateKey::from_openssh(pem)?;
        if key.is_encrypted() {
            return Err(CaError::Encrypted);
        }
        require_ed25519(key.public_key().key_data())?;
        Ok(SshCa { kind, key })
    }

    /// The OpenSSH private key, for sealing. Never log it.
    pub fn to_openssh(&self) -> Result<SecretString, CaError> {
        Ok(SecretString::from(
            self.key.to_openssh(LineEnding::LF)?.as_str(),
        ))
    }

    pub fn kind(&self) -> CaKind {
        self.kind
    }

    pub fn public_key(&self) -> &PublicKey {
        self.key.public_key()
    }

    pub fn key_data(&self) -> &KeyData {
        self.key.public_key().key_data()
    }

    /// `SHA256:…`, as `keel ssh-ca show` prints it and the enrollment token embeds it.
    pub fn fingerprint(&self) -> String {
        fingerprint(self.key_data())
    }

    /// The public key line (`ssh-ed25519 AAAA… keel-<kind>-ca`).
    pub fn public_line(&self) -> String {
        let mut pk = self.key.public_key().clone();
        pk.set_comment(format!("keel-{}-ca", self.kind.as_str()));
        pk.to_openssh().expect("encode an Ed25519 public key")
    }

    /// The `known_hosts` line that trusts this host CA for `host_pattern`.
    pub fn cert_authority_line(&self, host_pattern: &str) -> String {
        format!("@cert-authority {host_pattern} {}", self.public_line())
    }

    fn require(&self, want: CaKind) -> Result<(), CaError> {
        if self.kind == want {
            Ok(())
        } else {
            Err(CaError::WrongKind {
                have: self.kind.as_str(),
                want: want.as_str(),
            })
        }
    }

    /// Issue a runner certificate. The serial is random and non-zero; the server records it and
    /// re-issues on the (practically impossible) collision.
    pub fn issue_runner_cert(&self, req: &RunnerCertRequest<'_>) -> Result<Certificate, CaError> {
        self.require(CaKind::Runner)?;
        require_ed25519(req.public_key)?;
        if req.runner_id.is_empty() || req.inventory_name.is_empty() {
            return Err(CaError::Request(
                "runner id and inventory name are required",
            ));
        }
        if req.ttl.is_zero() {
            return Err(CaError::Request("ttl must be positive"));
        }
        let allowed: BTreeSet<&str> = req.allowed_profiles.iter().map(String::as_str).collect();
        let profiles: BTreeSet<&str> = req
            .assigned_profiles
            .iter()
            .map(String::as_str)
            .filter(|p| allowed.contains(p))
            .collect();

        let now = unix(req.now)?;
        let mut rng = rand::rng();
        let mut b = Builder::new_with_random_nonce(
            &mut rng,
            req.public_key.clone(),
            now,
            now + req.ttl.as_secs(),
        )?;
        b.serial(new_serial(&mut rng))?;
        b.cert_type(CertType::User)?;
        b.key_id(format!("{RUNNER_KEY_ID_PREFIX}{}", req.runner_id))?;
        b.valid_principal(format!("{HOST_PRINCIPAL_PREFIX}{}", req.inventory_name))?;
        for p in profiles {
            b.valid_principal(format!("{PROFILE_PRINCIPAL_PREFIX}{p}"))?;
        }
        Ok(b.sign(&self.key)?)
    }

    /// Issue the server's host certificate.
    pub fn issue_host_cert(&self, req: &HostCertRequest<'_>) -> Result<Certificate, CaError> {
        self.require(CaKind::Host)?;
        require_ed25519(req.public_key)?;
        let Some(first) = req.host_names.first() else {
            return Err(CaError::Request(
                "a host certificate needs at least one name",
            ));
        };
        let now = unix(req.now)?;
        let mut rng = rand::rng();
        let after = now.saturating_sub(HOST_CERT_BACKDATE.as_secs());
        let mut b = Builder::new_with_random_nonce(
            &mut rng,
            req.public_key.clone(),
            after,
            now + req.ttl.as_secs(),
        )?;
        b.serial(new_serial(&mut rng))?;
        b.cert_type(CertType::Host)?;
        b.key_id(format!("{HOST_KEY_ID_PREFIX}{first}"))?;
        for name in req.host_names {
            b.valid_principal(name.clone())?;
        }
        Ok(b.sign(&self.key)?)
    }
}

fn new_serial(rng: &mut impl rand::Rng) -> u64 {
    loop {
        let s: u64 = rng.random();
        if s != 0 {
            return s;
        }
    }
}

/// A CA a verifier trusts. A retiring CA carries `not_after`: the expiry of the last
/// certificate it issued, after which it is no longer trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedCa {
    pub key: KeyData,
    /// Unix seconds; `None` for the active CA.
    pub not_after: Option<u64>,
}

impl TrustedCa {
    pub fn active(key: KeyData) -> Self {
        TrustedCa {
            key,
            not_after: None,
        }
    }

    pub fn retiring(key: KeyData, not_after: u64) -> Self {
        TrustedCa {
            key,
            not_after: Some(not_after),
        }
    }

    fn trusted_at(&self, now: u64) -> bool {
        self.not_after.is_none_or(|t| now < t)
    }
}

/// Why a certificate was refused. Carries no certificate contents beyond the reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CertRefusal {
    #[error("wrong certificate type")]
    WrongType,
    #[error("not an Ed25519 key")]
    NotEd25519,
    #[error("signed by an untrusted CA")]
    UntrustedCa,
    #[error("signed by a retired CA")]
    RetiredCa,
    #[error("bad signature")]
    BadSignature,
    #[error("not yet valid")]
    NotYetValid,
    #[error("expired")]
    Expired,
    #[error("unexpected critical options")]
    CriticalOptions,
    #[error("unexpected extensions")]
    Extensions,
    #[error("bad key id")]
    BadKeyId,
    #[error("bad principals")]
    BadPrincipals,
    #[error("revoked")]
    Revoked,
}

/// What a valid runner certificate says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerIdentity {
    pub runner_id: String,
    pub inventory_name: String,
    /// Sorted.
    pub profiles: Vec<String>,
    pub serial: u64,
    /// Unix seconds.
    pub valid_before: u64,
    /// `SHA256:…` of the CA that signed it.
    pub ca_fingerprint: String,
}

fn check_common(
    cas: &[TrustedCa],
    cert: &Certificate,
    want: CertType,
    now: u64,
) -> Result<(), CertRefusal> {
    if cert.cert_type() != want {
        return Err(CertRefusal::WrongType);
    }
    if cert.public_key().algorithm() != Algorithm::Ed25519 {
        return Err(CertRefusal::NotEd25519);
    }
    let Some(ca) = cas.iter().find(|ca| &ca.key == cert.signature_key()) else {
        return Err(CertRefusal::UntrustedCa);
    };
    if !ca.trusted_at(now) {
        return Err(CertRefusal::RetiredCa);
    }
    if now < cert.valid_after() {
        return Err(CertRefusal::NotYetValid);
    }
    if now >= cert.valid_before() {
        return Err(CertRefusal::Expired);
    }
    let fp = ca.key.fingerprint(HashAlg::Sha256);
    cert.validate_at(now, [&fp])
        .map_err(|_| CertRefusal::BadSignature)?;
    if !cert.critical_options().is_empty() {
        return Err(CertRefusal::CriticalOptions);
    }
    Ok(())
}

/// The server's check of runner certificates.
#[derive(Debug, Clone, Default)]
pub struct RunnerCertVerifier {
    pub cas: Vec<TrustedCa>,
    pub krl: Krl,
}

impl RunnerCertVerifier {
    pub fn new(cas: Vec<TrustedCa>, krl: Krl) -> Self {
        RunnerCertVerifier { cas, krl }
    }

    pub fn verify(
        &self,
        cert: &Certificate,
        now: SystemTime,
    ) -> Result<RunnerIdentity, CertRefusal> {
        let now = unix(now).map_err(|_| CertRefusal::NotYetValid)?;
        check_common(&self.cas, cert, CertType::User, now)?;
        if !cert.extensions().is_empty() {
            return Err(CertRefusal::Extensions);
        }
        let runner_id = cert
            .key_id()
            .strip_prefix(RUNNER_KEY_ID_PREFIX)
            .filter(|id| !id.is_empty())
            .ok_or(CertRefusal::BadKeyId)?;
        // An empty principal list means "any principal" to OpenSSH; never accept it.
        let mut host = None;
        let mut profiles = Vec::new();
        for p in cert.valid_principals() {
            if let Some(name) = p
                .strip_prefix(HOST_PRINCIPAL_PREFIX)
                .filter(|n| !n.is_empty())
            {
                if host.replace(name).is_some() {
                    return Err(CertRefusal::BadPrincipals);
                }
            } else if let Some(profile) = p
                .strip_prefix(PROFILE_PRINCIPAL_PREFIX)
                .filter(|n| !n.is_empty())
            {
                profiles.push(profile.to_owned());
            } else {
                return Err(CertRefusal::BadPrincipals);
            }
        }
        let inventory_name = host.ok_or(CertRefusal::BadPrincipals)?.to_owned();
        if self.krl.is_certificate_revoked(cert) {
            return Err(CertRefusal::Revoked);
        }
        profiles.sort();
        profiles.dedup();
        Ok(RunnerIdentity {
            runner_id: runner_id.to_owned(),
            inventory_name,
            profiles,
            serial: cert.serial(),
            valid_before: cert.valid_before(),
            ca_fingerprint: fingerprint(cert.signature_key()),
        })
    }
}

/// The runner's check of the server's host certificate.
#[derive(Debug, Clone, Default)]
pub struct HostCertVerifier {
    pub cas: Vec<TrustedCa>,
    /// The name the runner connected to; it must be one of the certificate's principals.
    pub host_name: String,
}

impl HostCertVerifier {
    pub fn new(cas: Vec<TrustedCa>, host_name: impl Into<String>) -> Self {
        HostCertVerifier {
            cas,
            host_name: host_name.into(),
        }
    }

    pub fn verify(&self, cert: &Certificate, now: SystemTime) -> Result<(), CertRefusal> {
        let now = unix(now).map_err(|_| CertRefusal::NotYetValid)?;
        check_common(&self.cas, cert, CertType::Host, now)?;
        if !cert.valid_principals().iter().any(|p| p == &self.host_name) {
            return Err(CertRefusal::BadPrincipals);
        }
        Ok(())
    }
}
