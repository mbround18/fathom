//! `ReadSmart` → `Smart` (spec 067 US5 `disk_health`): the SMART verdict of each whole disk.
//!
//! The request carries nothing: the executor picks the disks itself from `/sys/block`, skipping
//! virtual devices, and runs `smartctl --json -H /dev/<name>` from a fixed absolute path, as an
//! argv array with an empty environment. A missing `smartctl` answers `installed: false`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use keel_exec::frames::{Frame, SmartDisk};
use tokio::time::Instant;

/// Where `smartctl` may be; nothing else is run, and `PATH` is never searched.
pub const SMARTCTL_PATHS: [&str; 2] = ["/usr/sbin/smartctl", "/usr/bin/smartctl"];
/// The most disks asked about.
const MAX_DISKS: usize = 32;
/// Every `smartctl` call together, inside the network half's 10 s wait.
const BUDGET: Duration = Duration::from_secs(8);
/// The most read from one `smartctl` call.
const MAX_OUTPUT: usize = 1024 * 1024;

/// Virtual block devices that have no SMART.
const VIRTUAL_PREFIXES: [&str; 7] = ["loop", "ram", "zram", "dm-", "md", "sr", "nbd"];

/// A `/sys/block` name worth asking about: one path component of safe characters, not virtual.
pub fn is_disk_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !VIRTUAL_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// The whole disks under `sys_block`, sorted.
pub fn list_disks(sys_block: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(sys_block)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| e.file_name().to_str().map(str::to_owned))
                .filter(|n| is_disk_name(n))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names.truncate(MAX_DISKS);
    names
}

/// `smartctl --json -H -- /dev/<name>`, argv only. `name` must pass [`is_disk_name`].
pub fn smartctl_command(smartctl: &Path, name: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(smartctl);
    let mut dev = OsString::from("/dev/");
    dev.push(name);
    cmd.args([
        OsString::from("--json"),
        OsString::from("-H"),
        OsString::from("--"),
        dev,
    ])
    .env_clear()
    .env("LANG", "C")
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    cmd
}

/// `smart_status.passed` from `smartctl --json` output.
pub fn parse_passed(output: &[u8]) -> Option<bool> {
    let v: serde_json::Value = serde_json::from_slice(output).ok()?;
    v.get("smart_status")?.get("passed")?.as_bool()
}

/// The `Smart` answer.
pub async fn read_smart(sys_block: &Path, smartctl: Option<PathBuf>) -> Frame {
    let Some(smartctl) = smartctl else {
        return Frame::Smart {
            installed: false,
            disks: vec![],
        };
    };
    let deadline = Instant::now() + BUDGET;
    let mut disks = Vec::new();
    for name in list_disks(sys_block) {
        let mut cmd = tokio::process::Command::from(smartctl_command(&smartctl, &name));
        cmd.kill_on_drop(true);
        let passed = match cmd.spawn() {
            Ok(child) => match tokio::time::timeout_at(deadline, child.wait_with_output()).await {
                // smartctl's exit status is a bit mask that is non-zero for many healthy disks;
                // the JSON verdict is what counts.
                Ok(Ok(out)) if out.stdout.len() <= MAX_OUTPUT => parse_passed(&out.stdout),
                _ => None,
            },
            Err(_) => None,
        };
        disks.push(SmartDisk {
            name,
            smart_passed: passed,
        });
    }
    Frame::Smart {
        installed: true,
        disks,
    }
}

/// The first `smartctl` of [`SMARTCTL_PATHS`] that exists.
pub fn find_smartctl() -> Option<PathBuf> {
    SMARTCTL_PATHS
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
}
