//! Runs outside Keel (spec 058 amendment A2).
//!
//! The runner role's Ansible callback appends one JSON line to
//! `/var/lib/keel-outside/runs.jsonl` for every `ansible-playbook` run on the host that Keel did
//! not start. The runner reads what is new since its saved offset and sends it in `OutsideRuns`
//! frames of at most 100 entries; it moves the offset only when the server acknowledges a batch
//! with `OutsideRunsAck`.
//!
//! - A line over 1 KiB, one that is not a v1 record, or one whose fields break their bounds is
//!   skipped and counted in `skipped`.
//! - A line still being written (no newline yet) waits for the next read.
//! - A record that shrank below the saved offset (truncated or replaced) is read from the start
//!   again; Keel de-duplicates what it has already seen.

use std::fs::File;
use std::io::{BufRead as _, BufReader, ErrorKind, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};

use keel_exec::frames::{Frame, MAX_OUTSIDE_RUNS, OutsideRun};
use serde::Deserialize;

/// Where the runner role keeps the record.
pub const DEFAULT_OUTSIDE_LOG: &str = "/var/lib/keel-outside/runs.jsonl";
/// The runner's saved read offset, in its state directory.
pub const OUTSIDE_OFFSET_FILE: &str = "outside-offset";
/// The longest line read, without its newline.
pub const MAX_LINE: usize = 1024;

/// One line of the record, as the callback writes it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    v: u32,
    at: String,
    user: String,
    playbook: String,
    commit: Option<String>,
    dirty: bool,
    check: bool,
}

/// What is new in the record: the frame's fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    pub entries: Vec<OutsideRun>,
    pub skipped: u32,
    /// The byte offset just past the last line this batch consumed.
    pub through: u64,
}

impl Batch {
    pub fn into_frame(self) -> Frame {
        Frame::OutsideRuns {
            entries: self.entries,
            skipped: self.skipped,
            through: self.through,
        }
    }
}

/// The record and where the runner has read it to.
#[derive(Debug, Clone)]
pub struct OutsideLog {
    log: PathBuf,
    offset_file: PathBuf,
}

impl OutsideLog {
    pub fn new(log: impl Into<PathBuf>, state_dir: &Path) -> Self {
        Self {
            log: log.into(),
            offset_file: state_dir.join(OUTSIDE_OFFSET_FILE),
        }
    }

    pub fn log(&self) -> &Path {
        &self.log
    }

    /// The saved offset; 0 when none was saved or the file does not parse.
    pub fn offset(&self) -> u64 {
        std::fs::read_to_string(&self.offset_file)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    }

    /// The server has stored everything up to `through`: move the offset there.
    pub fn ack(&self, through: u64) -> std::io::Result<()> {
        crate::write_private_atomic(&self.offset_file, format!("{through}\n").as_bytes())
    }

    /// What is new since the saved offset, at most [`MAX_OUTSIDE_RUNS`] entries. `None` when
    /// nothing is (no record, or no complete line past the offset).
    pub fn next_batch(&self) -> std::io::Result<Option<Batch>> {
        let file = match File::open(&self.log) {
            Ok(f) => f,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let len = file.metadata()?.len();
        let saved = self.offset();
        // Truncated or replaced: start over.
        let start = if len < saved { 0 } else { saved };
        let mut reader = BufReader::new(file);
        reader.seek(SeekFrom::Start(start))?;

        let mut batch = Batch {
            entries: Vec::new(),
            skipped: 0,
            through: start,
        };
        let mut line = Vec::with_capacity(MAX_LINE + 1);
        while batch.entries.len() < MAX_OUTSIDE_RUNS {
            let Some((consumed, too_long)) = read_line(&mut reader, &mut line)? else {
                break;
            };
            batch.through += consumed;
            if line.iter().all(u8::is_ascii_whitespace) && !too_long {
                continue;
            }
            match (!too_long).then(|| parse(&line)).flatten() {
                Some(run) => batch.entries.push(run),
                None => batch.skipped = batch.skipped.saturating_add(1),
            }
        }
        Ok((batch.through != saved).then_some(batch))
    }
}

/// Reads one complete line into `line` (without its newline), keeping at most `MAX_LINE + 1`
/// bytes of it. Returns the bytes consumed and whether the line was over `MAX_LINE`; `None` at
/// the end of the record or before a line that has no newline yet (nothing is consumed then).
fn read_line(
    reader: &mut BufReader<File>,
    line: &mut Vec<u8>,
) -> std::io::Result<Option<(u64, bool)>> {
    line.clear();
    let mut consumed = 0u64;
    let mut too_long = false;
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            // No newline before the end: wait for the writer, and re-read from the line's start.
            reader.seek_relative(-(consumed as i64))?;
            return Ok(None);
        }
        let (chunk, done) = match buf.iter().position(|&b| b == b'\n') {
            Some(i) => (&buf[..i], Some(i + 1)),
            None => (buf, None),
        };
        let room = (MAX_LINE + 1).saturating_sub(line.len());
        line.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if line.len() > MAX_LINE {
            too_long = true;
        }
        let used = done.unwrap_or(buf.len());
        reader.consume(used);
        consumed += used as u64;
        if done.is_some() {
            return Ok(Some((consumed, too_long)));
        }
    }
}

/// A v1 record within its bounds, or `None`.
fn parse(line: &[u8]) -> Option<OutsideRun> {
    let r: Record = serde_json::from_slice(line).ok()?;
    if r.v != 1 {
        return None;
    }
    let run = OutsideRun {
        at: r.at,
        user: r.user,
        playbook: r.playbook,
        commit: r.commit,
        dirty: r.dirty,
        check: r.check,
    };
    run.check().ok()?;
    Some(run)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    fn line(at: &str, playbook: &str, check: bool) -> String {
        format!(
            r#"{{"v":1,"at":"{at}","user":"root","playbook":"{playbook}","commit":"{COMMIT}","dirty":false,"check":{check}}}"#
        ) + "\n"
    }

    fn append(path: &Path, text: &str) {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    fn setup() -> (tempfile::TempDir, OutsideLog, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("runs.jsonl");
        let state = dir.path().join("state");
        std::fs::create_dir(&state).unwrap();
        let o = OutsideLog::new(&log, &state);
        (dir, o, log)
    }

    #[test]
    fn no_record_means_nothing_to_send() {
        let (_d, o, _log) = setup();
        assert_eq!(o.next_batch().unwrap(), None);
    }

    #[test]
    fn the_offset_moves_only_on_ack() {
        let (_d, o, log) = setup();
        let first = line("2026-10-27T12:00:00Z", "playbooks/debian.yaml", false);
        append(&log, &first);

        let b = o.next_batch().unwrap().expect("one entry");
        assert_eq!(b.entries.len(), 1);
        assert_eq!(b.entries[0].playbook, "playbooks/debian.yaml");
        assert_eq!(b.entries[0].commit.as_deref(), Some(COMMIT));
        assert_eq!(b.skipped, 0);
        assert_eq!(b.through, first.len() as u64);

        // Not acknowledged: the same batch again.
        assert_eq!(o.next_batch().unwrap(), Some(b.clone()));
        o.ack(b.through).unwrap();
        assert_eq!(o.offset(), first.len() as u64);
        assert_eq!(o.next_batch().unwrap(), None, "nothing new after the ack");

        let second = line("2026-10-27T13:00:00Z", "site.yaml", true);
        append(&log, &second);
        let b = o.next_batch().unwrap().expect("the new line only");
        assert_eq!(b.entries.len(), 1);
        assert_eq!(b.entries[0].playbook, "site.yaml");
        assert!(b.entries[0].check);
        assert_eq!(b.through, (first.len() + second.len()) as u64);
    }

    #[test]
    fn a_line_without_its_newline_waits() {
        let (_d, o, log) = setup();
        let full = line("2026-10-27T12:00:00Z", "a.yaml", false);
        append(&log, &full[..full.len() - 10]);
        assert_eq!(o.next_batch().unwrap(), None);
        append(&log, &full[full.len() - 10..]);
        let b = o.next_batch().unwrap().unwrap();
        assert_eq!(b.entries.len(), 1);
        assert_eq!(b.through, full.len() as u64);
    }

    #[test]
    fn a_shrunk_record_is_read_from_the_start() {
        let (_d, o, log) = setup();
        append(&log, &line("2026-10-27T12:00:00Z", "a.yaml", false));
        append(&log, &line("2026-10-27T12:01:00Z", "b.yaml", false));
        let b = o.next_batch().unwrap().unwrap();
        o.ack(b.through).unwrap();

        // Rotated: a shorter file in its place.
        let fresh = line("2026-10-28T09:00:00Z", "c.yaml", false);
        std::fs::write(&log, &fresh).unwrap();
        let b = o.next_batch().unwrap().expect("read from 0");
        assert_eq!(b.entries.len(), 1);
        assert_eq!(b.entries[0].playbook, "c.yaml");
        assert_eq!(b.through, fresh.len() as u64);
    }

    #[test]
    fn malformed_and_oversized_lines_are_skipped_and_counted() {
        let (_d, o, log) = setup();
        let good = line("2026-10-27T12:00:00Z", "a.yaml", false);
        let mut text = String::new();
        text += "not json\n";
        text += &format!("{}\n", "x".repeat(MAX_LINE + 500)); // over 1 KiB
        text += &good.replace(r#""v":1"#, r#""v":2"#); // unknown version
        text += &good.replace(r#""dirty""#, r#""shell":"id","dirty""#); // unknown field
        text += &good.replace("root", &"u".repeat(65)); // user over 64 bytes
        text += &good.replace("a.yaml", &"p".repeat(257)); // playbook over 256 bytes
        text += &good.replace(COMMIT, "ABC"); // bad commit
        text += "\n"; // a blank line is neither an entry nor skipped
        text += &good;
        text += &good.replace(&format!(r#""{COMMIT}""#), "null"); // no commit is fine
        append(&log, &text);

        let b = o.next_batch().unwrap().unwrap();
        assert_eq!(b.skipped, 7);
        assert_eq!(b.entries.len(), 2);
        assert_eq!(b.entries[1].commit, None);
        assert_eq!(b.through, text.len() as u64);
    }

    #[test]
    fn only_skipped_lines_still_move_the_offset() {
        let (_d, o, log) = setup();
        append(&log, "garbage\n");
        let b = o
            .next_batch()
            .unwrap()
            .expect("a batch with nothing but a count");
        assert!(b.entries.is_empty());
        assert_eq!(b.skipped, 1);
        assert_eq!(b.through, 8);
    }

    #[test]
    fn at_most_100_entries_a_batch() {
        let (_d, o, log) = setup();
        let mut text = String::new();
        for i in 0..150 {
            text += &line(
                &format!("2026-10-27T12:{:02}:{:02}Z", i / 60, i % 60),
                "a.yaml",
                false,
            );
        }
        append(&log, &text);
        let b = o.next_batch().unwrap().unwrap();
        assert_eq!(b.entries.len(), MAX_OUTSIDE_RUNS);
        o.ack(b.through).unwrap();
        let rest = o.next_batch().unwrap().unwrap();
        assert_eq!(rest.entries.len(), 50);
        assert_eq!(rest.through, text.len() as u64);
        assert!(matches!(
            rest.into_frame(),
            Frame::OutsideRuns { ref entries, .. } if entries.len() == 50
        ));
    }
}
