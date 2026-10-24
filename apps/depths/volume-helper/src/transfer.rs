//! `Size` and `Tar`: walks of a folder on fds beneath the root. Every folder below the start is
//! opened from its parent's fd with `O_NOFOLLOW`, so a link is never followed and a folder
//! swapped for a link mid-walk is never entered.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use depths_volumes::frame::{FrameError, FrameKind, write_frame, write_json};
use depths_volumes::mount::EntryKind;
use depths_volumes::op::{HelperError, Reply, SizeCap};
use depths_volumes::path::{RawName, RelPath};
use rustix::fs::{Dir, OFlags, Statx};
use rustix::io::Errno;
use tar::{Builder, EntryType, Header};

use crate::ops::{CHUNK, answer, changed_while_reading, io_error};
use crate::resolve::{Root, kind_of, open_in, os_error, owner_of, statx_at, statx_fd};

/// The names in a folder (an fd from the resolver), `.` and `..` left out, sorted by bytes.
pub(crate) fn names(dir: impl AsFd) -> Result<Vec<Vec<u8>>, HelperError> {
    let fd = open_in(dir, b".", OFlags::RDONLY | OFlags::DIRECTORY)?;
    let mut entries = Dir::new(fd).map_err(os_error)?;
    let mut names = Vec::new();
    while let Some(entry) = entries.read() {
        let entry = entry.map_err(os_error)?;
        let name = entry.file_name().to_bytes();
        if name != b"." && name != b".." {
            names.push(name.to_vec());
        }
    }
    names.sort();
    Ok(names)
}

/// The folder `name` in `dir`, opened `O_PATH` and never through a link, if it is still the
/// folder `stat` describes.
pub(crate) fn open_subdir(
    dir: impl AsFd,
    name: &[u8],
    stat: &Statx,
) -> Result<OwnedFd, HelperError> {
    let fd = open_in(
        dir,
        name,
        OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW,
    )?;
    let now = statx_fd(&fd)?;
    if now.stx_ino != stat.stx_ino || kind_of(&now) != EntryKind::Dir {
        return Err(changed_while_reading());
    }
    Ok(fd)
}

/// `Size`: the bytes of the regular files and the number of entries beneath a folder (a file
/// counts as one entry of its size). The walk stops as soon as either count passes its cap, and
/// the counts are then lower bounds. Links and special files count as entries of no bytes; a
/// hard-linked file counts once per name.
pub fn size(
    root: &Root,
    path: &RelPath,
    raw: Option<&RawName>,
    cap: SizeCap,
) -> Result<Reply, HelperError> {
    let (fd, stat) = root.open_any(path, raw)?;
    match kind_of(&stat) {
        EntryKind::Dir => {}
        EntryKind::File => {
            return Ok(Reply::Size {
                bytes: stat.stx_size,
                entries: 1,
                capped: stat.stx_size > cap.bytes || cap.entries == 0,
            });
        }
        _ => {
            return Ok(Reply::Size {
                bytes: 0,
                entries: 1,
                capped: cap.entries == 0,
            });
        }
    }
    let (mut bytes, mut entries) = (0u64, 0u64);
    let mut stack = vec![fd];
    while let Some(dir) = stack.pop() {
        for name in names(&dir)? {
            let stat = match statx_at(&dir, &name) {
                Ok(stat) => stat,
                Err(HelperError::NotFound) => continue,
                Err(e) => return Err(e),
            };
            entries += 1;
            match kind_of(&stat) {
                EntryKind::File => bytes = bytes.saturating_add(stat.stx_size),
                EntryKind::Dir => match open_subdir(&dir, &name, &stat) {
                    Ok(sub) => stack.push(sub),
                    Err(HelperError::NotFound) => {}
                    Err(e) => return Err(e),
                },
                EntryKind::Link | EntryKind::Special => {}
            }
            if bytes > cap.bytes || entries > cap.entries {
                return Ok(Reply::Size {
                    bytes,
                    entries,
                    capped: true,
                });
            }
        }
    }
    Ok(Reply::Size {
        bytes,
        entries,
        capped: false,
    })
}

/// `Tar`: `Reply::Tar`, then an uncompressed tar of the folder as Data… End. Entries are named
/// beneath the folder's own name (no prefix for the mount root), with each entry's mode, uid,
/// gid and mtime. A link is stored as a link, never followed; devices, FIFOs and sockets are
/// left out. If the walk fails part way, an Error frame takes the place of the End frame.
pub fn tar<W: Write>(
    root: &Root,
    path: &RelPath,
    raw: Option<&RawName>,
    out: &mut W,
) -> Result<(), FrameError> {
    let start = root.open_any(path, raw).and_then(|(fd, stat)| {
        if kind_of(&stat) == EntryKind::Dir {
            Ok((fd, stat))
        } else {
            Err(os_error(Errno::NOTDIR))
        }
    });
    let (fd, stat) = match start {
        Ok(start) => start,
        Err(e) => return answer(out, Err(e)),
    };
    write_json(out, FrameKind::Reply, &Reply::Tar)?;

    let prefix = match raw {
        Some(raw) => Some(raw.as_bytes().to_vec()),
        None => path.file_name().map(|name| name.as_bytes().to_vec()),
    };
    let mut builder = Builder::new(DataOut::new(out));
    let walked = tar_tree(&mut builder, fd, &stat, prefix);
    let walked = walked.and_then(|()| builder.finish().map_err(Fail::Io));
    if walked.is_err() {
        // Nothing more reaches the stream, the archive's trailer included.
        builder.get_mut().poisoned = true;
    }
    let mut data = builder.into_inner().unwrap_or_else(|_| unreachable!());
    if let Some(broken) = data.broken.take() {
        return Err(broken);
    }
    match walked {
        Ok(()) => {
            data.flush_frames()?;
            write_frame(out, FrameKind::End, &[])
        }
        Err(Fail::Helper(e)) => answer(out, Err(e)),
        Err(Fail::Io(e)) => answer(out, Err(io_error(&e))),
    }
}

/// Why a tar stopped.
enum Fail {
    Helper(HelperError),
    /// From the tar writer: a file that could not be read, or the output (see
    /// [`DataOut::broken`]).
    Io(io::Error),
}

impl From<HelperError> for Fail {
    fn from(e: HelperError) -> Self {
        Self::Helper(e)
    }
}

/// Depth first, each folder's entries in name order.
fn tar_tree<W: Write>(
    builder: &mut Builder<W>,
    top: OwnedFd,
    top_stat: &Statx,
    prefix: Option<Vec<u8>>,
) -> Result<(), Fail> {
    if let Some(prefix) = &prefix {
        append_dir(builder, top_stat, prefix)?;
    }
    let mut stack = vec![(top, prefix.unwrap_or_default())];
    while let Some((dir, base)) = stack.pop() {
        let mut subdirs = Vec::new();
        for name in names(&dir)? {
            let stat = match statx_at(&dir, &name) {
                Ok(stat) => stat,
                Err(HelperError::NotFound) => continue,
                Err(e) => return Err(e.into()),
            };
            let path = if base.is_empty() {
                name.clone()
            } else {
                [base.as_slice(), b"/", name.as_slice()].concat()
            };
            match kind_of(&stat) {
                EntryKind::File => append_file(builder, &dir, &name, &stat, &path)?,
                EntryKind::Dir => {
                    let sub = match open_subdir(&dir, &name, &stat) {
                        Ok(sub) => sub,
                        Err(HelperError::NotFound) => continue,
                        Err(e) => return Err(e.into()),
                    };
                    append_dir(builder, &stat, &path)?;
                    subdirs.push((sub, path));
                }
                EntryKind::Link => {
                    let target = rustix::fs::readlinkat(&dir, name.as_slice(), Vec::new())
                        .map_err(os_error)?;
                    let mut header = header(&stat, EntryType::Symlink, 0);
                    builder
                        .append_link(&mut header, as_path(&path), as_path(target.as_bytes()))
                        .map_err(Fail::Io)?;
                }
                EntryKind::Special => {}
            }
        }
        stack.extend(subdirs.into_iter().rev());
    }
    Ok(())
}

fn append_dir<W: Write>(builder: &mut Builder<W>, stat: &Statx, path: &[u8]) -> Result<(), Fail> {
    let mut header = header(stat, EntryType::Directory, 0);
    builder
        .append_data(&mut header, as_path(path), io::empty())
        .map_err(Fail::Io)
}

fn append_file<W: Write>(
    builder: &mut Builder<W>,
    dir: &OwnedFd,
    name: &[u8],
    stat: &Statx,
    path: &[u8],
) -> Result<(), Fail> {
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::NOCTTY;
    let file = match open_in(dir, name, flags) {
        Ok(fd) => File::from(fd),
        Err(HelperError::NotFound) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let opened = statx_fd(&file)?;
    if opened.stx_ino != stat.stx_ino || kind_of(&opened) != EntryKind::File {
        return Err(changed_while_reading().into());
    }
    let size = opened.stx_size;
    let mut header = header(&opened, EntryType::Regular, size);
    let content = Exact {
        file: (&file).take(size),
        left: size,
    };
    builder
        .append_data(&mut header, as_path(path), content)
        .map_err(Fail::Io)
}

fn header(stat: &Statx, kind: EntryType, size: u64) -> Header {
    let owner = owner_of(stat);
    let mut header = Header::new_gnu();
    header.set_entry_type(kind);
    header.set_size(size);
    header.set_mode(owner.mode);
    header.set_uid(u64::from(owner.uid));
    header.set_gid(u64::from(owner.gid));
    header.set_mtime(u64::try_from(stat.stx_mtime.tv_sec).unwrap_or(0));
    header
}

fn as_path(bytes: &[u8]) -> &Path {
    Path::new(OsStr::from_bytes(bytes))
}

/// Exactly the size in the header: a file that shrank while it was read fails the tar instead of
/// leaving a corrupt archive.
struct Exact<R> {
    file: R,
    left: u64,
}

impl<R: Read> Read for Exact<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.file.read(buf)?;
        if n == 0 && self.left > 0 && !buf.is_empty() {
            return Err(io::Error::from_raw_os_error(Errno::AGAIN.raw_os_error()));
        }
        self.left -= n as u64;
        Ok(n)
    }
}

/// Buffers what the tar writer produces into Data frames of [`CHUNK`] bytes.
struct DataOut<'a, W> {
    out: &'a mut W,
    buf: Vec<u8>,
    /// The output failed: a protocol-level failure, the helper exits.
    broken: Option<FrameError>,
    /// Set after a failure: everything written after it is dropped.
    poisoned: bool,
}

impl<'a, W: Write> DataOut<'a, W> {
    fn new(out: &'a mut W) -> Self {
        Self {
            out,
            buf: Vec::with_capacity(CHUNK),
            broken: None,
            poisoned: false,
        }
    }

    /// Sends what is buffered.
    fn flush_frames(&mut self) -> Result<(), FrameError> {
        if !self.buf.is_empty() {
            write_frame(self.out, FrameKind::Data, &self.buf)?;
            self.buf.clear();
        }
        Ok(())
    }
}

impl<W: Write> Write for DataOut<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.poisoned {
            return Ok(bytes.len());
        }
        if self.broken.is_some() {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        let n = bytes.len().min(CHUNK - self.buf.len());
        self.buf.extend_from_slice(&bytes[..n]);
        if self.buf.len() == CHUNK
            && let Err(e) = self.flush_frames()
        {
            self.broken = Some(e);
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
