//! Host metrics from `/proc` text: pure parsers, no I/O.
//!
//! The server reads each file (bounded) from the host's `/proc`, mounted read-only, and hands the
//! text here. Every parser returns `None` for text it does not recognise rather than guessing, so a
//! kernel with a different layout shows "n/a", never a wrong figure. Nothing here panics on any
//! input.

/// The aggregate `cpu` line of `/proc/stat`, in clock ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuTimes {
    /// Every tick counted, idle included.
    pub total: u64,
    /// `idle + iowait`.
    pub idle: u64,
}

/// What `/proc/stat` says about the CPUs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuStat {
    pub times: CpuTimes,
    /// Count of `cpuN` lines: the online logical CPUs.
    pub cores: u32,
}

/// Parse `/proc/stat`. `None` without a well-formed aggregate `cpu` line.
pub fn parse_stat(text: &str) -> Option<CpuStat> {
    let mut times = None;
    let mut cores: u32 = 0;
    for line in text.lines() {
        let mut fields = line.split_ascii_whitespace();
        let Some(label) = fields.next() else { continue };
        if label == "cpu" {
            // user nice system idle iowait irq softirq steal [guest guest_nice]. Guest time is
            // already counted in user and nice, so it is left out of the total.
            let values: Vec<u64> = fields
                .take(8)
                .map(str::parse)
                .collect::<Result<_, _>>()
                .ok()?;
            if values.len() < 4 {
                return None;
            }
            let total = values.iter().try_fold(0u64, |acc, v| acc.checked_add(*v))?;
            let idle = values[3].checked_add(values.get(4).copied().unwrap_or(0))?;
            times = Some(CpuTimes { total, idle });
        } else if label
            .strip_prefix("cpu")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        {
            cores = cores.saturating_add(1);
        }
    }
    Some(CpuStat {
        times: times?,
        cores,
    })
}

/// Busy share of all CPUs between two `/proc/stat` readings, 0 to 100. `None` when no time passed
/// or the counters went backwards (a different host, or a counter reset).
pub fn cpu_percent(prev: CpuTimes, now: CpuTimes) -> Option<f32> {
    let total = now.total.checked_sub(prev.total)?;
    let idle = now.idle.checked_sub(prev.idle)?;
    if total == 0 || idle > total {
        return None;
    }
    let busy = (total - idle) as f64 / total as f64 * 100.0;
    Some(busy.clamp(0.0, 100.0) as f32)
}

/// `/proc/loadavg`'s three averages.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Load {
    pub one: f32,
    pub five: f32,
    pub fifteen: f32,
}

/// Parse `/proc/loadavg`.
pub fn parse_loadavg(text: &str) -> Option<Load> {
    let mut f = text
        .split_ascii_whitespace()
        .map(|v| v.parse::<f32>().ok().filter(|x| x.is_finite() && *x >= 0.0));
    Some(Load {
        one: f.next()??,
        five: f.next()??,
        fifteen: f.next()??,
    })
}

/// Memory and swap from `/proc/meminfo`, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemInfo {
    pub mem_total: u64,
    /// `MemAvailable`, or `MemFree + Buffers + Cached` on kernels older than 3.14.
    pub mem_available: u64,
    pub swap_total: Option<u64>,
    pub swap_free: Option<u64>,
}

impl MemInfo {
    pub fn mem_used(&self) -> u64 {
        self.mem_total.saturating_sub(self.mem_available)
    }

    /// `(total, free, used)`, when the kernel reports swap at all.
    pub fn swap(&self) -> Option<(u64, u64, u64)> {
        let (total, free) = (self.swap_total?, self.swap_free?);
        let free = free.min(total);
        Some((total, free, total - free))
    }
}

/// Parse `/proc/meminfo`. `None` without `MemTotal`.
pub fn parse_meminfo(text: &str) -> Option<MemInfo> {
    let mut total = None;
    let mut available = None;
    let (mut free, mut buffers, mut cached) = (None, None, None);
    let (mut swap_total, mut swap_free) = (None, None);
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let Some(bytes) = kib(rest) else { continue };
        let slot = match key.trim() {
            "MemTotal" => &mut total,
            "MemAvailable" => &mut available,
            "MemFree" => &mut free,
            "Buffers" => &mut buffers,
            "Cached" => &mut cached,
            "SwapTotal" => &mut swap_total,
            "SwapFree" => &mut swap_free,
            _ => continue,
        };
        *slot = Some(bytes);
    }
    let mem_total = total?;
    let mem_available = match available {
        Some(a) => a,
        None => free?
            .saturating_add(buffers.unwrap_or(0))
            .saturating_add(cached.unwrap_or(0)),
    }
    .min(mem_total);
    Some(MemInfo {
        mem_total,
        mem_available,
        swap_total,
        swap_free,
    })
}

/// `"  16384 kB"` → bytes.
fn kib(value: &str) -> Option<u64> {
    let mut parts = value.split_ascii_whitespace();
    let n: u64 = parts.next()?.parse().ok()?;
    match parts.next() {
        Some("kB") | None => n.checked_mul(1024),
        Some(_) => None,
    }
}

/// Whole seconds since boot, from `/proc/uptime`.
pub fn parse_uptime(text: &str) -> Option<u64> {
    let first = text.split_ascii_whitespace().next()?;
    let secs: f64 = first.parse().ok()?;
    (secs.is_finite() && secs >= 0.0 && secs < u64::MAX as f64).then_some(secs as u64)
}

/// Disk figures from `statvfs`, in bytes: `(total, used, free to unprivileged writers)`.
pub fn disk_bytes(
    block_size: u64,
    blocks: u64,
    blocks_free: u64,
    blocks_avail: u64,
) -> Option<(u64, u64, u64)> {
    let total = blocks.checked_mul(block_size)?;
    let used = blocks
        .saturating_sub(blocks_free.min(blocks))
        .checked_mul(block_size)?;
    let free = blocks_avail.min(blocks).checked_mul(block_size)?;
    Some((total, used, free))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "\
cpu  4705 356 584 3699 23 23 0 0 0 0
cpu0 1393280 32966 572056 13343292 6130 0 17875 0 23933 0
cpu1 1335085 31540 573543 13314946 5848 0 1166 0 25017 0
cpu2 1 2 3 4 5 6 7 8 0 0
cpu3 1 2 3 4 5 6 7 8 0 0
intr 1462898 0 0
ctxt 115315
btime 769041601
processes 86031
procs_running 6
procs_blocked 0
softirq 1 2 3
";

    #[test]
    fn stat_reads_the_aggregate_line_and_counts_cores() {
        let s = parse_stat(STAT).unwrap();
        assert_eq!(s.cores, 4);
        assert_eq!(
            s.times,
            CpuTimes {
                total: 4705 + 356 + 584 + 3699 + 23 + 23,
                idle: 3699 + 23
            }
        );
    }

    #[test]
    fn stat_leaves_guest_time_out_of_the_total() {
        let s = parse_stat("cpu 10 0 0 10 0 0 0 0 999 999\n").unwrap();
        assert_eq!(s.times.total, 20);
    }

    #[test]
    fn stat_without_a_usable_cpu_line_is_none() {
        assert_eq!(parse_stat(""), None);
        assert_eq!(parse_stat("cpu0 1 2 3 4\n"), None);
        assert_eq!(parse_stat("cpu 1 2 x 4\n"), None);
        assert_eq!(parse_stat("cpu 1 2 3\n"), None);
        assert_eq!(
            parse_stat(&format!("cpu {} {} 0 0\n", u64::MAX, u64::MAX)),
            None
        );
        assert_eq!(parse_stat("\u{0}\u{ffff}garbage"), None);
    }

    #[test]
    fn stat_ignores_lines_that_only_look_like_cpus() {
        let s = parse_stat("cpu 1 1 1 1\ncpufreq 1\ncpu1 1 1 1 1\n").unwrap();
        assert_eq!(s.cores, 1);
    }

    #[test]
    fn cpu_percent_is_the_busy_share_of_the_delta() {
        let a = CpuTimes {
            total: 1000,
            idle: 800,
        };
        let b = CpuTimes {
            total: 1200,
            idle: 850,
        };
        assert_eq!(cpu_percent(a, b), Some(75.0));
        assert_eq!(cpu_percent(a, a), None, "no time passed");
        assert_eq!(cpu_percent(b, a), None, "counters went backwards");
        assert_eq!(
            cpu_percent(
                a,
                CpuTimes {
                    total: 1100,
                    idle: 1000
                }
            ),
            None,
            "idle outran total"
        );
        assert_eq!(
            cpu_percent(
                a,
                CpuTimes {
                    total: 1100,
                    idle: 900
                }
            ),
            Some(0.0)
        );
    }

    #[test]
    fn loadavg_reads_three_averages() {
        assert_eq!(
            parse_loadavg("0.52 1.03 2.50 3/1024 12345\n"),
            Some(Load {
                one: 0.52,
                five: 1.03,
                fifteen: 2.5
            })
        );
        assert_eq!(parse_loadavg("0.52 1.03"), None);
        assert_eq!(parse_loadavg("0.52 nan 1"), None);
        assert_eq!(parse_loadavg("-1 0 0"), None);
        assert_eq!(parse_loadavg(""), None);
    }

    const MEMINFO: &str = "\
MemTotal:       32601856 kB
MemFree:         1234567 kB
MemAvailable:   20000000 kB
Buffers:          100000 kB
Cached:          9000000 kB
SwapCached:            0 kB
SwapTotal:       8388604 kB
SwapFree:        8000000 kB
HugePages_Total:       0
Hugepagesize:       2048 kB
";

    #[test]
    fn meminfo_reads_memory_and_swap_in_bytes() {
        let m = parse_meminfo(MEMINFO).unwrap();
        assert_eq!(m.mem_total, 32_601_856 * 1024);
        assert_eq!(m.mem_available, 20_000_000 * 1024);
        assert_eq!(m.mem_used(), 12_601_856 * 1024);
        assert_eq!(
            m.swap(),
            Some((8_388_604 * 1024, 8_000_000 * 1024, 388_604 * 1024))
        );
    }

    #[test]
    fn meminfo_without_mem_available_sums_free_buffers_and_cached() {
        let m =
            parse_meminfo("MemTotal: 1000 kB\nMemFree: 100 kB\nBuffers: 50 kB\nCached: 200 kB\n")
                .unwrap();
        assert_eq!(m.mem_available, 350 * 1024);
        assert_eq!(m.swap(), None);
    }

    #[test]
    fn meminfo_clamps_available_to_total_and_free_swap_to_total() {
        let m = parse_meminfo(
            "MemTotal: 10 kB\nMemAvailable: 20 kB\nSwapTotal: 5 kB\nSwapFree: 9 kB\n",
        )
        .unwrap();
        assert_eq!(m.mem_used(), 0);
        assert_eq!(m.swap(), Some((5 * 1024, 5 * 1024, 0)));
    }

    #[test]
    fn meminfo_without_total_or_with_odd_units_is_none() {
        assert_eq!(parse_meminfo(""), None);
        assert_eq!(parse_meminfo("MemFree: 1 kB\n"), None);
        assert_eq!(parse_meminfo("MemTotal: 1 MB\nMemAvailable: 1 kB\n"), None);
        assert_eq!(parse_meminfo(&format!("MemTotal: {} kB\n", u64::MAX)), None);
        assert_eq!(
            parse_meminfo("MemTotal: 4 kB\n"),
            None,
            "no available figure at all"
        );
    }

    #[test]
    fn uptime_is_whole_seconds() {
        assert_eq!(parse_uptime("350735.47 234388.90\n"), Some(350_735));
        assert_eq!(parse_uptime("-1 0"), None);
        assert_eq!(parse_uptime("inf 0"), None);
        assert_eq!(parse_uptime(""), None);
    }

    #[test]
    fn disk_bytes_multiplies_blocks_and_reserves_root_blocks_from_free() {
        assert_eq!(
            disk_bytes(4096, 1000, 300, 250),
            Some((4096 * 1000, 4096 * 700, 4096 * 250))
        );
        assert_eq!(disk_bytes(4096, 10, 50, 50), Some((40960, 0, 40960)));
        assert_eq!(disk_bytes(u64::MAX, 2, 0, 0), None);
    }
}
