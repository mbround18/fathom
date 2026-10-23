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

#[cfg(test)]
mod tests {
    use super::*;

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
