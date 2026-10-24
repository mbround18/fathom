//! Retention for kept file versions (R9, data-model.md "FileVersion").
//!
//! Pure: the server's history store lists what it holds, asks [`prune`] what to drop, and
//! removes it. It runs after each write and once at start.

use std::collections::HashMap;
use std::hash::Hash;

/// Retention limits. The defaults are the `DEPTHS_FILES_HISTORY_*` defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Most versions kept per file (`DEPTHS_FILES_HISTORY_VERSIONS`).
    pub versions_per_file: usize,
    /// Oldest version kept, in seconds (`DEPTHS_FILES_HISTORY_DAYS`).
    pub max_age_secs: i64,
    /// Most bytes kept across every file (`DEPTHS_FILES_HISTORY_TOTAL`).
    pub total_bytes: u64,
    /// Content over this size is never kept (`DEPTHS_FILES_HISTORY_FILE_MAX`).
    pub file_max: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            versions_per_file: 20,
            max_age_secs: 30 * 86_400,
            total_bytes: 1024 * 1024 * 1024,
            file_max: 10 * 1024 * 1024,
        }
    }
}

impl Limits {
    /// Whether content of `size` bytes gets a version at all.
    pub fn keeps(&self, size: u64) -> bool {
        size <= self.file_max
    }
}

/// One kept version, as the policy sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionInfo<K> {
    /// Which file it belongs to: `(stack, mount, path)` or a digest of it.
    pub file: K,
    /// When it was kept, in seconds since the Unix epoch.
    pub at: i64,
    pub size: u64,
}

/// Which limit dropped a version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PruneReason {
    Count,
    Age,
    Total,
}

/// A version to drop: its index in the input, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pruned {
    pub index: usize,
    pub reason: PruneReason,
}

/// What to drop from `versions`, in order: per-file count, then age, then total size, each oldest
/// first. Versions at the same time are ordered by input index. A version exactly
/// `max_age_secs` old is kept, as is one dated in the future.
///
/// The total counts each version's size, even where two share a blob, so it errs towards
/// dropping.
pub fn prune<K: Eq + Hash>(versions: &[VersionInfo<K>], now: i64, limits: &Limits) -> Vec<Pruned> {
    let oldest_first = |a: &usize, b: &usize| (versions[*a].at, *a).cmp(&(versions[*b].at, *b));
    let mut dropped = vec![false; versions.len()];
    let mut out = Vec::new();

    // 1. Per file, keep the newest `versions_per_file`.
    let mut by_file: HashMap<&K, Vec<usize>> = HashMap::new();
    for (i, v) in versions.iter().enumerate() {
        by_file.entry(&v.file).or_default().push(i);
    }
    let mut over_count = Vec::new();
    for mut indices in by_file.into_values() {
        indices.sort_by(|a, b| oldest_first(b, a));
        over_count.extend(indices.into_iter().skip(limits.versions_per_file));
    }
    over_count.sort_by(oldest_first);
    for index in over_count {
        dropped[index] = true;
        out.push(Pruned {
            index,
            reason: PruneReason::Count,
        });
    }

    // 2. Age, then 3. total size, over what is left, oldest first.
    let mut left: Vec<usize> = (0..versions.len()).filter(|i| !dropped[*i]).collect();
    left.sort_by(oldest_first);
    let cutoff = now.saturating_sub(limits.max_age_secs);
    let (old, kept): (Vec<usize>, Vec<usize>) =
        left.into_iter().partition(|i| versions[*i].at < cutoff);
    out.extend(old.into_iter().map(|index| Pruned {
        index,
        reason: PruneReason::Age,
    }));

    let mut total: u64 = kept.iter().map(|i| versions[*i].size).sum();
    for index in kept {
        if total <= limits.total_bytes {
            break;
        }
        total -= versions[index].size;
        out.push(Pruned {
            index,
            reason: PruneReason::Total,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;
    const NOW: i64 = 1_800_000_000;

    fn v(file: &'static str, days_ago: i64, size: u64) -> VersionInfo<&'static str> {
        VersionInfo {
            file,
            at: NOW - days_ago * DAY,
            size,
        }
    }

    fn pruned(list: &[Pruned]) -> Vec<(usize, PruneReason)> {
        list.iter().map(|p| (p.index, p.reason)).collect()
    }

    #[test]
    fn defaults() {
        let l = Limits::default();
        assert_eq!(l.versions_per_file, 20);
        assert_eq!(l.max_age_secs, 30 * DAY);
        assert_eq!(l.total_bytes, 1 << 30);
        assert_eq!(l.file_max, 10 << 20);
        assert!(l.keeps(10 << 20));
        assert!(!l.keeps((10 << 20) + 1));
    }

    #[test]
    fn nothing_to_drop() {
        let list = [v("a", 1, 10), v("b", 2, 10)];
        assert!(prune(&list, NOW, &Limits::default()).is_empty());
        assert!(prune::<&str>(&[], NOW, &Limits::default()).is_empty());
    }

    #[test]
    fn per_file_count_drops_the_oldest_of_that_file() {
        let limits = Limits {
            versions_per_file: 2,
            ..Limits::default()
        };
        // a has 4 versions (indices 0..4, given out of order), b has 1.
        let list = [
            v("a", 3, 1),
            v("a", 1, 1),
            v("b", 9, 1),
            v("a", 4, 1),
            v("a", 2, 1),
        ];
        assert_eq!(
            pruned(&prune(&list, NOW, &limits)),
            [(3, PruneReason::Count), (0, PruneReason::Count)]
        );
    }

    #[test]
    fn age_drops_older_than_the_limit_oldest_first() {
        let mut list = vec![v("a", 31, 1), v("b", 30, 1), v("c", 45, 1), v("d", 0, 1)];
        list.push(VersionInfo {
            file: "e",
            at: NOW + DAY,
            size: 1,
        });
        assert_eq!(
            pruned(&prune(&list, NOW, &Limits::default())),
            [(2, PruneReason::Age), (0, PruneReason::Age)]
        );
    }

    #[test]
    fn total_drops_oldest_until_under_the_limit() {
        let limits = Limits {
            total_bytes: 100,
            ..Limits::default()
        };
        let list = [v("a", 1, 40), v("b", 3, 40), v("c", 2, 40), v("d", 4, 10)];
        // 130 bytes: drop d (10) → 120, then b (40) → 80.
        assert_eq!(
            pruned(&prune(&list, NOW, &limits)),
            [(3, PruneReason::Total), (1, PruneReason::Total)]
        );
        let exact = Limits {
            total_bytes: 130,
            ..Limits::default()
        };
        assert!(prune(&list, NOW, &exact).is_empty());
    }

    #[test]
    fn same_time_orders_by_index() {
        let limits = Limits {
            total_bytes: 0,
            ..Limits::default()
        };
        let list = [v("a", 1, 1), v("b", 1, 1)];
        assert_eq!(
            pruned(&prune(&list, NOW, &limits)),
            [(0, PruneReason::Total), (1, PruneReason::Total)]
        );
    }

    #[test]
    fn limits_combine_in_order_count_age_total() {
        let limits = Limits {
            versions_per_file: 2,
            max_age_secs: 10 * DAY,
            total_bytes: 50,
            file_max: 1 << 20,
        };
        let list = [
            v("a", 1, 20),  // 0: kept
            v("a", 2, 20),  // 1: newest two of a → kept by count; dropped by total
            v("a", 3, 20),  // 2: count
            v("b", 20, 5),  // 3: age
            v("b", 5, 30),  // 4: kept by age; dropped by total
            v("c", 50, 1),  // 5: age (older than 3)
            v("a", 60, 20), // 6: count (also old: count wins, listed once)
        ];
        // After count and age: 0 (20, 1d), 1 (20, 2d), 4 (30, 5d) = 70 > 50.
        // Oldest first: 4 → 40 ≤ 50, stop.
        assert_eq!(
            pruned(&prune(&list, NOW, &limits)),
            [
                (6, PruneReason::Count),
                (2, PruneReason::Count),
                (5, PruneReason::Age),
                (3, PruneReason::Age),
                (4, PruneReason::Total),
            ]
        );
    }
}
