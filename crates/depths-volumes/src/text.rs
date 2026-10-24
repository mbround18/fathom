//! A file's content as the editor sees it: the fingerprint a save is checked against (R7) and
//! the text rules (R8).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

use crate::mount::{Owner, WriteBlocked};

// ---------------------------------------------------------------------------
// Fingerprint
// ---------------------------------------------------------------------------

/// What a save is checked against: the content's sha256 plus `(size, mtime_ns, ino)` (R7).
///
/// Written as `sha256:<64 hex>:<size>:<mtime_ns>:<ino>`, and serialised as that string. The hash
/// catches two saves that share a coarse mtime; the inode catches a replaced file with equal
/// content. Two fingerprints match only when every part does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(try_from = "String", into = "String")]
#[ts(type = "string")]
pub struct Fingerprint {
    sha256: [u8; 32],
    size: u64,
    mtime_ns: i64,
    ino: u64,
}

/// A string that is not a fingerprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a fingerprint (sha256:<hex>:<size>:<mtime_ns>:<ino>)")]
pub struct FingerprintError;

impl Fingerprint {
    pub fn new(sha256: [u8; 32], size: u64, mtime_ns: i64, ino: u64) -> Self {
        Self { sha256, size, mtime_ns, ino }
    }

    /// From a hasher fed with the content as it was streamed.
    pub fn from_hasher(hasher: Sha256, size: u64, mtime_ns: i64, ino: u64) -> Self {
        Self::new(hasher.finalize().into(), size, mtime_ns, ino)
    }

    /// From the whole content in memory; the size is its length.
    pub fn of_content(content: &[u8], mtime_ns: i64, ino: u64) -> Self {
        Self::new(Sha256::digest(content).into(), content.len() as u64, mtime_ns, ino)
    }

    pub fn sha256(&self) -> &[u8; 32] {
        &self.sha256
    }

    pub fn sha256_hex(&self) -> String {
        hex(&self.sha256)
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn mtime_ns(&self) -> i64 {
        self.mtime_ns
    }

    pub fn ino(&self) -> u64 {
        self.ino
    }

    /// Same bytes, whatever the metadata says.
    pub fn same_content(&self, other: &Self) -> bool {
        self.sha256 == other.sha256 && self.size == other.size
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0xf) as usize] as char);
    }
    out
}

fn unhex_32(s: &str) -> Option<[u8; 32]> {
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    let bytes = s.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, pair) in bytes.chunks_exact(2).enumerate() {
        out[i] = digit(pair[0])? << 4 | digit(pair[1])?;
    }
    Some(out)
}

/// Decimal digits only, optionally with a leading `-` when `signed`: no `+`, no spaces, so each
/// fingerprint has one spelling.
fn plain_number(s: &str, signed: bool) -> bool {
    let digits = if signed { s.strip_prefix('-').unwrap_or(s) } else { s };
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sha256:{}:{}:{}:{}", hex(&self.sha256), self.size, self.mtime_ns, self.ino)
    }
}

impl FromStr for Fingerprint {
    type Err = FingerprintError;
    fn from_str(s: &str) -> Result<Self, FingerprintError> {
        let mut parts = s.split(':');
        let (Some("sha256"), Some(h), Some(size), Some(mtime), Some(ino), None) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            return Err(FingerprintError);
        };
        if !(plain_number(size, false) && plain_number(mtime, true) && plain_number(ino, false)) {
            return Err(FingerprintError);
        }
        Ok(Self {
            sha256: unhex_32(h).ok_or(FingerprintError)?,
            size: size.parse().map_err(|_| FingerprintError)?,
            mtime_ns: mtime.parse().map_err(|_| FingerprintError)?,
            ino: ino.parse().map_err(|_| FingerprintError)?,
        })
    }
}

impl TryFrom<String> for Fingerprint {
    type Error = FingerprintError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<Fingerprint> for String {
    fn from(value: Fingerprint) -> Self {
        value.to_string()
    }
}

// ---------------------------------------------------------------------------
// File content
// ---------------------------------------------------------------------------

/// Why a file is offered for download only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BinaryReason {
    /// A NUL in the first 8 KiB.
    Binary,
    /// Over the editor limit.
    TooLarge,
    /// Not valid UTF-8.
    NotUtf8,
}

/// The line endings a text file was found with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Eol {
    /// LF only, or no line ending at all.
    #[default]
    Lf,
    /// CRLF only.
    Crlf,
    /// Both. A save uses the most common one, `save_eol`.
    Mixed,
}

/// The line ending a save writes: [`Eol`] without `mixed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LineEnding {
    #[default]
    Lf,
    Crlf,
}

/// A `ReadFile` reply's body (data-model.md "FileContent").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FileContent {
    pub fingerprint: Fingerprint,
    pub size: u64,
    /// Present when the content is text within the editor limit, with LF endings and no BOM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub text: Option<String>,
    /// Set exactly when `text` is absent.
    pub binary_reason: Option<BinaryReason>,
    /// The file starts with a UTF-8 BOM, stripped from `text` and written back on save.
    pub bom: bool,
    pub eol: Eol,
    /// The ending a save writes back: `eol`, or for `mixed` the most common one. The client sends
    /// it as the save's ending.
    pub save_eol: LineEnding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub read_only: Option<WriteBlocked>,
    pub owner: Owner,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp() -> Fingerprint {
        Fingerprint::of_content(b"hello\n", 1_700_000_000_123_456_789, 42)
    }

    #[test]
    fn fingerprint_format() {
        assert_eq!(
            fp().to_string(),
            "sha256:5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03:6:1700000000123456789:42"
        );
        assert_eq!(fp().size(), 6);
        assert_eq!(fp().ino(), 42);
    }

    #[test]
    fn fingerprint_parse_round_trip_and_equality() {
        let a = fp();
        let parsed: Fingerprint = a.to_string().parse().unwrap();
        assert_eq!(parsed, a);
        let neg = Fingerprint::new([0xab; 32], 0, -5, 0);
        assert_eq!(neg.to_string().parse::<Fingerprint>().unwrap(), neg);

        let other_ino = Fingerprint::new(*a.sha256(), a.size(), a.mtime_ns(), 43);
        assert_ne!(other_ino, a);
        assert!(other_ino.same_content(&a));
        let other_mtime = Fingerprint::new(*a.sha256(), a.size(), a.mtime_ns() + 1, a.ino());
        assert_ne!(other_mtime, a);
        assert_ne!(Fingerprint::of_content(b"hellO\n", a.mtime_ns(), a.ino()), a);
    }

    #[test]
    fn streamed_and_whole_hashes_agree() {
        let mut h = Sha256::new();
        h.update(b"hel");
        h.update(b"lo\n");
        assert_eq!(Fingerprint::from_hasher(h, 6, fp().mtime_ns(), 42), fp());
    }

    #[test]
    fn fingerprint_rejects_other_spellings() {
        let good = fp().to_string();
        let hexpart = &good[7..71];
        for bad in [
            String::new(),
            "sha256".into(),
            good.replacen("sha256", "sha1", 1),
            good.replacen(hexpart, &hexpart.to_uppercase(), 1),
            good.replacen(hexpart, &hexpart[1..], 1),
            format!("{good}:1"),
            good.replacen(":6:", ":+6:", 1),
            good.replacen(":6:", ":-6:", 1),
            good.replacen(":6:", "::", 1),
            good.replacen(":42", ": 42", 1),
            "sha256:00:1:1:1".into(),
        ] {
            assert_eq!(bad.parse::<Fingerprint>(), Err(FingerprintError), "{bad:?}");
        }
    }

    #[test]
    fn fingerprint_serde_is_the_string() {
        let json = serde_json::to_string(&fp()).unwrap();
        assert_eq!(json, format!("\"{}\"", fp()));
        assert_eq!(serde_json::from_str::<Fingerprint>(&json).unwrap(), fp());
        assert!(serde_json::from_str::<Fingerprint>("\"sha256:zz\"").is_err());
    }

    #[test]
    fn file_content_json_shape() {
        let content = FileContent {
            fingerprint: fp(),
            size: 6,
            text: None,
            binary_reason: Some(BinaryReason::NotUtf8),
            bom: false,
            eol: Eol::Lf,
            save_eol: LineEnding::Lf,
            read_only: Some(WriteBlocked::HardLink),
            owner: Owner { uid: 1000, gid: 1000, mode: 0o640 },
        };
        let json = serde_json::to_value(&content).unwrap();
        assert_eq!(json["binary_reason"], "not_utf8");
        assert_eq!(json["read_only"], "hard_link");
        assert_eq!(json["owner"]["mode"], 0o640);
        assert!(json.get("text").is_none());
        assert_eq!(serde_json::from_value::<FileContent>(json).unwrap(), content);
    }
}
