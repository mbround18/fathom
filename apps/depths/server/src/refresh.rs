//! T072: every session is revalidated with its provider every [`EVERY`] (and on
//! `POST /auth/refresh`, in `http.rs`).
//!
//! - **Valid**: the fresh tokens are kept and the next check is [`EVERY`] later.
//! - **Rejected** (revoked grant, 401/403, another account): the session ends as `Revoked`, which
//!   closes its sockets (fail closed).
//! - **Unreachable** (connect error, timeout, 5xx, 429): the session is kept until it expires on
//!   its own; the check is retried with backoff (doubling from [`BACKOFF_MIN`] to [`EVERY`], and
//!   never sooner than the provider's `Retry-After`) and a warning is logged.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use tokio::time::Instant;
use web_auth::provider::Revalidation;
use web_auth::session::{EndReason, Session};

use crate::http::AppState;

/// How often a session is revalidated.
pub const EVERY: Duration = Duration::from_secs(15 * 60);
/// The first retry after an unreachable provider.
pub const BACKOFF_MIN: Duration = Duration::from_secs(30);
/// How often the loop looks for sessions that are due.
pub const TICK: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy)]
struct Slot {
    next: Instant,
    /// The current backoff, `None` while the provider answers.
    backoff: Option<Duration>,
}

/// What one pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Pass {
    pub checked: usize,
    pub ended: usize,
    pub deferred: usize,
}

/// The schedule: when each session is next due.
#[derive(Default)]
pub struct Refresher {
    slots: Mutex<HashMap<String, Slot>>,
}

impl Refresher {
    /// When `id` is next checked, if the refresher has seen it.
    pub fn next_check(&self, id: &str) -> Option<Instant> {
        self.slots.lock().ok()?.get(id).map(|s| s.next)
    }

    /// Checks every session that is due at `now`.
    pub async fn pass(&self, app: &AppState, now: Instant) -> Pass {
        let live = app.auth.sessions.live();
        let due: Vec<Session> = {
            let Ok(mut slots) = self.slots.lock() else {
                return Pass::default();
            };
            slots.retain(|id, _| live.iter().any(|s| &s.id == id));
            live.into_iter()
                .filter(|s| {
                    let slot = slots.entry(s.id.clone()).or_insert(Slot {
                        next: s.created_at + EVERY,
                        backoff: None,
                    });
                    slot.next <= now
                })
                .collect()
        };
        let mut pass = Pass::default();
        for s in due {
            let Some(provider) = app
                .providers
                .iter()
                .find(|(id, _)| *id == s.identity.provider.as_str())
                .map(|(_, p)| p.clone())
            else {
                // Its provider is no longer configured: nothing can vouch for it.
                app.auth.sessions.end(&s.id, EndReason::Revoked);
                pass.ended += 1;
                continue;
            };
            pass.checked += 1;
            let slot = match provider.revalidate(&s).await {
                Revalidation::Valid(refreshed) => {
                    app.auth.sessions.update_tokens(&s.id, refreshed.tokens);
                    Some(Slot {
                        next: now + EVERY,
                        backoff: None,
                    })
                }
                Revalidation::Rejected(e) => {
                    tracing::info!(provider = provider.id(), error = %e, "session rejected by its provider; ending it");
                    app.auth.sessions.end(&s.id, EndReason::Revoked);
                    pass.ended += 1;
                    None
                }
                Revalidation::Unreachable { error, retry_after } => {
                    let prev = self
                        .slots
                        .lock()
                        .ok()
                        .and_then(|m| m.get(&s.id).and_then(|x| x.backoff));
                    let delay = backoff(prev, retry_after);
                    tracing::warn!(
                        provider = provider.id(),
                        error = %error,
                        retry_in_secs = delay.as_secs(),
                        "provider unreachable; keeping the session and retrying"
                    );
                    pass.deferred += 1;
                    Some(Slot {
                        next: now + delay,
                        backoff: Some(delay),
                    })
                }
            };
            if let Ok(mut slots) = self.slots.lock() {
                match slot {
                    Some(slot) => {
                        slots.insert(s.id.clone(), slot);
                    }
                    None => {
                        slots.remove(&s.id);
                    }
                }
            }
        }
        pass
    }

    /// Runs [`Refresher::pass`] every [`TICK`] for as long as the server runs.
    pub fn spawn(self: std::sync::Arc<Self>, app: AppState) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut t = tokio::time::interval(TICK);
            t.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                t.tick().await;
                self.pass(&app, Instant::now()).await;
            }
        })
    }
}

/// The next retry delay: double the last (from [`BACKOFF_MIN`]), at most [`EVERY`], and never
/// sooner than the provider asked.
pub fn backoff(prev: Option<Duration>, retry_after: Option<Duration>) -> Duration {
    let doubled = prev.map_or(BACKOFF_MIN, |d| d.saturating_mul(2)).min(EVERY);
    retry_after.map_or(doubled, |r| doubled.max(r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_up_to_the_period_and_honours_retry_after() {
        assert_eq!(backoff(None, None), BACKOFF_MIN);
        assert_eq!(backoff(Some(BACKOFF_MIN), None), BACKOFF_MIN * 2);
        assert_eq!(backoff(Some(EVERY), None), EVERY);
        assert_eq!(
            backoff(None, Some(Duration::from_secs(120))),
            Duration::from_secs(120)
        );
        assert_eq!(
            backoff(Some(Duration::from_secs(600)), Some(Duration::from_secs(5))),
            EVERY.min(Duration::from_secs(1200))
        );
    }
}
