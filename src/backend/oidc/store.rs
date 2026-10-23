//! Credential store wrapper (033 T008, research R2).
//!
//! Refresh tokens, cached ID tokens and IdP admin API keys live in the OS keyring
//! (Secret Service / Credential Manager / Keychain), never in `~/.fathom/`. When no keyring is
//! usable (headless Linux, locked/absent Secret Service) values are held in process memory
//! only and [`CredentialStore::persistence`] says so, so the UI can tell the operator. There is
//! deliberately no plaintext file fallback.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// A secret string. `Debug` is redacted and there is no `Display`/`Serialize`, so it cannot
/// leak through `tracing`, protocol JSON or config by accident. Read it only via `expose`.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The raw value. Call only at the point of use (an HTTP header, a token request).
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}

/// What is stored, per issuer. Maps to keyring `service` / `user`:
/// `fathom.oidc.<issuer_id>` + `refresh_token` | `id_token`, `fathom.rauthy.<issuer_id>` +
/// `admin_api_key` (data-model.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
    RefreshToken,
    IdToken,
    AdminApiKey,
}

impl SecretKind {
    fn service(self, issuer_id: &str) -> String {
        match self {
            Self::RefreshToken | Self::IdToken => format!("fathom.oidc.{issuer_id}"),
            Self::AdminApiKey => format!("fathom.rauthy.{issuer_id}"),
        }
    }

    fn user(self) -> &'static str {
        match self {
            Self::RefreshToken => "refresh_token",
            Self::IdToken => "id_token",
            Self::AdminApiKey => "admin_api_key",
        }
    }
}

/// Where values currently live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Persistence {
    /// OS keyring: survives restarts.
    Keyring,
    /// No usable keyring: process memory only; the operator signs in again after a restart.
    MemoryOnly,
}

/// Failure of a raw backend. Carries no secret material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendError(pub String);

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BackendError {}

/// Raw key/value backend over (`service`, `user`). Injectable for tests.
pub trait SecretBackend: Send + Sync {
    fn get(&self, service: &str, user: &str) -> Result<Option<String>, BackendError>;
    fn set(&self, service: &str, user: &str, value: &str) -> Result<(), BackendError>;
    /// Deleting an absent entry is not an error.
    fn delete(&self, service: &str, user: &str) -> Result<(), BackendError>;
}

/// In-process map: the fallback and the fake for tests.
#[derive(Default)]
pub struct MemoryBackend {
    map: Mutex<HashMap<(String, String), String>>,
}

impl SecretBackend for MemoryBackend {
    fn get(&self, service: &str, user: &str) -> Result<Option<String>, BackendError> {
        let map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        Ok(map.get(&(service.to_owned(), user.to_owned())).cloned())
    }

    fn set(&self, service: &str, user: &str, value: &str) -> Result<(), BackendError> {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        map.insert((service.to_owned(), user.to_owned()), value.to_owned());
        Ok(())
    }

    fn delete(&self, service: &str, user: &str) -> Result<(), BackendError> {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        map.remove(&(service.to_owned(), user.to_owned()));
        Ok(())
    }
}

/// The real OS keyring (`keyring` crate).
pub struct OsKeyringBackend;

#[cfg(feature = "os-keyring")]
impl OsKeyringBackend {
    fn entry(service: &str, user: &str) -> Result<keyring::Entry, BackendError> {
        keyring::Entry::new(service, user).map_err(|e| BackendError(e.to_string()))
    }
}

/// Built without `os-keyring`: every call fails, so the store falls back to memory.
#[cfg(not(feature = "os-keyring"))]
impl SecretBackend for OsKeyringBackend {
    fn get(&self, _: &str, _: &str) -> Result<Option<String>, BackendError> {
        Err(BackendError("built without the OS credential store".into()))
    }
    fn set(&self, _: &str, _: &str, _: &str) -> Result<(), BackendError> {
        self.get("", "").map(|_| ())
    }
    fn delete(&self, _: &str, _: &str) -> Result<(), BackendError> {
        self.get("", "").map(|_| ())
    }
}

#[cfg(feature = "os-keyring")]
impl SecretBackend for OsKeyringBackend {
    fn get(&self, service: &str, user: &str) -> Result<Option<String>, BackendError> {
        match Self::entry(service, user)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(BackendError(e.to_string())),
        }
    }

    fn set(&self, service: &str, user: &str, value: &str) -> Result<(), BackendError> {
        Self::entry(service, user)?
            .set_password(value)
            .map_err(|e| BackendError(e.to_string()))
    }

    fn delete(&self, service: &str, user: &str) -> Result<(), BackendError> {
        match Self::entry(service, user)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(BackendError(e.to_string())),
        }
    }
}

/// Typed credential store with automatic, sticky fallback to memory.
///
/// The first *backend* failure (not "no entry") flips the store to [`Persistence::MemoryOnly`]
/// for the rest of the process: values written after that point are memory-only, and it never
/// retries the keyring mid-session (so a half-working keyring cannot split one session's
/// tokens across two places).
pub struct CredentialStore {
    primary: Box<dyn SecretBackend>,
    memory: MemoryBackend,
    degraded: AtomicBool,
}

impl CredentialStore {
    pub fn new(primary: Box<dyn SecretBackend>) -> Self {
        Self {
            primary,
            memory: MemoryBackend::default(),
            degraded: AtomicBool::new(false),
        }
    }

    /// The OS keyring, falling back to memory if unavailable.
    pub fn os() -> Self {
        Self::new(Box::new(OsKeyringBackend))
    }

    /// Memory only (tests, or when the operator opts out of the keyring).
    pub fn memory_only() -> Self {
        let store = Self::new(Box::new(MemoryBackend::default()));
        store.degraded.store(true, Ordering::SeqCst);
        store
    }

    pub fn persistence(&self) -> Persistence {
        if self.degraded.load(Ordering::SeqCst) {
            Persistence::MemoryOnly
        } else {
            Persistence::Keyring
        }
    }

    fn degrade(&self, why: &BackendError) {
        if !self.degraded.swap(true, Ordering::SeqCst) {
            // The error text comes from the OS keyring and never contains the secret.
            tracing::warn!("credential store unavailable, holding secrets in memory only: {why}");
        }
    }

    fn active(&self) -> &dyn SecretBackend {
        if self.degraded.load(Ordering::SeqCst) {
            &self.memory
        } else {
            self.primary.as_ref()
        }
    }

    pub fn get(&self, kind: SecretKind, issuer_id: &str) -> Option<Secret> {
        let (service, user) = (kind.service(issuer_id), kind.user());
        match self.active().get(&service, user) {
            Ok(v) => v.map(Secret::new),
            Err(e) => {
                self.degrade(&e);
                self.memory
                    .get(&service, user)
                    .ok()
                    .flatten()
                    .map(Secret::new)
            }
        }
    }

    pub fn set(&self, kind: SecretKind, issuer_id: &str, value: &Secret) {
        let (service, user) = (kind.service(issuer_id), kind.user());
        if let Err(e) = self.active().set(&service, user, value.expose()) {
            self.degrade(&e);
            // Infallible for the memory backend.
            let _ = self.memory.set(&service, user, value.expose());
        }
    }

    /// Remove one entry from every place it may live.
    pub fn delete(&self, kind: SecretKind, issuer_id: &str) {
        let (service, user) = (kind.service(issuer_id), kind.user());
        if let Err(e) = self.active().delete(&service, user) {
            self.degrade(&e);
        }
        let _ = self.memory.delete(&service, user);
    }

    /// Sign-out / issuer removal: drop the session tokens (the admin key is kept unless
    /// `include_admin_key`).
    pub fn clear_issuer(&self, issuer_id: &str, include_admin_key: bool) {
        self.delete(SecretKind::RefreshToken, issuer_id);
        self.delete(SecretKind::IdToken, issuer_id);
        if include_admin_key {
            self.delete(SecretKind::AdminApiKey, issuer_id);
        }
    }

    pub fn has(&self, kind: SecretKind, issuer_id: &str) -> bool {
        self.get(kind, issuer_id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Fake OS keyring recording every call, optionally failing.
    #[derive(Default)]
    struct Fake {
        inner: MemoryBackend,
        fail: AtomicBool,
        calls: Mutex<Vec<String>>,
    }

    impl Fake {
        fn check(&self, op: &str, service: &str, user: &str) -> Result<(), BackendError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{op} {service} {user}"));
            if self.fail.load(Ordering::SeqCst) {
                Err(BackendError("Secret Service unavailable".into()))
            } else {
                Ok(())
            }
        }
    }

    impl SecretBackend for Arc<Fake> {
        fn get(&self, s: &str, u: &str) -> Result<Option<String>, BackendError> {
            self.check("get", s, u)?;
            self.inner.get(s, u)
        }
        fn set(&self, s: &str, u: &str, v: &str) -> Result<(), BackendError> {
            self.check("set", s, u)?;
            self.inner.set(s, u, v)
        }
        fn delete(&self, s: &str, u: &str) -> Result<(), BackendError> {
            self.check("delete", s, u)?;
            self.inner.delete(s, u)
        }
    }

    fn store() -> (CredentialStore, Arc<Fake>) {
        let fake = Arc::new(Fake::default());
        (CredentialStore::new(Box::new(fake.clone())), fake)
    }

    #[test]
    fn round_trip_uses_documented_service_and_user_names() {
        let (s, fake) = store();
        s.set(SecretKind::RefreshToken, "rauthy", &Secret::new("rt-1"));
        s.set(SecretKind::AdminApiKey, "rauthy", &Secret::new("key-1"));
        assert_eq!(
            s.get(SecretKind::RefreshToken, "rauthy").unwrap().expose(),
            "rt-1"
        );
        assert_eq!(
            s.get(SecretKind::AdminApiKey, "rauthy").unwrap().expose(),
            "key-1"
        );
        assert!(s.get(SecretKind::IdToken, "rauthy").is_none());
        assert_eq!(s.persistence(), Persistence::Keyring);
        let calls = fake.calls.lock().unwrap().join("|");
        assert!(calls.contains("set fathom.oidc.rauthy refresh_token"));
        assert!(calls.contains("set fathom.rauthy.rauthy admin_api_key"));
    }

    #[test]
    fn issuers_are_isolated() {
        let (s, _) = store();
        s.set(SecretKind::RefreshToken, "a", &Secret::new("A"));
        s.set(SecretKind::RefreshToken, "b", &Secret::new("B"));
        assert_eq!(s.get(SecretKind::RefreshToken, "a").unwrap().expose(), "A");
        s.clear_issuer("a", false);
        assert!(s.get(SecretKind::RefreshToken, "a").is_none());
        assert_eq!(s.get(SecretKind::RefreshToken, "b").unwrap().expose(), "B");
    }

    #[test]
    fn clear_issuer_keeps_admin_key_unless_asked() {
        let (s, _) = store();
        s.set(SecretKind::RefreshToken, "a", &Secret::new("rt"));
        s.set(SecretKind::IdToken, "a", &Secret::new("id"));
        s.set(SecretKind::AdminApiKey, "a", &Secret::new("k"));
        s.clear_issuer("a", false);
        assert!(!s.has(SecretKind::RefreshToken, "a"));
        assert!(!s.has(SecretKind::IdToken, "a"));
        assert!(s.has(SecretKind::AdminApiKey, "a"));
        s.clear_issuer("a", true);
        assert!(!s.has(SecretKind::AdminApiKey, "a"));
    }

    #[test]
    fn delete_of_missing_entry_is_fine() {
        let (s, _) = store();
        s.delete(SecretKind::RefreshToken, "never");
        assert_eq!(s.persistence(), Persistence::Keyring);
    }

    #[test]
    fn keyring_failure_falls_back_to_memory_and_says_so() {
        let (s, fake) = store();
        fake.fail.store(true, Ordering::SeqCst);
        s.set(SecretKind::RefreshToken, "a", &Secret::new("rt"));
        assert_eq!(s.persistence(), Persistence::MemoryOnly);
        assert_eq!(s.get(SecretKind::RefreshToken, "a").unwrap().expose(), "rt");
        // Sticky: the keyring recovering mid-session does not split the session's secrets.
        fake.fail.store(false, Ordering::SeqCst);
        let before = fake.calls.lock().unwrap().len();
        s.set(SecretKind::IdToken, "a", &Secret::new("id"));
        assert_eq!(fake.calls.lock().unwrap().len(), before);
        assert_eq!(s.persistence(), Persistence::MemoryOnly);
        assert_eq!(s.get(SecretKind::IdToken, "a").unwrap().expose(), "id");
    }

    #[test]
    fn failing_read_degrades_without_panicking() {
        let (s, fake) = store();
        fake.fail.store(true, Ordering::SeqCst);
        assert!(s.get(SecretKind::RefreshToken, "a").is_none());
        assert_eq!(s.persistence(), Persistence::MemoryOnly);
    }

    #[test]
    fn memory_only_store_never_touches_a_keyring() {
        let s = CredentialStore::memory_only();
        assert_eq!(s.persistence(), Persistence::MemoryOnly);
        s.set(SecretKind::AdminApiKey, "a", &Secret::new("k"));
        assert_eq!(s.get(SecretKind::AdminApiKey, "a").unwrap().expose(), "k");
    }

    #[test]
    fn secret_debug_is_redacted() {
        let s = Secret::new("super-secret-token");
        assert!(!format!("{s:?}").contains("super-secret-token"));
        assert!(!format!("{:?}", Some(s)).contains("super-secret-token"));
    }
}
