//! `Create`, `Rename` and `Delete`. Each acts on names in folder fds from the resolver: the
//! entry itself is created, renamed or removed, and a link is never followed.

use std::fs::File;
use std::io::Write;
use std::os::fd::OwnedFd;

use depths_volumes::frame::{FrameError, FrameKind, write_data, write_frame, write_json};
use depths_volumes::mount::EntryKind;
use depths_volumes::op::{CreateKind, HelperError, Reply};
use depths_volumes::path::{Name, RawName, RelPath};
use rustix::fs::{AtFlags, Mode, OFlags, RenameFlags};
use rustix::io::Errno;

use crate::ops::{PRIOR_MAX, answer, changed_while_reading, check_owner, hash_file};
use crate::resolve::{
    Root, create_in, errno_message, errno_name, kind_of, open_in, owner_of, statx_at, statx_fd,
};
use crate::save::sync_dir;
use crate::transfer::{names, open_subdir};

/// `Create`: an empty file (`0666`) or folder (`0777`), less the umask, owned by the helper,
/// which runs as the folder's owner (R5). Refused `exists` when the name is taken.
pub fn create(
    root: &Root,
    parent: &RelPath,
    name: &Name,
    kind: CreateKind,
) -> Result<Reply, HelperError> {
    let dir = root.open_dir(parent)?;
    check_owner(&statx_fd(&dir)?)?;
    let name = name.as_str().as_bytes();
    match kind {
        CreateKind::File => {
            let flags = OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::NOFOLLOW;
            create_in(&dir, name, flags, 0o666).map_err(exists_or)?;
        }
        CreateKind::Dir => {
            rustix::fs::mkdirat(&dir, name, Mode::from_raw_mode(0o777)).map_err(exists_or)?;
        }
    }
    let _ = sync_dir(&dir);
    Ok(Reply::Created)
}

/// `Rename`: `from` (the entry itself, a link not followed) to `to`, both beneath the root. Never
/// replaces: refused `exists` when `to` is taken. Refused `mixed_owners` when the two folders'
/// owners differ, since no single uid could do it without privileges.
pub fn rename(root: &Root, from: &RelPath, to: &RelPath) -> Result<Reply, HelperError> {
    let (from_dir, from_name) = parent_and_name(root, from, None)?;
    let (to_dir, to_name) = parent_and_name(root, to, None)?;
    let (from_stat, to_stat) = (statx_fd(&from_dir)?, statx_fd(&to_dir)?);
    let (a, b) = (owner_of(&from_stat), owner_of(&to_stat));
    if (a.uid, a.gid) != (b.uid, b.gid) {
        return Err(HelperError::MixedOwners);
    }
    check_owner(&from_stat)?;
    statx_at(&from_dir, &from_name)?;
    match rustix::fs::renameat_with(
        &from_dir,
        from_name.as_slice(),
        &to_dir,
        to_name.as_slice(),
        RenameFlags::NOREPLACE,
    ) {
        Ok(()) => {}
        // No RENAME_NOREPLACE on this filesystem: check, then rename. The window is the one a
        // shell `mv -n` has.
        Err(Errno::INVAL) => {
            match statx_at(&to_dir, &to_name) {
                Ok(_) => return Err(HelperError::Exists),
                Err(HelperError::NotFound) => {}
                Err(e) => return Err(e),
            }
            rustix::fs::renameat(&from_dir, from_name.as_slice(), &to_dir, to_name.as_slice())
                .map_err(verbatim)?;
        }
        Err(e) => return Err(exists_or(e)),
    }
    let _ = sync_dir(&from_dir);
    let _ = sync_dir(&to_dir);
    Ok(Reply::Renamed)
}

/// `Delete`: a file, link or special file, or a folder (empty, or with `recursive` everything in
/// it). A link is removed, never followed. A single regular file's content follows as Data…
/// End when it is at most [`PRIOR_MAX`], for history.
pub fn delete<W: Write>(
    root: &Root,
    path: &RelPath,
    raw: Option<&RawName>,
    recursive: bool,
    out: &mut W,
) -> Result<(), FrameError> {
    match delete_entry(root, path, raw, recursive) {
        Ok((removed, prior)) => {
            let reply = Reply::Deleted {
                removed,
                prior: prior.is_some(),
            };
            write_json(out, FrameKind::Reply, &reply)?;
            if let Some(prior) = prior {
                write_data(out, &prior)?;
                write_frame(out, FrameKind::End, &[])?;
            }
            Ok(())
        }
        Err(e) => answer(out, Err(e)),
    }
}

fn delete_entry(
    root: &Root,
    path: &RelPath,
    raw: Option<&RawName>,
    recursive: bool,
) -> Result<(u64, Option<Vec<u8>>), HelperError> {
    let (dir, name) = parent_and_name(root, path, raw)?;
    check_owner(&statx_fd(&dir)?)?;
    let stat = statx_at(&dir, &name)?;
    match kind_of(&stat) {
        EntryKind::Dir if recursive => {
            let mut removed = 0;
            let base = path.to_bytes_with(raw);
            remove_tree(&dir, &name, &stat, &base, &mut removed)
                .map_err(|(at, e)| partial(removed, &at, e))?;
            let _ = sync_dir(&dir);
            Ok((removed, None))
        }
        EntryKind::Dir => {
            rustix::fs::unlinkat(&dir, name.as_slice(), AtFlags::REMOVEDIR).map_err(verbatim)?;
            let _ = sync_dir(&dir);
            Ok((1, None))
        }
        kind => {
            let prior = if kind == EntryKind::File {
                prior_of(&dir, &name, &stat)?
            } else {
                None
            };
            rustix::fs::unlinkat(&dir, name.as_slice(), AtFlags::empty()).map_err(verbatim)?;
            let _ = sync_dir(&dir);
            Ok((1, prior))
        }
    }
}

/// The content of the file about to be deleted, when it is at most [`PRIOR_MAX`].
fn prior_of(
    dir: &OwnedFd,
    name: &[u8],
    stat: &rustix::fs::Statx,
) -> Result<Option<Vec<u8>>, HelperError> {
    if stat.stx_size > PRIOR_MAX {
        return Ok(None);
    }
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::NOCTTY;
    let file = File::from(open_in(dir, name, flags)?);
    let opened = statx_fd(&file)?;
    if opened.stx_ino != stat.stx_ino || kind_of(&opened) != EntryKind::File {
        return Err(changed_while_reading());
    }
    Ok(hash_file(&file, PRIOR_MAX)?.1)
}

/// One folder being emptied: its fd, its path beneath the root, its name in its parent, and the
/// names in it not yet removed.
struct Level {
    fd: OwnedFd,
    path: Vec<u8>,
    name: Vec<u8>,
    left: Vec<Vec<u8>>,
}

/// Removes the folder `name` in `parent` and everything in it, depth first on fds: each folder
/// is opened from its parent's fd with `O_NOFOLLOW`, so a folder swapped for a link is never
/// entered, and a link is unlinked as a name. Stops at the first entry that cannot be removed
/// and returns its path and the error. The walk's memory is one fd and one name list per level.
fn remove_tree(
    parent: &OwnedFd,
    name: &[u8],
    stat: &rustix::fs::Statx,
    path: &[u8],
    removed: &mut u64,
) -> Result<(), (Vec<u8>, HelperError)> {
    let mut stack = vec![open(parent, name, stat, path.to_vec())?];
    while let Some(level) = stack.last_mut() {
        let Some(child) = level.left.pop() else {
            let done = stack.pop().unwrap_or_else(|| unreachable!());
            let dir = stack.last().map_or(parent, |up| &up.fd);
            rustix::fs::unlinkat(dir, done.name.as_slice(), AtFlags::REMOVEDIR)
                .map_err(|e| (done.path.clone(), verbatim(e)))?;
            *removed += 1;
            continue;
        };
        let child_path = [level.path.as_slice(), b"/", child.as_slice()].concat();
        let child_stat = match statx_at(&level.fd, &child) {
            Ok(stat) => stat,
            Err(HelperError::NotFound) => continue,
            Err(e) => return Err((child_path, e)),
        };
        if kind_of(&child_stat) == EntryKind::Dir {
            let next = open(&level.fd, &child, &child_stat, child_path)?;
            stack.push(next);
        } else {
            match rustix::fs::unlinkat(&level.fd, child.as_slice(), AtFlags::empty()) {
                Ok(()) | Err(Errno::NOENT) => *removed += 1,
                Err(e) => return Err((child_path, verbatim(e))),
            }
        }
    }
    Ok(())
}

/// Opens the folder `name` in `dir` for [`remove_tree`] and lists it.
fn open(
    dir: &OwnedFd,
    name: &[u8],
    stat: &rustix::fs::Statx,
    path: Vec<u8>,
) -> Result<Level, (Vec<u8>, HelperError)> {
    let fd = open_subdir(dir, name, stat).map_err(|e| (path.clone(), e))?;
    let mut left = names(&fd).map_err(|e| (path.clone(), e))?;
    left.reverse();
    Ok(Level {
        fd,
        path,
        name: name.to_vec(),
        left,
    })
}

/// `partial` once something was removed; the error itself when nothing was.
fn partial(removed: u64, at: &[u8], e: HelperError) -> HelperError {
    if removed == 0 {
        return e;
    }
    let (errno, message) = match e {
        HelperError::OsError { errno, message } => (errno, message),
        HelperError::NotFound => (errno_name(Errno::NOENT), errno_message(Errno::NOENT)),
        other => ("EAGAIN".to_owned(), other.to_string()),
    };
    HelperError::Partial {
        removed,
        failed_at: String::from_utf8_lossy(at).into_owned(),
        errno,
        message,
    }
}

/// The folder (an `O_PATH` fd beneath the root, links that stay inside followed) and the final
/// name of `path`. The mount root itself has no name: `invalid_name`.
fn parent_and_name(
    root: &Root,
    path: &RelPath,
    raw: Option<&RawName>,
) -> Result<(OwnedFd, Vec<u8>), HelperError> {
    let (dir, name) = match raw {
        Some(raw) => (path.clone(), raw.as_bytes().to_vec()),
        None => {
            let name = path.file_name().ok_or(HelperError::InvalidName)?;
            let dir = path.parent().unwrap_or_else(RelPath::root);
            (dir, name.as_bytes().to_vec())
        }
    };
    Ok((root.open_dir(&dir)?, name))
}

/// A system error from a call on a name (not a path walk): `ENOENT` is `not_found`, everything
/// else verbatim, `EXDEV` included.
fn verbatim(e: Errno) -> HelperError {
    match e {
        Errno::NOENT => HelperError::NotFound,
        e => HelperError::OsError {
            errno: errno_name(e),
            message: errno_message(e),
        },
    }
}

fn exists_or(e: Errno) -> HelperError {
    match e {
        Errno::EXIST | Errno::NOTEMPTY => HelperError::Exists,
        e => verbatim(e),
    }
}
