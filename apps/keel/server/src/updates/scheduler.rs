//! When scheduled checks run (contracts/config.md "Schedule", FR-005):
//!
//! - the first check 60–120 s after start, at a random point;
//! - then every interval, with ±5% jitter, so a fleet of Keels doesn't hit the registry in step;
//! - after a failure, one retry after 5 minutes, then the normal schedule;
//! - a manual check doesn't move the schedule (it never goes through here).
//!
//! The check and the random source are parameters, so tests count checks under
//! `tokio::time::pause` with a seeded source.

use std::future::Future;
use std::time::Duration;

/// The first check is at least this long after start...
pub const FIRST_MIN: Duration = Duration::from_secs(60);
/// ...plus up to this much more.
pub const FIRST_SPREAD: Duration = Duration::from_secs(60);
/// The one retry after a failed check.
pub const RETRY: Duration = Duration::from_secs(5 * 60);
/// ±5% on every regular interval.
pub const JITTER: f64 = 0.05;

/// A uniform draw in `[0, 1)`; anything else is clamped into it.
fn unit(u: f64) -> f64 {
    if u.is_finite() {
        u.clamp(0.0, 1.0)
    } else {
        0.5
    }
}

/// The delay before the first check, from a draw in `[0, 1)`.
#[must_use]
pub fn first_delay(u: f64) -> Duration {
    FIRST_MIN + FIRST_SPREAD.mul_f64(unit(u))
}

/// A regular delay: `interval` scaled into `[0.95, 1.05]` by a draw in `[0, 1)`.
#[must_use]
pub fn regular_delay(interval: Duration, u: f64) -> Duration {
    interval.mul_f64(1.0 - JITTER + 2.0 * JITTER * unit(u))
}

/// Runs scheduled checks forever. `check` returns whether the check succeeded; `random` draws in
/// `[0, 1)`; `next` hears each delay before it is slept, so the status can show the next check.
pub async fn run<C, F, R, N>(interval: Duration, mut random: R, mut check: C, mut next: N)
where
    C: FnMut() -> F,
    F: Future<Output = bool>,
    R: FnMut() -> f64,
    N: FnMut(Duration),
{
    let mut delay = first_delay(random());
    let mut retried = false;
    loop {
        next(delay);
        tokio::time::sleep(delay).await;
        let ok = check().await;
        delay = if !ok && !retried {
            retried = true;
            RETRY
        } else {
            retried = false;
            regular_delay(interval, random())
        };
    }
}

/// The process's random source: OS randomness, in `[0, 1)`. A failed read draws the middle.
#[must_use]
pub fn os_random() -> f64 {
    use rand::TryRng as _;
    rand::rngs::SysRng
        .try_next_u64()
        // The top 53 bits make an exact f64 in [0, 1).
        .map_or(0.5, |x| (x >> 11) as f64 / (1u64 << 53) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::time::Instant;

    const INTERVAL: Duration = Duration::from_secs(6 * 3600);

    /// A seeded linear congruential source in `[0, 1)`.
    fn seeded(seed: u64) -> impl FnMut() -> f64 {
        let mut s = seed;
        move || {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (s >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    /// Runs the scheduler for `for_` with checks answering from `outcomes` (then `true`), and
    /// returns when each check ran, from start.
    async fn times(seed: u64, outcomes: Vec<bool>, for_: Duration) -> Vec<Duration> {
        let start = Instant::now();
        let ran = Arc::new(Mutex::new(Vec::new()));
        let outcomes = Arc::new(Mutex::new(outcomes.into_iter()));
        let task = tokio::spawn({
            let ran = ran.clone();
            async move {
                run(
                    INTERVAL,
                    seeded(seed),
                    || {
                        ran.lock().unwrap().push(start.elapsed());
                        let ok = outcomes.lock().unwrap().next().unwrap_or(true);
                        async move { ok }
                    },
                    |_| {},
                )
                .await;
            }
        });
        tokio::time::sleep(for_).await;
        task.abort();
        ran.lock().unwrap().clone()
    }

    fn gaps(t: &[Duration]) -> Vec<Duration> {
        t.windows(2).map(|w| w[1] - w[0]).collect()
    }

    fn within_jitter(gap: Duration) -> bool {
        gap >= INTERVAL.mul_f64(0.95) && gap <= INTERVAL.mul_f64(1.05)
    }

    #[tokio::test(start_paused = true)]
    async fn first_check_in_60_to_120_s_then_every_interval_within_5_percent() {
        for seed in 0..20 {
            let t = times(seed, vec![], Duration::from_secs(4 * 24 * 3600)).await;
            assert!(
                t[0] >= Duration::from_secs(60) && t[0] <= Duration::from_secs(120),
                "seed {seed}: first at {:?}",
                t[0]
            );
            assert!(t.len() >= 15, "seed {seed}: {} checks", t.len());
            for g in gaps(&t) {
                assert!(within_jitter(g), "seed {seed}: gap {g:?}");
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_jitter_spreads_instances() {
        let a = times(1, vec![], Duration::from_secs(3 * 3600)).await;
        let b = times(2, vec![], Duration::from_secs(3 * 3600)).await;
        assert_ne!(a[0], b[0]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failure_retries_once_after_5_minutes_then_the_normal_schedule() {
        // Fails, retry succeeds: 5 minutes, then regular.
        let t = times(7, vec![false, true], Duration::from_secs(13 * 3600)).await;
        let g = gaps(&t);
        assert_eq!(g[0], RETRY);
        assert!(within_jitter(g[1]), "{:?}", g[1]);

        // Fails, and the retry fails too: no second retry, the normal schedule.
        let t = times(7, vec![false, false, true], Duration::from_secs(13 * 3600)).await;
        let g = gaps(&t);
        assert_eq!(g[0], RETRY);
        assert!(within_jitter(g[1]), "{:?}", g[1]);

        // A later regular failure gets its own retry.
        let t = times(7, vec![true, false, true], Duration::from_secs(13 * 3600)).await;
        let g = gaps(&t);
        assert!(within_jitter(g[0]));
        assert_eq!(g[1], RETRY);
    }

    #[tokio::test(start_paused = true)]
    async fn a_manual_check_does_not_move_the_schedule() {
        let quiet = times(3, vec![], Duration::from_secs(20 * 3600)).await;

        // The same seed, with a manual check (through the same counter) in the middle.
        let start = Instant::now();
        let ran = Arc::new(Mutex::new(Vec::new()));
        let task = tokio::spawn({
            let ran = ran.clone();
            async move {
                run(
                    INTERVAL,
                    seeded(3),
                    || {
                        ran.lock().unwrap().push(start.elapsed());
                        async { true }
                    },
                    |_| {},
                )
                .await;
            }
        });
        tokio::time::sleep(Duration::from_secs(3 * 3600)).await;
        let manual_at = start.elapsed();
        ran.lock().unwrap().push(manual_at);
        tokio::time::sleep(Duration::from_secs(17 * 3600)).await;
        task.abort();
        let mut busy = ran.lock().unwrap().clone();
        busy.retain(|t| *t != manual_at);
        assert_eq!(busy, quiet);
    }

    #[tokio::test(start_paused = true)]
    async fn next_hears_each_delay() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let task = tokio::spawn({
            let heard = heard.clone();
            async move {
                run(
                    INTERVAL,
                    seeded(5),
                    || async { true },
                    |d| {
                        heard.lock().unwrap().push(d);
                    },
                )
                .await;
            }
        });
        tokio::time::sleep(Duration::from_secs(3 * 3600)).await;
        task.abort();
        let heard = heard.lock().unwrap().clone();
        assert_eq!(heard.len(), 2);
        assert!(heard[0] >= FIRST_MIN && heard[0] <= FIRST_MIN + FIRST_SPREAD);
        assert!(within_jitter(heard[1]));
    }

    #[test]
    fn os_random_is_in_range() {
        for _ in 0..100 {
            let u = os_random();
            assert!((0.0..1.0).contains(&u));
        }
        assert_eq!(first_delay(f64::NAN), FIRST_MIN + FIRST_SPREAD / 2);
        assert_eq!(regular_delay(INTERVAL, 0.0), INTERVAL.mul_f64(0.95));
    }
}
