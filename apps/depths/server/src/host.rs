//! Host metrics (`WatchHost`): bounded reads of the host's `/proc` (`DEPTHS_HOST_PROC`), parsed by
//! `depths_core::host`, plus `statvfs` of the stacks directory.
//!
//! Every read is capped at [`MAX_PROC_READ`] bytes and every failure becomes a `None` figure, so
//! an odd or hostile mount can neither exhaust memory nor stop the watch.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use depths_core::host::{self, CpuTimes};
use depths_protocol::{HostDisk, HostLoad, HostMemory, HostSample, HostSwap};

/// The most read from any one `/proc` file. `/proc/stat` on a 256-core host is about 40 KiB, most
/// of it per-CPU lines after the aggregate one that matters.
pub const MAX_PROC_READ: u64 = 64 * 1024;

/// Reads at most [`MAX_PROC_READ`] bytes of `path` as text; `None` when it cannot be read.
fn read_bounded(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    file.take(MAX_PROC_READ).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// One watch's sampler. Keeps the previous CPU reading so each sample reports the share since the
/// last one.
pub struct HostProbe {
    proc_dir: PathBuf,
    disk_path: PathBuf,
    prev: Option<CpuTimes>,
}

impl HostProbe {
    pub fn new(proc_dir: PathBuf, disk_path: PathBuf) -> Self {
        Self {
            proc_dir,
            disk_path,
            prev: None,
        }
    }

    /// Reads `/proc/stat` once without reporting, so the first sample has a delta to work from.
    pub fn prime(&mut self) {
        self.prev = read_bounded(&self.proc_dir.join("stat"))
            .and_then(|t| host::parse_stat(&t))
            .map(|s| s.times);
    }

    /// One sample. Blocking (small file reads and `statvfs`): run it off the async workers.
    pub fn sample(
        &mut self,
        at: String,
        containers_running: u32,
        containers_stopped: u32,
    ) -> HostSample {
        let stat = read_bounded(&self.proc_dir.join("stat")).and_then(|t| host::parse_stat(&t));
        let cpu_percent = match (self.prev, stat) {
            (Some(prev), Some(now)) => host::cpu_percent(prev, now.times),
            _ => None,
        };
        // Keep the old baseline when this read failed, so the next good one still has a delta.
        if let Some(now) = stat {
            self.prev = Some(now.times);
        }
        let load = read_bounded(&self.proc_dir.join("loadavg"))
            .and_then(|t| host::parse_loadavg(&t))
            .map(|l| HostLoad {
                one: l.one,
                five: l.five,
                fifteen: l.fifteen,
            });
        let meminfo =
            read_bounded(&self.proc_dir.join("meminfo")).and_then(|t| host::parse_meminfo(&t));
        let memory = meminfo.map(|m| HostMemory {
            total_bytes: m.mem_total,
            available_bytes: m.mem_available,
            used_bytes: m.mem_used(),
        });
        let swap = meminfo
            .and_then(|m| m.swap())
            .map(|(total, free, used)| HostSwap {
                total_bytes: total,
                free_bytes: free,
                used_bytes: used,
            });
        let uptime_secs =
            read_bounded(&self.proc_dir.join("uptime")).and_then(|t| host::parse_uptime(&t));
        HostSample {
            at,
            cpu_percent,
            cpu_cores: stat.map(|s| s.cores).filter(|n| *n > 0),
            load,
            memory,
            swap,
            uptime_secs,
            disk: disk(&self.disk_path),
            containers_running,
            containers_stopped,
        }
    }
}

/// `statvfs` of `path`, in bytes.
fn disk(path: &Path) -> Option<HostDisk> {
    let v = rustix::fs::statvfs(path).ok()?;
    let (total, used, free) = host::disk_bytes(v.f_frsize, v.f_blocks, v.f_bfree, v.f_bavail)?;
    Some(HostDisk {
        total_bytes: total,
        used_bytes: used,
        free_bytes: free,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(stat: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("stat"), stat).unwrap();
        std::fs::write(d.path().join("loadavg"), "0.50 0.25 0.10 1/100 42\n").unwrap();
        std::fs::write(
            d.path().join("meminfo"),
            "MemTotal: 1000 kB\nMemAvailable: 400 kB\nSwapTotal: 100 kB\nSwapFree: 75 kB\n",
        )
        .unwrap();
        std::fs::write(d.path().join("uptime"), "123.45 99.0\n").unwrap();
        d
    }

    #[test]
    fn a_sample_reads_every_file_and_the_disk() {
        let d = fixture("cpu 10 0 10 80 0 0 0 0\ncpu0 10 0 10 80\ncpu1 0 0 0 0\n");
        let mut p = HostProbe::new(d.path().to_owned(), d.path().to_owned());
        p.prime();
        std::fs::write(
            d.path().join("stat"),
            "cpu 40 0 10 100 0 0 0 0\ncpu0 1 1 1 1\ncpu1 1 1 1 1\n",
        )
        .unwrap();
        let s = p.sample("t".into(), 2, 1);
        assert_eq!(s.cpu_percent, Some(60.0), "30 busy of 50 ticks");
        assert_eq!(s.cpu_cores, Some(2));
        assert_eq!(
            s.load,
            Some(HostLoad {
                one: 0.5,
                five: 0.25,
                fifteen: 0.1
            })
        );
        assert_eq!(
            s.memory,
            Some(HostMemory {
                total_bytes: 1000 * 1024,
                available_bytes: 400 * 1024,
                used_bytes: 600 * 1024
            })
        );
        assert_eq!(
            s.swap,
            Some(HostSwap {
                total_bytes: 100 * 1024,
                free_bytes: 75 * 1024,
                used_bytes: 25 * 1024
            })
        );
        assert_eq!(s.uptime_secs, Some(123));
        let disk = s.disk.expect("statvfs of a tempdir");
        assert!(disk.total_bytes > 0 && disk.used_bytes <= disk.total_bytes);
        assert_eq!((s.containers_running, s.containers_stopped), (2, 1));
    }

    #[test]
    fn the_first_sample_without_a_baseline_has_no_cpu_share() {
        let d = fixture("cpu 1 1 1 1\n");
        let mut p = HostProbe::new(d.path().to_owned(), d.path().to_owned());
        assert_eq!(p.sample("t".into(), 0, 0).cpu_percent, None);
        assert_eq!(
            p.sample("t".into(), 0, 0).cpu_percent,
            None,
            "no ticks passed"
        );
    }

    #[test]
    fn a_missing_or_garbage_proc_yields_nones_not_a_panic() {
        let d = tempfile::tempdir().unwrap();
        let mut p = HostProbe::new(d.path().join("nope"), d.path().join("nope"));
        p.prime();
        let s = p.sample("t".into(), 0, 0);
        assert_eq!(
            (
                s.cpu_percent,
                s.cpu_cores,
                s.load,
                s.memory,
                s.swap,
                s.uptime_secs,
                s.disk
            ),
            (None, None, None, None, None, None, None)
        );

        for f in ["stat", "loadavg", "meminfo", "uptime"] {
            std::fs::write(d.path().join(f), [0xff_u8, 0xfe, 0, b'\n', b'x']).unwrap();
        }
        let mut p = HostProbe::new(d.path().to_owned(), d.path().to_owned());
        p.prime();
        let s = p.sample("t".into(), 0, 0);
        assert_eq!(
            (s.cpu_percent, s.load, s.memory, s.uptime_secs),
            (None, None, None, None)
        );
    }

    #[test]
    fn reads_are_bounded() {
        let d = tempfile::tempdir().unwrap();
        let big = d.path().join("big");
        std::fs::write(&big, vec![b'a'; (MAX_PROC_READ as usize) * 2]).unwrap();
        assert_eq!(read_bounded(&big).unwrap().len() as u64, MAX_PROC_READ);
    }
}
