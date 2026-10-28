//! Results Keel has not acknowledged (spec 060 FR-037, contracts/runner-replay.md).
//!
//! A run that restarts Keel can end while Keel is down, so its `Finished` never arrives. The
//! runner writes each `Finished` to `<state_dir>/undelivered/<run_id>.json` (mode 0600, written
//! to a temporary file, fsynced and renamed) before sending it, and deletes the file once Keel
//! acknowledges it. After the next `Hello` it sends what is left, oldest first. A file older than
//! the maximum offline time is dropped unsent, with one warning line naming the run.
//!
//! There is no acknowledgement frame. Keel handles a session's frames in order and answers a
//! runner's `Ping` with `Pong`, so the `Pong` to a `Ping` sent after a `Finished` means that
//! `Finished` was handled; [`Acks`] does that accounting. A Keel that never answers leaves the
//! files to be replayed (it ignores duplicates) until they expire.

use std::collections::VecDeque;
use std::fs;
use std::io::{ErrorKind, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use keel_exec::frames::Frame;
use serde::{Deserialize, Serialize};

/// The maximum offline time when none is configured (Keel's `KEEL_RUNNER_MAX_OFFLINE`).
pub const DEFAULT_MAX_OFFLINE: Duration = Duration::from_secs(30 * 24 * 3600);

/// The directory under the state directory.
const DIR: &str = "undelivered";

#[derive(Serialize, Deserialize)]
struct Stored {
    /// Unix seconds.
    spooled_at: u64,
    frame: Frame,
}

/// One undelivered result.
#[derive(Clone, PartialEq)]
pub struct Pending {
    pub run_id: String,
    pub spooled_at: u64,
    pub frame: Frame,
}

/// `<state_dir>/undelivered`.
#[derive(Debug, Clone)]
pub struct Spool {
    dir: PathBuf,
    max_age: Duration,
}

/// A run id fit for a file name: what Keel sends (a UUID) and nothing that walks a path.
fn file_safe(run_id: &str) -> bool {
    !run_id.is_empty()
        && run_id.len() <= 64
        && run_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

impl Spool {
    pub fn new(state_dir: &Path, max_age: Duration) -> Self {
        Self {
            dir: state_dir.join(DIR),
            max_age,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, run_id: &str) -> PathBuf {
        self.dir.join(format!("{run_id}.json"))
    }

    /// Writes `frame` (a `Finished`; anything else is ignored) atomically with mode 0600.
    /// Returns whether it was written.
    ///
    /// # Errors
    ///
    /// The directory or the file could not be written.
    pub fn write(&self, frame: &Frame, now: u64) -> std::io::Result<bool> {
        let Frame::Finished { run_id, .. } = frame else {
            return Ok(false);
        };
        if !file_safe(run_id) {
            return Ok(false);
        }
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.dir)?;
        let body = serde_json::to_vec(&Stored {
            spooled_at: now,
            frame: frame.clone(),
        })
        .map_err(std::io::Error::other)?;
        let tmp = self.dir.join(format!(".{run_id}.json.tmp"));
        {
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?;
            f.write_all(&body)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, self.path(run_id))?;
        fs::File::open(&self.dir)?.sync_all()?;
        Ok(true)
    }

    /// The undelivered results, oldest first. Expired and unreadable files are deleted here,
    /// each with one warning line.
    ///
    /// # Errors
    ///
    /// The directory could not be read (a missing one is empty).
    pub fn pending(&self, now: u64) -> std::io::Result<Vec<Pending>> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut out = Vec::new();
        for entry in entries {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(run_id) = name.strip_suffix(".json").filter(|r| file_safe(r)) else {
                continue;
            };
            let stored = fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Stored>(&b).ok())
                .filter(|s| matches!(&s.frame, Frame::Finished { run_id: r, .. } if r == run_id));
            let Some(stored) = stored else {
                eprintln!(
                    "keel-runner: undelivered result for run {run_id} is unreadable; dropped"
                );
                let _ = fs::remove_file(&path);
                continue;
            };
            if now.saturating_sub(stored.spooled_at) > self.max_age.as_secs() {
                eprintln!(
                    "keel-runner: undelivered result for run {run_id} is older than the maximum \
                     offline time; dropped unsent"
                );
                let _ = fs::remove_file(&path);
                continue;
            }
            out.push(Pending {
                run_id: run_id.to_owned(),
                spooled_at: stored.spooled_at,
                frame: stored.frame,
            });
        }
        out.sort_by(|a, b| (a.spooled_at, &a.run_id).cmp(&(b.spooled_at, &b.run_id)));
        Ok(out)
    }

    /// Deletes `run_id`'s file, once Keel has it.
    pub fn remove(&self, run_id: &str) {
        if !file_safe(run_id) {
            return;
        }
        match fs::remove_file(self.path(run_id)) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => {
                eprintln!("keel-runner: deleting the undelivered result for run {run_id}: {e}")
            }
        }
    }
}

/// Ping/Pong accounting for one session: which results a `Pong` acknowledges.
#[derive(Debug, Default)]
pub struct Acks {
    pings: u64,
    pongs: u64,
    waiting: VecDeque<(u64, String)>,
}

impl Acks {
    /// Counts a `Ping` being sent.
    pub fn ping(&mut self) {
        self.pings += 1;
    }

    /// Counts a `Ping` sent right after `run_ids`' results: they are acknowledged by its `Pong`.
    pub fn ping_after(&mut self, run_ids: impl IntoIterator<Item = String>) {
        self.ping();
        let marker = self.pings;
        self.waiting
            .extend(run_ids.into_iter().map(|r| (marker, r)));
    }

    /// Counts a `Pong`; returns the runs it acknowledges.
    pub fn pong(&mut self) -> Vec<String> {
        self.pongs = (self.pongs + 1).min(self.pings);
        let mut acked = Vec::new();
        while let Some((marker, _)) = self.waiting.front() {
            if *marker > self.pongs {
                break;
            }
            if let Some((_, r)) = self.waiting.pop_front() {
                acked.push(r);
            }
        }
        acked
    }
}

/// Parses a duration such as `30d`, `12h`, `90m`, `45s` or a bare number of seconds.
///
/// # Errors
///
/// Anything else, or zero.
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    let (n, unit) = match s.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((i, _)) => s.split_at(i),
        None => (s, "s"),
    };
    let n: u64 = n.parse().map_err(|_| format!("not a duration: {s}"))?;
    let secs = match unit {
        "s" => Some(n),
        "m" => n.checked_mul(60),
        "h" => n.checked_mul(3600),
        "d" => n.checked_mul(86_400),
        _ => None,
    }
    .filter(|&v| v > 0)
    .ok_or_else(|| format!("not a duration: {s} (use e.g. 30d, 12h, 90m)"))?;
    Ok(Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    fn finished(run: &str) -> Frame {
        Frame::Finished {
            run_id: run.to_owned(),
            exit_code: 0,
            stats: BTreeMap::new(),
            ledger_line: String::new(),
        }
    }

    fn spool() -> (tempfile::TempDir, Spool) {
        let dir = tempfile::tempdir().unwrap();
        let s = Spool::new(dir.path(), Duration::from_secs(3600));
        (dir, s)
    }

    #[test]
    fn the_write_is_atomic_with_mode_0600() {
        let (_d, s) = spool();
        assert!(s.write(&finished("r-1"), 100).unwrap());
        let meta = fs::metadata(s.dir().join("r-1.json")).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        let dir = fs::metadata(s.dir()).unwrap();
        assert_eq!(dir.permissions().mode() & 0o777, 0o700);
        // No temporary file is left behind.
        let names: Vec<_> = fs::read_dir(s.dir())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("r-1.json")]);
        // Writing again replaces it whole.
        assert!(s.write(&finished("r-1"), 200).unwrap());
        let p = s.pending(200).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].spooled_at, 200);
        assert!(p[0].frame == finished("r-1"));
    }

    #[test]
    fn only_finished_frames_with_safe_run_ids_are_written() {
        let (_d, s) = spool();
        assert!(!s.write(&Frame::Ping, 1).unwrap());
        assert!(!s.write(&finished("../escape"), 1).unwrap());
        assert!(!s.write(&finished(""), 1).unwrap());
        assert!(s.pending(1).unwrap().is_empty());
    }

    #[test]
    fn replay_runs_oldest_first() {
        let (_d, s) = spool();
        s.write(&finished("c"), 30).unwrap();
        s.write(&finished("a"), 10).unwrap();
        s.write(&finished("b"), 20).unwrap();
        let order: Vec<String> = s
            .pending(40)
            .unwrap()
            .into_iter()
            .map(|p| p.run_id)
            .collect();
        assert_eq!(order, ["a", "b", "c"]);
    }

    #[test]
    fn the_file_is_deleted_on_acknowledgement() {
        let (_d, s) = spool();
        s.write(&finished("r-1"), 1).unwrap();
        s.write(&finished("r-2"), 2).unwrap();
        let mut acks = Acks::default();
        acks.ping(); // a keepalive before the results
        acks.ping_after(["r-1".to_owned(), "r-2".to_owned()]);
        // The keepalive's Pong acknowledges nothing.
        assert!(acks.pong().is_empty());
        for r in acks.pong() {
            s.remove(&r);
        }
        assert!(s.pending(3).unwrap().is_empty());
        // A stray Pong past the pings changes nothing.
        assert!(acks.pong().is_empty());
    }

    #[test]
    fn without_a_pong_the_file_stays_for_the_next_session() {
        let (_d, s) = spool();
        s.write(&finished("r-1"), 1).unwrap();
        let mut acks = Acks::default();
        acks.ping_after(["r-1".to_owned()]);
        drop(acks); // the session ends
        assert_eq!(s.pending(2).unwrap().len(), 1);
    }

    #[test]
    fn an_old_file_expires_unsent() {
        let (_d, s) = spool();
        s.write(&finished("old"), 0).unwrap();
        s.write(&finished("new"), 3000).unwrap();
        let p = s.pending(3601 + 1).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].run_id, "new");
        assert!(!s.dir().join("old.json").exists());
    }

    #[test]
    fn an_unreadable_file_is_dropped() {
        let (_d, s) = spool();
        s.write(&finished("ok"), 1).unwrap();
        fs::write(s.dir().join("bad.json"), b"{not json").unwrap();
        // A file whose frame names another run is not trusted either.
        fs::write(
            s.dir().join("other.json"),
            serde_json::to_vec(&Stored {
                spooled_at: 1,
                frame: finished("ok"),
            })
            .unwrap(),
        )
        .unwrap();
        let p = s.pending(2).unwrap();
        assert_eq!(p.len(), 1);
        assert!(!s.dir().join("bad.json").exists());
        assert!(!s.dir().join("other.json").exists());
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("30d"), Ok(DEFAULT_MAX_OFFLINE));
        assert_eq!(parse_duration("90m"), Ok(Duration::from_secs(5400)));
        assert_eq!(parse_duration("12h"), Ok(Duration::from_secs(43_200)));
        assert_eq!(parse_duration("45"), Ok(Duration::from_secs(45)));
        assert!(parse_duration("0d").is_err());
        assert!(parse_duration("3w").is_err());
        assert!(parse_duration("").is_err());
    }
}
