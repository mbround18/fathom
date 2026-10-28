//! Keel's host runner (spec 055 US7, contracts/runner.md, research R2).
//!
//! Two processes, two users:
//! - [`runner`] (`keel-runner`, user `keel-runner`): the network half. It holds the SSH channel
//!   to Keel, enrolls, renews its certificate, and relays run frames to and from the executor
//!   socket without interpreting them.
//! - [`executor`] (`keel-executor`, user `keel-exec`, one instance per socket connection): runs
//!   one job. It trusts nothing the network half says: it checks the ledger, the pack, the job's
//!   profiles and its local policy itself, then runs `ansible-playbook` against this host only.
//!
//! [`policy`] and [`venv`] are read by both halves.

pub mod executor;
pub mod policy;
pub mod runner;
pub mod venv;

use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;

/// Writes `bytes` to `path` through a `0600` temporary file in the same directory and a rename,
/// so a reader never sees a partial file.
pub(crate) fn write_private_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

/// Creates `path` exclusively with mode `0600` and writes `bytes`.
pub(crate) fn write_private_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}
