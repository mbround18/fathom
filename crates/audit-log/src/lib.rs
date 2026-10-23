//! `audit-log`: an append-only JSON-lines audit log (spec 050), shared by Fathom's hosted
//! server and Depths. No dependency on `fathom` or `kube`.
//!
//! - `<dir>/audit-log.jsonl`, directory `0700`, file `0600` (Unix).
//! - [`AuditLog::append`] serializes the entry and writes the whole line with one write on an
//!   `O_APPEND` descriptor, under one mutex per log, so concurrent appends never interleave.
//!   An append after a torn line (a crash mid-write) starts on a fresh line.
//! - Unparseable lines are skipped on read.
//! - Retention:
//!   - [`Retention::CompactAfter`]`(n)`: once the file holds more than `2n` entries it is
//!     rewritten (temp file, then rename) to the newest `n`. Fathom uses `10_000`.
//!   - [`Retention::RotateAtBytes`]`(max)`: before a write would take the live file past `max`
//!     bytes it is renamed to `audit-log.<n>.jsonl`, `n` one more than the highest so far.
//!     Entries are never rewritten or deleted; pruning old files is the operator's call.
//! - There is no edit or delete operation.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use serde::de::DeserializeOwned;

/// The live file's name under [`AuditOptions::dir`].
pub const AUDIT_FILE: &str = "audit-log.jsonl";
const STEM: &str = "audit-log";
const EXT: &str = "jsonl";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    /// Keep the newest `n` entries, compacting once the file passes `2n`.
    CompactAfter(usize),
    /// Rotate the live file before it would pass this many bytes.
    RotateAtBytes(u64),
}

#[derive(Debug, Clone)]
pub struct AuditOptions {
    pub dir: PathBuf,
    pub retention: Retention,
}

/// Where the next [`AuditLog::read`] page starts: entries older than this position.
///
/// Stable across appends and rotation. A [`Retention::CompactAfter`] compaction renumbers
/// lines, so a cursor taken before one may skip or repeat entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Cursor {
    /// File number: rotated files keep theirs, the live file is one past the highest.
    segment: u64,
    /// Index of the oldest entry already returned, among the file's non-blank lines.
    line: usize,
}

#[derive(Debug, Clone)]
pub struct Page<E> {
    /// Newest first.
    pub entries: Vec<E>,
    /// `Some` when the page filled up: pass it back for the older entries (the next page may
    /// turn out empty).
    pub next: Option<Cursor>,
}

pub struct AuditLog<E> {
    dir: PathBuf,
    path: PathBuf,
    retention: Retention,
    /// The one writer; also orders reads against rotation and compaction. Holds the live
    /// file's entry count once known (for [`Retention::CompactAfter`]).
    lines: Mutex<Option<usize>>,
    _e: PhantomData<fn() -> E>,
}

impl<E: Serialize + DeserializeOwned> AuditLog<E> {
    /// A log in `opts.dir` that touches the filesystem only on the first append.
    pub fn new(opts: AuditOptions) -> Self {
        let retention = match opts.retention {
            Retention::CompactAfter(n) => Retention::CompactAfter(n.max(1)),
            Retention::RotateAtBytes(b) => Retention::RotateAtBytes(b.max(1)),
        };
        Self {
            path: opts.dir.join(AUDIT_FILE),
            dir: opts.dir,
            retention,
            lines: Mutex::new(None),
            _e: PhantomData,
        }
    }

    /// Like [`AuditLog::new`], creating the directory (`0700`) and file (`0600`) now.
    pub fn open(opts: AuditOptions) -> io::Result<Self> {
        let log = Self::new(opts);
        log.open_append()?;
        Ok(log)
    }

    /// The live file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, entry: &E) -> io::Result<()> {
        let mut line = serde_json::to_string(entry).map_err(io::Error::other)?;
        line.push('\n');
        let mut lines = self.lines.lock().unwrap_or_else(|e| e.into_inner());
        let mut file = self.open_append()?;
        if let Retention::RotateAtBytes(max) = self.retention {
            let len = file.metadata()?.len();
            if len > 0 && len + line.len() as u64 > max {
                drop(file);
                let n = self.rotated()?.last().copied().unwrap_or(0) + 1;
                fs::rename(&self.path, self.segment_path(n))?;
                file = self.open_append()?;
            }
        }
        if needs_leading_newline(&mut file)? {
            line.insert(0, '\n');
        }
        // One write of the whole line on an `O_APPEND` descriptor.
        file.write_all(line.as_bytes())?;
        file.flush()?;
        if let Retention::CompactAfter(keep) = self.retention {
            let count = match *lines {
                Some(n) => n + 1,
                None => count_lines(&self.path)?,
            };
            *lines = Some(count);
            if count > keep.saturating_mul(2) {
                *lines = Some(self.compact(keep)?);
            }
        }
        Ok(())
    }

    /// Up to `limit` entries matching `filter`, newest first, older than `cursor`.
    pub fn read(
        &self,
        filter: impl Fn(&E) -> bool,
        cursor: Option<Cursor>,
        limit: usize,
    ) -> io::Result<Page<E>> {
        let _guard = self.lines.lock().unwrap_or_else(|e| e.into_inner());
        let mut entries = Vec::new();
        if limit == 0 {
            return Ok(Page {
                entries,
                next: cursor,
            });
        }
        let rotated = self.rotated()?;
        let live = rotated.last().copied().unwrap_or(0) + 1;
        let segments = std::iter::once(live).chain(rotated.into_iter().rev());
        for seg in segments {
            let below = match cursor {
                Some(c) if seg > c.segment => continue,
                Some(c) if seg == c.segment => c.line,
                _ => usize::MAX,
            };
            let path = if seg == live {
                self.path.clone()
            } else {
                self.segment_path(seg)
            };
            for (idx, e) in read_lines::<E>(&path)?.into_iter().rev() {
                if idx >= below || !filter(&e) {
                    continue;
                }
                entries.push(e);
                if entries.len() == limit {
                    return Ok(Page {
                        entries,
                        next: Some(Cursor {
                            segment: seg,
                            line: idx,
                        }),
                    });
                }
            }
        }
        Ok(Page {
            entries,
            next: None,
        })
    }

    /// Every readable entry, oldest first.
    pub fn all(&self) -> io::Result<Vec<E>> {
        let mut v = self.read(|_| true, None, usize::MAX)?.entries;
        v.reverse();
        Ok(v)
    }

    fn segment_path(&self, n: u64) -> PathBuf {
        self.dir.join(format!("{STEM}.{n}.{EXT}"))
    }

    /// Numbers of the rotated files, ascending.
    fn rotated(&self) -> io::Result<Vec<u64>> {
        let rd = match fs::read_dir(&self.dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut v: Vec<u64> = rd
            .filter_map(Result::ok)
            .filter_map(|e| {
                e.file_name()
                    .to_str()?
                    .strip_prefix(STEM)?
                    .strip_prefix('.')?
                    .strip_suffix(EXT)?
                    .strip_suffix('.')?
                    .parse()
                    .ok()
            })
            .collect();
        v.sort_unstable();
        Ok(v)
    }

    fn open_append(&self) -> io::Result<File> {
        create_private_dir(&self.dir)?;
        let mut opts = OpenOptions::new();
        opts.create(true).append(true).read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        opts.open(&self.path)
    }

    /// Keep the newest `keep` entries. Caller holds the writer mutex.
    fn compact(&self, keep: usize) -> io::Result<usize> {
        let entries = read_lines::<E>(&self.path)?;
        let kept = &entries[entries.len().saturating_sub(keep)..];
        let tmp = self.path.with_extension(format!("{EXT}.tmp"));
        {
            let mut opts = OpenOptions::new();
            opts.create(true).write(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = opts.open(&tmp)?;
            for (_, e) in kept {
                let mut line = serde_json::to_string(e).map_err(io::Error::other)?;
                line.push('\n');
                f.write_all(line.as_bytes())?;
            }
            f.sync_all()?;
        }
        fs::rename(&tmp, &self.path)?;
        Ok(kept.len())
    }
}

fn create_private_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir)
    }
}

/// True when the file is non-empty and does not end in `\n` (a torn previous write).
fn needs_leading_newline(file: &mut File) -> io::Result<bool> {
    let len = file.metadata()?.len();
    if len == 0 {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(len - 1))?;
    let mut last = [0u8; 1];
    file.read_exact(&mut last)?;
    Ok(last[0] != b'\n')
}

fn count_lines(path: &Path) -> io::Result<usize> {
    let f = File::open(path)?;
    Ok(BufReader::new(f)
        .split(b'\n')
        .filter(|l| l.as_ref().is_ok_and(|l| !l.is_empty()))
        .count())
}

/// The readable entries with their index among the file's non-blank lines. A missing file
/// reads as empty.
fn read_lines<E: DeserializeOwned>(path: &Path) -> io::Result<Vec<(usize, E)>> {
    let f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut skipped = 0usize;
    let mut out = Vec::new();
    let lines = BufReader::new(f)
        .split(b'\n')
        .filter_map(Result::ok)
        .filter(|l| !l.iter().all(u8::is_ascii_whitespace));
    for (idx, l) in lines.enumerate() {
        match serde_json::from_slice(&l) {
            Ok(e) => out.push((idx, e)),
            Err(_) => skipped += 1,
        }
    }
    if skipped > 0 {
        tracing::warn!(
            "audit log {}: skipped {skipped} unreadable line(s)",
            path.display()
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;

    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Ev {
        who: String,
        n: u32,
    }

    fn ev(who: &str, n: u32) -> Ev {
        Ev { who: who.into(), n }
    }

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    fn tmp(name: &str) -> (Tmp, PathBuf) {
        let root = tempfile::Builder::new()
            .prefix(&format!("audit-log-{name}-"))
            .tempdir()
            .unwrap()
            .keep();
        let dir = root.join("audit");
        (Tmp(root), dir)
    }

    fn open(dir: &Path, retention: Retention) -> AuditLog<Ev> {
        AuditLog::open(AuditOptions {
            dir: dir.to_owned(),
            retention,
        })
        .unwrap()
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[cfg(unix)]
    #[test]
    fn dir_is_0700_and_file_0600() {
        use std::os::unix::fs::PermissionsExt;
        let (_t, dir) = tmp("mode");
        let log = open(&dir, Retention::CompactAfter(100));
        log.append(&ev("a", 1)).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join(AUDIT_FILE)), 0o600);
        assert_eq!(log.path(), dir.join(AUDIT_FILE));
    }

    /// `O_APPEND`: an entry lands at the end even when something else wrote in between.
    #[test]
    fn appends_go_to_the_end_of_the_file() {
        let (_t, dir) = tmp("append");
        let log = open(&dir, Retention::CompactAfter(100));
        log.append(&ev("a", 1)).unwrap();
        {
            use std::io::Write;
            let mut f = fs::OpenOptions::new()
                .append(true)
                .open(dir.join(AUDIT_FILE))
                .unwrap();
            f.write_all(b"{\"who\":\"other\",\"n\":9}\n").unwrap();
        }
        log.append(&ev("a", 2)).unwrap();
        let raw = fs::read_to_string(dir.join(AUDIT_FILE)).unwrap();
        let lines: Vec<&str> = raw.lines().collect();
        assert_eq!(lines.len(), 3, "{raw}");
        assert_eq!(lines[2], r#"{"who":"a","n":2}"#);
        assert_eq!(log.all().unwrap(), [ev("a", 1), ev("other", 9), ev("a", 2)]);
    }

    /// One write per entry under a mutex: concurrent appenders never interleave.
    #[test]
    fn concurrent_appends_stay_whole_lines() {
        let (_t, dir) = tmp("concurrent");
        let log = Arc::new(open(&dir, Retention::CompactAfter(10_000)));
        let per = 200;
        let hs: Vec<_> = ["alice", "bob"]
            .into_iter()
            .map(|who| {
                let log = log.clone();
                std::thread::spawn(move || {
                    for i in 0..per {
                        log.append(&ev(who, i)).unwrap();
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        let all = log.all().unwrap();
        assert_eq!(all.len(), 2 * per as usize);
        for who in ["alice", "bob"] {
            let mine: Vec<u32> = all.iter().filter(|e| e.who == who).map(|e| e.n).collect();
            assert_eq!(mine, (0..per).collect::<Vec<_>>());
        }
    }

    #[test]
    fn skips_corrupt_lines_and_never_glues_to_a_torn_one() {
        let (_t, dir) = tmp("torn");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(AUDIT_FILE),
            "not json\n{\"who\":\"a\",\"n\":1}\n{\"who\":\"to",
        )
        .unwrap();
        let log = open(&dir, Retention::CompactAfter(100));
        assert_eq!(log.all().unwrap(), [ev("a", 1)]);
        log.append(&ev("b", 2)).unwrap();
        assert_eq!(log.all().unwrap(), [ev("a", 1), ev("b", 2)]);
    }

    #[test]
    fn a_missing_file_reads_as_empty_and_new_touches_nothing() {
        let (_t, dir) = tmp("missing");
        let log: AuditLog<Ev> = AuditLog::new(AuditOptions {
            dir: dir.clone(),
            retention: Retention::CompactAfter(10),
        });
        assert!(!dir.exists(), "new() is lazy");
        assert!(log.all().unwrap().is_empty());
        log.append(&ev("a", 1)).unwrap();
        assert_eq!(log.all().unwrap(), [ev("a", 1)]);
    }

    /// Fathom's behaviour: once the file holds more than twice the bound, keep the newest.
    #[test]
    fn compact_after_keeps_the_newest_past_twice_the_bound() {
        let (_t, dir) = tmp("compact");
        let log = open(&dir, Retention::CompactAfter(5));
        for i in 0..10 {
            log.append(&ev("a", i)).unwrap();
        }
        assert_eq!(log.all().unwrap().len(), 10, "not past 2x yet");
        log.append(&ev("a", 10)).unwrap();
        let all = log.all().unwrap();
        assert_eq!(
            all.iter().map(|e| e.n).collect::<Vec<_>>(),
            [6, 7, 8, 9, 10]
        );
        log.append(&ev("a", 11)).unwrap();
        assert_eq!(log.all().unwrap().len(), 6);
        assert_eq!(names(&dir), [AUDIT_FILE], "no temp file left behind");
    }

    #[test]
    fn compact_after_counts_lines_already_on_disk() {
        let (_t, dir) = tmp("compact-reopen");
        {
            let log = open(&dir, Retention::CompactAfter(2));
            for i in 0..4 {
                log.append(&ev("a", i)).unwrap();
            }
        }
        let log = open(&dir, Retention::CompactAfter(2));
        log.append(&ev("a", 4)).unwrap();
        assert_eq!(
            log.all().unwrap().iter().map(|e| e.n).collect::<Vec<_>>(),
            [3, 4]
        );
    }

    /// Rotation renames the live file to `audit-log.<n>.jsonl` and never rewrites an entry:
    /// every byte ever appended is still on disk, in order.
    #[test]
    fn rotate_at_bytes_renames_and_never_rewrites() {
        let (_t, dir) = tmp("rotate");
        let line_len = serde_json::to_string(&ev("a", 0)).unwrap().len() as u64 + 1;
        // Room for three entries per file.
        let log = open(&dir, Retention::RotateAtBytes(line_len * 3));
        for i in 0..8 {
            log.append(&ev("a", i)).unwrap();
        }
        assert_eq!(
            names(&dir),
            ["audit-log.1.jsonl", "audit-log.2.jsonl", AUDIT_FILE]
        );
        let first = fs::read_to_string(dir.join("audit-log.1.jsonl")).unwrap();
        assert_eq!(first.lines().count(), 3);
        assert!(first.starts_with(r#"{"who":"a","n":0}"#), "{first}");
        for f in ["audit-log.1.jsonl", "audit-log.2.jsonl", AUDIT_FILE] {
            assert!(
                fs::metadata(dir.join(f)).unwrap().len() <= line_len * 3,
                "{f}"
            );
        }
        assert_eq!(
            log.all().unwrap().iter().map(|e| e.n).collect::<Vec<_>>(),
            (0..8).collect::<Vec<_>>()
        );
        // A rotated file is never touched again.
        let before = fs::read(dir.join("audit-log.1.jsonl")).unwrap();
        for i in 8..20 {
            log.append(&ev("a", i)).unwrap();
        }
        assert_eq!(fs::read(dir.join("audit-log.1.jsonl")).unwrap(), before);
        assert_eq!(log.all().unwrap().len(), 20);
    }

    #[test]
    fn rotation_continues_numbering_after_reopen() {
        let (_t, dir) = tmp("rotate-reopen");
        let line_len = serde_json::to_string(&ev("a", 0)).unwrap().len() as u64 + 1;
        for _ in 0..2 {
            let log = open(&dir, Retention::RotateAtBytes(line_len));
            log.append(&ev("a", 1)).unwrap();
            log.append(&ev("a", 2)).unwrap();
        }
        assert_eq!(
            names(&dir),
            [
                "audit-log.1.jsonl",
                "audit-log.2.jsonl",
                "audit-log.3.jsonl",
                AUDIT_FILE
            ]
        );
    }

    /// Pages are newest first and filtered; the cursor continues across rotated files and is
    /// unaffected by later appends.
    #[test]
    fn read_pages_newest_first_across_rotations() {
        let (_t, dir) = tmp("read");
        let line_len = serde_json::to_string(&ev("a", 10)).unwrap().len() as u64 + 1;
        let log = open(&dir, Retention::RotateAtBytes(line_len * 4));
        for i in 10..30 {
            log.append(&ev(if i % 2 == 0 { "a" } else { "b" }, i))
                .unwrap();
        }
        let only_a = |e: &Ev| e.who == "a";
        let p1 = log.read(only_a, None, 4).unwrap();
        assert_eq!(
            p1.entries.iter().map(|e| e.n).collect::<Vec<_>>(),
            [28, 26, 24, 22]
        );
        // New entries do not shift the next page.
        log.append(&ev("a", 30)).unwrap();
        let p2 = log.read(only_a, p1.next, 4).unwrap();
        assert_eq!(
            p2.entries.iter().map(|e| e.n).collect::<Vec<_>>(),
            [20, 18, 16, 14]
        );
        let p3 = log.read(only_a, p2.next, 4).unwrap();
        assert_eq!(p3.entries.iter().map(|e| e.n).collect::<Vec<_>>(), [12, 10]);
        assert!(p3.next.is_none());
        let none = log.read(|_| true, None, 0).unwrap();
        assert!(none.entries.is_empty());
    }

    #[test]
    fn cursor_round_trips_as_json() {
        let c = Cursor {
            segment: 3,
            line: 7,
        };
        let s = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<Cursor>(&s).unwrap(), c);
    }
}
