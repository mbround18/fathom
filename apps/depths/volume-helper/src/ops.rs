//! The operations the helper performs on resolved fds.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::fs::FileExt;

use depths_volumes::frame::{
    FrameError, FrameKind, read_frame, write_data, write_frame, write_json,
};
use depths_volumes::mount::{Entry, EntryKind, PAGE_SIZE, name_order, page_of, rfc3339_utc};
use depths_volumes::op::{HelperError, Op, Reply, StatInfo};
use depths_volumes::path::{RawName, RelPath};
use depths_volumes::text::Fingerprint;
use rustix::fs::{Dir, FileType, OFlags, Statx};
use sha2::{Digest, Sha256};

use crate::resolve::{
    Root, errno_message, errno_name, kind_of, mtime_ns, os_error, owner_of, statx_at, statx_fd,
};
use crate::save::{Temp, sync_dir};

/// Bytes read from a file per call, and the most a Data frame the helper sends carries.
const CHUNK: usize = 256 * 1024;

/// Largest replaced or deleted content sent back as `prior` frames, for history (R9; the
/// default `DEPTHS_FILES_HISTORY_FILE_MAX`).
pub const PRIOR_MAX: u64 = 10 * 1024 * 1024;

/// Runs one op and writes its answer: a Reply frame (plus Data… End where the reply says so) or
/// an Error frame. The op reads the Data… End that follow it from `input` itself.
pub fn handle<R: Read, W: Write>(
    root: &Root,
    op: &Op,
    input: &mut R,
    out: &mut W,
) -> Result<(), FrameError> {
    match op {
        Op::Stat {
            path,
            raw_name,
            hash,
        } => answer(
            out,
            stat(root, path, raw_name.as_ref(), *hash).map(Reply::Stat),
        ),
        Op::List { path, page } => answer(out, list(root, path, *page)),
        Op::Read {
            path,
            raw_name,
            max,
        } => read(root, path, raw_name.as_ref(), *max, out),
        Op::Write { path, base, mode } => {
            with_data(input, out, |data| write(root, path, base, *mode, data))
        }
        Op::Untar { .. } => {
            drain_data(input)?;
            answer(out, Err(HelperError::Unsupported))
        }
        _ => {
            if op.sends_data() {
                drain_data(input)?;
            }
            answer(out, Err(HelperError::Unsupported))
        }
    }
}

/// Writes a Reply or an Error frame.
fn answer<W: Write>(out: &mut W, answer: Result<Reply, HelperError>) -> Result<(), FrameError> {
    match answer {
        Ok(reply) => write_json(out, FrameKind::Reply, &reply),
        Err(error) => write_json(out, FrameKind::Error, &error),
    }
}

/// Reads and drops Data frames up to the End frame.
fn drain_data<R: Read>(input: &mut R) -> Result<(), FrameError> {
    DataIn::new(input).finish()
}

/// The Data… End that follow an op, one frame at a time.
struct DataIn<'a, R> {
    input: &'a mut R,
    ended: bool,
}

impl<'a, R: Read> DataIn<'a, R> {
    fn new(input: &'a mut R) -> Self {
        Self {
            input,
            ended: false,
        }
    }

    /// The next Data frame's bytes, `None` once End was read.
    fn next(&mut self) -> Result<Option<Vec<u8>>, FrameError> {
        if self.ended {
            return Ok(None);
        }
        let frame = read_frame(self.input)?.ok_or(FrameError::Truncated)?;
        match frame.kind {
            FrameKind::End => {
                self.ended = true;
                Ok(None)
            }
            FrameKind::Data => Ok(Some(frame.body)),
            got => Err(FrameError::UnexpectedKind {
                expected: FrameKind::Data,
                got,
            }),
        }
    }

    /// Reads what is left of the stream.
    fn finish(&mut self) -> Result<(), FrameError> {
        while self.next()?.is_some() {}
        Ok(())
    }
}

/// Why an op that reads Data… End stopped early.
enum Stop {
    /// Refused or failed: the rest of the stream is read and dropped, and an Error frame sent.
    Refused(HelperError),
    /// The stream itself broke: a protocol error, the helper exits 70.
    Broken(FrameError),
}

impl From<HelperError> for Stop {
    fn from(e: HelperError) -> Self {
        Self::Refused(e)
    }
}

impl From<FrameError> for Stop {
    fn from(e: FrameError) -> Self {
        Self::Broken(e)
    }
}

/// What a save wrote, and the content it replaced when that was at most [`PRIOR_MAX`].
struct Saved {
    fingerprint: Fingerprint,
    prior: Option<Vec<u8>>,
}

/// Runs a save that reads Data… End and answers `Written`, then the `prior` frames.
fn with_data<R: Read, W: Write>(
    input: &mut R,
    out: &mut W,
    save: impl FnOnce(&mut DataIn<'_, R>) -> Result<Saved, Stop>,
) -> Result<(), FrameError> {
    let mut data = DataIn::new(input);
    let result = match save(&mut data) {
        Ok(saved) => Ok(saved),
        Err(Stop::Refused(e)) => Err(e),
        Err(Stop::Broken(e)) => return Err(e),
    };
    data.finish()?;
    match result {
        Ok(Saved { fingerprint, prior }) => {
            let reply = Reply::Written {
                fingerprint,
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

/// R5: the helper runs as the owner of what it changes. Any other owner means it changed since
/// the listing that led here.
fn check_owner(stat: &Statx) -> Result<(), HelperError> {
    let owner = owner_of(stat);
    if owner.uid == rustix::process::getuid().as_raw()
        && owner.gid == rustix::process::getgid().as_raw()
    {
        Ok(())
    } else {
        Err(HelperError::OwnerChanged)
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
        let (file, _) = root.open_read(path, raw)?;
        Some(hash_file(&file, 0)?.0)
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

/// `List`: page `page` of the folder, sorted folders first and then by name.
///
/// The whole folder is read with `getdents` and sorted on the kind it reports, then only the
/// page's entries are `statx`ed, so a large folder costs one sort per page, not one `statx` per
/// entry per page. An entry removed between the two is left out of the page.
pub fn list(root: &Root, path: &RelPath, page: u32) -> Result<Reply, HelperError> {
    let mut dir = Dir::new(root.open_dir_read(path)?).map_err(os_error)?;
    let mut names: Vec<(bool, Vec<u8>)> = Vec::new();
    while let Some(entry) = dir.read() {
        let entry = entry.map_err(os_error)?;
        let name = entry.file_name().to_bytes();
        if name == b"." || name == b".." {
            continue;
        }
        let is_dir = match entry.file_type() {
            FileType::Directory => true,
            FileType::Unknown => statx_at(dir.fd().map_err(os_error)?, name)
                .is_ok_and(|stat| kind_of(&stat) == EntryKind::Dir),
            _ => false,
        };
        names.push((is_dir, name.to_vec()));
    }
    names.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| name_order(&a.1, &b.1)));
    let (slice, last) = page_of(&names, page);

    let fd = dir.fd().map_err(os_error)?;
    let base = path.to_bytes_with(None);
    let mut entries = Vec::with_capacity(slice.len().min(PAGE_SIZE));
    for (_, name) in slice {
        match entry(root, fd, &base, name) {
            Ok(entry) => entries.push(entry),
            Err(HelperError::NotFound) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(Reply::Page {
        entries,
        page,
        last,
    })
}

/// One listing entry for `name` in the open folder `dir`, whose path beneath the root is `base`.
fn entry(root: &Root, dir: impl AsFd, base: &[u8], name: &[u8]) -> Result<Entry, HelperError> {
    let dir = dir.as_fd();
    let stat = statx_at(dir, name)?;
    let kind = kind_of(&stat);
    let (link, target) = if kind == EntryKind::Link {
        let full = if base == b"." {
            name.to_vec()
        } else {
            [base, b"/", name].concat()
        };
        let (link, target) = root.link_info(dir, name, &full)?;
        (Some(link), target)
    } else {
        (None, None)
    };
    let size = match &target {
        Some(target) => target.stx_size,
        None => stat.stx_size,
    };
    let owner = owner_of(&stat);
    let (display, raw) = match std::str::from_utf8(name) {
        Ok(text) => (text.to_owned(), None),
        Err(_) => (
            String::from_utf8_lossy(name).into_owned(),
            Some(RawName::from_bytes(name).map_err(|_| HelperError::InvalidName)?),
        ),
    };
    Ok(Entry {
        name: display,
        name_b64: raw,
        kind,
        write_blocked: Entry::blocked_by_shape(kind, stat.stx_nlink, link.as_ref()),
        link,
        size,
        mtime: rfc3339_utc(stat.stx_mtime.tv_sec),
        uid: owner.uid,
        gid: owner.gid,
        mode: owner.mode,
        nlink: stat.stx_nlink,
    })
}

/// `Read`: the file's fingerprint and owner, then its bytes as Data… End when it is at most
/// `max` bytes (`data: false` and nothing follows above it).
///
/// The fingerprint is computed on the open fd first, then the same fd is streamed from the
/// start. If the file shrinks in between, an Error frame takes the place of the End frame.
pub fn read<W: Write>(
    root: &Root,
    path: &RelPath,
    raw: Option<&RawName>,
    max: u64,
    out: &mut W,
) -> Result<(), FrameError> {
    let opened = root.open_read(path, raw).and_then(|(file, stat)| {
        let (fingerprint, _) = hash_file(&file, 0)?;
        Ok((file, stat, fingerprint))
    });
    let (file, stat, fingerprint) = match opened {
        Ok(opened) => opened,
        Err(e) => return answer(out, Err(e)),
    };
    let data = fingerprint.size() <= max;
    let reply = Reply::Content {
        fingerprint,
        owner: owner_of(&stat),
        write_blocked: Entry::blocked_by_shape(EntryKind::File, stat.stx_nlink, None),
        data,
    };
    write_json(out, FrameKind::Reply, &reply)?;
    if data {
        stream_file(&file, fingerprint.size(), out)?;
    }
    Ok(())
}

/// `Write`: replaces an existing file through a temp and a rename (R6), keeping its uid, gid and
/// mode (or setting `mode`), if it still matches `base`.
fn write<R: Read>(
    root: &Root,
    path: &RelPath,
    base: &Fingerprint,
    mode: Option<u32>,
    data: &mut DataIn<'_, R>,
) -> Result<Saved, Stop> {
    let target = root.open_write(path)?;
    check_owner(&target.stat)?;
    let replacing = Replacing {
        file: target.reopen(OFlags::RDONLY)?,
        stat: target.stat,
        base: *base,
    };
    let mode = mode.unwrap_or(owner_of(&target.stat).mode);
    save(
        target.parent.as_fd(),
        &target.name,
        Some(replacing),
        Some(mode),
        data,
    )
}

/// The file a save replaces: open for reading, as found, and the fingerprint it must still have.
struct Replacing {
    file: File,
    stat: Statx,
    base: Fingerprint,
}

/// Writes the Data… End into a temp in `dir`, then renames it over `name` (`replacing`) or gives
/// it `name` when that is free. The base check is made on the replaced file's open fd right
/// before the rename. The temp is removed on every failure.
fn save<R: Read>(
    dir: BorrowedFd<'_>,
    name: &[u8],
    replacing: Option<Replacing>,
    mode: Option<u32>,
    data: &mut DataIn<'_, R>,
) -> Result<Saved, Stop> {
    let temp = Temp::create(dir, mode)?;
    if let Some(old) = &replacing {
        let owner = owner_of(&old.stat);
        temp.set_owner(owner.uid, owner.gid)?;
    }
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    while let Some(chunk) = data.next()? {
        temp.file()
            .write_all_at(&chunk, size)
            .map_err(|e| io_error(&e))?;
        hasher.update(&chunk);
        size += chunk.len() as u64;
    }
    let stat = statx_fd(temp.file())?;
    let fingerprint = Fingerprint::from_hasher(hasher, size, mtime_ns(&stat), stat.stx_ino);
    let prior = match replacing {
        Some(old) => {
            let (current, prior) = hash_file(&old.file, PRIOR_MAX)?;
            if current != old.base {
                return Err(HelperError::ChangedOnDisk { current }.into());
            }
            // The name must still lead to the file that was hashed.
            let now = statx_at(dir, name)?;
            if now.stx_ino != old.stat.stx_ino || kind_of(&now) != EntryKind::File {
                return Err(changed_while_reading().into());
            }
            temp.replace(name)?;
            prior
        }
        None => {
            temp.create_new(name)?;
            None
        }
    };
    // The rename is done: a folder fsync that fails only weakens durability, and reporting the
    // save as failed would be wrong.
    let _ = sync_dir(dir);
    Ok(Saved { fingerprint, prior })
}

/// Sends the first `size` bytes of `file` as Data frames, then End; an Error frame instead of
/// End when the file cannot be read that far.
fn stream_file<W: Write>(file: &File, size: u64, out: &mut W) -> Result<(), FrameError> {
    let mut buf = vec![0u8; CHUNK];
    let mut offset = 0u64;
    while offset < size {
        let want = (size - offset).min(CHUNK as u64) as usize;
        match file.read_at(&mut buf[..want], offset) {
            Ok(0) => return answer(out, Err(changed_while_reading())),
            Ok(n) => {
                write_frame(out, FrameKind::Data, &buf[..n])?;
                offset += n as u64;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return answer(out, Err(io_error(&e))),
        }
    }
    write_frame(out, FrameKind::End, &[])
}

/// SHA-256 of `file`'s content plus `(size, mtime_ns, ino)` of the fd, `statx`ed after the read
/// (R7). When the content is at most `keep` bytes it is returned too.
pub(crate) fn hash_file(
    file: &File,
    keep: u64,
) -> Result<(Fingerprint, Option<Vec<u8>>), HelperError> {
    let mut hasher = Sha256::new();
    let mut kept = Some(Vec::new());
    let mut buf = vec![0u8; CHUNK];
    let mut size = 0u64;
    loop {
        match file.read_at(&mut buf, size) {
            Ok(0) => break,
            Ok(n) => {
                hasher.update(&buf[..n]);
                size += n as u64;
                kept = kept.filter(|_| size <= keep).map(|mut k| {
                    k.extend_from_slice(&buf[..n]);
                    k
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(io_error(&e)),
        }
    }
    let stat = statx_fd(file)?;
    let fingerprint = Fingerprint::from_hasher(hasher, size, mtime_ns(&stat), stat.stx_ino);
    Ok((fingerprint, kept))
}

fn changed_while_reading() -> HelperError {
    HelperError::OsError {
        errno: "EAGAIN".into(),
        message: "the file changed while it was being read".into(),
    }
}

pub(crate) fn io_error(e: &std::io::Error) -> HelperError {
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
