//! Per-session token renewal, revalidation and stream recycling (spec 031, T024, research R6).
//!
//! One task per session. It renews the tokens at 75% of the ID token's remaining lifetime
//! (never sooner than `min_spacing`, and never later than `revalidate`, which doubles as the
//! revocation probe: a revoked refresh token is noticed within one interval), swapping the
//! kube [`TokenCell`] so the next request, watcher reconnect or log restart carries the fresh
//! token. A rejected refresh token ends the session at once; an unreachable IdP is retried and
//! ends the session only once `grace` has passed. Ending emits `SessionEnded` to the tabs and
//! aborts the user's backend (when this was their last session). It never fails open.
//!
//! Every `recycle` interval the backend's streams are recycled so a lapsed user cannot hold one.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use fathom::protocol::SessionEndReason;
use secrecy::SecretString;
use tokio::time::{Instant, sleep_until};

use crate::config::KubeTokenKind;
use crate::kubeauth::TokenCell;
use crate::oidc::{OidcClient, OidcError};
use crate::session::{EndReason, SessionStore, Tokens};

/// The refresh-token grant, as a seam so tests need no IdP.
#[async_trait]
pub trait TokenRefresher: Send + Sync {
    async fn refresh(&self, refresh_token: &SecretString) -> Result<Tokens, OidcError>;
}

#[async_trait]
impl TokenRefresher for OidcClient {
    async fn refresh(&self, refresh_token: &SecretString) -> Result<Tokens, OidcError> {
        OidcClient::refresh(self, refresh_token).await
    }
}

/// What the refresh task needs from a user's backend.
pub trait BackendControl: Send + Sync {
    fn is_alive(&self) -> bool;
    /// Restart long-lived streams so they re-authenticate.
    fn recycle_streams(&self);
    /// Emit `SessionEnded` and stop the backend.
    fn end_session(&self, reason: SessionEndReason);
}

impl BackendControl for crate::tenant::UserBackend {
    fn is_alive(&self) -> bool {
        crate::tenant::UserBackend::is_alive(self)
    }
    fn recycle_streams(&self) {
        crate::tenant::UserBackend::recycle_streams(self);
    }
    fn end_session(&self, reason: SessionEndReason) {
        crate::tenant::UserBackend::end_session(self, reason);
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RefreshConfig {
    /// How long an unreachable IdP is tolerated before the session ends (`refreshGraceSeconds`).
    pub grace: Duration,
    /// Floor between refresh attempts.
    pub min_spacing: Duration,
    /// Upper bound between refreshes, the revocation probe (`revalidateSeconds`); zero disables.
    pub revalidate: Duration,
    /// Stream recycle interval (`streamRecycleMinutes`); zero disables.
    pub recycle: Duration,
    /// Delay between retries after a transient failure.
    pub retry: Duration,
}

impl Default for RefreshConfig {
    fn default() -> Self {
        Self {
            grace: Duration::from_secs(120),
            min_spacing: Duration::from_secs(20),
            revalidate: Duration::from_secs(60),
            recycle: Duration::from_secs(4 * 60),
            retry: Duration::from_secs(10),
        }
    }
}

pub struct RefreshParams {
    pub session_id: String,
    pub user_key: String,
    pub refresh_token: Option<SecretString>,
    pub expires_at: Instant,
    pub cell: TokenCell,
    /// Which token the Kubernetes API server validates (`FATHOM_KUBE_TOKEN`).
    pub kube_token: KubeTokenKind,
    pub store: Arc<SessionStore>,
    pub refresher: Arc<dyn TokenRefresher>,
    pub backend: Arc<dyn BackendControl>,
    pub cfg: RefreshConfig,
}

/// Delay until the next refresh for a token expiring at `expires_at`.
pub fn next_delay(now: Instant, expires_at: Instant, cfg: &RefreshConfig) -> Duration {
    let remaining = expires_at.saturating_duration_since(now);
    let mut d = remaining.mul_f64(0.75);
    if !cfg.revalidate.is_zero() {
        d = d.min(cfg.revalidate);
    }
    d.max(cfg.min_spacing)
}

fn map_reason(r: EndReason) -> SessionEndReason {
    match r {
        EndReason::SignedOut => SessionEndReason::SignedOut,
        EndReason::Expired => SessionEndReason::Expired,
        EndReason::Revoked => SessionEndReason::Revoked,
        EndReason::IdpUnavailable => SessionEndReason::IdpUnavailable,
        EndReason::LimitEvicted => SessionEndReason::LimitEvicted,
    }
}

/// Start the session's refresh/recycle task on the current Tokio runtime.
pub fn spawn(p: RefreshParams) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run(p))
}

async fn run(mut p: RefreshParams) {
    let mut refresh_at = Instant::now() + next_delay(Instant::now(), p.expires_at, &p.cfg);
    let mut recycle_at = (!p.cfg.recycle.is_zero()).then(|| Instant::now() + p.cfg.recycle);
    let mut failing_since: Option<Instant> = None;

    loop {
        let recycle_due = recycle_at.unwrap_or(refresh_at + Duration::from_secs(u32::MAX as u64));
        let recycle_first = recycle_at.is_some() && recycle_due < refresh_at;
        sleep_until(if recycle_first {
            recycle_due
        } else {
            refresh_at
        })
        .await;

        if !p.backend.is_alive() || p.store.count_for_user(&p.user_key) == 0 {
            return;
        }
        if recycle_first {
            p.backend.recycle_streams();
            recycle_at = Some(Instant::now() + p.cfg.recycle);
            continue;
        }

        let Some(rt) = p.refresh_token.clone() else {
            return end(&p, EndReason::Revoked);
        };
        match p.refresher.refresh(&rt).await {
            Ok(tokens) => {
                failing_since = None;
                let remaining = tokens.expires_at.saturating_duration_since(Instant::now());
                let bearer = match p.kube_token {
                    KubeTokenKind::Id => &tokens.id_token,
                    KubeTokenKind::Access => &tokens.access_token,
                };
                if let Err(e) = p.cell.set(bearer, Some(SystemTime::now() + remaining)) {
                    tracing::warn!("refreshed token unusable: {e}");
                    return end(&p, EndReason::Revoked);
                }
                p.refresh_token = tokens.refresh_token.clone();
                p.expires_at = tokens.expires_at;
                if !p.store.update_tokens(&p.session_id, tokens) {
                    return; // session already gone
                }
                refresh_at = Instant::now() + next_delay(Instant::now(), p.expires_at, &p.cfg);
            }
            Err(OidcError::RefreshTransient(e)) => {
                let since = *failing_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= p.cfg.grace {
                    tracing::warn!("IdP unreachable beyond grace; ending session: {e}");
                    return end(&p, EndReason::IdpUnavailable);
                }
                refresh_at = Instant::now() + p.cfg.retry;
            }
            Err(e) => {
                tracing::info!("refresh rejected; ending session: {e}");
                return end(&p, EndReason::Revoked);
            }
        }
    }
}

fn end(p: &RefreshParams, reason: EndReason) {
    p.store.end(&p.session_id, reason);
    p.cell.revoke();
    // The backend is per user and shared by their sessions: only the last one takes it down.
    if p.store.count_for_user(&p.user_key) == 0 {
        p.backend.end_session(map_reason(reason));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Identity, SessionConfig};
    use secrecy::ExposeSecret;
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct Fake {
        script: Mutex<VecDeque<Result<Tokens, OidcError>>>,
        calls: AtomicU32,
    }

    #[async_trait]
    impl TokenRefresher for Fake {
        async fn refresh(&self, _: &SecretString) -> Result<Tokens, OidcError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err(OidcError::RefreshTransient("down".into())))
        }
    }

    #[derive(Default)]
    struct Backend {
        recycled: AtomicU32,
        ended: Mutex<Vec<SessionEndReason>>,
    }

    impl BackendControl for Backend {
        fn is_alive(&self) -> bool {
            self.ended.lock().unwrap().is_empty()
        }
        fn recycle_streams(&self) {
            self.recycled.fetch_add(1, Ordering::SeqCst);
        }
        fn end_session(&self, r: SessionEndReason) {
            self.ended.lock().unwrap().push(r);
        }
    }

    fn tokens(id: &str, life: u64) -> Tokens {
        Tokens {
            id_token: SecretString::from(id.to_owned()),
            access_token: SecretString::from("a".to_owned()),
            refresh_token: Some(SecretString::from("r2".to_owned())),
            expires_at: Instant::now() + Duration::from_secs(life),
        }
    }

    struct Rig {
        store: Arc<SessionStore>,
        cell: TokenCell,
        backend: Arc<Backend>,
        fake: Arc<Fake>,
        handle: tokio::task::JoinHandle<()>,
        session_id: String,
    }

    fn rig(script: Vec<Result<Tokens, OidcError>>, cfg: RefreshConfig) -> Rig {
        let store = Arc::new(SessionStore::new(SessionConfig::default()));
        let ident = Identity {
            issuer: "https://idp".into(),
            sub: "s".into(),
            username: "alice".into(),
            groups: vec![],
            email: None,
        };
        let t0 = tokens("id0", 300);
        let s = store.create(ident, t0.clone()).unwrap();
        let cell = TokenCell::new(&t0.id_token, None).unwrap();
        let backend = Arc::new(Backend::default());
        let fake = Arc::new(Fake {
            script: Mutex::new(script.into()),
            calls: AtomicU32::new(0),
        });
        let handle = spawn(RefreshParams {
            session_id: s.id.clone(),
            user_key: s.user_key,
            refresh_token: t0.refresh_token,
            expires_at: t0.expires_at,
            cell: cell.clone(),
            kube_token: KubeTokenKind::Id,
            store: Arc::clone(&store),
            refresher: fake.clone(),
            backend: backend.clone(),
            cfg,
        });
        Rig {
            store,
            cell,
            backend,
            fake,
            handle,
            session_id: s.id,
        }
    }

    fn cfg() -> RefreshConfig {
        RefreshConfig {
            revalidate: Duration::ZERO,
            recycle: Duration::ZERO,
            ..Default::default()
        }
    }

    async fn advance(secs: u64) {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        tokio::time::advance(Duration::from_secs(secs)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    #[test]
    fn delay_is_75_percent_bounded() {
        let now = Instant::now();
        let c = RefreshConfig::default();
        assert_eq!(
            next_delay(now, now + Duration::from_secs(8), &c),
            c.min_spacing
        );
        let c0 = RefreshConfig {
            revalidate: Duration::ZERO,
            ..c
        };
        assert_eq!(
            next_delay(now, now + Duration::from_secs(400), &c0),
            Duration::from_secs(300)
        );
        assert_eq!(
            next_delay(now, now + Duration::from_secs(400), &c),
            c.revalidate
        );
    }

    #[tokio::test(start_paused = true)]
    async fn renews_at_75_percent_and_swaps_the_cell() {
        let r = rig(vec![Ok(tokens("id1", 300)), Ok(tokens("id2", 300))], cfg());
        advance(200).await;
        assert_eq!(r.fake.calls.load(Ordering::SeqCst), 0, "not before 75%");
        advance(30).await;
        assert_eq!(r.fake.calls.load(Ordering::SeqCst), 1);
        assert!(r.cell.is_set());
        let s = r.store.touch(&r.session_id).unwrap();
        assert_eq!(s.tokens.id_token.expose_secret(), "id1");
        advance(230).await;
        assert_eq!(
            r.fake.calls.load(Ordering::SeqCst),
            2,
            "rotated token chain continues"
        );
        assert!(r.backend.ended.lock().unwrap().is_empty());
        r.handle.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn rejected_refresh_ends_session_and_backend() {
        let r = rig(
            vec![Err(OidcError::RefreshRejected("invalid_grant".into()))],
            cfg(),
        );
        advance(240).await;
        assert!(r.store.is_empty());
        assert!(!r.cell.is_set());
        assert_eq!(
            *r.backend.ended.lock().unwrap(),
            vec![SessionEndReason::Revoked]
        );
        assert!(r.handle.is_finished());
    }

    #[tokio::test(start_paused = true)]
    async fn transient_failure_recovers_within_grace() {
        let r = rig(
            vec![
                Err(OidcError::RefreshTransient("x".into())),
                Err(OidcError::RefreshTransient("x".into())),
                Ok(tokens("id1", 300)),
            ],
            cfg(),
        );
        advance(260).await;
        advance(10).await;
        advance(10).await;
        assert_eq!(r.fake.calls.load(Ordering::SeqCst), 3);
        assert!(r.backend.ended.lock().unwrap().is_empty());
        assert!(!r.store.is_empty());
        r.handle.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn grace_exceeded_ends_with_idp_unavailable() {
        let r = rig(vec![], cfg());
        advance(225).await;
        for _ in 0..14 {
            advance(10).await;
        }
        assert_eq!(
            *r.backend.ended.lock().unwrap(),
            vec![SessionEndReason::IdpUnavailable]
        );
        assert!(r.store.is_empty());
        assert!(!r.cell.is_set());
    }

    #[tokio::test(start_paused = true)]
    async fn streams_recycle_on_interval() {
        let c = RefreshConfig {
            recycle: Duration::from_secs(240),
            revalidate: Duration::ZERO,
            ..Default::default()
        };
        let r = rig(vec![Ok(tokens("id1", 3000)), Ok(tokens("id2", 3000))], c);
        advance(241).await;
        assert_eq!(r.backend.recycled.load(Ordering::SeqCst), 1);
        advance(240).await;
        assert_eq!(r.backend.recycled.load(Ordering::SeqCst), 2);
        r.handle.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn revalidate_probes_before_token_expiry() {
        let c = RefreshConfig {
            revalidate: Duration::from_secs(60),
            recycle: Duration::ZERO,
            ..Default::default()
        };
        let r = rig(vec![Err(OidcError::RefreshRejected("revoked".into()))], c);
        advance(61).await;
        assert_eq!(
            *r.backend.ended.lock().unwrap(),
            vec![SessionEndReason::Revoked]
        );
    }
}
