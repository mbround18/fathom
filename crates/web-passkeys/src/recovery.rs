//! Recovery codes: a person who lost every passkey proves it with a code only the host operator
//! could issue (spec 054 R12, reused by Keel).
//!
//! - A code is 10 characters of Crockford base32 (50 bits), shown once and never stored: only
//!   its SHA-256 is kept, in a [`RecoveryRecord`].
//! - It expires after [`RECOVERY_TTL_SECS`] and works once. A new code replaces the old one.
//! - Redeeming it ([`redeem`]) is single use even under concurrency: the store's
//!   [`PasskeyStore::consume_recovery`] is a compare-and-set.
//!
//! Codes are never logged or audited.

use rand::TryRng as _;
use rand::rngs::SysRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::store::PasskeyStore;

/// Characters in a recovery code.
pub const RECOVERY_CODE_LEN: usize = 10;
/// How long a recovery code stays valid, in seconds (15 minutes).
pub const RECOVERY_TTL_SECS: i64 = 15 * 60;

/// Crockford base32: no I, L, O or U, so codes survive being read aloud.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A freshly issued code. The plaintext is for showing to the operator once; [`Debug`] does not
/// print it.
pub struct IssuedRecoveryCode {
    /// The code to show once.
    pub code: String,
    /// What to store.
    pub record: RecoveryRecord,
}

impl std::fmt::Debug for IssuedRecoveryCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IssuedRecoveryCode")
            .field("code", &"<redacted>")
            .field("record", &self.record)
            .finish()
    }
}

/// The stored form of a recovery code: at most one per user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryRecord {
    /// The user the code recovers.
    pub user: String,
    /// Lowercase hex SHA-256 of the normalized code.
    pub code_sha256: String,
    /// Unix seconds after which the code no longer works.
    pub expires: i64,
    /// Set once the code has been redeemed.
    pub used: bool,
}

/// Why issuing or redeeming a code failed.
#[derive(Debug, thiserror::Error)]
pub enum RecoveryError<E: std::error::Error + 'static> {
    /// The operating system's random number generator is unavailable.
    #[error("random number generator unavailable")]
    Rng,
    /// The code is wrong, used or expired, or the user has none. Deliberately one variant, so a
    /// caller cannot tell which.
    #[error("recovery code is wrong, used or expired")]
    Invalid,
    /// The store failed.
    #[error("passkey store: {0}")]
    Store(#[source] E),
}

/// Upper-cases a code and drops spaces and dashes, so `abcd-efgh-jk` and `ABCDEFGHJK` match.
/// Crockford's look-alikes are mapped: `O` to `0`, `I` and `L` to `1`.
pub fn normalize(code: &str) -> String {
    code.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        })
        .collect()
}

/// Lowercase hex SHA-256 of the normalized code.
pub fn hash_code(code: &str) -> String {
    hex(&Sha256::digest(normalize(code).as_bytes()))
}

/// Generates a code for `user`, valid until `now + RECOVERY_TTL_SECS`, with the given randomness.
/// Use [`issue`] in production; this exists so tests are deterministic.
pub fn issue_from_entropy(
    user: &str,
    now: i64,
    entropy: [u8; RECOVERY_CODE_LEN],
) -> IssuedRecoveryCode {
    // 32 symbols divide 256, so taking 5 bits per byte is unbiased.
    let code: String = entropy
        .iter()
        .map(|b| ALPHABET[usize::from(b & 0x1f)] as char)
        .collect();
    let record = RecoveryRecord {
        user: user.to_owned(),
        code_sha256: hash_code(&code),
        expires: now + RECOVERY_TTL_SECS,
        used: false,
    };
    IssuedRecoveryCode { code, record }
}

/// Generates a code for `user` from the operating system's random number generator, and stores
/// its hash (replacing any previous code). Returns the code to show once.
pub async fn issue<S: PasskeyStore>(
    store: &S,
    user: &str,
    now: i64,
) -> Result<IssuedRecoveryCode, RecoveryError<S::Error>> {
    let mut entropy = [0u8; RECOVERY_CODE_LEN];
    SysRng
        .try_fill_bytes(&mut entropy)
        .map_err(|_| RecoveryError::Rng)?;
    let issued = issue_from_entropy(user, now, entropy);
    store
        .put_recovery(issued.record.clone())
        .await
        .map_err(RecoveryError::Store)?;
    Ok(issued)
}

/// Checks `code` against a record without consuming it: the hash matches (constant time), it is
/// unused, and `now` is before its expiry.
pub fn matches(record: &RecoveryRecord, code: &str, now: i64) -> bool {
    let presented = hash_code(code);
    let same = presented.as_bytes().ct_eq(record.code_sha256.as_bytes());
    bool::from(same) && !record.used && now < record.expires
}

/// Redeems `user`'s recovery code: it must match, be unused and unexpired, and the store must
/// mark it used atomically. On success the caller removes the person's passkeys and lets the
/// session enrol (and audits both). A second redemption of the same code fails.
pub async fn redeem<S: PasskeyStore>(
    store: &S,
    user: &str,
    code: &str,
    now: i64,
) -> Result<(), RecoveryError<S::Error>> {
    let Some(record) = store.recovery(user).await.map_err(RecoveryError::Store)? else {
        return Err(RecoveryError::Invalid);
    };
    if record.user != user || !matches(&record, code, now) {
        return Err(RecoveryError::Invalid);
    }
    if store
        .consume_recovery(user, &record.code_sha256)
        .await
        .map_err(RecoveryError::Store)?
    {
        Ok(())
    } else {
        Err(RecoveryError::Invalid)
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[usize::from(b >> 4)] as char);
        s.push(HEX[usize::from(b & 0x0f)] as char);
    }
    s
}
