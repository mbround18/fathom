//! In-memory credential cache (FR-005, FR-006). Keyed by plugin invocation identity, honours
//! `expirationTimestamp` minus a skew (default TTL when absent), single-flight per key, and
//! nothing is ever persisted. Secrets live in `SecretString` (redacting `Debug`, zeroized on
//! drop).

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};

use super::exec::Credential;
use crate::protocol::PluginError;

pub const DEFAULT_SKEW_SECS: i64 = 60;
pub const DEFAULT_TTL_SECS: i64 = 300;
/// A failed fetch is remembered briefly so a burst of pollers doesn't each re-run a broken or
/// hanging plugin.
pub const FAILURE_TTL_SECS: i64 = 5;

type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

struct Entry {
    credential: Credential,
    fetched_at: DateTime<Utc>,
}

#[derive(Default)]
struct Slot {
    entry: Option<Entry>,
    failure: Option<(DateTime<Utc>, PluginError)>,
}

pub struct CredentialCache {
    slots: Mutex<HashMap<u64, Arc<tokio::sync::Mutex<Slot>>>>,
    /// context -> identity, so a 401 or kubeconfig reload can drop the right entries.
    contexts: Mutex<HashMap<String, u64>>,
    skew: Duration,
    default_ttl: Duration,
    clock: Clock,
}

impl Default for CredentialCache {
    fn default() -> Self {
        Self::new(
            Duration::seconds(DEFAULT_SKEW_SECS),
            Duration::seconds(DEFAULT_TTL_SECS),
            Arc::new(Utc::now),
        )
    }
}

impl CredentialCache {
    pub fn new(skew: Duration, default_ttl: Duration, clock: Clock) -> Self {
        Self {
            slots: Mutex::default(),
            contexts: Mutex::default(),
            skew,
            default_ttl,
            clock,
        }
    }

    fn is_valid(&self, entry: &Entry, now: DateTime<Utc>) -> bool {
        match entry.credential.expires_at {
            Some(exp) => now < exp - self.skew,
            None => now < entry.fetched_at + self.default_ttl,
        }
    }

    /// Return the cached credential for `key` or run `fetch` exactly once across concurrent
    /// callers (single-flight): waiters block on the slot and reuse the winner's result.
    pub async fn get_or_fetch<F, Fut>(
        &self,
        context: &str,
        key: u64,
        fetch: F,
    ) -> Result<Credential, PluginError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Credential, PluginError>>,
    {
        let slot = {
            self.contexts
                .lock()
                .expect("cache contexts lock")
                .insert(context.to_owned(), key);
            self.slots
                .lock()
                .expect("cache slots lock")
                .entry(key)
                .or_default()
                .clone()
        };
        let mut slot = slot.lock().await;
        let now = (self.clock)();
        if let Some(entry) = &slot.entry {
            if self.is_valid(entry, now) {
                return Ok(entry.credential.clone());
            }
            slot.entry = None;
        }
        if let Some((at, err)) = &slot.failure {
            if now < *at + Duration::seconds(FAILURE_TTL_SECS) {
                return Err(err.clone());
            }
            slot.failure = None;
        }
        match fetch().await {
            Ok(credential) => {
                slot.entry = Some(Entry {
                    credential: credential.clone(),
                    fetched_at: (self.clock)(),
                });
                Ok(credential)
            }
            Err(err) => {
                slot.failure = Some(((self.clock)(), err.clone()));
                Err(err)
            }
        }
    }

    /// Drop the entry (and any remembered failure) backing `context`. Returns whether the
    /// context had one, i.e. whether it is plugin-backed and worth one re-invocation.
    pub fn invalidate_context(&self, context: &str) -> bool {
        let Some(key) = self
            .contexts
            .lock()
            .expect("cache contexts lock")
            .get(context)
            .copied()
        else {
            return false;
        };
        self.slots.lock().expect("cache slots lock").remove(&key);
        true
    }

    /// Drop entries for contexts no longer in the kubeconfig (reload).
    pub fn retain_contexts(&self, live: &[String]) {
        let live: HashSet<&str> = live.iter().map(String::as_str).collect();
        let mut contexts = self.contexts.lock().expect("cache contexts lock");
        contexts.retain(|c, _| live.contains(c.as_str()));
        let keep: HashSet<u64> = contexts.values().copied().collect();
        self.slots
            .lock()
            .expect("cache slots lock")
            .retain(|k, _| keep.contains(k));
    }

    pub fn len(&self) -> usize {
        self.slots.lock().expect("cache slots lock").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{CloudProvider, PluginErrorKind};
    use secrecy::SecretString;
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

    fn cred(exp: Option<DateTime<Utc>>) -> Credential {
        Credential {
            token: Some(SecretString::from("tok".to_owned())),
            client_certificate_data: None,
            client_key_data: None,
            expires_at: exp,
        }
    }

    fn clocked() -> (CredentialCache, Arc<AtomicI64>) {
        let secs = Arc::new(AtomicI64::new(1_000_000));
        let s2 = secs.clone();
        let cache = CredentialCache::new(
            Duration::seconds(60),
            Duration::seconds(300),
            Arc::new(move || DateTime::from_timestamp(s2.load(Ordering::SeqCst), 0).unwrap()),
        );
        (cache, secs)
    }

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    #[tokio::test]
    async fn reuses_until_expiry_minus_skew() {
        let (cache, clock) = clocked();
        let calls = AtomicUsize::new(0);
        let fetch = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(cred(Some(at(1_000_000 + 900))))
        };
        cache.get_or_fetch("c", 1, fetch).await.unwrap();
        clock.store(1_000_000 + 800, Ordering::SeqCst);
        cache.get_or_fetch("c", 1, fetch).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // 900 - 60 skew = 840: past that, refetch.
        clock.store(1_000_000 + 841, Ordering::SeqCst);
        cache.get_or_fetch("c", 1, fetch).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn default_ttl_applies_without_expiry() {
        let (cache, clock) = clocked();
        let calls = AtomicUsize::new(0);
        let fetch = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(cred(None))
        };
        cache.get_or_fetch("c", 1, fetch).await.unwrap();
        clock.store(1_000_000 + 299, Ordering::SeqCst);
        cache.get_or_fetch("c", 1, fetch).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        clock.store(1_000_000 + 301, Ordering::SeqCst);
        cache.get_or_fetch("c", 1, fetch).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn distinct_keys_do_not_share() {
        let (cache, _) = clocked();
        let calls = AtomicUsize::new(0);
        let fetch = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(cred(None))
        };
        cache.get_or_fetch("a", 1, fetch).await.unwrap();
        cache.get_or_fetch("b", 2, fetch).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn concurrent_callers_single_flight() {
        let (cache, _) = clocked();
        let cache = Arc::new(cache);
        let calls = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let (cache, calls) = (cache.clone(), calls.clone());
            tasks.push(tokio::spawn(async move {
                cache
                    .get_or_fetch("c", 1, || async {
                        calls.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        Ok(cred(None))
                    })
                    .await
                    .unwrap();
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn invalidate_forces_one_refetch_and_reports_membership() {
        let (cache, _) = clocked();
        let calls = AtomicUsize::new(0);
        let fetch = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(cred(None))
        };
        assert!(!cache.invalidate_context("c"));
        cache.get_or_fetch("c", 1, fetch).await.unwrap();
        assert!(cache.invalidate_context("c"));
        cache.get_or_fetch("c", 1, fetch).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn reload_drops_removed_contexts() {
        let (cache, _) = clocked();
        let fetch = || async { Ok(cred(None)) };
        cache.get_or_fetch("keep", 1, fetch).await.unwrap();
        cache.get_or_fetch("gone", 2, fetch).await.unwrap();
        assert_eq!(cache.len(), 2);
        cache.retain_contexts(&["keep".to_owned()]);
        assert_eq!(cache.len(), 1);
        assert!(!cache.invalidate_context("gone"));
    }

    #[tokio::test]
    async fn failures_are_not_cached_as_credentials_but_burst_is_damped() {
        let (cache, clock) = clocked();
        let calls = AtomicUsize::new(0);
        let fetch = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(PluginError {
                kind: PluginErrorKind::PluginFailed,
                context: "c".into(),
                provider: CloudProvider::Eks,
                command: "aws".into(),
                message: "boom".into(),
                hint: None,
            })
        };
        assert!(cache.get_or_fetch("c", 1, fetch).await.is_err());
        assert!(cache.get_or_fetch("c", 1, fetch).await.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        clock.store(1_000_000 + FAILURE_TTL_SECS + 1, Ordering::SeqCst);
        assert!(cache.get_or_fetch("c", 1, fetch).await.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
