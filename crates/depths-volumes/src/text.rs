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

// ---------------------------------------------------------------------------
// Text codec (R8)
// ---------------------------------------------------------------------------

/// How far into a file a NUL marks it as binary.
pub const NUL_SCAN_BYTES: usize = 8 * 1024;
/// The UTF-8 byte order mark.
pub const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// A text file as the editor works on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// LF endings, BOM stripped.
    pub text: String,
    pub bom: bool,
    pub eol: Eol,
    pub save_eol: LineEnding,
}

/// `None` when `bytes` is text: within `editor_max`, no NUL in the first 8 KiB, valid UTF-8.
/// The checks run in that order, so the reason is the first that fails.
pub fn classify(bytes: &[u8], editor_max: u64) -> Option<BinaryReason> {
    if bytes.len() as u64 > editor_max {
        Some(BinaryReason::TooLarge)
    } else if bytes[..bytes.len().min(NUL_SCAN_BYTES)].contains(&0) {
        Some(BinaryReason::Binary)
    } else if std::str::from_utf8(bytes).is_err() {
        Some(BinaryReason::NotUtf8)
    } else {
        None
    }
}

/// The endings in `text`: a CRLF counts as one, a lone LF as the other, a lone CR as neither.
/// `save_eol` is the more common of the two (LF on a tie, and when there is none).
pub fn detect_eol(text: &str) -> (Eol, LineEnding) {
    let bytes = text.as_bytes();
    let (mut crlf, mut lf) = (0usize, 0usize);
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            if i > 0 && bytes[i - 1] == b'\r' {
                crlf += 1;
            } else {
                lf += 1;
            }
        }
    }
    let save = if crlf > lf { LineEnding::Crlf } else { LineEnding::Lf };
    let eol = match (crlf, lf) {
        (0, _) => Eol::Lf,
        (_, 0) => Eol::Crlf,
        _ => Eol::Mixed,
    };
    (eol, save)
}

/// The editor's view of `bytes`, or why it has none. A leading BOM is stripped and flagged, and
/// CRLF becomes LF (for `mixed`, every CRLF does). The trailing newline is kept as it is.
pub fn decode(bytes: &[u8], editor_max: u64) -> Result<Decoded, BinaryReason> {
    if let Some(reason) = classify(bytes, editor_max) {
        return Err(reason);
    }
    let (bom, body) = match bytes.strip_prefix(UTF8_BOM) {
        Some(rest) => (true, rest),
        None => (false, bytes),
    };
    let raw = std::str::from_utf8(body).map_err(|_| BinaryReason::NotUtf8)?;
    let (eol, save_eol) = detect_eol(raw);
    let text = if eol == Eol::Lf { raw.to_owned() } else { raw.replace("\r\n", "\n") };
    Ok(Decoded { text, bom, eol, save_eol })
}

/// The bytes a save writes: `text` (LF endings) with `eol` applied to every LF and the BOM put
/// back when `bom`. `encode(decode(x))` is `x` for every text `x` whose endings are not mixed.
pub fn encode(text: &str, bom: bool, eol: LineEnding) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + if bom { 3 } else { 0 });
    if bom {
        out.extend_from_slice(UTF8_BOM);
    }
    match eol {
        LineEnding::Lf => out.extend_from_slice(text.as_bytes()),
        LineEnding::Crlf => {
            for (i, line) in text.split('\n').enumerate() {
                if i > 0 {
                    out.extend_from_slice(b"\r\n");
                }
                out.extend_from_slice(line.as_bytes());
            }
        }
    }
    out
}

impl FileContent {
    /// The reply for a file whose fingerprint was taken and whose bytes are `content`, or `None`
    /// when they were not read because the file is over `editor_max`.
    pub fn build(
        fingerprint: Fingerprint,
        owner: Owner,
        read_only: Option<WriteBlocked>,
        content: Option<&[u8]>,
        editor_max: u64,
    ) -> Self {
        let decoded = content
            .ok_or(BinaryReason::TooLarge)
            .and_then(|bytes| decode(bytes, editor_max));
        let (text, binary_reason, bom, eol, save_eol) = match decoded {
            Ok(d) => (Some(d.text), None, d.bom, d.eol, d.save_eol),
            Err(reason) => (None, Some(reason), false, Eol::Lf, LineEnding::Lf),
        };
        Self {
            fingerprint,
            size: fingerprint.size(),
            text,
            binary_reason,
            bom,
            eol,
            save_eol,
            read_only,
            owner,
        }
    }
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

    // --- codec ---

    const MAX: u64 = 2 * 1024 * 1024;

    #[test]
    fn classify_rules_in_order() {
        assert_eq!(classify(b"plain\n", MAX), None);
        assert_eq!(classify(b"", MAX), None);
        assert_eq!(classify(b"a\0b", MAX), Some(BinaryReason::Binary));
        assert_eq!(classify(b"\xff\xfe", MAX), Some(BinaryReason::NotUtf8));
        assert_eq!(classify(b"abcd", 3), Some(BinaryReason::TooLarge));
        assert_eq!(classify(b"abc", 3), None);
        // A NUL past the first 8 KiB does not make it binary; it is still valid UTF-8.
        let mut late = vec![b'a'; NUL_SCAN_BYTES];
        late.push(0);
        assert_eq!(classify(&late, MAX), None);
        let mut early = vec![b'a'; NUL_SCAN_BYTES - 1];
        early.push(0);
        assert_eq!(classify(&early, MAX), Some(BinaryReason::Binary));
        // Too large wins over everything.
        assert_eq!(classify(b"\0\xff", 1), Some(BinaryReason::TooLarge));
    }

    #[test]
    fn bom_is_stripped_and_flagged() {
        let d = decode(b"\xEF\xBB\xBFkey = 1\n", MAX).unwrap();
        assert!(d.bom);
        assert_eq!(d.text, "key = 1\n");
        assert_eq!(encode(&d.text, d.bom, d.save_eol), b"\xEF\xBB\xBFkey = 1\n");
        assert!(!decode(b"x", MAX).unwrap().bom);
    }

    #[test]
    fn line_endings() {
        let d = decode(b"a\r\nb\r\n", MAX).unwrap();
        assert_eq!((d.text.as_str(), d.eol, d.save_eol), ("a\nb\n", Eol::Crlf, LineEnding::Crlf));
        let d = decode(b"a\nb", MAX).unwrap();
        assert_eq!((d.text.as_str(), d.eol), ("a\nb", Eol::Lf));
        assert_eq!(decode(b"no newline", MAX).unwrap().eol, Eol::Lf);
        // A lone CR is content, not an ending.
        let d = decode(b"a\rb\r\n", MAX).unwrap();
        assert_eq!((d.text.as_str(), d.eol), ("a\rb\n", Eol::Crlf));
    }

    #[test]
    fn mixed_saves_with_the_most_common_ending() {
        let d = decode(b"a\r\nb\r\nc\n", MAX).unwrap();
        assert_eq!((d.text.as_str(), d.eol, d.save_eol), ("a\nb\nc\n", Eol::Mixed, LineEnding::Crlf));
        assert_eq!(encode(&d.text, d.bom, d.save_eol), b"a\r\nb\r\nc\r\n");
        let d = decode(b"a\r\nb\nc\n", MAX).unwrap();
        assert_eq!((d.eol, d.save_eol), (Eol::Mixed, LineEnding::Lf));
        assert_eq!(encode(&d.text, d.bom, d.save_eol), b"a\nb\nc\n");
        // A tie saves LF.
        assert_eq!(decode(b"a\r\nb\n", MAX).unwrap().save_eol, LineEnding::Lf);
    }

    #[test]
    fn trailing_newline_is_kept_as_is() {
        for x in [&b"a"[..], b"a\n", b"a\n\n", b"a\r\n", b"a\r\n\r\n", b"\n", b"\r\n", b""] {
            let d = decode(x, MAX).unwrap();
            assert_eq!(encode(&d.text, d.bom, d.save_eol), x, "{x:?}");
        }
    }

    #[test]
    fn file_content_build() {
        let owner = Owner { uid: 1, gid: 2, mode: 0o600 };
        let bytes = b"\xEF\xBB\xBFa\r\n";
        let fp = Fingerprint::of_content(bytes, 0, 1);
        let c = FileContent::build(fp, owner, None, Some(bytes), MAX);
        assert_eq!(c.text.as_deref(), Some("a\n"));
        assert_eq!((c.binary_reason, c.bom, c.eol, c.size), (None, true, Eol::Crlf, 6));

        let big = FileContent::build(fp, owner, Some(WriteBlocked::HardLink), None, MAX);
        assert_eq!((big.text, big.binary_reason), (None, Some(BinaryReason::TooLarge)));
        assert_eq!(big.read_only, Some(WriteBlocked::HardLink));

        let bin = FileContent::build(fp, owner, None, Some(b"\0"), MAX);
        assert_eq!(bin.binary_reason, Some(BinaryReason::Binary));
    }

    /// A deterministic xorshift64* stream, so the property tests need no crate and never flake.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// Pieces that exercise every rule: endings, a lone CR, BOM bytes anywhere, multi-byte
    /// characters, a stray continuation byte (not UTF-8) and a NUL.
    const PIECES: &[&[u8]] = &[
        b"a",
        b"key = value",
        b" ",
        b"\t",
        "é".as_bytes(),
        "日本".as_bytes(),
        "🦀".as_bytes(),
        b"\n",
        b"\n",
        b"\r\n",
        b"\r\n",
        b"\r",
        UTF8_BOM,
        b"\x80",
        b"\0",
    ];

    fn generate(rng: &mut Rng, pieces: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        if rng.below(4) == 0 {
            out.extend_from_slice(UTF8_BOM);
        }
        for _ in 0..rng.below(40) {
            out.extend_from_slice(pieces[rng.below(pieces.len())]);
        }
        out
    }

    #[test]
    fn property_encode_decode_is_identity_unless_mixed() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let (mut identity, mut mixed, mut binary) = (0, 0, 0);
        for _ in 0..20_000 {
            let x = generate(&mut rng, PIECES);
            match decode(&x, MAX) {
                Ok(d) if d.eol != Eol::Mixed => {
                    assert_eq!(encode(&d.text, d.bom, d.save_eol), x, "{x:?}");
                    identity += 1;
                }
                // A mixed file saves with one ending, and saving that again changes nothing. (A
                // CR left before an LF in the decoded text reads back as a CRLF, so those are
                // only counted.)
                Ok(d) if d.text.contains('\r') => mixed += 1,
                Ok(d) => {
                    let saved = encode(&d.text, d.bom, d.save_eol);
                    let again = decode(&saved, MAX).unwrap();
                    assert_ne!(again.eol, Eol::Mixed);
                    assert_eq!(again.text, d.text);
                    assert_eq!(encode(&again.text, again.bom, again.save_eol), saved);
                    mixed += 1;
                }
                Err(_) => binary += 1,
            }
        }
        // The generator reaches every branch often enough to mean something.
        assert!(identity > 1000 && mixed > 1000 && binary > 1000, "{identity} {mixed} {binary}");
    }

    #[test]
    fn property_single_ending_files_always_round_trip() {
        let lf: Vec<&[u8]> = PIECES
            .iter()
            .copied()
            .filter(|p| !matches!(*p, b"\r\n" | b"\r" | b"\x80" | b"\0"))
            .collect();
        let crlf: Vec<&[u8]> =
            lf.iter().map(|p| if *p == b"\n" { &b"\r\n"[..] } else { *p }).collect();
        let mut rng = Rng(42);
        for _ in 0..5_000 {
            for (pieces, want) in [(&lf, Eol::Lf), (&crlf, Eol::Crlf)] {
                let x = generate(&mut rng, pieces);
                let d = decode(&x, MAX).unwrap();
                if x.contains(&b'\n') {
                    assert_eq!(d.eol, want, "{x:?}");
                }
                assert!(!d.text.contains("\r\n"));
                assert_eq!(encode(&d.text, d.bom, d.save_eol), x, "{x:?}");
            }
        }
    }
}
