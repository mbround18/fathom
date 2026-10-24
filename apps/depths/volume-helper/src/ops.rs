//! The operations the helper performs on resolved fds.

use std::io::{Read, Write};

use depths_volumes::frame::{FrameError, FrameKind, write_json};
use depths_volumes::mount::{Entry, EntryKind};
use depths_volumes::op::{HelperError, Op, Reply, StatInfo};
use depths_volumes::path::{RawName, RelPath};
use depths_volumes::text::Fingerprint;
use sha2::{Digest, Sha256};

use crate::resolve::{Root, errno_message, errno_name, kind_of, mtime_ns, owner_of};

/// Runs one op and writes its answer: a Reply frame (plus Data… End where the reply says so) or
/// an Error frame. Data frames that follow the op have already been read by the caller.
pub fn handle<W: Write>(root: &Root, op: &Op, out: &mut W) -> Result<(), FrameError> {
    let answer = match op {
        Op::Stat {
            path,
            raw_name,
            hash,
        } => stat(root, path, raw_name.as_ref(), *hash).map(Reply::Stat),
        _ => Err(HelperError::Unsupported),
    };
    match answer {
        Ok(reply) => write_json(out, FrameKind::Reply, &reply),
        Err(error) => write_json(out, FrameKind::Error, &error),
    }
}

/// `Stat`: the entry itself, a link not followed (its target's size when it stays inside).
/// With `hash`, a regular file's fingerprint is computed on an fd opened for it.
pub fn stat(
    root: &Root,
    path: &RelPath,
    raw: Option<&RawName>,
    hash: bool,
) -> Result<StatInfo, HelperError> {
    let found = root.lookup(path, raw)?;
    let kind = kind_of(&found.stat);
    let nlink = found.stat.stx_nlink;
    let size = match (&found.target, kind) {
        (Some(target), EntryKind::Link) => target.stx_size,
        _ => found.stat.stx_size,
    };
    let fingerprint = if hash && kind == EntryKind::File {
        Some(fingerprint(root, path, raw)?)
    } else {
        None
    };
    Ok(StatInfo {
        kind,
        size,
        mtime_ns: mtime_ns(&found.stat),
        ino: found.stat.stx_ino,
        nlink,
        owner: owner_of(&found.stat),
        write_blocked: Entry::blocked_by_shape(kind, nlink, found.link.as_ref()),
        link: found.link,
        fingerprint,
    })
}

/// SHA-256 of the content plus `(size, mtime_ns, ino)` of the open fd (R7).
fn fingerprint(
    root: &Root,
    path: &RelPath,
    raw: Option<&RawName>,
) -> Result<Fingerprint, HelperError> {
    let (mut file, stat) = root.open_read(path, raw)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                hasher.update(&buf[..n]);
                size += n as u64;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(io_error(&e)),
        }
    }
    Ok(Fingerprint::from_hasher(
        hasher,
        size,
        mtime_ns(&stat),
        stat.stx_ino,
    ))
}

fn io_error(e: &std::io::Error) -> HelperError {
    match e.raw_os_error() {
        Some(n) => {
            let errno = rustix::io::Errno::from_raw_os_error(n);
            HelperError::OsError {
                errno: errno_name(errno),
                message: errno_message(errno),
            }
        }
        None => HelperError::OsError {
            errno: "EIO".into(),
            message: e.to_string(),
        },
    }
}
