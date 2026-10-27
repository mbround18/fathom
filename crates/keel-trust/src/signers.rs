//! The host's trusted signers file (`/etc/keel-runner/trust/signers`).
//!
//! SSH `allowed_signers` lines first, then ASCII-armored OpenPGP public keys. Comment (`#`) and
//! blank lines are skipped. The only `allowed_signers` option accepted is `namespaces="…"`, and it
//! must include `git`; any other option (`cert-authority`, `valid-after`, …) makes the file
//! invalid, because ignoring it would trust more than the file says.
//!
//! Signer ids, as recorded in verdicts and the ledger: `SHA256:<base64>` for SSH keys and
//! `openpgp:<PRIMARY FINGERPRINT, upper-case hex>` for OpenPGP keys and their subkeys.

use pgp::composed::{Deserializable, SignedPublicKey};
use pgp::types::KeyDetails;
use ssh_key::{HashAlg, PublicKey};

/// A trusted SSH key.
#[derive(Debug, Clone)]
pub struct SshSigner {
    /// `SHA256:…`.
    pub id: String,
    /// The principals from the line (informational; Keel uses `keel`).
    pub principals: Vec<String>,
    /// The key.
    pub key: PublicKey,
}

/// A trusted OpenPGP certificate.
#[derive(Debug, Clone)]
pub struct PgpSigner {
    /// `openpgp:<PRIMARY FINGERPRINT>`.
    pub id: String,
    /// The certificate with its self-signatures, bindings and revocations.
    pub cert: SignedPublicKey,
}

/// The parsed signers file.
#[derive(Debug, Clone, Default)]
pub struct Signers {
    /// SSH keys, in file order.
    pub ssh: Vec<SshSigner>,
    /// OpenPGP certificates, in file order.
    pub pgp: Vec<PgpSigner>,
}

/// The signers file is invalid. The caller refuses signed runs until it is fixed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SignersError {
    /// An `allowed_signers` line could not be parsed, or has an option other than `namespaces`.
    #[error("signers line {0}: invalid allowed_signers entry")]
    SshLine(usize),
    /// An `allowed_signers` line restricts the key to namespaces that do not include `git`.
    #[error("signers line {0}: namespaces do not include git")]
    NotGit(usize),
    /// An OpenPGP block is not a valid public key, or is not terminated.
    #[error("signers line {0}: invalid OpenPGP public key block")]
    PgpBlock(usize),
    /// An `allowed_signers` line after the first OpenPGP block.
    #[error("signers line {0}: allowed_signers lines must come before OpenPGP keys")]
    Order(usize),
}

const PGP_BEGIN: &str = "-----BEGIN PGP PUBLIC KEY BLOCK-----";
const PGP_END: &str = "-----END PGP PUBLIC KEY BLOCK-----";

/// `openpgp:` and the upper-case hex fingerprint of a primary key.
pub fn pgp_signer_id(cert: &SignedPublicKey) -> String {
    format!(
        "openpgp:{}",
        hex_upper(cert.primary_key.fingerprint().as_bytes())
    )
}

fn hex_upper(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

impl Signers {
    /// Parses the file.
    pub fn parse(text: &str) -> Result<Self, SignersError> {
        let mut out = Self::default();
        let mut lines = text.lines().enumerate().peekable();
        while let Some((i, line)) = lines.next() {
            let n = i + 1;
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if trimmed == PGP_BEGIN {
                let mut block = String::from(PGP_BEGIN);
                block.push('\n');
                let mut ended = false;
                for (_, l) in lines.by_ref() {
                    block.push_str(l.trim_end());
                    block.push('\n');
                    if l.trim() == PGP_END {
                        ended = true;
                        break;
                    }
                }
                if !ended {
                    return Err(SignersError::PgpBlock(n));
                }
                let (cert, _) =
                    SignedPublicKey::from_string(&block).map_err(|_| SignersError::PgpBlock(n))?;
                out.pgp.push(PgpSigner {
                    id: pgp_signer_id(&cert),
                    cert,
                });
                continue;
            }
            if !out.pgp.is_empty() {
                return Err(SignersError::Order(n));
            }
            out.ssh.push(parse_allowed_signer(trimmed, n)?);
        }
        Ok(out)
    }

    /// Whether the file trusts nobody.
    pub fn is_empty(&self) -> bool {
        self.ssh.is_empty() && self.pgp.is_empty()
    }
}

fn parse_allowed_signer(line: &str, n: usize) -> Result<SshSigner, SignersError> {
    let bad = SignersError::SshLine(n);
    let (principals, rest) = line.split_once(char::is_whitespace).ok_or(bad.clone())?;
    let rest = rest.trim_start();
    let key_text = match PublicKey::from_openssh(rest) {
        Ok(_) => rest,
        Err(_) => {
            // `[options] keytype base64 [comment]`, where the only option is namespaces="…".
            let (options, key) = rest.split_once(char::is_whitespace).ok_or(bad.clone())?;
            let list = options
                .strip_prefix("namespaces=\"")
                .and_then(|s| s.strip_suffix('"'))
                .ok_or(bad.clone())?;
            if list.contains('"') {
                return Err(bad);
            }
            if !list.split(',').any(|ns| ns == "git") {
                return Err(SignersError::NotGit(n));
            }
            key.trim_start()
        }
    };
    let key = PublicKey::from_openssh(key_text).map_err(|_| bad.clone())?;
    let principals: Vec<String> = principals.split(',').map(str::to_owned).collect();
    if principals.iter().any(String::is_empty) {
        return Err(bad);
    }
    Ok(SshSigner {
        id: key.fingerprint(HashAlg::Sha256).to_string(),
        principals,
        key,
    })
}
