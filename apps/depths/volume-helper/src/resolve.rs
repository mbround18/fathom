//! Opening paths beneath the mount root (R3).
//!
//! Every path the helper touches is opened here, relative to the root's fd, with
//! `openat2(RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS)`: the kernel refuses `..` above the root,
//! absolute links (even ones that would land inside) and `/proc`-style magic links, and reports
//! them as `EXDEV`/`ELOOP`, which become [`HelperError::OutsideMount`]. The final component is
//! opened with `O_NOFOLLOW` against its parent and checked with `statx`, so a link is seen as a
//! link. Callers then work on the returned fds only: swapping a link after the lookup cannot
//! redirect them out of the root.

use std::fs::File;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::Path;

use depths_volumes::mount::{EntryKind, LinkInfo, Owner, WriteBlocked};
use depths_volumes::op::HelperError;
use depths_volumes::path::{RawName, RelPath};
use rustix::fs::{AtFlags, FileType, Mode, OFlags, ResolveFlags, Statx, StatxFlags};
use rustix::io::Errno;

/// Links followed in a row by [`Root::open_write`] before it gives up (`ELOOP`).
const MAX_LINK_HOPS: usize = 8;

/// Times an `openat2` that met a concurrent rename is retried before its `EAGAIN` is returned.
const MAX_EAGAIN_RETRIES: usize = 64;

/// `openat2(dirfd, path, flags, RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS)`. With `..` in the walk
/// the kernel answers `EAGAIN` when any rename or mount raced the lookup (anywhere on the
/// system); the call is safe to repeat, so it is, a bounded number of times.
fn open_beneath<P: rustix::path::Arg + Copy>(
    dirfd: impl AsFd,
    path: P,
    flags: OFlags,
) -> rustix::io::Result<OwnedFd> {
    let dirfd = dirfd.as_fd();
    let resolve = ResolveFlags::BENEATH | ResolveFlags::NO_MAGICLINKS;
    let mut tries = 0;
    loop {
        match rustix::fs::openat2(dirfd, path, flags, Mode::empty(), resolve) {
            Err(Errno::AGAIN) if tries < MAX_EAGAIN_RETRIES => tries += 1,
            other => return other,
        }
    }
}

/// The mount root, as an `O_PATH` directory fd. Everything is opened relative to it.
#[derive(Debug)]
pub struct Root {
    fd: OwnedFd,
}

/// What [`Root::lookup`] found: the final component itself, not followed.
#[derive(Debug)]
pub struct Found {
    pub stat: Statx,
    /// For a link: its target and whether it stays inside.
    pub link: Option<LinkInfo>,
    /// For a link that stays inside and resolves: the object it leads to.
    pub target: Option<Statx>,
}

/// An existing file a write may replace: its real parent (after following links inside the
/// mount), its name there, and its `statx`. Never a link, a special file or a hard-linked file.
#[derive(Debug)]
pub struct WriteTarget {
    pub parent: OwnedFd,
    pub name: Vec<u8>,
    pub stat: Statx,
}

impl Root {
    /// Opens `path` (the mount, `/volume` in the container) as the root.
    pub fn open(path: impl AsRef<Path>) -> rustix::io::Result<Self> {
        let fd = rustix::fs::open(
            path.as_ref(),
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        Ok(Self { fd })
    }

    /// A root from an already open directory fd.
    pub fn from_fd(fd: OwnedFd) -> Self {
        Self { fd }
    }

    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    pub fn into_fd(self) -> OwnedFd {
        self.fd
    }

    /// Whether `openat2` works here. `ENOSYS` (kernel before 5.6) or a seccomp refusal makes
    /// `Hello.openat2` false; there is no fallback resolver.
    pub fn probe_openat2(&self) -> bool {
        open_beneath(
            &self.fd,
            ".",
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        )
        .is_ok()
    }

    /// Opens a folder beneath the root, following links that stay inside.
    pub fn open_dir(&self, dir: &RelPath) -> Result<OwnedFd, HelperError> {
        self.open_dir_bytes(&dir.to_bytes_with(None))
    }

    fn open_dir_bytes(&self, dir: &[u8]) -> Result<OwnedFd, HelperError> {
        open_beneath(
            &self.fd,
            dir,
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        )
        .map_err(os_error)
    }

    /// `path` itself, the final component not followed (`raw_name`, when given, is the final
    /// component and `path` its folder). A link is reported with its target; `inside` is false
    /// for an absolute link or one that resolves out of the root.
    pub fn lookup(&self, path: &RelPath, raw: Option<&RawName>) -> Result<Found, HelperError> {
        let Some((dir, name)) = split(path, raw) else {
            let stat = statx_fd(&self.fd)?;
            return Ok(Found {
                stat,
                link: None,
                target: None,
            });
        };
        let parent = self.open_dir_bytes(&dir)?;
        let fd = open_nofollow(&parent, &name)?;
        let stat = statx_fd(&fd)?;
        if kind_of(&stat) != EntryKind::Link {
            return Ok(Found {
                stat,
                link: None,
                target: None,
            });
        }
        let (link, target) = self.link_info(&parent, &name, &path.to_bytes_with(raw))?;
        Ok(Found {
            stat,
            link: Some(link),
            target,
        })
    }

    /// The link `name` in `parent`, whose full path beneath the root is `full`: its target text,
    /// whether it stays inside, and the object it leads to when it does and resolves.
    pub fn link_info(
        &self,
        parent: impl AsFd,
        name: &[u8],
        full: &[u8],
    ) -> Result<(LinkInfo, Option<Statx>), HelperError> {
        let text = rustix::fs::readlinkat(parent, name, Vec::new())
            .map_err(os_error)?
            .into_bytes();
        let (inside, target) = if text.first() == Some(&b'/') {
            (false, None)
        } else {
            match self.open_path(full) {
                Ok(fd) => (true, Some(statx_fd(&fd)?)),
                Err(HelperError::OutsideMount) => (false, None),
                // Dangling, or a folder on the way is gone: still a link inside.
                Err(_) => (true, None),
            }
        };
        let link = LinkInfo {
            target: String::from_utf8_lossy(&text).into_owned(),
            inside,
        };
        Ok((link, target))
    }

    /// Opens a folder beneath the root for reading its entries, following links that stay
    /// inside.
    pub fn open_dir_read(&self, dir: &RelPath) -> Result<OwnedFd, HelperError> {
        open_beneath(
            &self.fd,
            dir.to_bytes_with(None).as_slice(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        )
        .map_err(os_error)
    }

    /// Opens a regular file for reading, following links that stay inside. A folder answers
    /// `EISDIR`; a device, FIFO or socket is refused `write_blocked { special }` and never opened.
    pub fn open_read(
        &self,
        path: &RelPath,
        raw: Option<&RawName>,
    ) -> Result<(File, Statx), HelperError> {
        let full = path.to_bytes_with(raw);
        let probe = statx_fd(&self.open_path(&full)?)?;
        check_readable(&probe)?;
        // An O_PATH fd cannot be read, so open again. O_NONBLOCK: if the name was swapped for a
        // FIFO in between, the open does not hang, and the identity check below refuses it.
        let fd = open_beneath(
            &self.fd,
            full.as_slice(),
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOCTTY | OFlags::CLOEXEC,
        )
        .map_err(os_error)?;
        let stat = statx_fd(&fd)?;
        check_readable(&stat)?;
        if !same_object(&probe, &stat) {
            return Err(changed_while_opening());
        }
        Ok((File::from(fd), stat))
    }

    /// The existing file a write to `path` replaces. Links that stay inside are followed (each
    /// hop is opened beneath the root again); a link out, or an absolute one, is refused
    /// `outside_mount`. A hard-linked file is refused `write_blocked { hard_link }`, a device,
    /// FIFO or socket `write_blocked { special }`.
    pub fn open_write(&self, path: &RelPath) -> Result<WriteTarget, HelperError> {
        let Some((mut dir, mut name)) = split(path, None) else {
            return Err(is_a_dir());
        };
        for _ in 0..MAX_LINK_HOPS {
            let parent = self.open_dir_bytes(&dir)?;
            let fd = open_nofollow(&parent, &name)?;
            let stat = statx_fd(&fd)?;
            match kind_of(&stat) {
                EntryKind::File if stat.stx_nlink > 1 => {
                    return Err(HelperError::WriteBlocked {
                        reason: WriteBlocked::HardLink,
                    });
                }
                EntryKind::File => return Ok(WriteTarget { parent, name, stat }),
                EntryKind::Dir => return Err(is_a_dir()),
                EntryKind::Special => {
                    return Err(HelperError::WriteBlocked {
                        reason: WriteBlocked::Special,
                    });
                }
                EntryKind::Link => {
                    let text = rustix::fs::readlinkat(&parent, name.as_slice(), Vec::new())
                        .map_err(os_error)?
                        .into_bytes();
                    if text.first() == Some(&b'/') {
                        return Err(HelperError::OutsideMount);
                    }
                    // The kernel resolves the link's `..` from the folder it sits in, beneath the
                    // root; nothing is resolved as text.
                    let (head, tail) = match text.iter().rposition(|&b| b == b'/') {
                        Some(i) => (&text[..i], &text[i + 1..]),
                        None => (&[][..], &text[..]),
                    };
                    if tail.is_empty() || tail == b"." || tail == b".." {
                        return Err(is_a_dir());
                    }
                    if !head.is_empty() {
                        if dir != b"." {
                            dir.push(b'/');
                        } else {
                            dir.clear();
                        }
                        dir.extend_from_slice(head);
                    }
                    name = tail.to_vec();
                }
            }
        }
        Err(HelperError::OutsideMount)
    }

    /// `path` (bytes, relative) opened `O_PATH`, every link followed beneath the root.
    fn open_path(&self, full: &[u8]) -> Result<OwnedFd, HelperError> {
        open_beneath(&self.fd, full, OFlags::PATH | OFlags::CLOEXEC).map_err(os_error)
    }
}

impl WriteTarget {
    /// Opens the target file again with `flags` (plus `O_NOFOLLOW`), refusing it unless it is
    /// still the same file the lookup found.
    pub fn reopen(&self, flags: OFlags) -> Result<File, HelperError> {
        let fd = open_beneath(
            &self.parent,
            self.name.as_slice(),
            flags | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::NOCTTY | OFlags::CLOEXEC,
        )
        .map_err(os_error)?;
        let stat = statx_fd(&fd)?;
        if !same_object(&self.stat, &stat) || kind_of(&stat) != EntryKind::File {
            return Err(changed_while_opening());
        }
        Ok(File::from(fd))
    }
}

/// The folder (bytes, `.` for the root) and the final name, or `None` for the root itself.
fn split(path: &RelPath, raw: Option<&RawName>) -> Option<(Vec<u8>, Vec<u8>)> {
    if let Some(raw) = raw {
        return Some((path.to_bytes_with(None), raw.as_bytes().to_vec()));
    }
    let name = path.file_name()?.as_bytes().to_vec();
    let dir = path.parent().unwrap_or_else(RelPath::root);
    Some((dir.to_bytes_with(None), name))
}

fn open_nofollow(parent: &OwnedFd, name: &[u8]) -> Result<OwnedFd, HelperError> {
    open_beneath(
        parent,
        name,
        OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
    )
    .map_err(os_error)
}

/// `statx` of `name` in `dir`, a link not followed.
pub fn statx_at(dir: impl AsFd, name: &[u8]) -> Result<Statx, HelperError> {
    rustix::fs::statx(
        dir,
        name,
        AtFlags::SYMLINK_NOFOLLOW,
        StatxFlags::BASIC_STATS,
    )
    .map_err(os_error)
}

/// `statx` of an open fd.
pub fn statx_fd(fd: impl AsFd) -> Result<Statx, HelperError> {
    rustix::fs::statx(
        fd,
        "",
        AtFlags::EMPTY_PATH | AtFlags::SYMLINK_NOFOLLOW,
        StatxFlags::BASIC_STATS,
    )
    .map_err(os_error)
}

fn check_readable(stat: &Statx) -> Result<(), HelperError> {
    match kind_of(stat) {
        EntryKind::File => Ok(()),
        EntryKind::Dir => Err(is_a_dir()),
        // `open_read` follows links, so a link here was swapped in under it.
        EntryKind::Link => Err(changed_while_opening()),
        EntryKind::Special => Err(HelperError::WriteBlocked {
            reason: WriteBlocked::Special,
        }),
    }
}

fn same_object(a: &Statx, b: &Statx) -> bool {
    a.stx_ino == b.stx_ino
        && a.stx_dev_major == b.stx_dev_major
        && a.stx_dev_minor == b.stx_dev_minor
}

/// The entry kind of a `statx` result.
pub fn kind_of(stat: &Statx) -> EntryKind {
    match FileType::from_raw_mode(u32::from(stat.stx_mode)) {
        FileType::RegularFile => EntryKind::File,
        FileType::Directory => EntryKind::Dir,
        FileType::Symlink => EntryKind::Link,
        _ => EntryKind::Special,
    }
}

/// Owner and permission bits of a `statx` result.
pub fn owner_of(stat: &Statx) -> Owner {
    Owner {
        uid: stat.stx_uid,
        gid: stat.stx_gid,
        mode: u32::from(stat.stx_mode) & 0o7777,
    }
}

/// Modification time in nanoseconds since the epoch.
pub fn mtime_ns(stat: &Statx) -> i64 {
    stat.stx_mtime.tv_sec * 1_000_000_000 + i64::from(stat.stx_mtime.tv_nsec)
}

fn is_a_dir() -> HelperError {
    os_error(Errno::ISDIR)
}

fn changed_while_opening() -> HelperError {
    HelperError::OsError {
        errno: "EAGAIN".into(),
        message: "the entry changed while it was being opened".into(),
    }
}

/// Maps a system error: `EXDEV`/`ELOOP` (a link or `..` out of the root, a magic link, a loop)
/// to `outside_mount`, `ENOENT` to `not_found`, anything else verbatim.
pub fn os_error(e: Errno) -> HelperError {
    match e {
        Errno::XDEV | Errno::LOOP => HelperError::OutsideMount,
        Errno::NOENT => HelperError::NotFound,
        e => HelperError::OsError {
            errno: errno_name(e),
            message: errno_message(e),
        },
    }
}

/// The system's text for `e`, without std's ` (os error N)` suffix.
pub fn errno_message(e: Errno) -> String {
    let text = std::io::Error::from(e).to_string();
    match text.rfind(" (os error ") {
        Some(i) => text[..i].to_owned(),
        None => text,
    }
}

/// The symbolic name of `e` (`EACCES`), or `E<number>` for one not listed.
pub fn errno_name(e: Errno) -> String {
    const NAMES: &[(Errno, &str)] = &[
        (Errno::PERM, "EPERM"),
        (Errno::NOENT, "ENOENT"),
        (Errno::IO, "EIO"),
        (Errno::BADF, "EBADF"),
        (Errno::AGAIN, "EAGAIN"),
        (Errno::NOMEM, "ENOMEM"),
        (Errno::ACCESS, "EACCES"),
        (Errno::BUSY, "EBUSY"),
        (Errno::EXIST, "EEXIST"),
        (Errno::XDEV, "EXDEV"),
        (Errno::NOTDIR, "ENOTDIR"),
        (Errno::ISDIR, "EISDIR"),
        (Errno::INVAL, "EINVAL"),
        (Errno::NFILE, "ENFILE"),
        (Errno::MFILE, "EMFILE"),
        (Errno::TXTBSY, "ETXTBSY"),
        (Errno::FBIG, "EFBIG"),
        (Errno::NOSPC, "ENOSPC"),
        (Errno::ROFS, "EROFS"),
        (Errno::MLINK, "EMLINK"),
        (Errno::NAMETOOLONG, "ENAMETOOLONG"),
        (Errno::NOSYS, "ENOSYS"),
        (Errno::NOTEMPTY, "ENOTEMPTY"),
        (Errno::LOOP, "ELOOP"),
        (Errno::DQUOT, "EDQUOT"),
        (Errno::STALE, "ESTALE"),
        (Errno::OPNOTSUPP, "EOPNOTSUPP"),
    ];
    NAMES
        .iter()
        .find(|(n, _)| *n == e)
        .map(|(_, s)| (*s).to_owned())
        .unwrap_or_else(|| format!("E{}", e.raw_os_error()))
}
