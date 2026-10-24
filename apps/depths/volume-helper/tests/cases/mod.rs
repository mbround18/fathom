//! The escape case table (SC-003), shared by the helper's escape suite and the server's
//! `FakeVolumeHelper` tests (T030), which include this file with
//! `#[path = "../../volume-helper/tests/cases/mod.rs"] mod cases;`.
//!
//! [`build_tree`] lays out a mount root beside an `outside` folder whose files are sentinels:
//! whatever a case does, their content, mtime and inode must not change ([`snapshot`]).
//! Needs only `depths-volumes`, `rustix` (fs) and std.

#![allow(dead_code)]

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::{Path, PathBuf};

use depths_volumes::mount::WriteBlocked;
use depths_volumes::op::Op;
use depths_volumes::path::{PathError, RawName, RelPath};
use depths_volumes::text::Fingerprint;

/// Every sentinel's content starts with this; nothing a case reads may contain it.
pub const SENTINEL_MARK: &[u8] = b"SENTINEL";

/// What a case does to its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// `Stat`: the entry itself, a final link not followed.
    Stat,
    /// `Read`: the file's bytes, links inside followed.
    Read,
    /// `Write`: replace an existing file, links inside followed.
    Write,
}

/// How a case must end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// Acts on an object inside the root.
    Inside,
    /// `Stat` of a link that leaves the mount: listed with `inside: false` and
    /// `write_blocked: outside_link`, never followed.
    OutsideLink,
    /// Refused `outside_mount`.
    OutsideMount,
    /// Refused `write_blocked` with this reason.
    Blocked(WriteBlocked),
    /// Refused `not_found`.
    NotFound,
    /// The path is not a valid `RelPath`/`RawName`; it never reaches the helper.
    InvalidPath,
}

impl Expect {
    /// True when the case is refused (the server audits one `files.refused` entry for it).
    pub fn is_refused(self) -> bool {
        matches!(
            self,
            Self::OutsideMount | Self::Blocked(_) | Self::InvalidPath
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Case {
    pub name: &'static str,
    pub access: Access,
    /// The path as a client sends it, unvalidated.
    pub path: &'static str,
    /// When set, the final component's raw bytes, `path` being its folder.
    pub raw_name: Option<&'static [u8]>,
    pub expect: Expect,
}

impl Case {
    /// The validated path, or why it is refused before any op is built.
    pub fn rel(&self) -> Result<(RelPath, Option<RawName>), PathError> {
        let path = RelPath::parse(self.path)?;
        let raw = self.raw_name.map(RawName::from_bytes).transpose()?;
        Ok((path, raw))
    }

    /// The helper op for this case; `None` when the path is invalid.
    pub fn op(&self) -> Option<Op> {
        let (path, raw_name) = self.rel().ok()?;
        Some(match self.access {
            Access::Stat => Op::Stat {
                path,
                raw_name,
                hash: false,
            },
            Access::Read => Op::Read {
                path,
                raw_name,
                max: 1024 * 1024,
            },
            // Refusals come before the base check, so any base will do.
            Access::Write => Op::Write {
                path,
                base: Fingerprint::of_content(b"", 0, 0),
                mode: None,
            },
        })
    }
}

const fn case(name: &'static str, access: Access, path: &'static str, expect: Expect) -> Case {
    Case {
        name,
        access,
        path,
        raw_name: None,
        expect,
    }
}

/// The table. Paths refer to [`build_tree`]'s layout.
pub fn cases() -> Vec<Case> {
    use Access::{Read, Stat, Write};
    use Expect::*;
    vec![
        // `..` in many positions and absolute paths: refused by the path rule.
        case("dotdot alone", Read, "..", InvalidPath),
        case(
            "dotdot to the sentinel",
            Read,
            "../outside/secret.txt",
            InvalidPath,
        ),
        case(
            "dotdot after a folder",
            Read,
            "sub/../../outside/secret.txt",
            InvalidPath,
        ),
        case("dotdot at the end", Stat, "sub/..", InvalidPath),
        case(
            "dotdot in the middle",
            Write,
            "sub/deep/../../a.txt",
            InvalidPath,
        ),
        case(
            "dotdot through a link",
            Read,
            "link_in_dir/../../outside/secret.txt",
            InvalidPath,
        ),
        case("absolute path", Read, "/etc/passwd", InvalidPath),
        case(
            "absolute path to the sentinel",
            Write,
            "/outside/secret.txt",
            InvalidPath,
        ),
        Case {
            name: "raw name dotdot",
            access: Read,
            path: "sub",
            raw_name: Some(b".."),
            expect: InvalidPath,
        },
        Case {
            name: "raw name with a slash",
            access: Read,
            path: "",
            raw_name: Some(b"../outside/secret.txt"),
            expect: InvalidPath,
        },
        // Plain access inside.
        case("stat of the root", Stat, "", Inside),
        case("read a file", Read, "a.txt", Inside),
        case("write a file", Write, "a.txt", Inside),
        case("read a nested file", Read, "sub/deep/c.txt", Inside),
        // Relative links that stay inside are followed.
        case("read a link inside", Read, "link_in", Inside),
        case("write through a link inside", Write, "link_in", Inside),
        case(
            "read through a folder link inside",
            Read,
            "link_in_dir/b.txt",
            Inside,
        ),
        case(
            "read a link with dotdot that stays inside",
            Read,
            "link_up_in",
            Inside,
        ),
        case(
            "read a link to the parent folder",
            Read,
            "sub/link_parent",
            Inside,
        ),
        case(
            "write a link to the parent folder",
            Write,
            "sub/link_parent",
            Inside,
        ),
        case("read a chain inside", Read, "chain1", Inside),
        case("write a chain inside", Write, "chain1", Inside),
        // Relative links out.
        case("stat a link out", Stat, "link_out", OutsideLink),
        case("read a link out", Read, "link_out", OutsideMount),
        case("write a link out", Write, "link_out", OutsideMount),
        case("read a deep link out", Read, "link_out_deep", OutsideMount),
        case(
            "write a deep link out",
            Write,
            "sub/deep/link_out_far",
            OutsideMount,
        ),
        case("stat a folder link out", Stat, "link_out_dir", OutsideLink),
        case(
            "stat through a folder link out",
            Stat,
            "link_out_dir/secret.txt",
            OutsideMount,
        ),
        case(
            "read through a folder link out",
            Read,
            "link_out_dir/secret.txt",
            OutsideMount,
        ),
        case(
            "write through a folder link out",
            Write,
            "link_out_dir/secret.txt",
            OutsideMount,
        ),
        case("read a chain out", Read, "chain_out1", OutsideMount),
        case("write a chain out", Write, "chain_out1", OutsideMount),
        case("read a link through /proc", Read, "link_proc", OutsideMount),
        // Absolute links are never followed, even into the mount.
        case("stat an absolute link out", Stat, "abs_out", OutsideLink),
        case("read an absolute link out", Read, "abs_out", OutsideMount),
        case("write an absolute link out", Write, "abs_out", OutsideMount),
        case("stat an absolute link inside", Stat, "abs_in", OutsideLink),
        case("read an absolute link inside", Read, "abs_in", OutsideMount),
        case(
            "write an absolute link inside",
            Write,
            "abs_in",
            OutsideMount,
        ),
        case(
            "read an absolute link to /etc",
            Read,
            "abs_etc",
            OutsideMount,
        ),
        case(
            "read through an absolute link to /proc",
            Read,
            "abs_proc_root/etc/hostname",
            OutsideMount,
        ),
        // Loops.
        case("read a link loop", Read, "loop1", OutsideMount),
        case("write a link loop", Write, "loop1", OutsideMount),
        // Hard links: readable, never written.
        case("read a hard link to outside", Read, "hard_out", Inside),
        case(
            "write a hard link to outside",
            Write,
            "hard_out",
            Blocked(WriteBlocked::HardLink),
        ),
        case(
            "write a hard link inside",
            Write,
            "hard_in",
            Blocked(WriteBlocked::HardLink),
        ),
        case(
            "write through a link to a hard link",
            Write,
            "link_hard",
            Blocked(WriteBlocked::HardLink),
        ),
        // Special files: never opened.
        case("read a fifo", Read, "fifo", Blocked(WriteBlocked::Special)),
        case(
            "write a fifo",
            Write,
            "fifo",
            Blocked(WriteBlocked::Special),
        ),
        case(
            "read a socket",
            Read,
            "sock",
            Blocked(WriteBlocked::Special),
        ),
        case(
            "write a socket",
            Write,
            "sock",
            Blocked(WriteBlocked::Special),
        ),
        case(
            "read through a link to a fifo",
            Read,
            "link_fifo",
            Blocked(WriteBlocked::Special),
        ),
        case("stat a fifo", Stat, "fifo", Inside),
        // Names that are not UTF-8, addressed by their bytes.
        Case {
            name: "read a non-UTF-8 name",
            access: Read,
            path: "sub",
            raw_name: Some(b"bad\xff.txt"),
            expect: Inside,
        },
        Case {
            name: "stat a non-UTF-8 name",
            access: Stat,
            path: "sub",
            raw_name: Some(b"bad\xff.txt"),
            expect: Inside,
        },
        Case {
            name: "read a non-UTF-8 link out",
            access: Read,
            path: "",
            raw_name: Some(b"out\xfe"),
            expect: OutsideMount,
        },
        // Missing.
        case("read a missing file", Read, "missing.txt", NotFound),
        case("write a missing file", Write, "sub/missing.txt", NotFound),
    ]
}

/// The tree a case runs against.
pub struct Tree {
    /// The mount root.
    pub root: PathBuf,
    /// Files outside the root that no case may change.
    pub sentinels: Vec<PathBuf>,
}

/// Lays out, under `base` (an empty temp dir):
///
/// ```text
/// outside/secret.txt  outside/dir/x  outside/hard_target.txt     sentinels
/// root/a.txt  root/sub/b.txt  root/sub/deep/c.txt  root/sub/bad\xff.txt
/// root/link_in -> sub/b.txt           root/link_in_dir -> sub
/// root/link_up_in -> sub/../a.txt     root/sub/link_parent -> ../a.txt
/// root/chain1 -> chain2 -> chain3 -> a.txt
/// root/link_out -> ../outside/secret.txt   root/link_out_dir -> ../outside
/// root/link_out_deep -> sub/../../outside/secret.txt
/// root/sub/deep/link_out_far -> ../../../outside/dir/x
/// root/chain_out1 -> chain_out2 -> ../outside/secret.txt
/// root/link_proc -> ../../../../../../../../../../proc/self/root/etc/hostname
/// root/abs_out -> <base>/outside/secret.txt   root/abs_in -> <root>/a.txt
/// root/abs_etc -> /etc/passwd   root/abs_proc_root -> /proc/self/root
/// root/out\xfe -> ../outside/secret.txt
/// root/loop1 -> loop2 -> loop1
/// root/hard_out (hard link of outside/hard_target.txt)   root/hard_in (of sub/deep/c.txt)
/// root/link_hard -> hard_in   root/fifo   root/sock   root/link_fifo -> fifo
/// ```
pub fn build_tree(base: &Path) -> Tree {
    let outside = base.join("outside");
    let root = base.join("root");
    std::fs::create_dir_all(outside.join("dir")).unwrap();
    std::fs::create_dir_all(root.join("sub/deep")).unwrap();

    let secret = outside.join("secret.txt");
    let x = outside.join("dir/x");
    let hard_target = outside.join("hard_target.txt");
    std::fs::write(&secret, b"SENTINEL secret").unwrap();
    std::fs::write(&x, b"SENTINEL x").unwrap();
    std::fs::write(&hard_target, b"shared inode").unwrap();

    std::fs::write(root.join("a.txt"), b"hello a").unwrap();
    std::fs::write(root.join("sub/b.txt"), b"hello b").unwrap();
    std::fs::write(root.join("sub/deep/c.txt"), b"hello c").unwrap();
    std::fs::write(
        root.join("sub").join(OsStr::from_bytes(b"bad\xff.txt")),
        b"hello bad",
    )
    .unwrap();

    let links: &[(&str, &str)] = &[
        ("link_in", "sub/b.txt"),
        ("link_in_dir", "sub"),
        ("link_up_in", "sub/../a.txt"),
        ("sub/link_parent", "../a.txt"),
        ("chain1", "chain2"),
        ("chain2", "chain3"),
        ("chain3", "a.txt"),
        ("link_out", "../outside/secret.txt"),
        ("link_out_dir", "../outside"),
        ("link_out_deep", "sub/../../outside/secret.txt"),
        ("sub/deep/link_out_far", "../../../outside/dir/x"),
        ("chain_out1", "chain_out2"),
        ("chain_out2", "../outside/secret.txt"),
        (
            "link_proc",
            "../../../../../../../../../../proc/self/root/etc/hostname",
        ),
        ("abs_etc", "/etc/passwd"),
        ("abs_proc_root", "/proc/self/root"),
        ("loop1", "loop2"),
        ("loop2", "loop1"),
        ("link_hard", "hard_in"),
        ("link_fifo", "fifo"),
    ];
    for (name, target) in links {
        symlink(target, root.join(name)).unwrap();
    }
    symlink(&secret, root.join("abs_out")).unwrap();
    symlink(root.join("a.txt"), root.join("abs_in")).unwrap();
    symlink(
        "../outside/secret.txt",
        root.join(OsStr::from_bytes(b"out\xfe")),
    )
    .unwrap();

    std::fs::hard_link(&hard_target, root.join("hard_out")).unwrap();
    std::fs::hard_link(root.join("sub/deep/c.txt"), root.join("hard_in")).unwrap();

    rustix::fs::mknodat(
        rustix::fs::CWD,
        root.join("fifo").as_path(),
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o644),
        0,
    )
    .unwrap();
    // Bound and dropped: the socket file stays.
    drop(std::os::unix::net::UnixListener::bind(root.join("sock")).unwrap());

    Tree {
        root,
        sentinels: vec![secret, x, hard_target],
    }
}

/// Content, mtime (ns) and inode of each path.
pub fn snapshot(paths: &[PathBuf]) -> Vec<(Vec<u8>, i64, i64, u64)> {
    paths
        .iter()
        .map(|p| {
            let meta = std::fs::symlink_metadata(p).unwrap();
            (
                std::fs::read(p).unwrap(),
                meta.mtime(),
                meta.mtime_nsec(),
                meta.ino(),
            )
        })
        .collect()
}
