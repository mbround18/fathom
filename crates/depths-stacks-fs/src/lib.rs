//! `depths-stacks-fs`: Depths stacks-root adapter: Dockge-compatible directory layout on disk (spec 050).
//!
//! [`FsStackStore`] implements `depths_core::ports::StackStore` over `<root>/<name>/compose.yaml`
//! (and the other three compose file names) plus `.env`.
//!
//! Confinement (FR-005, T095). Every access goes through a directory fd, never a joined path:
//!
//! - The root is opened once, canonical. A stack directory is `openat(root, name, O_NOFOLLOW |
//!   O_DIRECTORY)`, and `name` is a [`StackName`] (one component, no `/`, no `..`), so the fd is
//!   the real child directory or the open fails. A symlinked stack directory is listed, read-only,
//!   and never followed.
//! - A file is `openat(dir, file, O_NOFOLLOW)`. A symlink is followed one hop only when its target
//!   is a plain name in the same directory (`compose.yaml -> real.yaml`); anything else is
//!   [`StoreError::Escapes`].
//! - What was opened is what is checked (`fstat` on the fd), so a swap between check and use
//!   lands on the fd already held, not on a new path.
//!
//! Writes go to a temp file created `O_EXCL | O_NOFOLLOW` in the same directory, take the old
//! file's mode and owner, are fsynced, and are `renameat`ed over the target. A rename never
//! follows a symlink at the target, so a file swapped for a link is replaced, not written through.

use std::ffi::CStr;
use std::fs::{File, Metadata};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use depths_core::discovery::{
    ComposeFile, DirListing, Discovered, ENV_FILE, GLOBAL_ENV_FILE, discover,
};
use depths_core::hash::{ContentHash, content_hash_of};
use depths_core::ports::{
    DefinitionState, StackEntry, StackFiles, StackListing, StackStore, StoreError,
};
use depths_core::status::parse_services;
use depths_core::{StackName, valid_name};
use rustix::fs::{
    Access, AtFlags, Dir, FileType, Gid, Mode, OFlags, Uid, accessat, fchmod, fchown, fsync,
    mkdirat, openat, readlinkat, renameat, unlinkat,
};
use rustix::io::Errno;

/// `DEPTHS_MAX_FILE_BYTES`' default.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 1024 * 1024;

/// A new compose file's mode.
const COMPOSE_MODE: u32 = 0o644;
/// A new `.env`'s mode: it holds secrets.
const ENV_MODE: u32 = 0o600;
const STACK_DIR_MODE: u32 = 0o755;
const TEMP_PREFIX: &str = ".depths-tmp-";

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// The stacks root on disk.
#[derive(Debug)]
pub struct FsStackStore {
    root: PathBuf,
    root_fd: OwnedFd,
    max_file_bytes: u64,
}

impl FsStackStore {
    /// Opens the stacks root, which must exist and be a directory.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, StoreError> {
        let root = root.as_ref().canonicalize().map_err(|e| StoreError::Io {
            message: format!("stacks root {}: {e}", root.as_ref().display()),
        })?;
        let root_fd = rustix::fs::open(
            &root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| StoreError::Io {
            message: format!("stacks root {}: {}", root.display(), io::Error::from(e)),
        })?;
        Ok(Self {
            root,
            root_fd,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        })
    }

    /// Sets `DEPTHS_MAX_FILE_BYTES`.
    pub fn with_max_file_bytes(mut self, limit: u64) -> Self {
        self.max_file_bytes = limit;
        self
    }

    /// `root/<name>` as a directory fd, never through a symlink.
    fn open_stack(&self, name: &str) -> Result<OwnedFd, StoreError> {
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        match openat(&self.root_fd, name, flags, Mode::empty()) {
            Ok(fd) => Ok(fd),
            Err(Errno::NOENT) => Err(StoreError::NotFound),
            Err(Errno::LOOP | Errno::NOTDIR) => {
                if readlinkat(&self.root_fd, name, Vec::new()).is_ok() {
                    Err(StoreError::Escapes {
                        reason: "the stack directory is a symlink, which Depths does not follow"
                            .into(),
                    })
                } else {
                    Err(StoreError::NotAStack)
                }
            }
            Err(e) => Err(io_error(&format!("stack directory {name}"), e.into())),
        }
    }

    /// The stack directory's files, opened.
    fn open_files(&self, dir: BorrowedFd<'_>, d: &Discovered) -> Result<Opened, StoreError> {
        let compose_name = d.compose_file.file_name();
        let compose = open_in(dir, compose_name)?.ok_or(StoreError::NotAStack)?;
        let env = if d.has_env {
            open_in(dir, ENV_FILE)?
        } else {
            None
        };
        Ok(Opened {
            compose_file: d.compose_file,
            compose,
            env,
        })
    }

    fn entry_named(&self, name: &str) -> Result<StackEntry, StoreError> {
        let mut e = StackEntry {
            name: name.to_owned(),
            valid_name: valid_name(name),
            compose_file: None,
            shadowed_files: vec![],
            has_env: false,
            content_hash: None,
            modified_at: None,
            definition: DefinitionState::NotLoaded,
            read_error: None,
            read_only_reason: None,
            too_large: false,
        };
        let dir = match self.open_stack(name) {
            Ok(dir) => dir,
            Err(StoreError::Escapes { reason }) => {
                e.read_error = Some(reason.clone());
                e.read_only_reason = Some(reason);
                return Ok(e);
            }
            Err(err @ (StoreError::NotFound | StoreError::NotAStack)) => return Err(err),
            Err(err) => {
                e.read_error = Some(err.to_string());
                return Ok(e);
            }
        };
        let listing = match list_dir(dir.as_fd()) {
            Ok(l) => l,
            Err(err) => {
                e.read_error = Some(err.to_string());
                return Ok(e);
            }
        };
        let d = discover(&listing).ok_or(StoreError::NotAStack)?;
        e.compose_file = Some(d.compose_file);
        e.shadowed_files = d.shadowed_files.clone();
        e.has_env = d.has_env;

        let opened = match self.open_files(dir.as_fd(), &d) {
            Ok(o) => o,
            Err(err) => {
                e.read_error = Some(err.to_string());
                return Ok(e);
            }
        };
        e.modified_at = opened.modified_at();
        e.read_only_reason = read_only_reason(dir.as_fd(), &opened);
        match opened.read(self.max_file_bytes) {
            Ok(b) => {
                e.content_hash = Some(b.hash());
                e.definition = match std::str::from_utf8(&b.compose) {
                    Ok(text) => match parse_services(text) {
                        Ok(s) => DefinitionState::Parsed(s),
                        Err(m) => DefinitionState::Unparseable(m),
                    },
                    Err(_) => DefinitionState::Unparseable("not UTF-8 text".into()),
                };
            }
            Err(StoreError::TooLarge { .. }) => e.too_large = true,
            Err(err) => e.read_error = Some(err.to_string()),
        }
        Ok(e)
    }

    /// The stack directory, its discovery and its files, for `read` and `write_atomic`.
    fn load(&self, name: &StackName) -> Result<(OwnedFd, Opened), StoreError> {
        let dir = self.open_stack(name.as_str())?;
        let d = discover(&list_dir(dir.as_fd())?).ok_or(StoreError::NotAStack)?;
        let opened = self.open_files(dir.as_fd(), &d)?;
        Ok((dir, opened))
    }
}

impl StackStore for FsStackStore {
    fn root(&self) -> &Path {
        &self.root
    }

    fn global_env(&self) -> bool {
        let flags = OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC;
        openat(&self.root_fd, GLOBAL_ENV_FILE, flags, Mode::empty())
            .ok()
            .and_then(|fd| File::from(fd).metadata().ok())
            .is_some_and(|m| m.is_file())
    }

    fn list(&self) -> Result<StackListing, StoreError> {
        let mut stacks = Vec::new();
        let dir = Dir::read_from(&self.root_fd).map_err(|e| io_error("stacks root", e.into()))?;
        for entry in dir {
            let entry = entry.map_err(|e| io_error("stacks root", e.into()))?;
            let Some(name) = child_name(entry.file_name()) else {
                continue;
            };
            let candidate = match entry.file_type() {
                FileType::Directory => true,
                // Listed only when it points at a directory; never opened through.
                FileType::Symlink | FileType::Unknown => openat(
                    &self.root_fd,
                    name,
                    OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .is_ok(),
                _ => false,
            };
            if !candidate {
                continue;
            }
            match self.entry_named(name) {
                Ok(e) => stacks.push(e),
                Err(StoreError::NotFound | StoreError::NotAStack) => {}
                Err(e) => return Err(e),
            }
        }
        stacks.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(StackListing {
            root: self.root.clone(),
            global_env: self.global_env(),
            stacks,
        })
    }

    fn entry(&self, name: &StackName) -> Result<StackEntry, StoreError> {
        self.entry_named(name.as_str())
    }

    fn read(&self, name: &StackName) -> Result<StackFiles, StoreError> {
        let (_dir, opened) = self.load(name)?;
        opened.read(self.max_file_bytes)?.into_files()
    }

    fn write_atomic(
        &self,
        name: &StackName,
        compose: &str,
        env: Option<&str>,
        base_hash: &ContentHash,
    ) -> Result<ContentHash, StoreError> {
        let (dir, opened) = self.load(name)?;
        let dir = dir.as_fd();
        let read_only = read_only_reason(dir, &opened);
        let compose_target = (opened.compose.resolved.clone(), opened.compose.meta.clone());
        let env_target = opened
            .env
            .as_ref()
            .map(|o| (o.resolved.clone(), o.meta.clone()));
        let compose_file = opened.compose_file;

        let current = opened.read(self.max_file_bytes)?;
        if &current.hash() != base_hash {
            return Err(StoreError::Conflict {
                current: Box::new(current.into_files()?),
            });
        }
        if let Some(reason) = read_only {
            return Err(StoreError::ReadOnly { reason });
        }

        replace(
            dir,
            &compose_target.0,
            compose.as_bytes(),
            Some(&compose_target.1),
            COMPOSE_MODE,
        )?;
        let env_after: Option<Vec<u8>> = match (env, env_target) {
            (Some(text), Some((target, meta))) => {
                replace(dir, &target, text.as_bytes(), Some(&meta), ENV_MODE)?;
                Some(text.as_bytes().to_vec())
            }
            (Some(text), None) if !text.is_empty() => {
                replace(dir, ENV_FILE, text.as_bytes(), None, ENV_MODE)?;
                Some(text.as_bytes().to_vec())
            }
            (Some(_), None) => None,
            (None, _) => current.env,
        };
        fsync(dir).map_err(|e| io_error("stack directory", e.into()))?;
        Ok(content_hash_of(
            compose_file,
            compose.as_bytes(),
            env_after.as_deref(),
        ))
    }

    fn create(
        &self,
        name: &StackName,
        compose: &str,
        env: Option<&str>,
    ) -> Result<ContentHash, StoreError> {
        match mkdirat(
            &self.root_fd,
            name.as_str(),
            Mode::from_raw_mode(STACK_DIR_MODE),
        ) {
            Ok(()) => {}
            Err(Errno::EXIST) => return Err(StoreError::AlreadyExists),
            Err(e) => return Err(write_error("stacks root", e.into())),
        }
        let written = (|| {
            let dir = self.open_stack(name.as_str())?;
            let dir = dir.as_fd();
            let compose_file = ComposeFile::DEFAULT;
            replace(
                dir,
                compose_file.file_name(),
                compose.as_bytes(),
                None,
                COMPOSE_MODE,
            )?;
            let env = env.filter(|e| !e.is_empty());
            if let Some(text) = env {
                replace(dir, ENV_FILE, text.as_bytes(), None, ENV_MODE)?;
            }
            fsync(dir).map_err(|e| io_error("stack directory", e.into()))?;
            Ok(content_hash_of(
                compose_file,
                compose.as_bytes(),
                env.map(str::as_bytes),
            ))
        })();
        match written {
            Ok(hash) => {
                fsync(&self.root_fd).map_err(|e| io_error("stacks root", e.into()))?;
                Ok(hash)
            }
            Err(e) => {
                // Only what this call made: the directory did not exist before mkdirat.
                let _ = std::fs::remove_dir_all(self.root.join(name.as_str()));
                Err(e)
            }
        }
    }

    fn delete_dir(&self, name: &StackName) -> Result<(), StoreError> {
        // Refuses a symlinked stack directory. If the name is swapped for a symlink after this
        // check, `remove_dir_all` removes the link itself: it never follows one, at any depth.
        drop(self.open_stack(name.as_str())?);
        std::fs::remove_dir_all(self.root.join(name.as_str()))
            .map_err(|e| write_error(&format!("removing {name}"), e))?;
        fsync(&self.root_fd).map_err(|e| io_error("stacks root", e.into()))
    }
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

/// One file of a stack, open.
struct OpenFile {
    /// The name the stack uses (`compose.yaml`, `.env`).
    logical: &'static str,
    /// The file actually opened: `logical`, or its one-hop symlink target in the same directory.
    resolved: String,
    file: File,
    meta: Metadata,
}

struct Opened {
    compose_file: ComposeFile,
    compose: OpenFile,
    env: Option<OpenFile>,
}

impl Opened {
    fn modified_at(&self) -> Option<SystemTime> {
        let compose = self.compose.meta.modified().ok();
        let env = self.env.as_ref().and_then(|e| e.meta.modified().ok());
        compose.max(env)
    }

    fn read(self, limit: u64) -> Result<Contents, StoreError> {
        let compose = read_bounded(self.compose, limit)?;
        let env = self.env.map(|e| read_bounded(e, limit)).transpose()?;
        Ok(Contents {
            compose_file: self.compose_file,
            compose,
            env,
        })
    }
}

struct Contents {
    compose_file: ComposeFile,
    compose: Vec<u8>,
    env: Option<Vec<u8>>,
}

impl Contents {
    fn hash(&self) -> ContentHash {
        content_hash_of(self.compose_file, &self.compose, self.env.as_deref())
    }

    fn into_files(self) -> Result<StackFiles, StoreError> {
        let content_hash = self.hash();
        let text = |bytes: Vec<u8>, file: &str| {
            String::from_utf8(bytes).map_err(|_| StoreError::NotText { file: file.into() })
        };
        Ok(StackFiles {
            compose_file: self.compose_file,
            compose: text(self.compose, self.compose_file.file_name())?,
            env: self.env.map(|e| text(e, ENV_FILE)).transpose()?,
            content_hash,
        })
    }
}

/// Opens `file` in `dir` for reading. `None` when it is not there.
fn open_in(dir: BorrowedFd<'_>, file: &'static str) -> Result<Option<OpenFile>, StoreError> {
    // NONBLOCK: a FIFO named compose.yaml must not hang the open; it is refused below.
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let escapes = || StoreError::Escapes {
        reason: format!("{file} is a symlink out of the stack directory"),
    };
    let (fd, resolved) = match openat(dir, file, flags, Mode::empty()) {
        Ok(fd) => (fd, file.to_owned()),
        Err(Errno::NOENT) => return Ok(None),
        Err(Errno::LOOP) => {
            let target = readlinkat(dir, file, Vec::new()).map_err(|_| escapes())?;
            let target = target
                .to_str()
                .ok()
                .filter(|t| is_plain_name(t))
                .ok_or_else(escapes)?
                .to_owned();
            // One hop: a link to a link is refused by O_NOFOLLOW here.
            match openat(dir, target.as_str(), flags, Mode::empty()) {
                Ok(fd) => (fd, target),
                Err(Errno::LOOP) => return Err(escapes()),
                Err(e) => return Err(io_error(&format!("{file} -> {target}"), e.into())),
            }
        }
        Err(e) => return Err(io_error(file, e.into())),
    };
    let file_handle = File::from(fd);
    let meta = file_handle.metadata().map_err(|e| io_error(file, e))?;
    if !meta.is_file() {
        return Err(StoreError::Io {
            message: format!("{file} is not a regular file"),
        });
    }
    Ok(Some(OpenFile {
        logical: file,
        resolved,
        file: file_handle,
        meta,
    }))
}

/// Reads at most `limit` bytes, whatever the size the file reports.
fn read_bounded(f: OpenFile, limit: u64) -> Result<Vec<u8>, StoreError> {
    let too_large = |bytes| StoreError::TooLarge {
        file: f.logical.into(),
        bytes,
        limit,
    };
    if f.meta.len() > limit {
        return Err(too_large(f.meta.len()));
    }
    let mut buf = Vec::with_capacity(f.meta.len() as usize);
    (&f.file)
        .take(limit + 1)
        .read_to_end(&mut buf)
        .map_err(|e| io_error(f.logical, e))?;
    if buf.len() as u64 > limit {
        return Err(too_large(buf.len() as u64));
    }
    Ok(buf)
}

/// Replaces `target` in `dir` with `bytes`: temp file, fsync, rename. Keeps `existing`'s mode and
/// owner, or uses `new_mode` for a new file.
fn replace(
    dir: BorrowedFd<'_>,
    target: &str,
    bytes: &[u8],
    existing: Option<&Metadata>,
    new_mode: u32,
) -> Result<(), StoreError> {
    let tmp = format!(
        "{TEMP_PREFIX}{}-{}",
        std::process::id(),
        TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let flags = OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let fd = openat(dir, tmp.as_str(), flags, Mode::from_raw_mode(0o600))
        .map_err(|e| write_error(target, e.into()))?;
    let mut file = File::from(fd);

    let written = (|| {
        file.write_all(bytes).map_err(|e| write_error(target, e))?;
        let mode = match existing {
            Some(meta) => {
                let mine = file.metadata().map_err(|e| io_error(target, e))?;
                if (mine.uid(), mine.gid()) != (meta.uid(), meta.gid()) {
                    fchown(
                        &file,
                        Some(Uid::from_raw(meta.uid())),
                        Some(Gid::from_raw(meta.gid())),
                    )
                    .map_err(|e| StoreError::ReadOnly {
                        reason: format!("cannot keep {target}'s owner: {}", io::Error::from(e)),
                    })?;
                }
                meta.mode() & 0o7777
            }
            None => new_mode,
        };
        fchmod(&file, Mode::from_raw_mode(mode)).map_err(|e| write_error(target, e.into()))?;
        file.sync_all().map_err(|e| write_error(target, e))?;
        renameat(dir, tmp.as_str(), dir, target).map_err(|e| write_error(target, e.into()))
    })();
    if written.is_err() {
        let _ = unlinkat(dir, tmp.as_str(), AtFlags::empty());
    }
    written
}

/// Why the stack cannot be written, if it cannot.
fn read_only_reason(dir: BorrowedFd<'_>, opened: &Opened) -> Option<String> {
    let writable = |path: &str| accessat(dir, path, Access::WRITE_OK, AtFlags::empty()).is_ok();
    if !writable(".") {
        return Some("the stack directory is not writable by Depths".into());
    }
    for f in std::iter::once(&opened.compose).chain(opened.env.as_ref()) {
        if !writable(&f.resolved) {
            return Some(format!("{} is not writable by Depths", f.logical));
        }
    }
    None
}

fn list_dir(dir: BorrowedFd<'_>) -> Result<DirListing, StoreError> {
    let entries = Dir::read_from(dir).map_err(|e| io_error("stack directory", e.into()))?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| io_error("stack directory", e.into()))?;
        if let Some(name) = child_name(entry.file_name()) {
            names.push(name.to_owned());
        }
    }
    Ok(DirListing::new(names))
}

/// A directory entry's name, skipping `.`, `..` and names that are not UTF-8.
fn child_name(raw: &CStr) -> Option<&str> {
    raw.to_str().ok().filter(|n| *n != "." && *n != "..")
}

/// A name with no directory part: a sibling in the same directory.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/')
}

fn io_error(what: &str, e: io::Error) -> StoreError {
    StoreError::Io {
        message: format!("{what}: {e}"),
    }
}

fn write_error(what: &str, e: io::Error) -> StoreError {
    match e.kind() {
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => {
            StoreError::ReadOnly {
                reason: format!("{what}: {e}"),
            }
        }
        _ => io_error(what, e),
    }
}
