//! An in-memory [`PasskeyStore`] for tests.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use web_passkeys::recovery::RecoveryRecord;
use web_passkeys::{PasskeyStore, StoredPasskey};

#[derive(Debug, thiserror::Error)]
#[error("memory store: {0}")]
pub struct MemError(pub &'static str);

#[derive(Default)]
pub struct MemStore {
    passkeys: Mutex<HashMap<String, Vec<StoredPasskey>>>,
    recovery: Mutex<HashMap<String, RecoveryRecord>>,
}

#[async_trait]
impl PasskeyStore for MemStore {
    type Error = MemError;

    async fn passkeys(&self, user: &str) -> Result<Vec<StoredPasskey>, MemError> {
        Ok(self
            .passkeys
            .lock()
            .unwrap()
            .get(user)
            .cloned()
            .unwrap_or_default())
    }

    async fn add_passkey(&self, user: &str, passkey: StoredPasskey) -> Result<(), MemError> {
        let mut all = self.passkeys.lock().unwrap();
        if all.values().flatten().any(|p| p.id == passkey.id) {
            return Err(MemError("duplicate credential"));
        }
        all.entry(user.to_owned()).or_default().push(passkey);
        Ok(())
    }

    async fn update_passkey(&self, user: &str, passkey: &StoredPasskey) -> Result<(), MemError> {
        let mut all = self.passkeys.lock().unwrap();
        let slot = all
            .get_mut(user)
            .and_then(|v| v.iter_mut().find(|p| p.id == passkey.id))
            .ok_or(MemError("no such passkey"))?;
        *slot = passkey.clone();
        Ok(())
    }

    async fn remove_passkey(&self, user: &str, id: &str) -> Result<bool, MemError> {
        let mut all = self.passkeys.lock().unwrap();
        let Some(v) = all.get_mut(user) else {
            return Ok(false);
        };
        let before = v.len();
        v.retain(|p| p.id != id);
        Ok(v.len() != before)
    }

    async fn remove_all_passkeys(&self, user: &str) -> Result<usize, MemError> {
        Ok(self
            .passkeys
            .lock()
            .unwrap()
            .remove(user)
            .map_or(0, |v| v.len()))
    }

    async fn put_recovery(&self, record: RecoveryRecord) -> Result<(), MemError> {
        self.recovery
            .lock()
            .unwrap()
            .insert(record.user.clone(), record);
        Ok(())
    }

    async fn recovery(&self, user: &str) -> Result<Option<RecoveryRecord>, MemError> {
        Ok(self.recovery.lock().unwrap().get(user).cloned())
    }

    async fn consume_recovery(&self, user: &str, code_sha256: &str) -> Result<bool, MemError> {
        let mut all = self.recovery.lock().unwrap();
        match all.get_mut(user) {
            Some(r) if r.code_sha256 == code_sha256 && !r.used => {
                r.used = true;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}
