//! The temp file a save writes before it renames it into place (R6). Never an in-place
//! truncate.
//!
//! The temp is an unnamed `O_TMPFILE` in the target's folder where the filesystem and `/proc`
//! allow it, so a helper killed mid-write leaves nothing behind: the kernel frees the inode with
//! the fd. It gets a name (`.depths-save-<random>`, via `linkat` of `/proc/self/fd/<n>`) only
//! for the `renameat` over the target. Elsewhere it is created by that name with `O_EXCL` and
//! unlinked on every failure.

use std::fs::File;
use std::hash::{BuildHasher, Hasher};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};

use depths_volumes::op::HelperError;
use rustix::fs::{AtFlags, Gid, OFlags, RenameFlags, StatxFlags, Uid};
use rustix::io::Errno;

use crate::resolve::{create_in, open_in, os_error, statx_fd};

/// Prefix of a temp file's name, when it has one.
pub const TEMP_PREFIX: &str = ".depths-save-";

/// Names tried before an `EEXIST` is returned.
const NAME_TRIES: usize = 8;

/// A temp file in `dir`, removed on drop unless it was committed.
#[derive(Debug)]
pub struct Temp<'a> {
    dir: BorrowedFd<'a>,
    file: File,
    /// The temp's name in `dir`, while it has one.
    name: Option<Vec<u8>>,
}

impl<'a> Temp<'a> {
    /// A new temp in `dir` (a folder fd from the resolver). With `mode` it is created `0600` and
    /// then set to exactly `mode`, so no other bits show while it is written; without, it gets
    /// the default `0666` less the umask, as any new file.
    pub fn create(dir: BorrowedFd<'a>, mode: Option<u32>) -> Result<Self, HelperError> {
        let create_mode = if mode.is_some() { 0o600 } else { 0o666 };
        let temp = match Self::unnamed(dir, create_mode)? {
            Some(temp) => temp,
            None => Self::named(dir, create_mode)?,
        };
        if let Some(mode) = mode {
            rustix::fs::fchmod(&temp.file, rustix::fs::Mode::from_raw_mode(mode & 0o7777))
                .map_err(os_error)?;
        }
        Ok(temp)
    }

    /// An `O_TMPFILE`, or `None` when the filesystem cannot make one or `/proc` cannot link it.
    fn unnamed(dir: BorrowedFd<'a>, mode: u32) -> Result<Option<Self>, HelperError> {
        let fd = match create_in(dir, b".", OFlags::TMPFILE | OFlags::RDWR, mode) {
            Ok(fd) => fd,
            Err(Errno::OPNOTSUPP | Errno::ISDIR | Errno::INVAL | Errno::NOENT) => return Ok(None),
            Err(e) => return Err(os_error(e)),
        };
        let file = File::from(fd);
        let own = statx_fd(&file)?;
        let linkable = rustix::fs::statx(
            rustix::fs::CWD,
            proc_path(&file).as_str(),
            AtFlags::empty(),
            StatxFlags::BASIC_STATS,
        )
        .is_ok_and(|via_proc| via_proc.stx_ino == own.stx_ino);
        Ok(linkable.then_some(Self {
            dir,
            file,
            name: None,
        }))
    }

    /// A temp created by a fresh `.depths-save-<random>` name with `O_EXCL`.
    pub fn named(dir: BorrowedFd<'a>, mode: u32) -> Result<Self, HelperError> {
        let mut last = Errno::EXIST;
        for _ in 0..NAME_TRIES {
            let name = temp_name();
            let flags = OFlags::CREATE | OFlags::EXCL | OFlags::RDWR | OFlags::NOFOLLOW;
            match create_in(dir, &name, flags, mode) {
                Ok(fd) => {
                    return Ok(Self {
                        dir,
                        file: File::from(fd),
                        name: Some(name),
                    });
                }
                Err(Errno::EXIST) => last = Errno::EXIST,
                Err(e) => return Err(os_error(e)),
            }
        }
        Err(os_error(last))
    }

    pub fn file(&self) -> &File {
        &self.file
    }

    /// Sets the owner (a no-op when it already is the helper's, which R5 makes the usual case).
    pub fn set_owner(&self, uid: u32, gid: u32) -> Result<(), HelperError> {
        rustix::fs::fchown(
            &self.file,
            Some(Uid::from_raw(uid)),
            Some(Gid::from_raw(gid)),
        )
        .map_err(os_error)
    }

    /// `fsync`s the temp and renames it over `name` in its folder.
    pub fn replace(mut self, name: &[u8]) -> Result<(), HelperError> {
        self.file.sync_all().map_err(|e| crate::ops::io_error(&e))?;
        if self.name.is_none() {
            self.name = Some(self.link_somewhere()?);
        }
        let temp = self.name.as_deref().unwrap_or_default();
        rustix::fs::renameat(self.dir, temp, self.dir, name).map_err(os_error)?;
        self.name = None;
        Ok(())
    }

    /// `fsync`s the temp and gives it `name`, refused `exists` when `name` exists.
    pub fn create_new(mut self, name: &[u8]) -> Result<(), HelperError> {
        self.file.sync_all().map_err(|e| crate::ops::io_error(&e))?;
        let Some(temp) = self.name.clone() else {
            return self.link_as(name).map_err(exists_or);
        };
        match rustix::fs::renameat_with(self.dir, &temp, self.dir, name, RenameFlags::NOREPLACE) {
            Ok(()) => {
                self.name = None;
                Ok(())
            }
            // No RENAME_NOREPLACE on this filesystem: a hard link cannot replace either, and
            // drop removes the temp's name.
            Err(Errno::INVAL) => {
                rustix::fs::linkat(self.dir, &temp, self.dir, name, AtFlags::empty())
                    .map_err(exists_or)
            }
            Err(e) => Err(exists_or(e)),
        }
    }

    /// Links the unnamed temp as `name` in its folder.
    fn link_as(&self, name: &[u8]) -> Result<(), Errno> {
        rustix::fs::linkat(
            rustix::fs::CWD,
            proc_path(&self.file).as_str(),
            self.dir,
            name,
            AtFlags::SYMLINK_FOLLOW,
        )
    }

    /// Links the unnamed temp under a fresh temp name and returns it.
    fn link_somewhere(&self) -> Result<Vec<u8>, HelperError> {
        for _ in 0..NAME_TRIES {
            let name = temp_name();
            match self.link_as(&name) {
                Ok(()) => return Ok(name),
                Err(Errno::EXIST) => {}
                Err(e) => return Err(os_error(e)),
            }
        }
        Err(os_error(Errno::EXIST))
    }
}

impl Drop for Temp<'_> {
    fn drop(&mut self) {
        if let Some(name) = self.name.take() {
            let _ = rustix::fs::unlinkat(self.dir, name.as_slice(), AtFlags::empty());
        }
    }
}

/// `fsync`s a folder, after a rename or link in it.
pub fn sync_dir(dir: impl AsFd) -> Result<(), HelperError> {
    let fd = open_in(dir, b".", OFlags::RDONLY | OFlags::DIRECTORY)?;
    rustix::fs::fsync(&fd).map_err(os_error)
}

fn proc_path(file: &File) -> String {
    format!("/proc/self/fd/{}", file.as_raw_fd())
}

/// `.depths-save-<16 hex>`. `RandomState` is seeded from the OS once per thread and then
/// stepped, which is all a name that `O_EXCL` guards needs.
fn temp_name() -> Vec<u8> {
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(0);
    format!("{TEMP_PREFIX}{:016x}", hasher.finish()).into_bytes()
}

fn exists_or(e: Errno) -> HelperError {
    match e {
        Errno::EXIST => HelperError::Exists,
        e => os_error(e),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::os::fd::OwnedFd;

    use super::*;

    fn dir_fd(path: &std::path::Path) -> OwnedFd {
        rustix::fs::open(
            path,
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .unwrap()
    }

    fn names(path: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(path)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_named_temp_replaces_and_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), b"old").unwrap();
        let fd = dir_fd(dir.path());
        let temp = Temp::named(fd.as_fd(), 0o600).unwrap();
        assert_eq!(names(dir.path()).len(), 2);
        temp.file().write_all(b"new").unwrap();
        temp.replace(b"a").unwrap();
        assert_eq!(std::fs::read(dir.path().join("a")).unwrap(), b"new");
        assert_eq!(names(dir.path()), ["a"]);
    }

    #[test]
    fn a_named_temp_dropped_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let fd = dir_fd(dir.path());
        let temp = Temp::named(fd.as_fd(), 0o600).unwrap();
        assert!(names(dir.path())[0].starts_with(TEMP_PREFIX));
        drop(temp);
        assert!(names(dir.path()).is_empty());
    }

    #[test]
    fn create_new_refuses_an_existing_name_either_way() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), b"old").unwrap();
        let fd = dir_fd(dir.path());
        let named = Temp::named(fd.as_fd(), 0o600).unwrap();
        assert_eq!(named.create_new(b"a"), Err(HelperError::Exists));
        let unnamed = Temp::create(fd.as_fd(), None).unwrap();
        assert!(unnamed.name.is_none(), "this filesystem has O_TMPFILE");
        assert_eq!(unnamed.create_new(b"a"), Err(HelperError::Exists));
        assert_eq!(names(dir.path()), ["a"]);
        assert_eq!(std::fs::read(dir.path().join("a")).unwrap(), b"old");

        let named = Temp::named(fd.as_fd(), 0o600).unwrap();
        named.file().write_all(b"b").unwrap();
        named.create_new(b"b").unwrap();
        let unnamed = Temp::create(fd.as_fd(), None).unwrap();
        unnamed.file().write_all(b"c").unwrap();
        unnamed.create_new(b"c").unwrap();
        assert_eq!(names(dir.path()), ["a", "b", "c"]);
        assert_eq!(std::fs::read(dir.path().join("c")).unwrap(), b"c");
    }

    #[test]
    fn temp_names_differ() {
        assert_ne!(temp_name(), temp_name());
    }
}
