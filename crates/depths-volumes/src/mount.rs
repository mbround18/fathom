//! What a mount is, and what a listing of one holds (data-model.md "Mount", "Entry").

use std::borrow::Cow;
use std::cmp::Ordering;

use serde::{Deserialize, Serialize};
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
}
