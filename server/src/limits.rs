//! Per-user and global resource limits (spec 031, T018): RAII guards over atomic counters.
//! A refusal is a [`LimitReached`], which the shell reports as `SystemEvent::LimitReached`
//! or an HTTP 429.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use fathom::protocol::{LimitKind, PublicLimits};

/// Configured ceilings. Defaults match `data-model.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLimits {
    pub max_sessions: u32,
    pub max_sessions_per_user: u32,
    pub max_watches_per_user: u32,
    pub max_streams_per_user: u32,
    pub max_tabs_per_user: u32,
    /// Log/watch stream recycle interval (`streamRecycleMinutes`).
    pub stream_recycle_minutes: u64,
    /// Idle time with no tabs before a backend is parked (`idleParkMinutes`).
    pub idle_park_minutes: u64,
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            max_sessions: 50,
            max_sessions_per_user: 3,
            max_watches_per_user: 30,
            max_streams_per_user: 10,
            max_tabs_per_user: 5,
            stream_recycle_minutes: 4,
            idle_park_minutes: 10,
        }
    }
}

impl SessionLimits {
    /// The read-only view sent to the UI in `SubscriptionInfo.limits`.
    pub fn public(&self) -> PublicLimits {
        PublicLimits {
            max_watches: self.max_watches_per_user,
            max_streams: self.max_streams_per_user,
            max_tabs: self.max_tabs_per_user,
        }
    }
}

/// A limit refused the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitReached {
    pub kind: LimitKind,
    pub max: u32,
}

impl std::fmt::Display for LimitReached {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?} limit of {} reached", self.kind, self.max)
    }
}

impl std::error::Error for LimitReached {}

#[derive(Debug)]
struct Counter {
    current: AtomicU32,
    max: u32,
    kind: LimitKind,
}

impl Counter {
    fn new(kind: LimitKind, max: u32) -> Arc<Self> {
        Arc::new(Self {
            current: AtomicU32::new(0),
            max,
            kind,
        })
    }

    fn acquire(self: &Arc<Self>) -> Result<Permit, LimitReached> {
        self.current
            .try_update(Ordering::AcqRel, Ordering::Acquire, |c| {
                (c < self.max).then_some(c + 1)
            })
            .map_err(|_| LimitReached {
                kind: self.kind,
                max: self.max,
            })?;
        Ok(Permit(Arc::clone(self)))
    }

    fn get(&self) -> u32 {
        self.current.load(Ordering::Acquire)
    }
}

/// Releases its slot on drop.
#[derive(Debug)]
pub struct Permit(Arc<Counter>);

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.current.fetch_sub(1, Ordering::AcqRel);
    }
}

/// One user's watch / stream / tab counters (owned by their `UserBackend`).
#[derive(Debug, Clone)]
pub struct UserCounters {
    watches: Arc<Counter>,
    streams: Arc<Counter>,
    tabs: Arc<Counter>,
}

impl UserCounters {
    pub fn new(limits: &SessionLimits) -> Self {
        Self {
            watches: Counter::new(LimitKind::Watches, limits.max_watches_per_user),
            streams: Counter::new(LimitKind::Streams, limits.max_streams_per_user),
            tabs: Counter::new(LimitKind::Tabs, limits.max_tabs_per_user),
        }
    }
    pub fn watch(&self) -> Result<Permit, LimitReached> {
        self.watches.acquire()
    }
    pub fn stream(&self) -> Result<Permit, LimitReached> {
        self.streams.acquire()
    }
    pub fn tab(&self) -> Result<Permit, LimitReached> {
        self.tabs.acquire()
    }
    pub fn watches(&self) -> u32 {
        self.watches.get()
    }
    pub fn streams(&self) -> u32 {
        self.streams.get()
    }
    pub fn tabs(&self) -> u32 {
        self.tabs.get()
    }
}

/// Process-wide session accounting: global cap plus per-user cap.
#[derive(Debug)]
pub struct Limiter {
    limits: SessionLimits,
    global: Arc<Counter>,
    per_user: Mutex<HashMap<String, Arc<Counter>>>,
}

/// A session slot: holds both the global and the per-user permit.
#[derive(Debug)]
pub struct SessionPermit {
    _global: Permit,
    _user: Permit,
}

impl Limiter {
    pub fn new(limits: SessionLimits) -> Self {
        Self {
            global: Counter::new(LimitKind::Sessions, limits.max_sessions),
            limits,
            per_user: Mutex::new(HashMap::new()),
        }
    }

    pub fn limits(&self) -> &SessionLimits {
        &self.limits
    }

    /// Reserve a session for `user_key`. Checks the per-user cap first so one user cannot be
    /// told "global full" when the real reason is their own cap.
    pub fn acquire_session(&self, user_key: &str) -> Result<SessionPermit, LimitReached> {
        let counter = {
            let mut map = self.per_user.lock().unwrap_or_else(|e| e.into_inner());
            // Drop idle entries so the map does not grow with every user ever seen.
            map.retain(|_, c| c.get() > 0 || Arc::strong_count(c) > 1);
            Arc::clone(map.entry(user_key.to_owned()).or_insert_with(|| {
                Counter::new(LimitKind::Sessions, self.limits.max_sessions_per_user)
            }))
        };
        let user = counter.acquire()?;
        let global = self.global.acquire()?;
        Ok(SessionPermit {
            _global: global,
            _user: user,
        })
    }

    pub fn sessions(&self) -> u32 {
        self.global.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small() -> SessionLimits {
        SessionLimits {
            max_sessions: 3,
            max_sessions_per_user: 2,
            max_watches_per_user: 2,
            max_streams_per_user: 1,
            max_tabs_per_user: 2,
            ..Default::default()
        }
    }

    #[test]
    fn watch_cap_reports_kind_and_max_and_releases_on_drop() {
        let c = UserCounters::new(&small());
        let a = c.watch().unwrap();
        let _b = c.watch().unwrap();
        let err = c.watch().unwrap_err();
        assert_eq!(
            err,
            LimitReached {
                kind: LimitKind::Watches,
                max: 2
            }
        );
        assert_eq!(c.watches(), 2);
        drop(a);
        assert_eq!(c.watches(), 1);
        assert!(c.watch().is_ok());
    }

    #[test]
    fn streams_and_tabs_are_independent() {
        let c = UserCounters::new(&small());
        let _s = c.stream().unwrap();
        assert_eq!(c.stream().unwrap_err().kind, LimitKind::Streams);
        let _t1 = c.tab().unwrap();
        let _t2 = c.tab().unwrap();
        assert_eq!(
            c.tab().unwrap_err(),
            LimitReached {
                kind: LimitKind::Tabs,
                max: 2
            }
        );
        assert!(c.watch().is_ok());
    }

    #[test]
    fn users_do_not_share_counters() {
        let a = UserCounters::new(&small());
        let b = UserCounters::new(&small());
        let _s = a.stream().unwrap();
        assert!(b.stream().is_ok());
    }

    #[test]
    fn per_user_session_cap_then_global_cap() {
        let l = Limiter::new(small());
        let _a1 = l.acquire_session("alice").unwrap();
        let _a2 = l.acquire_session("alice").unwrap();
        let e = l.acquire_session("alice").unwrap_err();
        assert_eq!(
            e,
            LimitReached {
                kind: LimitKind::Sessions,
                max: 2
            }
        );
        let _b1 = l.acquire_session("bob").unwrap();
        // global is 3: a third user is refused with the global max
        let e = l.acquire_session("carol").unwrap_err();
        assert_eq!(
            e,
            LimitReached {
                kind: LimitKind::Sessions,
                max: 3
            }
        );
        assert_eq!(l.sessions(), 3);
    }

    #[test]
    fn failed_global_acquire_releases_the_user_slot() {
        let l = Limiter::new(SessionLimits {
            max_sessions: 1,
            max_sessions_per_user: 1,
            ..small()
        });
        let g = l.acquire_session("alice").unwrap();
        assert!(l.acquire_session("bob").is_err());
        drop(g);
        // bob's failed attempt must not have leaked a per-user slot
        assert!(l.acquire_session("bob").is_ok());
    }

    #[test]
    fn session_permit_drop_frees_both() {
        let l = Limiter::new(small());
        let p = l.acquire_session("alice").unwrap();
        assert_eq!(l.sessions(), 1);
        drop(p);
        assert_eq!(l.sessions(), 0);
        assert!(l.acquire_session("alice").is_ok());
    }

    #[test]
    fn concurrent_acquire_never_exceeds_max() {
        let c = UserCounters::new(&SessionLimits {
            max_watches_per_user: 5,
            ..small()
        });
        let held = Arc::new(Mutex::new(Vec::new()));
        std::thread::scope(|s| {
            for _ in 0..16 {
                let (c, held) = (c.clone(), Arc::clone(&held));
                s.spawn(move || {
                    if let Ok(p) = c.watch() {
                        held.lock().unwrap().push(p);
                    }
                });
            }
        });
        assert_eq!(held.lock().unwrap().len(), 5);
        assert_eq!(c.watches(), 5);
    }

    #[test]
    fn public_view_mirrors_config() {
        let p = small().public();
        assert_eq!((p.max_watches, p.max_streams, p.max_tabs), (2, 1, 2));
    }
}
