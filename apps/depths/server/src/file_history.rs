//! The file history store (spec 053 R9): earlier content of volume files, kept on the server.
//!
//! - `<dir>/blobs/<sha256>`: content, addressed by its hash, so the same content is kept once.
//! - `<dir>/files/<sha256(stack \0 mount \0 path)>.json`: one file's versions, oldest first.
//!
//! A blob is written (temp + rename) before the JSON that names it, and the JSON atomically, as
//! [`crate::users`] writes the registry, so a crash leaves at worst a blob nothing names or a
//! temp file. Both are removed when the store opens, which also prunes it with
//! [`depths_volumes::history::prune`], as every write does.
//!
//! Errors are logged by kind, never with a path or a file name.

use std::collections::{HashMap, HashSet};
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, SecondsFormat, Utc};
use depths_protocol::{FileVersion, VersionId, VersionReason};
use depths_volumes::history::{Limits, VersionInfo, prune};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::files::{FileKey, HistorySink};

/// Seconds since the Unix epoch, now.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// History on disk under one directory. Implements [`HistorySink`].
pub struct FileHistory {
    blobs: PathBuf,
    files: PathBuf,
    limits: Limits,
    clock: Clock,
    /// One change to the store at a time.
    lock: Mutex<()>,
}

/// One file's JSON.
#[derive(Debug, Serialize, Deserialize)]
struct Record {
    stack: String,
    mount: String,
    path: String,
    /// The sequence number the next version gets.
    next_seq: u64,
    /// Oldest first.
    versions: Vec<Stored>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Stored {
    /// `<blob>-<seq>`.
    id: String,
    /// RFC 3339, UTC, to the second.
    at: String,
    author: String,
    size: u64,
    blob: String,
    reason: VersionReason,
}

impl Record {
    fn of(file: &FileKey) -> Self {
        Self {
            stack: file.stack.as_str().to_owned(),
            mount: file.mount.as_str().to_owned(),
            path: file.path.as_str().to_owned(),
            next_seq: 1,
            versions: Vec::new(),
        }
    }

    fn is(&self, file: &FileKey) -> bool {
        self.stack == file.stack.as_str()
            && self.mount == file.mount.as_str()
            && self.path == file.path.as_str()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn secs(at: &str) -> i64 {
    DateTime::parse_from_rfc3339(at).map_or(0, |t| t.timestamp())
}

fn private_dir(dir: &Path) -> io::Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

/// Writes `body` to `path` by a 0600 temp file in the same folder and a rename, then syncs the
/// folder.
fn write_atomic(path: &Path, body: &[u8]) -> io::Result<()> {
    let dir = path.parent().ok_or_else(|| io::Error::other("no parent"))?;
    let tmp = dir.join(format!(".{}.tmp", web_auth::session::random_token()));
    let write = || -> io::Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(body)?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        fs::File::open(dir)?.sync_all()
    };
    write().inspect_err(|_| drop(fs::remove_file(&tmp)))
}

fn warn(what: &str, e: &io::Error) {
    tracing::warn!(error.kind = %e.kind(), "file history: {what}");
}

impl FileHistory {
    /// Opens (creating) the store in `dir`, removes what an interrupted write left, and prunes.
    pub fn open(dir: impl AsRef<Path>, limits: Limits) -> io::Result<Self> {
        Self::open_with_clock(dir, limits, Arc::new(|| Utc::now().timestamp()))
    }

    /// [`FileHistory::open`], with the clock that dates versions and prunes them by age.
    pub fn open_with_clock(
        dir: impl AsRef<Path>,
        limits: Limits,
        clock: Clock,
    ) -> io::Result<Self> {
        let dir = dir.as_ref();
        let h = Self {
            blobs: dir.join("blobs"),
            files: dir.join("files"),
            limits,
            clock,
            lock: Mutex::new(()),
        };
        private_dir(dir)?;
        private_dir(&h.blobs)?;
        private_dir(&h.files)?;
        for sub in [&h.blobs, &h.files] {
            for entry in fs::read_dir(sub)? {
                let name = entry?.file_name();
                if name.to_string_lossy().ends_with(".tmp") {
                    drop(fs::remove_file(sub.join(name)));
                }
            }
        }
        h.prune_now()?;
        Ok(h)
    }

    fn record_path(&self, file: &FileKey) -> PathBuf {
        let mut key = Vec::new();
        key.extend_from_slice(file.stack.as_str().as_bytes());
        key.push(0);
        key.extend_from_slice(file.mount.as_str().as_bytes());
        key.push(0);
        key.extend_from_slice(file.path.as_str().as_bytes());
        self.files.join(format!("{}.json", sha256_hex(&key)))
    }

    fn load(&self, file: &FileKey) -> io::Result<Option<Record>> {
        let body = match fs::read(self.record_path(file)) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let r: Record = serde_json::from_slice(&body).map_err(io::Error::other)?;
        Ok(r.is(file).then_some(r))
    }

    fn store(path: &Path, r: &Record) -> io::Result<()> {
        if r.versions.is_empty() {
            return match fs::remove_file(path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        }
        let body = serde_json::to_vec_pretty(r).map_err(io::Error::other)?;
        write_atomic(path, &body)
    }

    fn try_keep(
        &self,
        file: &FileKey,
        author: &str,
        reason: VersionReason,
        bytes: &[u8],
    ) -> io::Result<()> {
        let blob = sha256_hex(bytes);
        let blob_path = self.blobs.join(&blob);
        if !blob_path.exists() {
            write_atomic(&blob_path, bytes)?;
        }
        let path = self.record_path(file);
        let mut r = self.load(file)?.unwrap_or_else(|| Record::of(file));
        let seq = r.next_seq;
        r.next_seq += 1;
        let at = DateTime::<Utc>::from_timestamp((self.clock)(), 0)
            .unwrap_or_default()
            .to_rfc3339_opts(SecondsFormat::Secs, true);
        r.versions.push(Stored {
            id: format!("{blob}-{seq}"),
            at,
            author: author.to_owned(),
            size: bytes.len() as u64,
            blob,
            reason,
        });
        Self::store(&path, &r)
    }

    /// Prunes every file's versions, drops a version whose blob is gone, and removes the blobs no
    /// version names.
    fn prune_now(&self) -> io::Result<()> {
        let _held = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        self.prune_held()
    }

    fn prune_held(&self) -> io::Result<()> {
        let mut records: Vec<(PathBuf, Record, bool)> = Vec::new();
        // An unreadable JSON's blobs are unknown, so no blob is removed while there is one.
        let mut unreadable = false;
        for entry in fs::read_dir(&self.files)? {
            let path = entry?.path();
            if path.extension().is_none_or(|x| x != "json") {
                continue;
            }
            match fs::read(&path)
                .and_then(|b| serde_json::from_slice::<Record>(&b).map_err(io::Error::other))
            {
                Ok(r) => records.push((path, r, false)),
                Err(e) => {
                    warn("cannot read a history file", &e);
                    unreadable = true;
                }
            }
        }
        let present: HashSet<String> = fs::read_dir(&self.blobs)?
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .collect();
        for (_, r, changed) in &mut records {
            let before = r.versions.len();
            r.versions.retain(|v| present.contains(&v.blob));
            *changed |= r.versions.len() != before;
        }

        let mut infos = Vec::new();
        let mut at = Vec::new();
        for (ri, (_, r, _)) in records.iter().enumerate() {
            for (vi, v) in r.versions.iter().enumerate() {
                infos.push(VersionInfo {
                    file: ri,
                    at: secs(&v.at),
                    size: v.size,
                });
                at.push((ri, vi));
            }
        }
        let mut dropped: HashMap<usize, HashSet<usize>> = HashMap::new();
        for p in prune(&infos, (self.clock)(), &self.limits) {
            let (ri, vi) = at[p.index];
            dropped.entry(ri).or_default().insert(vi);
        }
        for (ri, gone) in dropped {
            let (_, r, changed) = &mut records[ri];
            let mut vi = 0;
            r.versions.retain(|_| {
                vi += 1;
                !gone.contains(&(vi - 1))
            });
            *changed = true;
        }
        for (path, r, changed) in &records {
            if *changed {
                Self::store(path, r)?;
            }
        }

        if unreadable {
            return Ok(());
        }
        let named: HashSet<&str> = records
            .iter()
            .flat_map(|(_, r, _)| r.versions.iter().map(|v| v.blob.as_str()))
            .collect();
        for blob in present.iter().filter(|b| !named.contains(b.as_str())) {
            if let Err(e) = fs::remove_file(self.blobs.join(blob)) {
                warn("cannot remove a blob", &e);
            }
        }
        Ok(())
    }
}

impl HistorySink for FileHistory {
    fn keep(&self, file: &FileKey, author: &str, reason: VersionReason, bytes: &[u8]) {
        if !self.limits.keeps(bytes.len() as u64) {
            return;
        }
        let _held = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = self.try_keep(file, author, reason, bytes) {
            warn("cannot keep a version", &e);
            return;
        }
        if let Err(e) = self.prune_held() {
            warn("cannot prune", &e);
        }
    }

    fn versions(&self, file: &FileKey) -> Vec<FileVersion> {
        let _held = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let r = match self.load(file) {
            Ok(Some(r)) => r,
            Ok(None) => return Vec::new(),
            Err(e) => {
                warn("cannot read a history file", &e);
                return Vec::new();
            }
        };
        r.versions
            .into_iter()
            .rev()
            .filter_map(|v| {
                Some(FileVersion {
                    id: VersionId::new(v.id).ok()?,
                    at: v.at,
                    author: v.author,
                    size: v.size,
                    reason: v.reason,
                })
            })
            .collect()
    }

    fn version(&self, file: &FileKey, id: &VersionId) -> Option<Vec<u8>> {
        let _held = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let r = self.load(file).ok()??;
        let v = r.versions.iter().find(|v| v.id == id.as_str())?;
        if v.blob.len() != 64 || !v.blob.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let bytes = fs::read(self.blobs.join(&v.blob))
            .inspect_err(|e| warn("cannot read a blob", e))
            .ok()?;
        (sha256_hex(&bytes) == v.blob).then_some(bytes)
    }
}
