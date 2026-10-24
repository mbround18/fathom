//! What a mount is, and what a listing of one holds (data-model.md "Mount", "Entry").

use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

use crate::path::RawName;

// ---------------------------------------------------------------------------
// Entries
// ---------------------------------------------------------------------------

/// Entries per listing page.
pub const PAGE_SIZE: usize = 5000;

/// What an entry is. `special` is a device, FIFO or socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Dir,
    Link,
    Special,
}

/// A symbolic link's target, as read (lossy for non-UTF-8 bytes). `inside: false` means it leaves
/// the mount, or is absolute: it is never followed (R3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct LinkInfo {
    pub target: String,
    pub inside: bool,
}

/// Why writes to an entry are off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WriteBlocked {
    /// More than one hard link: a save by rename would split them.
    HardLink,
    /// No service mounts it read-write.
    ReadOnlyMount,
    /// A device, FIFO or socket.
    Special,
    /// A link that leaves the mount.
    OutsideLink,
}

/// Owner and permission bits, as the editor header shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Owner {
    pub uid: u32,
    pub gid: u32,
    /// Permission bits (`st_mode & 0o7777`).
    pub mode: u32,
}

/// One entry of a folder listing. It carries no content fingerprint: hashing every file would
/// make listings slow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Entry {
    /// Display name, lossy when the on-disk name is not UTF-8.
    pub name: String,
    /// The raw name, present only when it is not valid UTF-8. Such an entry can be downloaded and
    /// deleted (send this back as `raw_name`), not opened in the editor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name_b64: Option<RawName>,
    pub kind: EntryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub link: Option<LinkInfo>,
    /// Bytes; for a link, the target's size when it is inside.
    pub size: u64,
    /// RFC 3339, UTC.
    pub mtime: String,
    pub uid: u32,
    pub gid: u32,
    /// Permission bits.
    pub mode: u32,
    pub nlink: u32,
    pub write_blocked: Option<WriteBlocked>,
}

impl Entry {
    /// The on-disk name bytes: the raw name when there is one, otherwise `name`.
    pub fn name_bytes(&self) -> Cow<'_, [u8]> {
        match &self.name_b64 {
            Some(raw) => Cow::Borrowed(raw.as_bytes()),
            None => Cow::Borrowed(self.name.as_bytes()),
        }
    }

    /// The reason writes are off for an entry of this shape, before the mount's own mode is
    /// considered: special files, links out of the mount and hard-linked files.
    pub fn blocked_by_shape(
        kind: EntryKind,
        nlink: u32,
        link: Option<&LinkInfo>,
    ) -> Option<WriteBlocked> {
        match kind {
            EntryKind::Special => Some(WriteBlocked::Special),
            EntryKind::Link if !link.is_some_and(|l| l.inside) => Some(WriteBlocked::OutsideLink),
            EntryKind::File if nlink > 1 => Some(WriteBlocked::HardLink),
            _ => None,
        }
    }
}

/// Listing order: folders first, then by name in byte order with ASCII case folded; names that
/// fold equal are ordered by their raw bytes, so the order is total.
pub fn entry_order(a: &Entry, b: &Entry) -> Ordering {
    let dir = |e: &Entry| e.kind != EntryKind::Dir;
    dir(a).cmp(&dir(b)).then_with(|| name_order(&a.name_bytes(), &b.name_bytes()))
}

/// Byte order with ASCII case folded, then raw bytes.
pub fn name_order(a: &[u8], b: &[u8]) -> Ordering {
    a.iter()
        .map(u8::to_ascii_lowercase)
        .cmp(b.iter().map(u8::to_ascii_lowercase))
        .then_with(|| a.cmp(b))
}

pub fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(entry_order);
}

/// Number of pages for `total` entries; an empty folder is one empty page.
pub fn page_count(total: usize) -> u32 {
    total.div_ceil(PAGE_SIZE).max(1) as u32
}

/// Page `page` (from 0) of a sorted listing, and whether it is the last one. A page past the end
/// is empty and last.
pub fn page_of<T>(items: &[T], page: u32) -> (&[T], bool) {
    let start = (page as usize).saturating_mul(PAGE_SIZE).min(items.len());
    let end = start.saturating_add(PAGE_SIZE).min(items.len());
    (&items[start..end], end == items.len())
}

/// `secs` since the Unix epoch as RFC 3339 UTC to the second (`1970-01-01T00:00:00Z`). Pure, so
/// the helper needs no time crate.
pub fn rfc3339_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date (H. Hinnant's `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

// ---------------------------------------------------------------------------
// Mounts
// ---------------------------------------------------------------------------

/// A mount's stable, opaque id: the first 16 hex digits of `sha256(kind \0 source)`. The server
/// computes it, so the client names a mount without ever supplying a source.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(try_from = "String", into = "String")]
#[ts(type = "string")]
pub struct MountId(String);

/// A string that is not 16 lower-case hex digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a mount id")]
pub struct InvalidMountId;

impl MountId {
    pub fn compute(kind: MountKind, source: &str) -> Self {
        let mut h = Sha256::new();
        h.update(kind.as_str().as_bytes());
        h.update([0u8]);
        h.update(source.as_bytes());
        let digest = h.finalize();
        Self(digest[..8].iter().map(|b| format!("{b:02x}")).collect())
    }

    /// Accepts exactly 16 lower-case hex digits, as a client sends back what it was given.
    pub fn parse(raw: &str) -> Result<Self, InvalidMountId> {
        (raw.len() == 16 && raw.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
            .then(|| Self(raw.to_owned()))
            .ok_or(InvalidMountId)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for MountId {
    type Error = InvalidMountId;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<MountId> for String {
    fn from(value: MountId) -> Self {
        value.0
    }
}

impl fmt::Display for MountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What backs a mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MountKind {
    Volume,
    Bind,
    Tmpfs,
    /// `npipe`, `image`, `cluster` and anything newer.
    Other,
}

impl MountKind {
    /// From an inspect or compose `Type`. Empty is read as `bind` (the 051 rule: old engines
    /// leave it out for binds).
    pub fn from_inspect(raw: &str) -> Self {
        match raw {
            "volume" => Self::Volume,
            "" | "bind" => Self::Bind,
            "tmpfs" => Self::Tmpfs,
            _ => Self::Other,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Volume => "volume",
            Self::Bind => "bind",
            Self::Tmpfs => "tmpfs",
            Self::Other => "other",
        }
    }
}

/// Whether the mount's source exists. Only `ready` mounts get a helper: `docker run` on a missing
/// named volume would create it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MountState {
    Ready,
    /// A named volume that does not exist before the first deploy.
    NotCreated,
    /// Its source could not be worked out.
    Unresolved,
    /// The volume or host path is gone.
    Missing,
}

/// One service that mounts it, and where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MountService {
    pub service: String,
    pub destination: String,
    pub read_only: bool,
}

/// Why a mount cannot be browsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum NotBrowsableCode {
    Tmpfs,
    Other,
    /// A bind rated root or escape (`/`, `/etc`, the runtime socket, ...).
    RiskyBind,
    /// A bind of a single file or a socket.
    SingleFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NotBrowsable {
    pub code: NotBrowsableCode,
    pub message: String,
}

/// `{ ok: true }` or `{ ok: false, reason }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Browsable {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<NotBrowsable>,
}

impl Browsable {
    pub fn yes() -> Self {
        Self { ok: true, reason: None }
    }

    pub fn no(code: NotBrowsableCode, message: impl Into<String>) -> Self {
        Self { ok: false, reason: Some(NotBrowsable { code, message: message.into() }) }
    }
}

/// The caller's rating of a bind's host path (depths-core's `bind_reason()`), so this crate does
/// not depend on depths-core: `None` when it raised nothing, `Info` for an info-level reason,
/// `Risky` with the reason's message for root or escape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindRating {
    None,
    Info,
    Risky(String),
}

/// Whether a mount may be browsed: a volume always, a bind unless rated risky, tmpfs and other
/// kinds never. A single-file bind is the caller's to refuse (it needs a stat), with
/// [`NotBrowsableCode::SingleFile`].
pub fn browsability(kind: MountKind, rating: &BindRating) -> Browsable {
    match (kind, rating) {
        (MountKind::Volume, _) => Browsable::yes(),
        (MountKind::Bind, BindRating::None | BindRating::Info) => Browsable::yes(),
        (MountKind::Bind, BindRating::Risky(why)) => Browsable::no(NotBrowsableCode::RiskyBind, why),
        (MountKind::Tmpfs, _) => Browsable::no(
            NotBrowsableCode::Tmpfs,
            "a tmpfs mount lives in the container's memory; there is nothing on disk to browse",
        ),
        (MountKind::Other, _) => {
            Browsable::no(NotBrowsableCode::Other, "this kind of mount cannot be browsed")
        }
    }
}

/// True when at least one service mounts it read-write.
pub fn any_writable(services: &[MountService]) -> bool {
    services.iter().any(|s| !s.read_only)
}

/// A mount of a stack, as the UI lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Mount {
    pub id: MountId,
    pub kind: MountKind,
    /// Volume name, or host path for a bind. Shown to editors only.
    pub source: String,
    pub services: Vec<MountService>,
    /// True when any service mounts it read-write.
    pub writable: bool,
    pub state: MountState,
    pub browsable: Browsable,
    /// Services with a running container that mounts it.
    pub running: Vec<String>,
}

impl Mount {
    /// Computes `id` from `kind` and `source`, and `writable` from `services`.
    pub fn new(
        kind: MountKind,
        source: String,
        services: Vec<MountService>,
        state: MountState,
        browsable: Browsable,
        running: Vec<String>,
    ) -> Self {
        Self {
            id: MountId::compute(kind, &source),
            writable: any_writable(&services),
            kind,
            source,
            services,
            state,
            browsable,
            running,
        }
    }
}

/// What the helper container mounts at `/volume`. Server-only: never serialised, never sent to
/// the UI, and only ever derived by the server itself (R10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelperSource {
    Volume(String),
    Bind(PathBuf),
}

/// How the helper mounts the source: read-only unless the op writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpMode {
    ReadOnly,
    ReadWrite,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, kind: EntryKind) -> Entry {
        Entry {
            name: name.into(),
            name_b64: None,
            kind,
            link: None,
            size: 0,
            mtime: rfc3339_utc(0),
            uid: 0,
            gid: 0,
            mode: 0o644,
            nlink: 1,
            write_blocked: None,
        }
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn folders_first_then_case_folded_bytes() {
        let mut list = vec![
            entry("b.txt", EntryKind::File),
            entry("Zeta", EntryKind::Dir),
            entry("a.txt", EntryKind::File),
            entry("B.txt", EntryKind::File),
            entry("alpha", EntryKind::Dir),
            entry("_x", EntryKind::Link),
            entry("é", EntryKind::File),
            entry("10", EntryKind::File),
            entry("9", EntryKind::File),
        ];
        sort_entries(&mut list);
        assert_eq!(
            names(&list),
            ["alpha", "Zeta", "10", "9", "_x", "a.txt", "B.txt", "b.txt", "é"]
        );
    }

    #[test]
    fn raw_names_sort_by_their_bytes() {
        let mut raw = entry("\u{fffd}", EntryKind::File);
        raw.name_b64 = Some(RawName::from_bytes(b"a\xff".to_vec()).unwrap());
        let mut list = vec![entry("b", EntryKind::File), raw, entry("a", EntryKind::File)];
        sort_entries(&mut list);
        assert_eq!(names(&list), ["a", "\u{fffd}", "b"]);
    }

    #[test]
    fn paging() {
        let items: Vec<u32> = (0..12_001).collect();
        assert_eq!(page_count(items.len()), 3);
        assert_eq!(page_count(0), 1);
        assert_eq!(page_count(5000), 1);
        let (p0, last0) = page_of(&items, 0);
        assert_eq!((p0.len(), p0[0], last0), (5000, 0, false));
        let (p2, last2) = page_of(&items, 2);
        assert_eq!((p2.len(), p2[0], last2), (2001, 10_000, true));
        let (past, last) = page_of(&items, 9);
        assert!(past.is_empty() && last);
        let empty: [u32; 0] = [];
        assert_eq!(page_of(&empty, 0), (&empty[..], true));
    }

    #[test]
    fn shape_blocks() {
        let inside = LinkInfo { target: "b".into(), inside: true };
        let outside = LinkInfo { target: "/etc".into(), inside: false };
        assert_eq!(Entry::blocked_by_shape(EntryKind::File, 1, None), None);
        assert_eq!(Entry::blocked_by_shape(EntryKind::File, 2, None), Some(WriteBlocked::HardLink));
        assert_eq!(Entry::blocked_by_shape(EntryKind::Dir, 3, None), None);
        assert_eq!(Entry::blocked_by_shape(EntryKind::Special, 1, None), Some(WriteBlocked::Special));
        assert_eq!(Entry::blocked_by_shape(EntryKind::Link, 1, Some(&inside)), None);
        assert_eq!(
            Entry::blocked_by_shape(EntryKind::Link, 1, Some(&outside)),
            Some(WriteBlocked::OutsideLink)
        );
    }

    #[test]
    fn entry_json_shape() {
        let mut e = entry("a", EntryKind::Link);
        e.link = Some(LinkInfo { target: "../x".into(), inside: false });
        e.write_blocked = Some(WriteBlocked::OutsideLink);
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["kind"], "link");
        assert_eq!(json["write_blocked"], "outside_link");
        assert_eq!(json["link"]["inside"], false);
        assert!(json.get("name_b64").is_none());
        assert_eq!(serde_json::from_value::<Entry>(json).unwrap(), e);
        let plain = serde_json::to_value(entry("b", EntryKind::File)).unwrap();
        assert!(plain["write_blocked"].is_null());
    }

    #[test]
    fn rfc3339() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339_utc(1_792_879_349), "2026-10-24T22:02:29Z");
        assert_eq!(rfc3339_utc(-1), "1969-12-31T23:59:59Z");
    }

    // --- mounts ---

    #[test]
    fn mount_id_is_stable_and_kind_scoped() {
        let a = MountId::compute(MountKind::Volume, "game_saves");
        assert_eq!(a, MountId::compute(MountKind::Volume, "game_saves"));
        assert_eq!(a.as_str().len(), 16);
        // Pinned so a change to the scheme (which would break bookmarked URLs) is deliberate.
        assert_eq!(a.as_str(), "f9e20beff8c6e82e");
        assert_ne!(a, MountId::compute(MountKind::Bind, "game_saves"));
        assert_ne!(a, MountId::compute(MountKind::Volume, "game_save"));
        // The separator keeps kind and source from running into each other.
        assert_ne!(
            MountId::compute(MountKind::Bind, "x"),
            MountId::compute(MountKind::Volume, "\0x")
        );
    }

    #[test]
    fn mount_id_parse() {
        let id = MountId::compute(MountKind::Bind, "/srv/data");
        assert_eq!(MountId::parse(id.as_str()), Ok(id.clone()));
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<MountId>(&json).unwrap(), id);
        for bad in ["", "abc", "4C4C1A0A0C4A7BCA", "4c4c1a0a0c4a7bcg", "4c4c1a0a0c4a7bca0"] {
            assert_eq!(MountId::parse(bad), Err(InvalidMountId), "{bad:?}");
        }
    }

    #[test]
    fn empty_kind_reads_as_bind() {
        assert_eq!(MountKind::from_inspect(""), MountKind::Bind);
        assert_eq!(MountKind::from_inspect("bind"), MountKind::Bind);
        assert_eq!(MountKind::from_inspect("volume"), MountKind::Volume);
        assert_eq!(MountKind::from_inspect("tmpfs"), MountKind::Tmpfs);
        assert_eq!(MountKind::from_inspect("npipe"), MountKind::Other);
        assert_eq!(MountKind::from_inspect("image"), MountKind::Other);
        assert_eq!(
            MountId::compute(MountKind::from_inspect(""), "/a"),
            MountId::compute(MountKind::Bind, "/a")
        );
    }

    #[test]
    fn browsable_rules() {
        let risky = BindRating::Risky("mounts /etc".into());
        for rating in [BindRating::None, BindRating::Info, risky.clone()] {
            assert!(browsability(MountKind::Volume, &rating).ok);
            let tmpfs = browsability(MountKind::Tmpfs, &rating);
            assert_eq!(tmpfs.reason.unwrap().code, NotBrowsableCode::Tmpfs);
            let other = browsability(MountKind::Other, &rating);
            assert_eq!(other.reason.unwrap().code, NotBrowsableCode::Other);
        }
        assert_eq!(browsability(MountKind::Bind, &BindRating::None), Browsable::yes());
        assert_eq!(browsability(MountKind::Bind, &BindRating::Info), Browsable::yes());
        assert_eq!(
            browsability(MountKind::Bind, &risky),
            Browsable::no(NotBrowsableCode::RiskyBind, "mounts /etc")
        );
    }

    #[test]
    fn browsable_json_shape() {
        assert_eq!(serde_json::to_value(Browsable::yes()).unwrap(), serde_json::json!({"ok": true}));
        let no = Browsable::no(NotBrowsableCode::SingleFile, "a single file");
        assert_eq!(
            serde_json::to_value(&no).unwrap(),
            serde_json::json!({"ok": false, "reason": {"code": "single_file", "message": "a single file"}})
        );
    }

    #[test]
    fn mount_new_computes_id_and_writable() {
        let svc = |name: &str, read_only| MountService {
            service: name.into(),
            destination: "/data".into(),
            read_only,
        };
        let ro = Mount::new(
            MountKind::Volume,
            "v".into(),
            vec![svc("a", true), svc("b", true)],
            MountState::Ready,
            Browsable::yes(),
            vec![],
        );
        assert!(!ro.writable);
        assert_eq!(ro.id, MountId::compute(MountKind::Volume, "v"));
        let rw = Mount::new(
            MountKind::Volume,
            "v".into(),
            vec![svc("a", true), svc("b", false)],
            MountState::NotCreated,
            Browsable::yes(),
            vec!["b".into()],
        );
        assert!(rw.writable);
        let json = serde_json::to_value(&rw).unwrap();
        assert_eq!(json["state"], "not_created");
        assert_eq!(json["kind"], "volume");
        assert_eq!(serde_json::from_value::<Mount>(json).unwrap(), rw);
    }
}
