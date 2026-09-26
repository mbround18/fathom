//! Secret store seam (039-observability-settings T001, research R2).
//!
//! A small trait over (`service`, `user`) so command handlers can be tested with an in-memory
//! store while production uses the OS keyring. Reuses the [`Secret`] wrapper and the raw
//! backends from `oidc::store`. Unlike `CredentialStore` there is deliberately no memory
//! fallback: a keyring failure is reported to the caller so the UI never claims a key is saved
//! when it is not (FR-010).

use crate::backend::oidc::store::{
    BackendError, MemoryBackend, OsKeyringBackend, Secret, SecretBackend,
};

/// Keyring service for Grafana API keys; the user is the kube context name.
pub const GRAFANA_SERVICE: &str = "fathom.grafana";

pub trait SecretStore: Send + Sync {
    fn get(&self, service: &str, user: &str) -> Result<Option<Secret>, BackendError>;
    fn set(&self, service: &str, user: &str, value: &Secret) -> Result<(), BackendError>;
    /// Deleting an absent entry is not an error.
    fn delete(&self, service: &str, user: &str) -> Result<(), BackendError>;
}

/// Adapter from any raw [`SecretBackend`].
pub struct BackendStore<B: SecretBackend>(pub B);

impl<B: SecretBackend> SecretStore for BackendStore<B> {
    fn get(&self, service: &str, user: &str) -> Result<Option<Secret>, BackendError> {
        Ok(self.0.get(service, user)?.map(Secret::new))
    }
    fn set(&self, service: &str, user: &str, value: &Secret) -> Result<(), BackendError> {
        self.0.set(service, user, value.expose())
    }
    fn delete(&self, service: &str, user: &str) -> Result<(), BackendError> {
        self.0.delete(service, user)
    }
}

/// The real OS keyring.
pub fn keyring_store() -> BackendStore<OsKeyringBackend> {
    BackendStore(OsKeyringBackend)
}

/// In-memory store for tests.
pub fn memory_store() -> BackendStore<MemoryBackend> {
    BackendStore(MemoryBackend::default())
}

pub fn grafana_key_set(store: &dyn SecretStore, context: &str) -> bool {
    matches!(store.get(GRAFANA_SERVICE, context), Ok(Some(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_delete() {
        let s = memory_store();
        assert!(!grafana_key_set(&s, "c"));
        s.set(GRAFANA_SERVICE, "c", &Secret::new("k")).unwrap();
        assert!(grafana_key_set(&s, "c"));
        assert!(!grafana_key_set(&s, "other"));
        s.delete(GRAFANA_SERVICE, "c").unwrap();
        assert!(!grafana_key_set(&s, "c"));
    }
}
