//! The host ledger: one hash-chained JSON line per run start and end (contract signed-mode.md).
//!
//! `hash = SHA-256(prev ‖ JCS(line without "hash"))`, where `prev` is the previous line's hash
//! (32 zero bytes for the first line). `seq` starts at 1 and has no gaps. A job's **base** is the
//! `target` of the start line of its latest run that ended `succeeded` or `failed`; a cancelled
//! run, or one with a start and no end, does not move it.
//!
//! This module parses and checks text and builds the next line; the executor does the file I/O.
//! Any line that does not fit (unknown or missing keys, wrong hash, gaps, an `end` without a
//! `start`, a second `start` for a run) makes the whole ledger [`LedgerError::Corrupt`].

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::object::CommitId;

/// How a run was authorized, as recorded on its start line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Auth {
    /// Every new commit was signed by a trusted signer.
    Signed {
        /// Signer ids (`SHA256:…` or `openpgp:<fingerprint>`), sorted, unique.
        signers: Vec<String>,
    },
    /// An override approval.
    Override {
        /// Credential ids of the approvals that passed, in order.
        approvals: Vec<String>,
        /// Their approver labels (from `approvers.json`), in the same order.
        approvers: Vec<String>,
    },
    /// A baseline approval (first run).
    Baseline {
        /// Credential ids of the approvals that passed, in order.
        approvals: Vec<String>,
        /// Their approver labels, in the same order.
        approvers: Vec<String>,
    },
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// Finished with success. Moves the base.
    Succeeded,
    /// Finished with failure. Moves the base: the host has partly applied the target.
    Failed,
    /// Stopped before running anything that changes the host. Does not move the base.
    Cancelled,
}

impl Outcome {
    /// Whether a run that ended this way moves the job's base.
    pub fn moves_base(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

/// A line to append, before `seq`, `prev` and `hash` are filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A run starts.
    Start {
        /// `YYYY-MM-DDTHH:MM:SSZ`.
        at: String,
        /// Run id.
        run: String,
        /// Job name.
        job: String,
        /// The base the authorization was checked against.
        base: Option<CommitId>,
        /// The commit being applied.
        target: CommitId,
        /// How it was authorized.
        auth: Auth,
    },
    /// A run ends.
    End {
        /// `YYYY-MM-DDTHH:MM:SSZ`.
        at: String,
        /// Run id (must have a start line).
        run: String,
        /// Job name (must match the start line).
        job: String,
        /// How it ended.
        outcome: Outcome,
        /// The process exit code, if there was one.
        exit: Option<i64>,
    },
}

/// One parsed, verified line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// 1, 2, 3, …
    pub seq: u64,
    /// The entry.
    pub entry: Entry,
    /// This line's hash.
    pub hash: [u8; 32],
}

/// The ledger cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LedgerError {
    /// The chain or a line is broken: the executor refuses all work (`ledger_corrupt`).
    #[error("ledger corrupt at line {line}: {detail}")]
    Corrupt {
        /// 1-based line number.
        line: usize,
        /// What is wrong.
        detail: &'static str,
    },
    /// The run already has a start line (`replay`).
    #[error("run already started")]
    Replay,
    /// An end for a run with no start, a second end, or a job that does not match.
    #[error("end does not match a started run")]
    NoSuchRun,
    /// The entry itself is invalid (empty run or job, bad timestamp).
    #[error("invalid entry")]
    InvalidEntry,
}

struct RunState {
    job: String,
    target: CommitId,
    ended: Option<Outcome>,
    start_seq: u64,
}

/// A verified ledger.
pub struct Ledger {
    lines: Vec<Line>,
    runs: HashMap<String, RunState>,
}

const START_KEYS: &[&str] = &[
    "seq", "at", "run", "job", "event", "base", "target", "auth", "prev", "hash",
];
const END_KEYS: &[&str] = &[
    "seq", "at", "run", "job", "event", "outcome", "exit", "prev", "hash",
];

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

fn line_hash(prev: &[u8; 32], without_hash: &Value) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(prev);
    h.update(serde_jcs::to_vec(without_hash).expect("canonical JSON of a JSON value"));
    h.finalize().into()
}

fn entry_valid(at: &str, run: &str, job: &str) -> bool {
    crate::challenge::parse_utc(at).is_some()
        && !run.is_empty()
        && !job.is_empty()
        && !run.chars().any(char::is_control)
        && !job.chars().any(char::is_control)
}

impl Ledger {
    /// An empty ledger (a host that has never run anything).
    pub fn empty() -> Self {
        Self {
            lines: Vec::new(),
            runs: HashMap::new(),
        }
    }

    /// Parses and verifies the whole file. Every line, including the last, must end in `\n`.
    pub fn parse(text: &str) -> Result<Self, LedgerError> {
        let mut ledger = Self::empty();
        if text.is_empty() {
            return Ok(ledger);
        }
        let Some(body) = text.strip_suffix('\n') else {
            return Err(LedgerError::Corrupt {
                line: text.lines().count(),
                detail: "unterminated line",
            });
        };
        for (i, raw) in body.split('\n').enumerate() {
            let n = i + 1;
            let corrupt = |detail| LedgerError::Corrupt { line: n, detail };
            let mut value: Value = serde_json::from_str(raw).map_err(|_| corrupt("not JSON"))?;
            let obj = value.as_object_mut().ok_or(corrupt("not an object"))?;
            let hash = obj
                .remove("hash")
                .and_then(|h| h.as_str().and_then(unhex32))
                .ok_or(corrupt("hash"))?;
            let prev = obj
                .get("prev")
                .and_then(Value::as_str)
                .and_then(unhex32)
                .ok_or(corrupt("prev"))?;
            if prev != ledger.head() {
                return Err(corrupt("prev does not match the previous hash"));
            }
            if line_hash(&prev, &value) != hash {
                return Err(corrupt("hash does not match"));
            }
            let obj = value.as_object().expect("checked above");
            let seq = obj
                .get("seq")
                .and_then(Value::as_u64)
                .ok_or(corrupt("seq"))?;
            if seq != ledger.lines.len() as u64 + 1 {
                return Err(corrupt("seq is not contiguous"));
            }
            let entry = parse_entry(obj).ok_or(corrupt("fields"))?;
            ledger
                .apply(seq, &entry)
                .map_err(|_| corrupt("run sequence"))?;
            ledger.lines.push(Line { seq, entry, hash });
        }
        Ok(ledger)
    }

    /// The last line's hash, or 32 zero bytes for an empty ledger.
    pub fn head(&self) -> [u8; 32] {
        self.lines.last().map_or([0; 32], |l| l.hash)
    }

    /// The verified lines.
    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// Whether `run` already has a start line.
    pub fn has_start(&self, run: &str) -> bool {
        self.runs.contains_key(run)
    }

    /// The job's base: the target of its latest run that ended `succeeded` or `failed`.
    pub fn base(&self, job: &str) -> Option<CommitId> {
        self.runs
            .values()
            .filter(|r| r.job == job && r.ended.is_some_and(Outcome::moves_base))
            .max_by_key(|r| r.start_seq)
            .map(|r| r.target.clone())
    }

    /// Every job's base.
    pub fn bases(&self) -> BTreeMap<String, CommitId> {
        let mut jobs: Vec<&str> = self.runs.values().map(|r| r.job.as_str()).collect();
        jobs.sort_unstable();
        jobs.dedup();
        jobs.into_iter()
            .filter_map(|j| Some((j.to_owned(), self.base(j)?)))
            .collect()
    }

    fn apply(&mut self, seq: u64, entry: &Entry) -> Result<(), LedgerError> {
        match entry {
            Entry::Start {
                at,
                run,
                job,
                target,
                ..
            } => {
                if !entry_valid(at, run, job) {
                    return Err(LedgerError::InvalidEntry);
                }
                if self.runs.contains_key(run) {
                    return Err(LedgerError::Replay);
                }
                self.runs.insert(
                    run.clone(),
                    RunState {
                        job: job.clone(),
                        target: target.clone(),
                        ended: None,
                        start_seq: seq,
                    },
                );
            }
            Entry::End {
                at,
                run,
                job,
                outcome,
                ..
            } => {
                if !entry_valid(at, run, job) {
                    return Err(LedgerError::InvalidEntry);
                }
                match self.runs.get_mut(run) {
                    Some(state) if state.job == *job && state.ended.is_none() => {
                        state.ended = Some(*outcome)
                    }
                    _ => return Err(LedgerError::NoSuchRun),
                }
            }
        }
        Ok(())
    }

    /// Appends `entry` and returns the line to write (with its trailing `\n`). The ledger is
    /// unchanged on error; a start for a run that already started is [`LedgerError::Replay`].
    pub fn append(&mut self, entry: Entry) -> Result<String, LedgerError> {
        let seq = self.lines.len() as u64 + 1;
        let prev = self.head();
        let mut obj = Map::new();
        obj.insert("seq".into(), seq.into());
        match &entry {
            Entry::Start {
                at,
                run,
                job,
                base,
                target,
                auth,
            } => {
                obj.insert("at".into(), at.clone().into());
                obj.insert("run".into(), run.clone().into());
                obj.insert("job".into(), job.clone().into());
                obj.insert("event".into(), "start".into());
                obj.insert(
                    "base".into(),
                    base.as_ref().map_or(Value::Null, |b| b.as_str().into()),
                );
                obj.insert("target".into(), target.as_str().into());
                obj.insert(
                    "auth".into(),
                    serde_json::to_value(auth).expect("auth serializes"),
                );
            }
            Entry::End {
                at,
                run,
                job,
                outcome,
                exit,
            } => {
                obj.insert("at".into(), at.clone().into());
                obj.insert("run".into(), run.clone().into());
                obj.insert("job".into(), job.clone().into());
                obj.insert("event".into(), "end".into());
                obj.insert(
                    "outcome".into(),
                    serde_json::to_value(outcome).expect("outcome serializes"),
                );
                obj.insert("exit".into(), exit.map_or(Value::Null, Value::from));
            }
        }
        obj.insert("prev".into(), hex(&prev).into());
        let mut value = Value::Object(obj);
        let hash = line_hash(&prev, &value);

        // Check before mutating: apply only changes `runs`, and only on success.
        self.apply(seq, &entry)?;
        value
            .as_object_mut()
            .expect("object")
            .insert("hash".into(), hex(&hash).into());
        self.lines.push(Line { seq, entry, hash });
        let mut text = serde_json::to_string(&value).expect("value serializes");
        text.push('\n');
        Ok(text)
    }
}

fn exact_keys(obj: &Map<String, Value>, keys: &[&str]) -> bool {
    // `hash` was removed before this check.
    obj.len() + 1 == keys.len() && keys.iter().all(|k| *k == "hash" || obj.contains_key(*k))
}

fn parse_entry(obj: &Map<String, Value>) -> Option<Entry> {
    let s = |k: &str| obj.get(k)?.as_str().map(str::to_owned);
    let (at, run, job) = (s("at")?, s("run")?, s("job")?);
    match obj.get("event")?.as_str()? {
        "start" => {
            if !exact_keys(obj, START_KEYS) {
                return None;
            }
            let base = match obj.get("base")? {
                Value::Null => None,
                Value::String(b) => Some(CommitId::parse(b).ok()?),
                _ => return None,
            };
            let target = CommitId::parse(obj.get("target")?.as_str()?).ok()?;
            let auth: Auth = serde_json::from_value(obj.get("auth")?.clone()).ok()?;
            Some(Entry::Start {
                at,
                run,
                job,
                base,
                target,
                auth,
            })
        }
        "end" => {
            if !exact_keys(obj, END_KEYS) {
                return None;
            }
            let outcome: Outcome = serde_json::from_value(obj.get("outcome")?.clone()).ok()?;
            let exit = match obj.get("exit")? {
                Value::Null => None,
                v => Some(v.as_i64()?),
            };
            Some(Entry::End {
                at,
                run,
                job,
                outcome,
                exit,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    //! T091: chain verification, base tracking, replay and corruption.
    use super::*;

    fn id(c: char) -> CommitId {
        CommitId::parse(&c.to_string().repeat(40)).unwrap()
    }

    fn start(run: &str, job: &str, base: Option<char>, target: char) -> Entry {
        Entry::Start {
            at: "2026-01-01T00:00:00Z".into(),
            run: run.into(),
            job: job.into(),
            base: base.map(id),
            target: id(target),
            auth: Auth::Signed {
                signers: vec!["SHA256:abc".into()],
            },
        }
    }

    fn end(run: &str, job: &str, outcome: Outcome) -> Entry {
        Entry::End {
            at: "2026-01-01T00:01:00Z".into(),
            run: run.into(),
            job: job.into(),
            outcome,
            exit: Some(0),
        }
    }

    fn sample() -> String {
        let mut l = Ledger::empty();
        let mut text = String::new();
        for e in [
            start("r1", "web", None, '1'),
            end("r1", "web", Outcome::Succeeded),
            start("r2", "web", Some('1'), '2'),
            end("r2", "web", Outcome::Failed),
            start("r3", "web", Some('2'), '3'),
            end("r3", "web", Outcome::Cancelled),
            start("r4", "db", None, '4'),
        ] {
            text.push_str(&l.append(e).unwrap());
        }
        text
    }

    #[test]
    fn round_trips_and_tracks_bases() {
        let text = sample();
        let l = Ledger::parse(&text).unwrap();
        assert_eq!(l.lines().len(), 7);
        // Failed moves the base; cancelled and an unfinished run do not.
        assert_eq!(l.base("web"), Some(id('2')));
        assert_eq!(l.base("db"), None);
        assert_eq!(l.bases(), BTreeMap::from([("web".to_owned(), id('2'))]));
        assert!(l.has_start("r4"));
        assert!(!l.has_start("r5"));
    }

    #[test]
    fn first_line_is_fixed() {
        let mut l = Ledger::empty();
        let line = l.append(start("r1", "web", None, '1')).unwrap();
        assert!(line.contains(r#""seq":1,"#), "{line}");
        assert!(line.contains(&format!(r#""prev":"{}""#, "0".repeat(64))));
        assert!(line.ends_with("}\n"));
        let parsed = Ledger::parse(&line).unwrap();
        assert_eq!(parsed.head(), l.head());
        assert_ne!(l.head(), [0; 32]);
    }

    #[test]
    fn replay_is_refused_and_leaves_the_ledger_unchanged() {
        let mut l = Ledger::parse(&sample()).unwrap();
        let head = l.head();
        assert_eq!(
            l.append(start("r1", "web", Some('2'), '5')),
            Err(LedgerError::Replay)
        );
        assert_eq!(l.head(), head);
        assert_eq!(
            l.append(end("r9", "web", Outcome::Succeeded)),
            Err(LedgerError::NoSuchRun)
        );
        assert_eq!(
            l.append(end("r1", "web", Outcome::Succeeded)),
            Err(LedgerError::NoSuchRun)
        );
        assert_eq!(
            l.append(end("r4", "web", Outcome::Succeeded)),
            Err(LedgerError::NoSuchRun)
        );
        assert_eq!(l.head(), head);
        l.append(end("r4", "db", Outcome::Succeeded)).unwrap();
        assert_eq!(l.base("db"), Some(id('4')));
    }

    fn corrupt(text: &str) -> bool {
        matches!(Ledger::parse(text), Err(LedgerError::Corrupt { .. }))
    }

    #[test]
    fn any_edit_breaks_the_chain() {
        let text = sample();
        let lines: Vec<&str> = text.lines().collect();
        // A changed value.
        assert!(corrupt(&text.replacen("\"web\"", "\"wab\"", 1)));
        // A dropped line, a swapped pair, a truncated tail line.
        let drop: String = lines
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 2)
            .map(|(_, l)| format!("{l}\n"))
            .collect();
        assert!(corrupt(&drop));
        let mut swapped = lines.clone();
        swapped.swap(1, 2);
        assert!(corrupt(&(swapped.join("\n") + "\n")));
        assert!(corrupt(&text[..text.len() - 1]));
        assert!(corrupt(&text[..text.len() - 5]));
        // Not JSON, extra key, blank line.
        assert!(corrupt(&format!("{text}x\n")));
        assert!(corrupt(&format!("{text}\n")));
        // Dropping the last line keeps a valid prefix.
        let prefix: String = lines[..6].iter().map(|l| format!("{l}\n")).collect();
        assert!(Ledger::parse(&prefix).is_ok());
    }

    #[test]
    fn a_correctly_hashed_but_invalid_line_is_corrupt() {
        // Build lines by hand with valid hashes but bad content.
        let make = |obj: Value, prev: [u8; 32]| -> (String, [u8; 32]) {
            let h = line_hash(&prev, &obj);
            let mut o = obj;
            o.as_object_mut()
                .unwrap()
                .insert("hash".into(), hex(&h).into());
            (serde_json::to_string(&o).unwrap() + "\n", h)
        };
        let zero = hex(&[0; 32]);
        let base_start = serde_json::json!({"seq":1,"at":"2026-01-01T00:00:00Z","run":"r1","job":"web","event":"start","base":null,"target":"1".repeat(40),"auth":{"kind":"signed","signers":[]},"prev":zero});
        assert!(Ledger::parse(&make(base_start.clone(), [0; 32]).0).is_ok());

        let mut extra = base_start.clone();
        extra["note"] = "x".into();
        assert!(corrupt(&make(extra, [0; 32]).0));
        let mut missing_base = base_start.clone();
        missing_base.as_object_mut().unwrap().remove("base");
        assert!(corrupt(&make(missing_base, [0; 32]).0));
        let mut seq2 = base_start.clone();
        seq2["seq"] = 2.into();
        assert!(corrupt(&make(seq2, [0; 32]).0));
        let mut bad_auth = base_start.clone();
        bad_auth["auth"] = serde_json::json!({"kind":"trusted"});
        assert!(corrupt(&make(bad_auth, [0; 32]).0));
        let mut upper = base_start.clone();
        upper["target"] = "A".repeat(40).into();
        assert!(corrupt(&make(upper, [0; 32]).0));

        // An end without a start, and a second start for the same run.
        let end_only = serde_json::json!({"seq":1,"at":"2026-01-01T00:00:00Z","run":"r1","job":"web","event":"end","outcome":"succeeded","exit":0,"prev":zero});
        assert!(corrupt(&make(end_only, [0; 32]).0));
        let (l1, h1) = make(base_start.clone(), [0; 32]);
        let mut again = base_start;
        again["seq"] = 2.into();
        again["prev"] = hex(&h1).into();
        let (l2, _) = make(again, h1);
        assert!(corrupt(&(l1 + &l2)));
    }
}
