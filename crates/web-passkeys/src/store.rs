//! Where an application keeps passkeys and recovery codes.
//!
//! This crate has no storage of its own. Keel implements [`PasskeyStore`] over PostgreSQL
//! (apps/keel/server/src/passkeys.rs) and Depths over `<data_dir>/passkeys.json` (spec 054).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use webauthn_rs::prelude::Passkey;

use crate::cose::CoseKey;
use crate::recovery::RecoveryRecord;
use crate::verify::CoseAlg;

/// One registered passkey.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredPasskey {
    /// The credential id, base64url without padding.
    pub id: String,
    /// The person's name for it: 1 to 64 characters, trimmed, unique per user.
    pub name: String,
    /// RFC 3339 UTC.
    pub created: String,
    /// RFC 3339 UTC.
    pub last_used: Option<String>,
    /// The credential as webauthn-rs keeps it (public key, counter, flags).
    pub credential: Passkey,
}

impl StoredPasskey {
    /// The credential's public key as COSE_Key bytes and its algorithm, for pinning elsewhere
    /// (Keel's `approvers.json`). `None` if the key is not ES256 or EdDSA.
    pub fn pinned_key(&self) -> Option<(Vec<u8>, CoseAlg)> {
        let key = CoseKey::from_webauthn(self.credential.get_public_key()).ok()?;
        let alg = key.alg();
        Some((key.encode(), alg))
    }
}

/// Storage for passkeys and recovery codes. Implementations decide the backing store; every
/// method must be durable before it returns `Ok`.
#[async_trait]
pub trait PasskeyStore: Send + Sync {
    /// The store's error.
    type Error: std::error::Error + Send + Sync + 'static;

    /// The user's passkeys, in registration order.
    async fn passkeys(&self, user: &str) -> Result<Vec<StoredPasskey>, Self::Error>;

    /// Adds a passkey. Fails if the credential id is already registered (to anyone).
    async fn add_passkey(&self, user: &str, passkey: StoredPasskey) -> Result<(), Self::Error>;

    /// Replaces the stored passkey with the same id (counter and `last_used` after a use).
    async fn update_passkey(&self, user: &str, passkey: &StoredPasskey) -> Result<(), Self::Error>;

    /// Removes one passkey by credential id. Returns whether it existed.
    async fn remove_passkey(&self, user: &str, id: &str) -> Result<bool, Self::Error>;

    /// Removes all of the user's passkeys (after recovery). Returns how many were removed.
    async fn remove_all_passkeys(&self, user: &str) -> Result<usize, Self::Error>;

    /// Stores a recovery code record, replacing the user's previous one.
    async fn put_recovery(&self, record: RecoveryRecord) -> Result<(), Self::Error>;

    /// The user's recovery code record, if any.
    async fn recovery(&self, user: &str) -> Result<Option<RecoveryRecord>, Self::Error>;

    /// Atomically marks the user's record used, if it still has this hash and is unused.
    /// Returns `false` when it was already used or replaced: the caller then refuses.
    async fn consume_recovery(&self, user: &str, code_sha256: &str) -> Result<bool, Self::Error>;
}
