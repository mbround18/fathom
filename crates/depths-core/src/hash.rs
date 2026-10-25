//! A stack's `content_hash`: what the editor loaded, what a preview is bound to, and what a save
//! is checked against (FR-019).
//!
//! sha256 over a versioned tag, the compose file's name, a digest of its bytes, and a digest of
//! `.env`'s bytes or a marker that there is none. Each file is digested separately so an adapter
//! can stream a file larger than it will load ([`FileDigest`]), and the fields cannot run into one
//! another: renaming the compose file, or moving bytes between the two files, changes the hash.

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::discovery::ComposeFile;

/// sha256, lower-case hex.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct ContentHash(String);

impl ContentHash {
    /// Accepts exactly 64 lower-case hex digits, as a client sends back what it was given.
    pub fn parse(hex: &str) -> Option<Self> {
        (hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
        .then(|| Self(hex.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ContentHash {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Self::parse(&raw).ok_or_else(|| serde::de::Error::custom("not a sha256 hex digest"))
    }
}

/// A streaming sha256 of one file's bytes.
#[derive(Default)]
pub struct FileDigest(Sha256);

impl FileDigest {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    pub fn finish(self) -> [u8; 32] {
        self.0.finalize().into()
    }

    pub fn of(bytes: &[u8]) -> [u8; 32] {
        let mut d = Self::new();
        d.update(bytes);
        d.finish()
    }
}

/// The hash of a stack from its files' digests.
pub fn content_hash(
    compose_file: ComposeFile,
    compose: [u8; 32],
    env: Option<[u8; 32]>,
) -> ContentHash {
    let mut h = Sha256::new();
    h.update(b"depths-stack-v1\0");
    h.update(compose_file.file_name().as_bytes());
    h.update(b"\0");
    h.update(compose);
    match env {
        Some(d) => {
            h.update(b"\x01");
            h.update(d);
        }
        None => h.update(b"\x00"),
    }
    let out: [u8; 32] = h.finalize().into();
    ContentHash(out.iter().map(|b| format!("{b:02x}")).collect())
}

/// The hash of a stack from its files' bytes.
pub fn content_hash_of(
    compose_file: ComposeFile,
    compose: &[u8],
    env: Option<&[u8]>,
) -> ContentHash {
    content_hash(
        compose_file,
        FileDigest::of(compose),
        env.map(FileDigest::of),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: ComposeFile = ComposeFile::ComposeYaml;

    #[test]
    fn the_hash_is_stable_hex() {
        let a = content_hash_of(C, b"services: {}\n", None);
        assert_eq!(a, content_hash_of(C, b"services: {}\n", None));
        assert_eq!(a.as_str().len(), 64);
        assert_eq!(ContentHash::parse(a.as_str()), Some(a));
    }

    #[test]
    fn any_change_to_either_file_or_the_file_name_changes_it() {
        let base = content_hash_of(C, b"x", Some(b"A=1"));
        assert_ne!(base, content_hash_of(C, b"y", Some(b"A=1")));
        assert_ne!(base, content_hash_of(C, b"x", Some(b"A=2")));
        assert_ne!(base, content_hash_of(C, b"x", None));
        assert_ne!(
            content_hash_of(C, b"x", None),
            content_hash_of(C, b"x", Some(b""))
        );
        assert_ne!(
            base,
            content_hash_of(ComposeFile::ComposeYml, b"x", Some(b"A=1"))
        );
        assert_ne!(
            content_hash_of(C, b"xA", Some(b"")),
            content_hash_of(C, b"x", Some(b"A"))
        );
    }

    #[test]
    fn streaming_and_whole_digests_agree() {
        let mut d = FileDigest::new();
        d.update(b"serv");
        d.update(b"ices: {}\n");
        assert_eq!(d.finish(), FileDigest::of(b"services: {}\n"));
    }

    #[test]
    fn only_a_well_formed_digest_parses() {
        assert!(ContentHash::parse("").is_none());
        assert!(ContentHash::parse(&"A".repeat(64)).is_none());
        assert!(ContentHash::parse(&"0".repeat(63)).is_none());
        assert!(ContentHash::parse(&"g".repeat(64)).is_none());
        assert!(serde_json::from_str::<ContentHash>(r#""../etc""#).is_err());
    }
}
