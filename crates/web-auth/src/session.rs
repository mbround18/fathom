//! In-memory user sessions (spec 031, moved here by spec 050).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;
use tokio::time::Instant;

/// Which kind of identity provider signed the person in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// OpenID Connect. Fathom's hosted server only ever uses this one.
    #[default]
    Oidc,
    Github,
    Discord,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Oidc => "oidc",
            Provider::Github => "github",
            Provider::Discord => "discord",
        }
    }
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub provider: Provider,
    pub issuer: String,
    pub sub: String,
    pub username: String,
    /// Provider groups. Only OIDC fills them (Fathom's `groups_required`); others leave it empty.
    pub groups: Vec<String>,
    pub email: Option<String>,
}

impl Identity {
    /// Stable per-user key: `sha256(iss|sub)[..16]` as hex (32 chars). The provider is not part
    /// of it: each provider has its own issuer, and OIDC keys stay what they were before 050.
    pub fn user_key(&self) -> String {
        let d = Sha256::digest(format!("{}|{}", self.issuer, self.sub).as_bytes());
        d[..16].iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Raw tokens held for the session. Never printed (SecretString redacts Debug).
#[derive(Debug, Clone)]
pub struct Tokens {
    pub id_token: SecretString,
    pub access_token: SecretString,
    pub refresh_token: Option<SecretString>,
    pub expires_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    SignedOut,
    Expired,
    Revoked,
    IdpUnavailable,
    LimitEvicted,
}

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub absolute: Duration,
    pub idle: Duration,
    pub max_sessions: usize,
    pub max_sessions_per_user: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            absolute: Duration::from_secs(8 * 3600),
            idle: Duration::from_secs(30 * 60),
            max_sessions: 50,
            max_sessions_per_user: 3,
        }
    }
}

/// `Debug` redacts `id` (the session cookie value) and `csrf`; tokens redact themselves.
#[derive(Clone)]
pub struct Session {
    pub id: String,
    pub csrf: String,
    pub identity: Identity,
    pub user_key: String,
    pub tokens: Tokens,
    pub created_at: Instant,
    pub last_seen: Instant,
    pub absolute_deadline: Instant,
    /// Successful token refreshes so far (exposed as `refreshCount` in `/api/session`).
    pub refresh_count: u64,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("id", &"[redacted]")
            .field("csrf", &"[redacted]")
            .field("identity", &self.identity)
            .field("user_key", &self.user_key)
            .field("tokens", &self.tokens)
            .field("created_at", &self.created_at)
            .field("last_seen", &self.last_seen)
            .field("absolute_deadline", &self.absolute_deadline)
            .field("refresh_count", &self.refresh_count)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct Ended {
    pub id: String,
    pub user_key: String,
    pub reason: EndReason,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SessionError {
    #[error("global session limit reached ({0})")]
    GlobalLimit(usize),
}

pub fn random_token() -> String {
    let mut b = [0u8; 32];
    rand::rng().fill_bytes(&mut b);
    URL_SAFE_NO_PAD.encode(b)
}

#[derive(Clone)]
pub struct SessionStore {
    cfg: Arc<SessionConfig>,
    inner: Arc<Mutex<HashMap<String, Session>>>,
    ended: broadcast::Sender<Ended>,
}

impl SessionStore {
    pub fn new(cfg: SessionConfig) -> Self {
        let (ended, _) = broadcast::channel(256);
        Self {
            cfg: Arc::new(cfg),
            inner: Default::default(),
            ended,
        }
    }

    /// Subscribe to session-end notifications (used to tear down backends / close sockets).
    pub fn subscribe_ended(&self) -> broadcast::Receiver<Ended> {
        self.ended.subscribe()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn count_for_user(&self, user_key: &str) -> usize {
        self.inner
            .lock()
            .unwrap()
            .values()
            .filter(|s| s.user_key == user_key)
            .count()
    }

    /// Create a session. Over the per-user cap the oldest of that user's sessions is evicted;
    /// over the global cap creation fails.
    pub fn create(&self, identity: Identity, tokens: Tokens) -> Result<Session, SessionError> {
        let now = Instant::now();
        let user_key = identity.user_key();
        let mut evicted = Vec::new();
        let session = {
            let mut map = self.inner.lock().unwrap();
            let mut mine: Vec<(Instant, String)> = map
                .values()
                .filter(|s| s.user_key == user_key)
                .map(|s| (s.created_at, s.id.clone()))
                .collect();
            mine.sort();
            let overflow = (mine.len() + 1).saturating_sub(self.cfg.max_sessions_per_user.max(1));
            for (_, id) in mine.into_iter().take(overflow) {
                map.remove(&id);
                evicted.push(Ended {
                    id,
                    user_key: user_key.clone(),
                    reason: EndReason::LimitEvicted,
                });
            }
            if map.len() >= self.cfg.max_sessions {
                return Err(SessionError::GlobalLimit(self.cfg.max_sessions));
            }
            let s = Session {
                id: random_token(),
                csrf: random_token(),
                identity,
                user_key,
                tokens,
                created_at: now,
                last_seen: now,
                absolute_deadline: now + self.cfg.absolute,
                refresh_count: 0,
            };
            map.insert(s.id.clone(), s.clone());
            s
        };
        for e in evicted {
            let _ = self.ended.send(e);
        }
        Ok(session)
    }

    fn expiry(&self, s: &Session, now: Instant) -> Option<EndReason> {
        if now >= s.absolute_deadline || now.duration_since(s.last_seen) >= self.cfg.idle {
            Some(EndReason::Expired)
        } else {
            None
        }
    }

    /// Look up a live session and refresh its idle timer. Expired sessions are ended.
    pub fn touch(&self, id: &str) -> Option<Session> {
        let now = Instant::now();
        let mut ended = None;
        let out = {
            let mut map = self.inner.lock().unwrap();
            match map.get_mut(id) {
                None => None,
                Some(s) => {
                    if let Some(reason) = self.expiry(s, now) {
                        ended = Some(Ended {
                            id: s.id.clone(),
                            user_key: s.user_key.clone(),
                            reason,
                        });
                        None
                    } else {
                        s.last_seen = now;
                        Some(s.clone())
                    }
                }
            }
        };
        if let Some(e) = ended {
            self.inner.lock().unwrap().remove(&e.id);
            let _ = self.ended.send(e);
        }
        out
    }

    /// Replace the tokens after a refresh.
    pub fn update_tokens(&self, id: &str, tokens: Tokens) -> bool {
        match self.inner.lock().unwrap().get_mut(id) {
            Some(s) => {
                s.tokens = tokens;
                s.refresh_count += 1;
                true
            }
            None => false,
        }
    }

    /// End a session (sign-out, revocation, IdP failure). Returns the removed session.
    pub fn end(&self, id: &str, reason: EndReason) -> Option<Session> {
        let removed = self.inner.lock().unwrap().remove(id);
        if let Some(s) = &removed {
            let _ = self.ended.send(Ended {
                id: s.id.clone(),
                user_key: s.user_key.clone(),
                reason,
            });
        }
        removed
    }

    /// Remove every expired session; returns what was ended.
    pub fn sweep(&self) -> Vec<Ended> {
        let now = Instant::now();
        let mut out = Vec::new();
        {
            let mut map = self.inner.lock().unwrap();
            let dead: Vec<(String, EndReason)> = map
                .values()
                .filter_map(|s| self.expiry(s, now).map(|r| (s.id.clone(), r)))
                .collect();
            for (id, reason) in dead {
                if let Some(s) = map.remove(&id) {
                    out.push(Ended {
                        id,
                        user_key: s.user_key,
                        reason,
                    });
                }
            }
        }
        for e in &out {
            let _ = self.ended.send(e.clone());
        }
        out
    }

    /// Spawn the periodic sweeper.
    pub fn spawn_sweeper(&self, every: Duration) -> tokio::task::JoinHandle<()> {
        let store = self.clone();
        tokio::spawn(async move {
            let mut t = tokio::time::interval(every);
            loop {
                t.tick().await;
                store.sweep();
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    fn ident(sub: &str) -> Identity {
        Identity {
            provider: Provider::Oidc,
            issuer: "https://idp".into(),
            sub: sub.into(),
            username: sub.into(),
            groups: vec!["g".into()],
            email: None,
        }
    }
    fn toks() -> Tokens {
        Tokens {
            id_token: SecretString::from("id"),
            access_token: SecretString::from("at"),
            refresh_token: None,
            expires_at: Instant::now() + Duration::from_secs(3600),
        }
    }
    fn cfg() -> SessionConfig {
        SessionConfig {
            absolute: Duration::from_secs(600),
            idle: Duration::from_secs(60),
            max_sessions: 3,
            max_sessions_per_user: 2,
        }
    }

    #[test]
    fn ids_are_unique_and_256_bit() {
        let a = random_token();
        assert_eq!(URL_SAFE_NO_PAD.decode(&a).unwrap().len(), 32);
        assert_ne!(a, random_token());
        let s = SessionStore::new(cfg());
        let x = s.create(ident("u1"), toks()).unwrap();
        assert_ne!(x.id, x.csrf);
    }

    #[test]
    fn provider_defaults_to_oidc_and_serializes_lowercase() {
        assert_eq!(Provider::default(), Provider::Oidc);
        for (p, s) in [
            (Provider::Oidc, "oidc"),
            (Provider::Github, "github"),
            (Provider::Discord, "discord"),
        ] {
            assert_eq!(p.as_str(), s);
            assert_eq!(p.to_string(), s);
        }
    }

    /// Adding `provider` must not move an OIDC user's key (tenant state is keyed by it).
    #[test]
    fn user_key_unchanged_by_provider() {
        let mut gh = ident("a");
        gh.provider = Provider::Github;
        assert_eq!(gh.user_key(), ident("a").user_key());
    }

    #[test]
    fn user_key_stable_and_distinct() {
        assert_eq!(ident("a").user_key(), ident("a").user_key());
        assert_ne!(ident("a").user_key(), ident("b").user_key());
        assert_eq!(ident("a").user_key().len(), 32);
    }

    #[test]
    fn debug_hides_tokens() {
        let t = toks();
        assert!(!format!("{t:?}").contains("\"at\""));
        assert_eq!(t.id_token.expose_secret(), "id");
    }

    #[tokio::test(start_paused = true)]
    async fn idle_expiry_and_touch_extends() {
        let s = SessionStore::new(cfg());
        let x = s.create(ident("u"), toks()).unwrap();
        tokio::time::advance(Duration::from_secs(50)).await;
        assert!(s.touch(&x.id).is_some());
        tokio::time::advance(Duration::from_secs(50)).await;
        assert!(s.touch(&x.id).is_some());
        tokio::time::advance(Duration::from_secs(61)).await;
        assert!(s.touch(&x.id).is_none());
        assert!(s.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn absolute_expiry_despite_activity() {
        let s = SessionStore::new(cfg());
        let x = s.create(ident("u"), toks()).unwrap();
        for _ in 0..11 {
            tokio::time::advance(Duration::from_secs(55)).await;
            let _ = s.touch(&x.id);
        }
        assert!(s.touch(&x.id).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn sweeper_removes_and_notifies() {
        let s = SessionStore::new(cfg());
        let mut rx = s.subscribe_ended();
        let x = s.create(ident("u"), toks()).unwrap();
        let h = s.spawn_sweeper(Duration::from_secs(10));
        tokio::time::sleep(Duration::from_secs(75)).await;
        assert!(s.is_empty());
        let e = rx.try_recv().unwrap();
        assert_eq!((e.id, e.reason), (x.id, EndReason::Expired));
        h.abort();
    }

    #[test]
    fn sign_out_ends_and_notifies() {
        let s = SessionStore::new(cfg());
        let mut rx = s.subscribe_ended();
        let x = s.create(ident("u"), toks()).unwrap();
        assert!(s.end(&x.id, EndReason::SignedOut).is_some());
        assert!(s.end(&x.id, EndReason::SignedOut).is_none());
        assert_eq!(rx.try_recv().unwrap().reason, EndReason::SignedOut);
    }

    #[tokio::test]
    async fn per_user_cap_evicts_oldest() {
        let s = SessionStore::new(cfg());
        let mut rx = s.subscribe_ended();
        let a = s.create(ident("u"), toks()).unwrap();
        tokio::time::sleep(Duration::from_millis(2)).await;
        let b = s.create(ident("u"), toks()).unwrap();
        tokio::time::sleep(Duration::from_millis(2)).await;
        let c = s.create(ident("u"), toks()).unwrap();
        assert_eq!(s.count_for_user(&c.user_key), 2);
        assert!(s.touch(&a.id).is_none());
        assert!(s.touch(&b.id).is_some());
        let e = rx.try_recv().unwrap();
        assert_eq!((e.id, e.reason), (a.id, EndReason::LimitEvicted));
    }

    #[test]
    fn global_cap_rejects() {
        let s = SessionStore::new(cfg());
        for u in ["a", "b", "c"] {
            s.create(ident(u), toks()).unwrap();
        }
        assert_eq!(
            s.create(ident("d"), toks()).unwrap_err(),
            SessionError::GlobalLimit(3)
        );
    }

    #[test]
    fn update_tokens_swaps() {
        let s = SessionStore::new(cfg());
        let x = s.create(ident("u"), toks()).unwrap();
        let mut t = toks();
        t.id_token = SecretString::from("new");
        assert!(s.update_tokens(&x.id, t));
        assert_eq!(
            s.touch(&x.id).unwrap().tokens.id_token.expose_secret(),
            "new"
        );
    }
}
