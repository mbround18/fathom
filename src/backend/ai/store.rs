//! Credential storage for the AI-assist surface (038-sdk-login-ollama, US2).
//!
//! Secrets live only in the OS credential store (service `fathom`, account
//! `ai-assist/<provider>`). There is **no silent plaintext fallback** (FR-006): when the OS
//! store is unavailable, saving is refused unless the operator explicitly opted in to the
//! labelled 0600 file store. A *locked* store never falls back, even with the opt-in - the
//! operator is told the store is locked.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use secrecy::{ExposeSecret, SecretString};

use crate::protocol::v1::{BlockedReason, Provider};

pub(crate) const SERVICE: &str = "fathom";

pub(crate) fn account_name(provider: Provider) -> String {
    format!("ai-assist/{}", provider.as_str())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StoreError {
    /// The store exists but is locked / access was denied (e.g. keychain prompt dismissed).
    Locked(String),
    /// No usable store (e.g. headless Linux without Secret Service).
    Unavailable(String),
    Other(String),
}

impl StoreError {
    pub(crate) fn blocked_reason(&self) -> Option<BlockedReason> {
        match self {
            StoreError::Locked(_) => Some(BlockedReason::StoreLocked),
            StoreError::Unavailable(_) => Some(BlockedReason::StoreUnavailable),
            StoreError::Other(_) => None,
        }
    }

    pub(crate) fn code(&self) -> &'static str {
        match self {
            StoreError::Locked(_) => "store_locked",
            StoreError::Unavailable(_) => "store_unavailable",
            StoreError::Other(_) => "store_error",
        }
    }

    pub(crate) fn message(&self) -> &str {
        match self {
            StoreError::Locked(m) | StoreError::Unavailable(m) | StoreError::Other(m) => m,
        }
    }

    pub(crate) fn hint(&self) -> &'static str {
        match self {
            StoreError::Locked(_) => {
                "Unlock your OS credential store (keychain / Secret Service / Credential Manager) and try again."
            }
            StoreError::Unavailable(_) => {
                "No OS credential store is available. Start a Secret Service provider (e.g. gnome-keyring), or explicitly opt in to the 0600 file store in Settings -> AI Assist."
            }
            StoreError::Other(_) => "Check the message above and try again.",
        }
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message())
    }
}

impl std::error::Error for StoreError {}

/// One secret store. Implemented by the OS keyring, the opt-in file store, and (in tests)
/// an in-memory double.
pub(crate) trait SecretBackend: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<SecretString>, StoreError>;
    fn set(&self, account: &str, secret: &str) -> Result<(), StoreError>;
    /// Deleting a missing entry is `Ok`.
    fn delete(&self, account: &str) -> Result<(), StoreError>;
}

// ---------------------------------------------------------------------------- OS keyring

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub(crate) struct KeyringBackend;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn map_keyring_err(err: keyring::Error) -> StoreError {
    match err {
        keyring::Error::NoStorageAccess(e) => StoreError::Locked(e.to_string()),
        keyring::Error::PlatformFailure(e) => StoreError::Unavailable(e.to_string()),
        other => StoreError::Other(other.to_string()),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl SecretBackend for KeyringBackend {
    fn get(&self, account: &str) -> Result<Option<SecretString>, StoreError> {
        let entry = keyring::Entry::new(SERVICE, account).map_err(map_keyring_err)?;
        match entry.get_password() {
            Ok(pw) => Ok(Some(SecretString::from(pw))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(map_keyring_err(e)),
        }
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), StoreError> {
        let entry = keyring::Entry::new(SERVICE, account).map_err(map_keyring_err)?;
        entry.set_password(secret).map_err(map_keyring_err)
    }

    fn delete(&self, account: &str) -> Result<(), StoreError> {
        let entry = keyring::Entry::new(SERVICE, account).map_err(map_keyring_err)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(map_keyring_err(e)),
        }
    }
}

/// Platforms without a supported OS store: always unavailable (never a silent fallback).
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) struct KeyringBackend;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
impl SecretBackend for KeyringBackend {
    fn get(&self, _: &str) -> Result<Option<SecretString>, StoreError> {
        Err(StoreError::Unavailable(
            "no OS credential store on this platform".into(),
        ))
    }
    fn set(&self, _: &str, _: &str) -> Result<(), StoreError> {
        self.get("").map(|_| ())
    }
    fn delete(&self, _: &str) -> Result<(), StoreError> {
        self.get("").map(|_| ())
    }
}

// ------------------------------------------------------------------- opt-in 0600 file

/// The explicitly opted-in, labelled plaintext file (`~/.fathom/ai-credentials.json`,
/// mode 0600 on unix). Only ever used when `file_store_optin` is set.
pub(crate) struct FileBackend {
    path: PathBuf,
}

impl FileBackend {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub(crate) fn default_path() -> Option<PathBuf> {
        crate::backend::persistence::fathom_config_dir()
            .ok()
            .map(|d| d.join("ai-credentials.json"))
    }

    fn read_map(&self) -> Result<BTreeMap<String, String>, StoreError> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| {
                StoreError::Other(format!("{} is not valid: {e}", self.path.display()))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(StoreError::Other(format!("{}: {e}", self.path.display()))),
        }
    }

    fn write_map(&self, map: &BTreeMap<String, String>) -> Result<(), StoreError> {
        let io = |e: std::io::Error| StoreError::Other(format!("{}: {e}", self.path.display()));
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let body =
            serde_json::to_string_pretty(map).map_err(|e| StoreError::Other(e.to_string()))?;
        {
            use std::io::Write as _;
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                opts.mode(0o600);
            }
            let mut file = opts.open(&tmp).map_err(io)?;
            file.write_all(body.as_bytes()).map_err(io)?;
            file.sync_all().map_err(io)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).map_err(io)?;
        }
        std::fs::rename(&tmp, &self.path).map_err(io)
    }
}

impl SecretBackend for FileBackend {
    fn get(&self, account: &str) -> Result<Option<SecretString>, StoreError> {
        Ok(self.read_map()?.remove(account).map(SecretString::from))
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), StoreError> {
        let mut map = self.read_map()?;
        map.insert(account.to_owned(), secret.to_owned());
        self.write_map(&map)
    }

    fn delete(&self, account: &str) -> Result<(), StoreError> {
        let mut map = self.read_map()?;
        if map.remove(account).is_some() {
            self.write_map(&map)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------- composite

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoredIn {
    OsKeychain,
    FileOptin,
}

pub(crate) struct CredentialStore {
    primary: Box<dyn SecretBackend>,
    file: Box<dyn SecretBackend>,
    file_optin: AtomicBool,
}

impl CredentialStore {
    pub(crate) fn new(primary: Box<dyn SecretBackend>, file: Box<dyn SecretBackend>) -> Self {
        Self {
            primary,
            file,
            file_optin: AtomicBool::new(false),
        }
    }

    /// The production store: OS keyring plus the (dormant unless opted in) file backend.
    pub(crate) fn system() -> Self {
        let path =
            FileBackend::default_path().unwrap_or_else(|| PathBuf::from("ai-credentials.json"));
        Self::new(Box::new(KeyringBackend), Box::new(FileBackend::new(path)))
    }

    pub(crate) fn set_file_optin(&self, optin: bool) {
        self.file_optin.store(optin, Ordering::SeqCst);
    }

    pub(crate) fn file_optin(&self) -> bool {
        self.file_optin.load(Ordering::SeqCst)
    }

    /// Save a secret. Returns where it went. Refuses (never silently falls back) when the OS
    /// store is unavailable and the operator has not opted in; never falls back when locked.
    pub(crate) fn save(
        &self,
        provider: Provider,
        secret: &SecretString,
    ) -> Result<StoredIn, StoreError> {
        let account = account_name(provider);
        match self.primary.set(&account, secret.expose_secret()) {
            Ok(()) => Ok(StoredIn::OsKeychain),
            Err(StoreError::Unavailable(msg)) => {
                if self.file_optin() {
                    self.file.set(&account, secret.expose_secret())?;
                    Ok(StoredIn::FileOptin)
                } else {
                    Err(StoreError::Unavailable(msg))
                }
            }
            Err(other) => Err(other),
        }
    }

    pub(crate) fn get(&self, provider: Provider) -> Result<Option<SecretString>, StoreError> {
        let account = account_name(provider);
        match self.primary.get(&account) {
            Ok(Some(s)) => Ok(Some(s)),
            Ok(None) => self.get_from_file(&account),
            Err(StoreError::Unavailable(msg)) => {
                if self.file_optin() {
                    self.get_from_file(&account)
                } else {
                    Err(StoreError::Unavailable(msg))
                }
            }
            Err(other) => Err(other),
        }
    }

    fn get_from_file(&self, account: &str) -> Result<Option<SecretString>, StoreError> {
        if self.file_optin() {
            self.file.get(account)
        } else {
            Ok(None)
        }
    }

    /// Save then read back and compare. Used by login and migration so a credential is never
    /// reported stored unless it is verifiably retrievable.
    pub(crate) fn save_verified(
        &self,
        provider: Provider,
        secret: &SecretString,
    ) -> Result<StoredIn, StoreError> {
        let stored = self.save(provider, secret)?;
        match self.get(provider)? {
            Some(back) if back.expose_secret() == secret.expose_secret() => Ok(stored),
            _ => Err(StoreError::Other(
                "credential store read-back did not match what was written".into(),
            )),
        }
    }

    /// Delete from every backend that may hold it.
    pub(crate) fn delete(&self, provider: Provider) -> Result<(), StoreError> {
        let account = account_name(provider);
        let primary = self.primary.delete(&account);
        let file = if self.file_optin() {
            self.file.delete(&account)
        } else {
            Ok(())
        };
        match (primary, file) {
            (Ok(()), Ok(())) => Ok(()),
            // An unavailable primary has nothing to delete there.
            (Err(StoreError::Unavailable(_)), Ok(())) => Ok(()),
            (Err(e), _) | (_, Err(e)) => Err(e),
        }
    }

    // Async wrappers: keychain access can block (Secret Service unlock prompts), so it
    // never runs on the async worker.

    pub(crate) async fn save_verified_async(
        self: &Arc<Self>,
        provider: Provider,
        secret: SecretString,
    ) -> Result<StoredIn, StoreError> {
        let this = Arc::clone(self);
        join(tokio::task::spawn_blocking(move || this.save_verified(provider, &secret)).await)
    }

    pub(crate) async fn get_async(
        self: &Arc<Self>,
        provider: Provider,
    ) -> Result<Option<SecretString>, StoreError> {
        let this = Arc::clone(self);
        join(tokio::task::spawn_blocking(move || this.get(provider)).await)
    }

    pub(crate) async fn delete_async(
        self: &Arc<Self>,
        provider: Provider,
    ) -> Result<(), StoreError> {
        let this = Arc::clone(self);
        join(tokio::task::spawn_blocking(move || this.delete(provider)).await)
    }
}

fn join<T>(res: Result<Result<T, StoreError>, tokio::task::JoinError>) -> Result<T, StoreError> {
    res.unwrap_or_else(|e| {
        Err(StoreError::Other(format!(
            "credential store task failed: {e}"
        )))
    })
}

// ------------------------------------------------------------------------ test double

/// In-memory keyring double with switchable failure modes.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(crate) enum Mode {
        Ok,
        Locked,
        Unavailable,
        /// `set` reports success but stores nothing (defeats read-back verification).
        LossyWrites,
    }

    pub(crate) struct MemoryBackend {
        map: Mutex<HashMap<String, String>>,
        mode: Mutex<Mode>,
    }

    impl MemoryBackend {
        pub(crate) fn new() -> Arc<Self> {
            Arc::new(Self {
                map: Mutex::new(HashMap::new()),
                mode: Mutex::new(Mode::Ok),
            })
        }

        pub(crate) fn set_mode(&self, mode: Mode) {
            *self.mode.lock().unwrap() = mode;
        }

        pub(crate) fn raw(&self, account: &str) -> Option<String> {
            self.map.lock().unwrap().get(account).cloned()
        }

        fn check(&self) -> Result<Mode, StoreError> {
            match *self.mode.lock().unwrap() {
                Mode::Locked => Err(StoreError::Locked("keychain locked".into())),
                Mode::Unavailable => Err(StoreError::Unavailable("no secret service".into())),
                m => Ok(m),
            }
        }
    }

    /// `Arc<MemoryBackend>` is itself a backend so tests keep a handle after boxing it.
    impl SecretBackend for Arc<MemoryBackend> {
        fn get(&self, account: &str) -> Result<Option<SecretString>, StoreError> {
            self.check()?;
            Ok(self
                .map
                .lock()
                .unwrap()
                .get(account)
                .cloned()
                .map(SecretString::from))
        }
        fn set(&self, account: &str, secret: &str) -> Result<(), StoreError> {
            if self.check()? != Mode::LossyWrites {
                self.map
                    .lock()
                    .unwrap()
                    .insert(account.to_owned(), secret.to_owned());
            }
            Ok(())
        }
        fn delete(&self, account: &str) -> Result<(), StoreError> {
            self.check()?;
            self.map.lock().unwrap().remove(account);
            Ok(())
        }
    }

    pub(crate) struct Fixture {
        pub store: Arc<CredentialStore>,
        pub keyring: Arc<MemoryBackend>,
        pub file_dir: tempfile::TempDir,
    }

    impl Fixture {
        pub(crate) fn new() -> Self {
            let keyring = MemoryBackend::new();
            let file_dir = tempfile::tempdir().unwrap();
            let file = FileBackend::new(file_dir.path().join("ai-credentials.json"));
            let store = Arc::new(CredentialStore::new(
                Box::new(keyring.clone()),
                Box::new(file),
            ));
            Self {
                store,
                keyring,
                file_dir,
            }
        }

        pub(crate) fn file_path(&self) -> PathBuf {
            self.file_dir.path().join("ai-credentials.json")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    fn secret(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    #[test]
    fn keyring_double_roundtrip_and_delete() {
        let fx = Fixture::new();
        assert!(fx.store.get(Provider::Claude).unwrap().is_none());
        let stored = fx
            .store
            .save_verified(Provider::Claude, &secret("sk-ant-1"))
            .unwrap();
        assert_eq!(stored, StoredIn::OsKeychain);
        assert_eq!(
            fx.store
                .get(Provider::Claude)
                .unwrap()
                .unwrap()
                .expose_secret(),
            "sk-ant-1"
        );
        assert_eq!(
            fx.keyring.raw("ai-assist/claude").as_deref(),
            Some("sk-ant-1")
        );
        fx.store.delete(Provider::Claude).unwrap();
        assert!(fx.store.get(Provider::Claude).unwrap().is_none());
        // Deleting a missing entry is fine.
        fx.store.delete(Provider::Claude).unwrap();
    }

    #[test]
    fn accounts_are_namespaced_per_provider() {
        assert_eq!(account_name(Provider::Github), "ai-assist/github");
        assert_eq!(account_name(Provider::Openai), "ai-assist/openai");
        let fx = Fixture::new();
        fx.store.save(Provider::Github, &secret("gh")).unwrap();
        fx.store.save(Provider::Openai, &secret("oa")).unwrap();
        assert_eq!(
            fx.store
                .get(Provider::Github)
                .unwrap()
                .unwrap()
                .expose_secret(),
            "gh"
        );
        assert_eq!(
            fx.store
                .get(Provider::Openai)
                .unwrap()
                .unwrap()
                .expose_secret(),
            "oa"
        );
    }

    #[test]
    fn locked_store_is_reported_and_never_falls_back_even_with_optin() {
        let fx = Fixture::new();
        fx.keyring.set_mode(Mode::Locked);
        let err = fx.store.save(Provider::Claude, &secret("k")).unwrap_err();
        assert!(matches!(err, StoreError::Locked(_)), "{err:?}");
        assert_eq!(err.blocked_reason(), Some(BlockedReason::StoreLocked));
        fx.store.set_file_optin(true);
        let err = fx.store.save(Provider::Claude, &secret("k")).unwrap_err();
        assert!(matches!(err, StoreError::Locked(_)), "{err:?}");
        assert!(!fx.file_path().exists(), "locked must never write the file");
    }

    #[test]
    fn unavailable_store_refuses_by_default_no_silent_plaintext() {
        let fx = Fixture::new();
        fx.keyring.set_mode(Mode::Unavailable);
        let err = fx.store.save(Provider::Claude, &secret("k")).unwrap_err();
        assert!(matches!(err, StoreError::Unavailable(_)), "{err:?}");
        assert_eq!(err.blocked_reason(), Some(BlockedReason::StoreUnavailable));
        assert!(
            !fx.file_path().exists(),
            "default must not create a plaintext file"
        );
        // Reads also report the unavailable store rather than "not configured".
        assert!(fx.store.get(Provider::Claude).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn optin_file_store_is_0600_and_used_only_when_keyring_unavailable() {
        use std::os::unix::fs::PermissionsExt as _;
        let fx = Fixture::new();
        fx.keyring.set_mode(Mode::Unavailable);
        fx.store.set_file_optin(true);
        let stored = fx
            .store
            .save_verified(Provider::Claude, &secret("file-secret"))
            .unwrap();
        assert_eq!(stored, StoredIn::FileOptin);
        let mode = std::fs::metadata(fx.file_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(
            fx.store
                .get(Provider::Claude)
                .unwrap()
                .unwrap()
                .expose_secret(),
            "file-secret"
        );
        fx.store.delete(Provider::Claude).unwrap();
        assert!(fx.store.get(Provider::Claude).unwrap().is_none());

        // With a working keyring, the opt-in does not divert writes to the file.
        fx.keyring.set_mode(Mode::Ok);
        let stored = fx.store.save(Provider::Openai, &secret("k2")).unwrap();
        assert_eq!(stored, StoredIn::OsKeychain);
    }

    #[test]
    fn save_verified_detects_lossy_store() {
        let fx = Fixture::new();
        fx.keyring.set_mode(Mode::LossyWrites);
        let err = fx
            .store
            .save_verified(Provider::Claude, &secret("k"))
            .unwrap_err();
        assert!(err.message().contains("read-back"), "{err}");
    }

    #[tokio::test]
    async fn async_wrappers_work() {
        let fx = Fixture::new();
        fx.store
            .save_verified_async(Provider::Github, secret("t"))
            .await
            .unwrap();
        assert!(
            fx.store
                .get_async(Provider::Github)
                .await
                .unwrap()
                .is_some()
        );
        fx.store.delete_async(Provider::Github).await.unwrap();
        assert!(
            fx.store
                .get_async(Provider::Github)
                .await
                .unwrap()
                .is_none()
        );
    }
}
