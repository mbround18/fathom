//! base64url without padding, the encoding WebAuthn uses for credential ids and challenges.
//!
//! Exposed so crates that only verify assertions (keel-trust) read pinned keys and credential ids
//! in the same encoding without their own base64 dependency. Decoding is strict: padding,
//! whitespace and the standard alphabet are refused.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// Encodes bytes as base64url without padding.
pub fn encode(bytes: impl AsRef<[u8]>) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Decodes base64url without padding. `None` for anything else.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(text.as_bytes()).ok()
}

/// serde helpers for a `Vec<u8>` field written as a base64url string.
pub mod serde_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    /// Writes the bytes as base64url without padding.
    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::encode(bytes))
    }

    /// Reads base64url without padding.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        super::decode(&text).ok_or_else(|| serde::de::Error::custom("invalid base64url"))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn strict_round_trip() {
        assert_eq!(super::encode([0xfb, 0xff]), "-_8");
        assert_eq!(super::decode("-_8"), Some(vec![0xfb, 0xff]));
        assert_eq!(super::decode("-_8="), None);
        assert_eq!(super::decode("+/8"), None);
        assert_eq!(super::decode(" -_8"), None);
    }
}
