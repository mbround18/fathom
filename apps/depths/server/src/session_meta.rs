//! Per-session address and browser (spec 052 data-model.md "SessionMeta").
//!
//! `web-auth` is shared with Fathom, so its `Session` is not extended: this side table, keyed by
//! the session handle, holds what the sessions page shows. It lives in memory like the sessions
//! themselves and drops a row when web-auth broadcasts `Ended`.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use web_auth::session::SessionStore;

/// Longest browser description kept, in characters.
pub const MAX_BROWSER_CHARS: usize = 80;
/// A session's network use is recorded at most this often (and whenever its address changes).
pub const RECORD_EVERY: Duration = Duration::from_secs(60 * 60);

/// What the sessions page knows about one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMeta {
    /// The client address of the latest request.
    pub address: IpAddr,
    /// A short plain-text description of the User-Agent, like `Firefox on Linux`.
    pub browser: String,
    pub created: DateTime<Utc>,
    pub last_used: DateTime<Utc>,
    /// When the network use was last written to the registry.
    pub last_recorded: Instant,
}

/// The side table.
#[derive(Default)]
pub struct SessionTable {
    rows: Mutex<HashMap<String, SessionMeta>>,
}

impl SessionTable {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, SessionMeta>> {
        self.rows.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// At sign-in: the session's first address, browser and creation time. Counts as recorded.
    pub fn insert(
        &self,
        handle: &str,
        address: IpAddr,
        user_agent: Option<&str>,
        now: DateTime<Utc>,
    ) {
        self.lock().insert(
            handle.to_owned(),
            SessionMeta {
                address,
                browser: browser_of(user_agent.unwrap_or_default()),
                created: now,
                last_used: now,
                last_recorded: Instant::now(),
            },
        );
    }

    /// A request in `handle` from `address`. Returns whether its network use should be recorded
    /// now: the address changed, the last record is over an hour old, or the session has no row
    /// (made before this table existed).
    pub fn seen(&self, handle: &str, address: IpAddr, now: DateTime<Utc>) -> bool {
        let mut rows = self.lock();
        match rows.get_mut(handle) {
            Some(m) => {
                m.last_used = now;
                let due = m.address != address || m.last_recorded.elapsed() >= RECORD_EVERY;
                m.address = address;
                if due {
                    m.last_recorded = Instant::now();
                }
                due
            }
            None => {
                rows.insert(
                    handle.to_owned(),
                    SessionMeta {
                        address,
                        browser: String::new(),
                        created: now,
                        last_used: now,
                        last_recorded: Instant::now(),
                    },
                );
                true
            }
        }
    }

    pub fn get(&self, handle: &str) -> Option<SessionMeta> {
        self.lock().get(handle).cloned()
    }

    pub fn remove(&self, handle: &str) {
        self.lock().remove(handle);
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Drops rows whose session ended, as web-auth announces them. Runs until the store drops.
    pub fn spawn_reaper(self: &Arc<Self>, store: &SessionStore) -> tokio::task::JoinHandle<()> {
        let mut ended = store.subscribe_ended();
        let table = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                match ended.recv().await {
                    Ok(e) => match table.upgrade() {
                        Some(t) => t.remove(&e.id),
                        None => return,
                    },
                    // Missed some: listing sessions calls `retain_live`, which catches up.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
        })
    }

    /// Keeps only the rows of sessions in `live`.
    pub fn retain_live(&self, live: &[String]) {
        self.lock().retain(|k, _| live.iter().any(|l| l == k));
    }
}

/// A short, non-reversible label for a session handle: the first 12 hex digits of its SHA-256.
/// Never the cookie.
pub fn id_hint(handle: &str) -> String {
    let digest = depths_core::hash::FileDigest::of(handle.as_bytes());
    digest[..6].iter().map(|b| format!("{b:02x}")).collect()
}

/// `Firefox on Linux`, `Chrome on Android`, ... from a User-Agent; plain text, at most
/// [`MAX_BROWSER_CHARS`] characters. Unknown agents show their first characters.
pub fn browser_of(ua: &str) -> String {
    let ua: String = ua.chars().filter(|c| !c.is_control()).collect();
    let has = |s: &str| ua.contains(s);
    let browser = if has("Edg/") {
        Some("Edge")
    } else if has("OPR/") || has("Opera") {
        Some("Opera")
    } else if has("Firefox/") {
        Some("Firefox")
    } else if has("Chrome/") || has("Chromium/") || has("CriOS/") {
        Some("Chrome")
    } else if has("Safari/") {
        Some("Safari")
    } else if has("curl/") {
        Some("curl")
    } else {
        None
    };
    let os = if has("Android") {
        Some("Android")
    } else if has("iPhone") || has("iPad") || has("iOS") {
        Some("iOS")
    } else if has("Windows") {
        Some("Windows")
    } else if has("Mac OS X") || has("Macintosh") {
        Some("macOS")
    } else if has("CrOS") {
        Some("ChromeOS")
    } else if has("Linux") || has("X11") {
        Some("Linux")
    } else {
        None
    };
    let s = match (browser, os) {
        (Some(b), Some(o)) => format!("{b} on {o}"),
        (Some(b), None) => b.to_owned(),
        (None, Some(o)) => format!("Unknown browser on {o}"),
        (None, None) if ua.trim().is_empty() => "Unknown browser".to_owned(),
        (None, None) => ua.trim().to_owned(),
    };
    s.chars().take(MAX_BROWSER_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn browsers_are_short_plain_text() {
        let rows = [
            (
                "Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0",
                "Firefox on Linux",
            ),
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36 Edg/130.0",
                "Edge on Windows",
            ),
            (
                "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 Version/17.0 Mobile/15E148 Safari/604.1",
                "Safari on iOS",
            ),
            (
                "Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 Chrome/130.0 Mobile Safari/537.36",
                "Chrome on Android",
            ),
            ("curl/8.5.0", "curl"),
            ("", "Unknown browser"),
        ];
        for (ua, want) in rows {
            assert_eq!(browser_of(ua), want, "{ua}");
        }
        let long = format!("x{}\u{1b}[31m", "y".repeat(200));
        let b = browser_of(&long);
        assert_eq!(b.chars().count(), MAX_BROWSER_CHARS);
        assert!(!b.contains('\u{1b}'));
    }

    #[test]
    fn id_hints_are_short_stable_and_not_the_handle() {
        let h = id_hint("secret-handle");
        assert_eq!(h.len(), 12);
        assert_eq!(h, id_hint("secret-handle"));
        assert_ne!(h, id_hint("other-handle"));
        assert!(!h.contains("secret"));
    }

    #[test]
    fn recording_is_throttled_but_follows_address_changes() {
        let t = SessionTable::new();
        let now = Utc::now();
        t.insert("s", ip("203.0.113.7"), Some("curl/8"), now);
        assert!(
            !t.seen("s", ip("203.0.113.7"), now),
            "just recorded at sign-in"
        );
        assert!(t.seen("s", ip("198.51.100.1"), now), "new address");
        assert!(!t.seen("s", ip("198.51.100.1"), now));
        assert!(t.seen("unknown", ip("198.51.100.1"), now), "no row yet");
        assert_eq!(t.get("s").unwrap().address, ip("198.51.100.1"));
        t.retain_live(&["s".to_owned()]);
        assert_eq!(t.len(), 1);
        t.remove("s");
        assert!(t.is_empty());
    }
}
