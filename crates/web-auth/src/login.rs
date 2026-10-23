//! Pending logins between the redirect to the provider and its callback, keyed by the value of
//! a short-lived login cookie. Generic over what is kept (`L`): the OIDC [`LoginState`] by
//! default, or a [`crate::provider::PendingLogin`].
//!
//! [`LoginState`]: crate::oidc::LoginState

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use tokio::time::Instant;

use crate::oidc::LoginState;

/// How long a started login may take to come back.
pub const DEFAULT_LOGIN_TTL: Duration = Duration::from_secs(600);
/// Pending logins kept at once; the oldest is evicted past this.
pub const DEFAULT_LOGIN_CAP: usize = 2000;

pub struct LoginStore<L = LoginState> {
    ttl: Duration,
    cap: usize,
    inner: Mutex<HashMap<String, (L, String, Instant)>>,
}

impl<L> Default for LoginStore<L> {
    fn default() -> Self {
        Self::new(DEFAULT_LOGIN_TTL, DEFAULT_LOGIN_CAP)
    }
}

impl<L> LoginStore<L> {
    pub fn new(ttl: Duration, cap: usize) -> Self {
        Self {
            ttl,
            cap: cap.max(1),
            inner: Mutex::default(),
        }
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Keep `login` and the path to return to; the returned id is the login cookie's value.
    pub fn put(&self, login: L, return_to: String) -> String {
        let id = crate::session::random_token();
        let now = Instant::now();
        let mut m = self.inner.lock().unwrap();
        let ttl = self.ttl;
        m.retain(|_, (_, _, t)| now.duration_since(*t) < ttl);
        if m.len() >= self.cap {
            let oldest = m
                .iter()
                .min_by_key(|(_, (_, _, t))| *t)
                .map(|(k, _)| k.clone());
            if let Some(oldest) = oldest {
                m.remove(&oldest);
            }
        }
        m.insert(id.clone(), (login, return_to, now));
        id
    }

    /// Single use: the entry is removed whether or not the callback succeeds.
    pub fn take(&self, id: &str) -> Option<(L, String)> {
        let (l, r, t) = self.inner.lock().unwrap().remove(id)?;
        (Instant::now().duration_since(t) < self.ttl).then_some((l, r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_use() {
        let s: LoginStore = LoginStore::default();
        let id = s.put(
            LoginState {
                state: "s".into(),
                nonce: "n".into(),
                pkce_verifier: "v".into(),
            },
            "/x".into(),
        );
        assert_eq!(s.take(&id).map(|(_, r)| r).as_deref(), Some("/x"));
        assert!(s.take(&id).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn expires_after_ttl() {
        let s: LoginStore<u8> = LoginStore::new(Duration::from_secs(10), 10);
        let id = s.put(1, "/".into());
        tokio::time::advance(Duration::from_secs(11)).await;
        assert!(s.take(&id).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn evicts_the_oldest_at_the_cap() {
        let s: LoginStore<u8> = LoginStore::new(Duration::from_secs(600), 2);
        let a = s.put(1, "/a".into());
        tokio::time::advance(Duration::from_millis(1)).await;
        let b = s.put(2, "/b".into());
        tokio::time::advance(Duration::from_millis(1)).await;
        let c = s.put(3, "/c".into());
        assert!(s.take(&a).is_none());
        assert!(s.take(&b).is_some());
        assert!(s.take(&c).is_some());
    }
}
