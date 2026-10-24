//! T090: per-client rate limits.
//!
//! A token bucket per key (the client IP from [`crate::client_ip`]), refilled continuously at
//! `per_minute / 60` a second up to `burst`. The table holds at most `max_keys` keys; past that
//! the least recently used are evicted, so a flood of spoofed-looking sources cannot grow memory.
//! An evicted key starts again with a full bucket, which only ever errs towards letting a real
//! client in.
//!
//! Time is passed in, so the tests are deterministic.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The limits of contracts/protocol.md and T090.
pub const AUTH_PER_MINUTE: u32 = 10;
pub const AUTH_BURST: u32 = 5;
pub const API_PER_MINUTE: u32 = 120;
pub const API_BURST: u32 = 120;
pub const WS_UPGRADES_PER_MINUTE: u32 = 20;
pub const WS_UPGRADE_BURST: u32 = 20;
pub const MAX_KEYS: usize = 10_000;

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    at: Instant,
    used: u64,
}

#[derive(Debug)]
pub struct RateLimiter<K> {
    rate_per_sec: f64,
    burst: f64,
    max_keys: usize,
    inner: Mutex<Inner<K>>,
}

#[derive(Debug)]
struct Inner<K> {
    buckets: HashMap<K, Bucket>,
    tick: u64,
}

impl<K: Hash + Eq + Clone> RateLimiter<K> {
    pub fn new(per_minute: u32, burst: u32, max_keys: usize) -> Self {
        Self {
            rate_per_sec: f64::from(per_minute) / 60.0,
            burst: f64::from(burst.max(1)),
            max_keys: max_keys.max(1),
            inner: Mutex::new(Inner {
                buckets: HashMap::new(),
                tick: 0,
            }),
        }
    }

    /// Takes one token for `key`. `Err(wait)` says how long until one is available.
    pub fn check(&self, key: &K, now: Instant) -> Result<(), Duration> {
        let mut g = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        g.tick += 1;
        let tick = g.tick;
        if !g.buckets.contains_key(key) && g.buckets.len() >= self.max_keys {
            evict(&mut g.buckets, self.max_keys);
        }
        let b = g.buckets.entry(key.clone()).or_insert(Bucket {
            tokens: self.burst,
            at: now,
            used: tick,
        });
        let elapsed = now.saturating_duration_since(b.at).as_secs_f64();
        b.tokens = (b.tokens + elapsed * self.rate_per_sec).min(self.burst);
        b.at = now;
        b.used = tick;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            Ok(())
        } else {
            let wait = (1.0 - b.tokens) / self.rate_per_sec;
            Err(Duration::from_secs_f64(wait.max(0.001)))
        }
    }

    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .buckets
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Drops the least recently used tenth (at least one), so eviction is amortised.
fn evict<K: Hash + Eq + Clone>(buckets: &mut HashMap<K, Bucket>, max: usize) {
    let n = (max / 10).max(1);
    let mut by_age: Vec<(u64, K)> = buckets.iter().map(|(k, b)| (b.used, k.clone())).collect();
    by_age.sort_unstable_by_key(|(used, _)| *used);
    for (_, k) in by_age.into_iter().take(n) {
        buckets.remove(&k);
    }
}

/// The three HTTP-level limiters, keyed by client IP.
#[derive(Debug)]
pub struct Limits {
    pub auth: RateLimiter<std::net::IpAddr>,
    pub api: RateLimiter<std::net::IpAddr>,
    pub ws_upgrade: RateLimiter<std::net::IpAddr>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            auth: RateLimiter::new(AUTH_PER_MINUTE, AUTH_BURST, MAX_KEYS),
            api: RateLimiter::new(API_PER_MINUTE, API_BURST, MAX_KEYS),
            ws_upgrade: RateLimiter::new(WS_UPGRADES_PER_MINUTE, WS_UPGRADE_BURST, MAX_KEYS),
        }
    }
}

/// Open WebSockets per session and per IP (T090).
pub const SOCKETS_PER_SESSION: usize = 8;
pub const SOCKETS_PER_IP: usize = 32;
/// Commands per socket: 20 a second, burst 50 (T090). Excess closes the socket with 4429.
pub const WS_COMMANDS_PER_MINUTE: u32 = 20 * 60;
pub const WS_COMMAND_BURST: u32 = 50;

type Counts = (HashMap<String, usize>, HashMap<std::net::IpAddr, usize>);

/// Counts open sockets. [`SocketCounts::acquire`] hands out a permit that gives its slot back
/// when dropped, so a socket that ends in any way is no longer counted.
#[derive(Debug, Default)]
pub struct SocketCounts {
    inner: Mutex<Counts>,
}

#[derive(Debug)]
pub struct SocketPermit {
    counts: std::sync::Arc<SocketCounts>,
    session: String,
    ip: std::net::IpAddr,
}

impl SocketCounts {
    fn lock(&self) -> std::sync::MutexGuard<'_, Counts> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A slot for one more socket of `session` from `ip`, or `None` past either cap.
    pub fn acquire(
        self: &std::sync::Arc<Self>,
        session: &str,
        ip: std::net::IpAddr,
    ) -> Option<SocketPermit> {
        let mut g = self.lock();
        let s = g.0.get(session).copied().unwrap_or(0);
        let i = g.1.get(&ip).copied().unwrap_or(0);
        if s >= SOCKETS_PER_SESSION || i >= SOCKETS_PER_IP {
            return None;
        }
        g.0.insert(session.to_owned(), s + 1);
        g.1.insert(ip, i + 1);
        Some(SocketPermit {
            counts: self.clone(),
            session: session.to_owned(),
            ip,
        })
    }

    pub fn for_session(&self, session: &str) -> usize {
        self.lock().0.get(session).copied().unwrap_or(0)
    }

    pub fn for_ip(&self, ip: std::net::IpAddr) -> usize {
        self.lock().1.get(&ip).copied().unwrap_or(0)
    }
}

fn release<K: Hash + Eq>(m: &mut HashMap<K, usize>, k: &K) {
    if let Some(n) = m.get_mut(k) {
        *n -= 1;
        if *n == 0 {
            m.remove(k);
        }
    }
}

impl Drop for SocketPermit {
    fn drop(&mut self) {
        let mut g = self.counts.lock();
        let (sessions, ips) = &mut *g;
        release(sessions, &self.session);
        release(ips, &self.ip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sockets_are_capped_per_session_and_per_ip_and_given_back() {
        let c = std::sync::Arc::new(SocketCounts::default());
        let ip: std::net::IpAddr = "198.51.100.7".parse().unwrap();
        let held: Vec<_> = (0..SOCKETS_PER_SESSION)
            .map(|_| c.acquire("s", ip).unwrap())
            .collect();
        assert!(c.acquire("s", ip).is_none(), "ninth socket of one session");
        let more: Vec<_> = (0..SOCKETS_PER_IP - SOCKETS_PER_SESSION)
            .map(|i| c.acquire(&format!("o{i}"), ip).unwrap())
            .collect();
        assert!(c.acquire("other", ip).is_none(), "33rd socket from one IP");
        assert!(
            c.acquire("other", "198.51.100.8".parse().unwrap())
                .is_some()
        );
        drop(held);
        assert_eq!(c.for_session("s"), 0);
        assert!(c.acquire("s", ip).is_some());
        drop(more);
        assert_eq!(c.for_ip(ip), 0);
    }

    #[test]
    fn the_command_limit_is_a_burst_of_fifty_then_twenty_a_second() {
        let l = RateLimiter::new(WS_COMMANDS_PER_MINUTE, WS_COMMAND_BURST, 1);
        let t0 = Instant::now();
        assert_eq!((0..100).filter(|_| l.check(&(), t0).is_ok()).count(), 50);
        let t1 = t0 + Duration::from_secs(1);
        assert_eq!((0..100).filter(|_| l.check(&(), t1).is_ok()).count(), 20);
    }

    #[test]
    fn the_auth_limit_allows_a_burst_of_five_then_ten_a_minute() {
        let l = RateLimiter::new(AUTH_PER_MINUTE, AUTH_BURST, MAX_KEYS);
        let t0 = Instant::now();
        for _ in 0..5 {
            assert!(l.check(&1u8, t0).is_ok());
        }
        let wait = l.check(&1u8, t0).unwrap_err();
        assert_eq!(wait.as_secs(), 6, "{wait:?}");
        // Another client is unaffected.
        assert!(l.check(&2u8, t0).is_ok());
        // One token every 6s: 7s after the start is just over one.
        assert!(l.check(&1u8, t0 + Duration::from_secs(5)).is_err());
        assert!(l.check(&1u8, t0 + Duration::from_secs(7)).is_ok());
        assert!(l.check(&1u8, t0 + Duration::from_secs(7)).is_err());
    }

    #[test]
    fn a_minute_of_quiet_refills_to_the_burst_not_beyond() {
        let l = RateLimiter::new(AUTH_PER_MINUTE, AUTH_BURST, MAX_KEYS);
        let t0 = Instant::now();
        let later = t0 + Duration::from_secs(3600);
        assert!(l.check(&1u8, t0).is_ok());
        let ok = (0..20).filter(|_| l.check(&1u8, later).is_ok()).count();
        assert_eq!(ok, 5);
    }

    #[test]
    fn the_api_and_ws_limits() {
        let t0 = Instant::now();
        let l = Limits::default();
        let ip: std::net::IpAddr = "198.51.100.1".parse().unwrap();
        assert_eq!(
            (0..200).filter(|_| l.api.check(&ip, t0).is_ok()).count(),
            120
        );
        assert_eq!(
            (0..50)
                .filter(|_| l.ws_upgrade.check(&ip, t0).is_ok())
                .count(),
            20
        );
    }

    #[test]
    fn the_table_is_capped_and_evicts_least_recently_used() {
        let l = RateLimiter::new(10, 1, 100);
        let t0 = Instant::now();
        // Key 0 is hot: exhausted, and touched after the others.
        for k in 0..100u32 {
            let _ = l.check(&k, t0);
        }
        assert!(l.check(&0, t0).is_err());
        for k in 100..10_000u32 {
            let _ = l.check(&k, t0);
        }
        assert!(l.len() <= 100, "{}", l.len());
    }

    #[test]
    fn the_cap_is_ten_thousand() {
        let l = Limits::default();
        let t0 = Instant::now();
        for i in 0..12_000u32 {
            let ip = std::net::IpAddr::from(i.to_be_bytes());
            let _ = l.auth.check(&ip, t0);
        }
        assert!(l.auth.len() <= MAX_KEYS);
    }
}
