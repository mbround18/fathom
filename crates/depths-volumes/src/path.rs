//! A path inside a mount (data-model.md "RelPath", R3).
//!
//! A [`RelPath`] is checked when it is built, so a value of this type is always relative, free of
//! NUL and `..`, and within the length limits. It is never resolved textually: the helper opens it
//! with `openat2(RESOLVE_BENEATH)` under the mount root, and this check is the first of two.
//!
//! A name that is not valid UTF-8 on disk cannot be a `RelPath` component. Such an entry is listed
//! with a lossy `name` and its raw bytes as `name_b64` ([`RawName`]); an op that may address it
//! (stat, read, size, tar, delete) takes the parent as its `path` plus the raw final component as
//! `raw_name`. [`RelPath::to_bytes_with`] joins the two for the helper.

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Longest single component, in bytes (`NAME_MAX`).
pub const MAX_COMPONENT_BYTES: usize = 255;
/// Longest path, in bytes (`PATH_MAX`).
pub const MAX_PATH_BYTES: usize = 4096;

/// Why a path or a name was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("the path contains a NUL byte")]
    Nul,
    #[error("the path is absolute")]
    Absolute,
    #[error("the path contains `..`")]
    DotDot,
    #[error("a path component is longer than {MAX_COMPONENT_BYTES} bytes")]
    ComponentTooLong,
    #[error("the path is longer than {MAX_PATH_BYTES} bytes")]
    TooLong,
    /// A name (one component) was empty, `.`, or contained `/`.
    #[error("not a single file name")]
    NotAName,
    #[error("not valid base64")]
    Base64,
}

/// A validated, normalised relative path inside a mount. `""` is the root.
///
/// Normalised means components joined by a single `/`, with no leading or trailing `/` and no
/// `.` components. It serialises as a plain string and checks itself on the way in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(try_from = "String", into = "String")]
#[ts(type = "string")]
pub struct RelPath(String);

impl RelPath {
    /// The mount root.
    pub fn root() -> Self {
        Self(String::new())
    }

    /// Validates and normalises `raw`: a leading `/`, a NUL, a `..` component, a component over
    /// 255 bytes or a path over 4,096 bytes is refused; empty and `.` components are dropped.
    pub fn parse(raw: &str) -> Result<Self, PathError> {
        if raw.len() > MAX_PATH_BYTES {
            return Err(PathError::TooLong);
        }
        if raw.contains('\0') {
            return Err(PathError::Nul);
        }
        if raw.starts_with('/') {
            return Err(PathError::Absolute);
        }
        let mut out = String::with_capacity(raw.len());
        for part in raw.split('/') {
            match part {
                "" | "." => continue,
                ".." => return Err(PathError::DotDot),
                _ if part.len() > MAX_COMPONENT_BYTES => return Err(PathError::ComponentTooLong),
                _ => {
                    if !out.is_empty() {
                        out.push('/');
                    }
                    out.push_str(part);
                }
            }
        }
        Ok(Self(out))
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The containing folder; `None` for the root.
    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        Some(match self.0.rfind('/') {
            Some(i) => Self(self.0[..i].to_owned()),
            None => Self::root(),
        })
    }

    /// The last component; `None` for the root.
    pub fn file_name(&self) -> Option<&str> {
        if self.is_root() {
            return None;
        }
        Some(self.0.rsplit('/').next().unwrap_or(&self.0))
    }

    /// The components, root first. Empty for the root.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/').filter(|c| !c.is_empty())
    }

    /// This path with one more component. `name` must be a single name ([`Name::new`]'s rule).
    pub fn join(&self, name: &str) -> Result<Self, PathError> {
        check_name(name.as_bytes())?;
        let len = if self.is_root() { name.len() } else { self.0.len() + 1 + name.len() };
        if len > MAX_PATH_BYTES {
            return Err(PathError::TooLong);
        }
        Ok(if self.is_root() {
            Self(name.to_owned())
        } else {
            Self(format!("{}/{name}", self.0))
        })
    }

    /// The bytes the helper opens: this path, plus `raw` as a final component when given.
    /// The root alone is `.`, as `openat2` needs a non-empty path.
    pub fn to_bytes_with(&self, raw: Option<&RawName>) -> Vec<u8> {
        let mut out = self.0.as_bytes().to_vec();
        if let Some(raw) = raw {
            if !out.is_empty() {
                out.push(b'/');
            }
            out.extend_from_slice(raw.as_bytes());
        }
        if out.is_empty() {
            out.push(b'.');
        }
        out
    }
}

impl TryFrom<String> for RelPath {
    type Error = PathError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<RelPath> for String {
    fn from(value: RelPath) -> Self {
        value.0
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One component's rule, on raw bytes: not empty, not `.` or `..`, no `/` or NUL, ≤ 255 bytes.
fn check_name(name: &[u8]) -> Result<(), PathError> {
    if name.contains(&0) {
        Err(PathError::Nul)
    } else if name == b".." {
        Err(PathError::DotDot)
    } else if name.is_empty() || name == b"." || name.contains(&b'/') {
        Err(PathError::NotAName)
    } else if name.len() > MAX_COMPONENT_BYTES {
        Err(PathError::ComponentTooLong)
    } else {
        Ok(())
    }
}

/// A single validated UTF-8 file name: what `Create` and `Put` take for the new entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(try_from = "String", into = "String")]
#[ts(type = "string")]
pub struct Name(String);

impl Name {
    /// Not empty, not `.` or `..`, no `/` or NUL, at most 255 bytes.
    pub fn new(value: impl Into<String>) -> Result<Self, PathError> {
        let value = value.into();
        check_name(value.as_bytes())?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Name {
    type Error = PathError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Name> for String {
    fn from(value: Name) -> Self {
        value.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A single file name as raw bytes, for names that are not valid UTF-8 on disk.
///
/// On the wire it is standard base64 (`name_b64` in an Entry). It follows the same component rule
/// as [`Name`], on bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(try_from = "String", into = "String")]
#[ts(type = "string")]
pub struct RawName(Vec<u8>);

impl RawName {
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Result<Self, PathError> {
        let bytes = bytes.into();
        check_name(&bytes)?;
        Ok(Self(bytes))
    }

    pub fn from_b64(b64: &str) -> Result<Self, PathError> {
        let bytes = STANDARD.decode(b64).map_err(|_| PathError::Base64)?;
        Self::from_bytes(bytes)
    }

    pub fn to_b64(&self) -> String {
        STANDARD.encode(&self.0)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The display form, with U+FFFD for invalid sequences.
    pub fn lossy(&self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }
}

impl TryFrom<String> for RawName {
    type Error = PathError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_b64(&value)
    }
}

impl From<RawName> for String {
    fn from(value: RawName) -> Self {
        value.to_b64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<String, PathError> {
        RelPath::parse(s).map(String::from)
    }

    #[test]
    fn root_forms() {
        assert_eq!(p(""), Ok(String::new()));
        assert_eq!(p("."), Ok(String::new()));
        assert_eq!(p("./"), Ok(String::new()));
        assert!(RelPath::root().is_root());
        assert_eq!(RelPath::root().parent(), None);
        assert_eq!(RelPath::root().file_name(), None);
        assert_eq!(RelPath::root().components().count(), 0);
    }

    #[test]
    fn empty_and_dot_components_are_dropped() {
        assert_eq!(p("a//./b"), Ok("a/b".into()));
        assert_eq!(p("a/b/"), Ok("a/b".into()));
        assert_eq!(p("./a/./"), Ok("a".into()));
    }

    #[test]
    fn dotdot_is_rejected_never_resolved() {
        assert_eq!(p("a/../b"), Err(PathError::DotDot));
        assert_eq!(p(".."), Err(PathError::DotDot));
        assert_eq!(p("a/.."), Err(PathError::DotDot));
        // Only a whole component is `..`.
        assert_eq!(p("a/..b/c.."), Ok("a/..b/c..".into()));
    }

    #[test]
    fn leading_slash_and_nul_are_rejected() {
        assert_eq!(p("/a"), Err(PathError::Absolute));
        assert_eq!(p("/"), Err(PathError::Absolute));
        assert_eq!(p("a\0b"), Err(PathError::Nul));
    }

    #[test]
    fn length_limits() {
        let c255 = "x".repeat(255);
        assert!(p(&c255).is_ok());
        assert_eq!(p(&"x".repeat(256)), Err(PathError::ComponentTooLong));
        // Multi-byte: 128 × 2-byte chars = 256 bytes.
        assert_eq!(p(&"é".repeat(128)), Err(PathError::ComponentTooLong));
        let long = vec!["a"; 2048].join("/"); // 4,095 bytes
        assert!(p(&long).is_ok());
        assert_eq!(p(&format!("{long}/b")), Err(PathError::TooLong));
    }

    #[test]
    fn parent_name_components() {
        let path = RelPath::parse("a/b/c.txt").unwrap();
        assert_eq!(path.file_name(), Some("c.txt"));
        assert_eq!(path.parent().unwrap().as_str(), "a/b");
        assert_eq!(path.components().collect::<Vec<_>>(), ["a", "b", "c.txt"]);
        let top = RelPath::parse("a").unwrap();
        assert_eq!(top.parent(), Some(RelPath::root()));
        assert_eq!(top.file_name(), Some("a"));
        assert_eq!(path.to_string(), "a/b/c.txt");
    }

    #[test]
    fn join_takes_one_component() {
        let root = RelPath::root();
        assert_eq!(root.join("a").unwrap().as_str(), "a");
        assert_eq!(root.join("a").unwrap().join("b").unwrap().as_str(), "a/b");
        for bad in ["", ".", "a/b", "/a"] {
            assert_eq!(root.join(bad), Err(PathError::NotAName), "{bad:?}");
        }
        assert_eq!(root.join(".."), Err(PathError::DotDot));
        assert_eq!(root.join("a\0"), Err(PathError::Nul));
        assert_eq!(root.join(&"x".repeat(256)), Err(PathError::ComponentTooLong));
        let long = RelPath::parse(&vec!["a"; 2048].join("/")).unwrap();
        assert_eq!(long.join("bb"), Err(PathError::TooLong));
    }

    #[test]
    fn serde_validates_on_the_way_in() {
        let path: RelPath = serde_json::from_str("\"a//b\"").unwrap();
        assert_eq!(path.as_str(), "a/b");
        assert_eq!(serde_json::to_string(&path).unwrap(), "\"a/b\"");
        assert!(serde_json::from_str::<RelPath>("\"../x\"").is_err());
        assert!(serde_json::from_str::<RelPath>("\"/etc\"").is_err());
        assert!(serde_json::from_str::<Name>("\"a/b\"").is_err());
        assert_eq!(serde_json::from_str::<Name>("\"a.txt\"").unwrap().as_str(), "a.txt");
    }

    #[test]
    fn raw_names_round_trip_as_base64() {
        let raw = RawName::from_bytes(b"caf\xe9".to_vec()).unwrap();
        let json = serde_json::to_string(&raw).unwrap();
        assert_eq!(json, "\"Y2Fm6Q==\"");
        assert_eq!(serde_json::from_str::<RawName>(&json).unwrap(), raw);
        assert_eq!(raw.lossy(), "caf\u{fffd}");
        assert_eq!(RawName::from_bytes(b"a/b".to_vec()), Err(PathError::NotAName));
        assert_eq!(RawName::from_bytes(b"..".to_vec()), Err(PathError::DotDot));
        assert_eq!(RawName::from_b64("!!"), Err(PathError::Base64));
    }

    #[test]
    fn bytes_for_the_helper() {
        let raw = RawName::from_bytes(b"\xff".to_vec()).unwrap();
        assert_eq!(RelPath::root().to_bytes_with(None), b".");
        assert_eq!(RelPath::root().to_bytes_with(Some(&raw)), b"\xff");
        let dir = RelPath::parse("a/b").unwrap();
        assert_eq!(dir.to_bytes_with(Some(&raw)), b"a/b/\xff");
        assert_eq!(dir.to_bytes_with(None), b"a/b");
    }
}
