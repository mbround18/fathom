//! A pure WebAuthn assertion check, with no relying-party state.
//!
//! [`verify_assertion`] is what a party that only holds a pinned public key can check: Keel's
//! hosts verify override approvals with it (spec 055, contracts/signed-mode.md, "Override
//! verification on the host", steps 3 to 5), without the ceremony state or the sign counter that
//! only the server has. Keel's server calls it too, through
//! [`crate::ceremony::Ceremonies::verify_bound_assertion`].
//!
//! What it checks, in order, failing on the first problem:
//!
//! 1. `client_data_json`: `type == "webauthn.get"`, `challenge` (base64url) equals the expected
//!    challenge, `origin` equals the expected origin (exactly, or with one trailing `/`), and
//!    the assertion is not cross-origin.
//! 2. `authenticator_data`: `rpIdHash == SHA-256(rp_id)`, flags UP and UV set, no attested
//!    credential data, and no trailing bytes unless the extension flag is set.
//! 3. The signature, ES256 (DER-encoded ECDSA P-256 with SHA-256) or EdDSA (Ed25519), over
//!    `authenticator_data ‖ SHA-256(client_data_json)`, with the COSE key's own `alg` matching
//!    the pinned one.
//!
//! Nothing here logs: challenges and assertions are never written anywhere.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Authenticator data flag: user present.
pub const FLAG_UP: u8 = 0x01;
/// Authenticator data flag: user verified.
pub const FLAG_UV: u8 = 0x04;
/// Authenticator data flag: attested credential data included (registration only).
pub const FLAG_AT: u8 = 0x40;
/// Authenticator data flag: extension data included.
pub const FLAG_ED: u8 = 0x80;

/// Length of the fixed part of authenticator data: `rpIdHash` (32), flags (1), counter (4).
const AUTH_DATA_MIN: usize = 37;

/// The two signature algorithms a pinned approver key may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum CoseAlg {
    /// ECDSA over P-256 with SHA-256 (COSE `-7`).
    #[serde(rename = "ES256")]
    Es256,
    /// Ed25519 (COSE `-8`).
    #[serde(rename = "EdDSA")]
    EdDsa,
}

impl CoseAlg {
    /// The COSE algorithm identifier.
    pub fn cose_id(self) -> i64 {
        match self {
            CoseAlg::Es256 => -7,
            CoseAlg::EdDsa => -8,
        }
    }

    /// The algorithm for a COSE identifier, if it is one of the two supported.
    pub fn from_cose_id(id: i64) -> Option<Self> {
        match id {
            -7 => Some(CoseAlg::Es256),
            -8 => Some(CoseAlg::EdDsa),
            _ => None,
        }
    }
}

/// Why an assertion did not verify. Each variant names the check that failed; none carries
/// assertion material.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    /// `client_data_json` is not the JSON object WebAuthn defines.
    #[error("client data is not valid JSON client data")]
    ClientDataMalformed,
    /// `type` is not `webauthn.get`.
    #[error("client data type is not webauthn.get")]
    WrongType,
    /// The challenge differs from the expected one, or is not base64url.
    #[error("challenge does not match")]
    WrongChallenge,
    /// The origin differs from the expected one.
    #[error("origin does not match")]
    WrongOrigin,
    /// The assertion was made in a cross-origin frame.
    #[error("cross-origin assertion")]
    CrossOrigin,
    /// The authenticator data is too short, carries attested credential data, or has trailing
    /// bytes without the extension flag.
    #[error("authenticator data is malformed")]
    AuthenticatorDataMalformed,
    /// `rpIdHash` is not SHA-256 of the expected relying-party id.
    #[error("relying party id hash does not match")]
    WrongRpId,
    /// The user-present flag is not set.
    #[error("user presence flag not set")]
    UserNotPresent,
    /// The user-verified flag is not set.
    #[error("user verification flag not set")]
    UserNotVerified,
    /// The public key is not a COSE key of the pinned algorithm.
    #[error("credential public key is not a valid COSE key for the algorithm")]
    BadPublicKey,
    /// The signature does not verify.
    #[error("signature does not verify")]
    BadSignature,
}

#[derive(Deserialize)]
struct ClientData {
    #[serde(rename = "type")]
    type_: String,
    challenge: String,
    origin: String,
    #[serde(rename = "crossOrigin", default)]
    cross_origin: Option<bool>,
}

/// The parsed fixed part of authenticator data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthenticatorData {
    /// SHA-256 of the relying-party id the authenticator used.
    pub rp_id_hash: [u8; 32],
    /// The flags byte.
    pub flags: u8,
    /// The signature counter (`0` for authenticators that keep none).
    pub sign_count: u32,
}

impl AuthenticatorData {
    /// Parses the fixed part of an assertion's authenticator data. Attested credential data (a
    /// registration-only field) and trailing bytes without the extension flag are refused.
    pub fn parse(bytes: &[u8]) -> Result<Self, VerifyError> {
        if bytes.len() < AUTH_DATA_MIN {
            return Err(VerifyError::AuthenticatorDataMalformed);
        }
        let flags = bytes[32];
        if flags & FLAG_AT != 0 {
            return Err(VerifyError::AuthenticatorDataMalformed);
        }
        if flags & FLAG_ED == 0 && bytes.len() != AUTH_DATA_MIN {
            return Err(VerifyError::AuthenticatorDataMalformed);
        }
        let mut rp_id_hash = [0u8; 32];
        rp_id_hash.copy_from_slice(&bytes[..32]);
        let sign_count = u32::from_be_bytes([bytes[33], bytes[34], bytes[35], bytes[36]]);
        Ok(Self {
            rp_id_hash,
            flags,
            sign_count,
        })
    }
}

/// Verifies a WebAuthn assertion against a pinned public key, with no server state.
///
/// - `credential_public_key`: the credential's COSE_Key bytes, as pinned at registration.
/// - `alg`: the pinned algorithm; the COSE key's own `alg` must agree.
/// - `authenticator_data`, `client_data_json`, `signature`: the assertion, as the browser
///   returned it (signature DER for ES256, 64 raw bytes for EdDSA).
/// - `expected_challenge`: the raw challenge bytes the request carried.
/// - `origin`: the exact expected origin, e.g. `https://keel.example.org`.
/// - `rp_id`: the relying-party id, e.g. `keel.example.org`.
///
/// Returns `Ok(())` only if every check passes. The sign counter is not checked here (see the
/// module docs).
#[allow(clippy::too_many_arguments)]
pub fn verify_assertion(
    credential_public_key: &[u8],
    alg: CoseAlg,
    authenticator_data: &[u8],
    client_data_json: &[u8],
    signature: &[u8],
    expected_challenge: &[u8],
    origin: &str,
    rp_id: &str,
) -> Result<(), VerifyError> {
    // 1. Client data.
    let client: ClientData =
        serde_json::from_slice(client_data_json).map_err(|_| VerifyError::ClientDataMalformed)?;
    if client.type_ != "webauthn.get" {
        return Err(VerifyError::WrongType);
    }
    let challenge = URL_SAFE_NO_PAD
        .decode(client.challenge.as_bytes())
        .map_err(|_| VerifyError::WrongChallenge)?;
    if challenge.len() != expected_challenge.len()
        || !bool::from(challenge.as_slice().ct_eq(expected_challenge))
    {
        return Err(VerifyError::WrongChallenge);
    }
    if !same_origin(&client.origin, origin) {
        return Err(VerifyError::WrongOrigin);
    }
    if client.cross_origin == Some(true) {
        return Err(VerifyError::CrossOrigin);
    }

    // 2. Authenticator data.
    let auth = AuthenticatorData::parse(authenticator_data)?;
    let expected_rp: [u8; 32] = Sha256::digest(rp_id.as_bytes()).into();
    if auth.rp_id_hash != expected_rp {
        return Err(VerifyError::WrongRpId);
    }
    if auth.flags & FLAG_UP == 0 {
        return Err(VerifyError::UserNotPresent);
    }
    if auth.flags & FLAG_UV == 0 {
        return Err(VerifyError::UserNotVerified);
    }

    // 3. Signature over authenticator_data ‖ SHA-256(client_data_json).
    let key = crate::cose::CoseKey::decode(credential_public_key)
        .map_err(|_| VerifyError::BadPublicKey)?;
    if key.alg() != alg {
        return Err(VerifyError::BadPublicKey);
    }
    let mut signed = Vec::with_capacity(authenticator_data.len() + 32);
    signed.extend_from_slice(authenticator_data);
    signed.extend_from_slice(&Sha256::digest(client_data_json));
    key.verify(&signed, signature)
}

/// The client's origin is the expected one: identical, or identical plus one trailing `/` (an
/// empty path, so still the same scheme, host and port; some clients serialize it that way).
fn same_origin(client: &str, expected: &str) -> bool {
    let expected = expected.strip_suffix('/').unwrap_or(expected);
    client == expected || client.strip_suffix('/') == Some(expected)
}
