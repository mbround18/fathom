//! What the server asks the volume helper to do, and what it answers (contracts/helper.md).
//!
//! One op at a time per helper: the server sends an `Op` frame (plus Data… End for `Write`,
//! `Put` and `Untar`), and the helper answers with one `Reply` or one `Error` frame (plus
//! Data… End where a reply says so). The helper runs as the uid:gid of the object it acts on
//! (R5) and re-checks that owner itself, so no op carries one.
//!
//! An op that may address a name that is not UTF-8 on disk (`Stat`, `Read`, `Delete`, `Size`,
//! `Tar`) takes `raw_name`: then `path` is the parent folder and `raw_name` the final component.

use serde::{Deserialize, Serialize};

use crate::frame::{Frame, FrameError, FrameKind};
use crate::mount::{Entry, EntryKind, LinkInfo, OpMode, Owner, WriteBlocked};
use crate::path::{Name, RawName, RelPath};
use crate::text::Fingerprint;

/// The helper protocol version, sent in [`Hello`].
pub const PROTO: u32 = 1;

/// The helper's first frame. `openat2: false` makes the server report `helper_unavailable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub proto: u32,
    pub openat2: bool,
    /// The uid:gid the helper runs as.
    pub uid: u32,
    pub gid: u32,
}

/// What `Create` makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreateKind {
    File,
    Dir,
}

/// Where a bounded walk stops: whichever limit it reaches first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizeCap {
    pub bytes: u64,
    pub entries: u64,
}

/// One request to the helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// → [`Reply::Stat`]. With `hash`, a regular file's fingerprint is computed too (one file,
    /// for an upload preview).
    Stat {
        path: RelPath,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw_name: Option<RawName>,
        #[serde(default)]
        hash: bool,
    },
    /// → [`Reply::Page`]: page `page` (from 0) of the sorted listing.
    List { path: RelPath, page: u32 },
    /// → [`Reply::Content`], then the bytes as Data… End when the file is at most `max` bytes.
    Read {
        path: RelPath,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw_name: Option<RawName>,
        max: u64,
    },
    /// Followed by the new content as Data… End. Atomic replace (R6) if the file still matches
    /// `base`; the old mode is kept unless `mode` is given. → [`Reply::Written`].
    Write {
        path: RelPath,
        base: Fingerprint,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<u32>,
    },
    /// An empty file or folder, owned by the helper's uid:gid. → [`Reply::Created`].
    Create {
        parent: RelPath,
        name: Name,
        kind: CreateKind,
    },
    /// Within the mount; refused when `to` exists. → [`Reply::Renamed`].
    Rename { from: RelPath, to: RelPath },
    /// A folder needs `recursive`; links are removed, never followed. → [`Reply::Deleted`].
    Delete {
        path: RelPath,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw_name: Option<RawName>,
        recursive: bool,
    },
    /// A bounded walk. → [`Reply::Size`].
    Size {
        path: RelPath,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw_name: Option<RawName>,
        cap: SizeCap,
    },
    /// → [`Reply::Tar`], then an uncompressed tar of the folder as Data… End.
    Tar {
        path: RelPath,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw_name: Option<RawName>,
    },
    /// Declared for a later version; the helper answers [`HelperError::Unsupported`].
    Untar { dir: RelPath },
    /// Followed by the file's bytes as Data… End. Refused `exists` when `name` exists and no
    /// `replace` is given, `changed_on_disk` when it does not match `replace`. →
    /// [`Reply::Written`].
    Put {
        dir: RelPath,
        name: Name,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        replace: Option<Fingerprint>,
    },
}

impl Op {
    /// Read-only unless the op writes; the server mounts the source to match.
    pub fn mode(&self) -> OpMode {
        match self {
            Self::Stat { .. }
            | Self::List { .. }
            | Self::Read { .. }
            | Self::Size { .. }
            | Self::Tar { .. } => OpMode::ReadOnly,
            Self::Write { .. }
            | Self::Create { .. }
            | Self::Rename { .. }
            | Self::Delete { .. }
            | Self::Untar { .. }
            | Self::Put { .. } => OpMode::ReadWrite,
        }
    }

    /// True when Data… End frames follow the op.
    pub fn sends_data(&self) -> bool {
        matches!(
            self,
            Self::Write { .. } | Self::Put { .. } | Self::Untar { .. }
        )
    }
}

/// `Stat`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatInfo {
    pub kind: EntryKind,
    pub size: u64,
    pub mtime_ns: i64,
    pub ino: u64,
    pub nlink: u32,
    pub owner: Owner,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<LinkInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_blocked: Option<WriteBlocked>,
    /// Present when the op asked for `hash` and this is a regular file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<Fingerprint>,
}

/// One successful answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum Reply {
    Stat(StatInfo),
    Page {
        entries: Vec<Entry>,
        page: u32,
        last: bool,
    },
    /// `Read`'s header. When `data`, the bytes follow as Data… End; otherwise the file is over
    /// the op's `max` and nothing follows. The size is `fingerprint.size()`.
    Content {
        fingerprint: Fingerprint,
        owner: Owner,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        write_blocked: Option<WriteBlocked>,
        data: bool,
    },
    /// `Write` and `Put`. When `prior`, the replaced content follows as Data… End, for history
    /// (R9).
    Written {
        fingerprint: Fingerprint,
        prior: bool,
    },
    Created,
    Renamed,
    /// `removed` entries. When `prior` (a single file was deleted), its content follows as
    /// Data… End, for history.
    Deleted {
        removed: u64,
        prior: bool,
    },
    /// `capped` when the walk stopped at the cap; the counts are then lower bounds.
    Size {
        bytes: u64,
        entries: u64,
        capped: bool,
    },
    /// The tar follows as Data… End.
    Tar,
}

/// One refused or failed op. `errno` is the symbolic name (`EACCES`) and `message` the system's
/// text, both shown verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HelperError {
    /// A link or `..` would leave the mount (`EXDEV`/`ELOOP` from `openat2`).
    #[error("the path leads outside the mount")]
    OutsideMount,
    /// `reason` is `hard_link`, `special` or `outside_link`.
    #[error("writes to this entry are off ({reason:?})")]
    WriteBlocked { reason: WriteBlocked },
    #[error("the file changed on disk")]
    ChangedOnDisk { current: Fingerprint },
    #[error("the owner changed since it was listed")]
    OwnerChanged,
    #[error("too large")]
    TooLarge,
    #[error("not a text file")]
    NotText,
    #[error("the name already exists")]
    Exists,
    #[error("not found")]
    NotFound,
    #[error("invalid name")]
    InvalidName,
    /// A rename between folders with different owners.
    #[error("the source and destination folders have different owners")]
    MixedOwners,
    /// A recursive delete stopped at `failed_at` (lossy) after removing `removed` entries.
    #[error("removed {removed} entries, then {failed_at}: {message}")]
    Partial {
        removed: u64,
        failed_at: String,
        errno: String,
        message: String,
    },
    #[error("{errno}: {message}")]
    OsError { errno: String, message: String },
    /// An op this helper does not implement (`Untar`).
    #[error("not supported by this helper")]
    Unsupported,
    /// The frames did not follow the protocol; the helper exits 70 after sending it.
    #[error("protocol error: {message}")]
    Protocol { message: String },
}

impl Hello {
    pub fn frame(&self) -> Result<Frame, FrameError> {
        Frame::json(FrameKind::Hello, self)
    }
}

impl Op {
    pub fn frame(&self) -> Result<Frame, FrameError> {
        Frame::json(FrameKind::Op, self)
    }
}

impl Reply {
    pub fn frame(&self) -> Result<Frame, FrameError> {
        Frame::json(FrameKind::Reply, self)
    }
}

impl HelperError {
    pub fn frame(&self) -> Result<Frame, FrameError> {
        Frame::json(FrameKind::Error, self)
    }
}

/// The helper's answer to an op: a Reply frame, or an Error frame. Any other kind is a protocol
/// error.
pub fn parse_answer(frame: &Frame) -> Result<Result<Reply, HelperError>, FrameError> {
    match frame.kind {
        FrameKind::Error => Ok(Err(frame.parse(FrameKind::Error)?)),
        _ => Ok(Ok(frame.parse(FrameKind::Reply)?)),
    }
}

#[cfg(test)]
mod tests {
    use serde::de::DeserializeOwned;
    use serde_json::json;

    use super::*;
    use crate::frame::{read_frame, write_frame};

    fn rp(s: &str) -> RelPath {
        RelPath::parse(s).unwrap()
    }

    fn fp() -> Fingerprint {
        Fingerprint::of_content(b"x", 7, 9)
    }

    fn raw() -> Option<RawName> {
        Some(RawName::from_bytes(b"\xff".to_vec()).unwrap())
    }

    fn round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: &T) {
        let json = serde_json::to_string(value).unwrap();
        assert_eq!(&serde_json::from_str::<T>(&json).unwrap(), value, "{json}");
    }

    fn all_ops() -> Vec<Op> {
        vec![
            Op::Stat {
                path: rp("a"),
                raw_name: None,
                hash: false,
            },
            Op::Stat {
                path: rp(""),
                raw_name: raw(),
                hash: true,
            },
            Op::List {
                path: rp("a/b"),
                page: 2,
            },
            Op::Read {
                path: rp("a.txt"),
                raw_name: None,
                max: 2 << 20,
            },
            Op::Read {
                path: rp("d"),
                raw_name: raw(),
                max: 1,
            },
            Op::Write {
                path: rp("a.txt"),
                base: fp(),
                mode: None,
            },
            Op::Write {
                path: rp("a.txt"),
                base: fp(),
                mode: Some(0o640),
            },
            Op::Create {
                parent: rp("d"),
                name: Name::new("n").unwrap(),
                kind: CreateKind::File,
            },
            Op::Create {
                parent: rp(""),
                name: Name::new("d").unwrap(),
                kind: CreateKind::Dir,
            },
            Op::Rename {
                from: rp("a"),
                to: rp("b/a"),
            },
            Op::Delete {
                path: rp("a"),
                raw_name: None,
                recursive: false,
            },
            Op::Delete {
                path: rp("d"),
                raw_name: raw(),
                recursive: true,
            },
            Op::Size {
                path: rp("d"),
                raw_name: None,
                cap: SizeCap {
                    bytes: 1 << 31,
                    entries: 10_000,
                },
            },
            Op::Tar {
                path: rp("d"),
                raw_name: None,
            },
            Op::Tar {
                path: rp(""),
                raw_name: raw(),
            },
            Op::Untar { dir: rp("d") },
            Op::Put {
                dir: rp("d"),
                name: Name::new("up.bin").unwrap(),
                replace: None,
            },
            Op::Put {
                dir: rp("d"),
                name: Name::new("up.bin").unwrap(),
                replace: Some(fp()),
            },
        ]
    }

    fn all_replies() -> Vec<Reply> {
        let owner = Owner {
            uid: 1000,
            gid: 100,
            mode: 0o644,
        };
        vec![
            Reply::Stat(StatInfo {
                kind: EntryKind::File,
                size: 1,
                mtime_ns: -1,
                ino: 9,
                nlink: 2,
                owner,
                link: None,
                write_blocked: Some(WriteBlocked::HardLink),
                fingerprint: Some(fp()),
            }),
            Reply::Stat(StatInfo {
                kind: EntryKind::Link,
                size: 0,
                mtime_ns: 0,
                ino: 1,
                nlink: 1,
                owner,
                link: Some(LinkInfo {
                    target: "/etc".into(),
                    inside: false,
                }),
                write_blocked: Some(WriteBlocked::OutsideLink),
                fingerprint: None,
            }),
            Reply::Page {
                entries: vec![Entry {
                    name: "a".into(),
                    name_b64: None,
                    kind: EntryKind::Dir,
                    link: None,
                    size: 4096,
                    mtime: "2026-10-24T00:00:00Z".into(),
                    uid: 0,
                    gid: 0,
                    mode: 0o755,
                    nlink: 2,
                    write_blocked: None,
                }],
                page: 0,
                last: true,
            },
            Reply::Content {
                fingerprint: fp(),
                owner,
                write_blocked: None,
                data: true,
            },
            Reply::Content {
                fingerprint: fp(),
                owner,
                write_blocked: Some(WriteBlocked::Special),
                data: false,
            },
            Reply::Written {
                fingerprint: fp(),
                prior: true,
            },
            Reply::Written {
                fingerprint: fp(),
                prior: false,
            },
            Reply::Created,
            Reply::Renamed,
            Reply::Deleted {
                removed: 3,
                prior: false,
            },
            Reply::Size {
                bytes: 10,
                entries: 2,
                capped: true,
            },
            Reply::Tar,
        ]
    }

    fn all_errors() -> Vec<HelperError> {
        vec![
            HelperError::OutsideMount,
            HelperError::WriteBlocked {
                reason: WriteBlocked::HardLink,
            },
            HelperError::WriteBlocked {
                reason: WriteBlocked::Special,
            },
            HelperError::WriteBlocked {
                reason: WriteBlocked::OutsideLink,
            },
            HelperError::ChangedOnDisk { current: fp() },
            HelperError::OwnerChanged,
            HelperError::TooLarge,
            HelperError::NotText,
            HelperError::Exists,
            HelperError::NotFound,
            HelperError::InvalidName,
            HelperError::MixedOwners,
            HelperError::Partial {
                removed: 4,
                failed_at: "d/locked".into(),
                errno: "EACCES".into(),
                message: "Permission denied".into(),
            },
            HelperError::OsError {
                errno: "ENOSPC".into(),
                message: "No space left on device".into(),
            },
            HelperError::Unsupported,
            HelperError::Protocol {
                message: "expected an Op frame".into(),
            },
        ]
    }

    #[test]
    fn every_variant_round_trips() {
        round_trip(&Hello {
            proto: PROTO,
            openat2: true,
            uid: 0,
            gid: 0,
        });
        all_ops().iter().for_each(round_trip);
        all_replies().iter().for_each(round_trip);
        all_errors().iter().for_each(round_trip);
    }

    #[test]
    fn every_variant_round_trips_through_frames() {
        let mut buf = Vec::new();
        for op in all_ops() {
            op.frame().unwrap().write_to(&mut buf).unwrap();
        }
        for reply in all_replies() {
            reply.frame().unwrap().write_to(&mut buf).unwrap();
        }
        for err in all_errors() {
            err.frame().unwrap().write_to(&mut buf).unwrap();
        }
        let mut r = buf.as_slice();
        for op in all_ops() {
            assert_eq!(
                read_frame(&mut r)
                    .unwrap()
                    .unwrap()
                    .parse::<Op>(FrameKind::Op)
                    .unwrap(),
                op
            );
        }
        for reply in all_replies() {
            assert_eq!(
                parse_answer(&read_frame(&mut r).unwrap().unwrap()).unwrap(),
                Ok(reply)
            );
        }
        for err in all_errors() {
            assert_eq!(
                parse_answer(&read_frame(&mut r).unwrap().unwrap()).unwrap(),
                Err(err)
            );
        }
    }

    #[test]
    fn wire_shapes() {
        assert_eq!(
            serde_json::to_value(Hello {
                proto: 1,
                openat2: false,
                uid: 1000,
                gid: 1000
            })
            .unwrap(),
            json!({"proto": 1, "openat2": false, "uid": 1000, "gid": 1000})
        );
        assert_eq!(
            serde_json::to_value(Op::List {
                path: rp("a"),
                page: 0
            })
            .unwrap(),
            json!({"op": "list", "path": "a", "page": 0})
        );
        assert_eq!(
            serde_json::to_value(Op::Delete {
                path: rp(""),
                raw_name: raw(),
                recursive: false
            })
            .unwrap(),
            json!({"op": "delete", "path": "", "raw_name": "/w==", "recursive": false})
        );
        assert_eq!(
            serde_json::to_value(Reply::Written {
                fingerprint: fp(),
                prior: true
            })
            .unwrap(),
            json!({"reply": "written", "fingerprint": fp().to_string(), "prior": true})
        );
        assert_eq!(
            serde_json::to_value(HelperError::WriteBlocked {
                reason: WriteBlocked::HardLink
            })
            .unwrap(),
            json!({"kind": "write_blocked", "reason": "hard_link"})
        );
        assert_eq!(
            serde_json::to_value(HelperError::OutsideMount).unwrap(),
            json!({"kind": "outside_mount"})
        );
        assert_eq!(
            serde_json::to_value(HelperError::OsError {
                errno: "EIO".into(),
                message: "x".into()
            })
            .unwrap(),
            json!({"kind": "os_error", "errno": "EIO", "message": "x"})
        );
    }

    #[test]
    fn ops_validate_their_paths_on_the_way_in() {
        assert!(
            serde_json::from_value::<Op>(json!({"op": "list", "path": "../x", "page": 0})).is_err()
        );
        assert!(serde_json::from_value::<Op>(json!({"op": "stat", "path": "/etc"})).is_err());
        assert!(
            serde_json::from_value::<Op>(
                json!({"op": "create", "parent": "", "name": "a/b", "kind": "file"})
            )
            .is_err()
        );
        assert!(serde_json::from_value::<Op>(json!({"op": "chmod", "path": ""})).is_err());
        // Optional fields may be left out.
        assert_eq!(
            serde_json::from_value::<Op>(json!({"op": "stat", "path": "a"})).unwrap(),
            Op::Stat {
                path: rp("a"),
                raw_name: None,
                hash: false
            }
        );
    }

    #[test]
    fn modes_and_data() {
        for op in all_ops() {
            let writes = matches!(
                op,
                Op::Write { .. }
                    | Op::Create { .. }
                    | Op::Rename { .. }
                    | Op::Delete { .. }
                    | Op::Untar { .. }
                    | Op::Put { .. }
            );
            assert_eq!(op.mode() == OpMode::ReadWrite, writes, "{op:?}");
        }
        assert!(
            Op::Write {
                path: rp("a"),
                base: fp(),
                mode: None
            }
            .sends_data()
        );
        assert!(
            !Op::Tar {
                path: rp("a"),
                raw_name: None
            }
            .sends_data()
        );
    }

    #[test]
    fn an_answer_must_be_a_reply_or_an_error() {
        let mut buf = Vec::new();
        write_frame(&mut buf, FrameKind::Data, b"{}").unwrap();
        let frame = read_frame(&mut buf.as_slice()).unwrap().unwrap();
        assert!(matches!(
            parse_answer(&frame),
            Err(FrameError::UnexpectedKind { .. })
        ));
    }
}
