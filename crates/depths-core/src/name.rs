//! Names that reach a path or an argv.
//!
//! A stack name is a directory under the stacks root, a compose project name and an argument to
//! `docker compose`. The rule is compose-go's project-name rule (`loader.go`): a name "matches
//! `^[a-z0-9][a-z0-9_-]*$`". That rule alone already keeps a name from being `..`, absolute,
//! hidden, or an option (`-p`), which is why every name is checked before it is used (T095), and
//! why the ports take a [`StackName`], which cannot be built any other way.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Whether `name` is a valid stack (compose project) name: matches `^[a-z0-9][a-z0-9_-]*$`.
pub fn valid_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    match bytes.next() {
        Some(b) if b.is_ascii_lowercase() || b.is_ascii_digit() => {}
        _ => return false,
    }
    bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// Whether `name` is a service name Depths will put in an argv.
///
/// Compose allows `[a-zA-Z0-9._-]+`. Depths also requires an alphanumeric first character, so a
/// service name can never read as an option (`-d`) or a relative path (`.`, `..`).
pub fn valid_service_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    match bytes.next() {
        Some(b) if b.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// A name that passed [`valid_name`]. The only way to name a stack to a port.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct StackName(String);

/// A name that failed [`valid_name`] or [`valid_service_name`]. Carries the name as given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidName(pub String);

impl fmt::Display for InvalidName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid name {:?}", self.0)
    }
}

impl std::error::Error for InvalidName {}

impl StackName {
    pub fn parse(name: &str) -> Result<Self, InvalidName> {
        if valid_name(name) {
            Ok(Self(name.to_owned()))
        } else {
            Err(InvalidName(name.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StackName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for StackName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Deserialising validates, so a name from the wire or `users.json` is checked on the way in.
impl<'de> Deserialize<'de> for StackName {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// A service name that passed [`valid_service_name`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ServiceName(String);

impl ServiceName {
    pub fn parse(name: &str) -> Result<Self, InvalidName> {
        if valid_service_name(name) {
            Ok(Self(name.to_owned()))
        } else {
            Err(InvalidName(name.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ServiceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ServiceName {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_what_compose_accepts() {
        for name in ["web", "a_b-1", "0x", "a", "9", "stack-2_x"] {
            assert!(valid_name(name), "{name:?} should be valid");
        }
    }

    #[test]
    fn rejects_everything_else() {
        for name in [
            "Web", "-a", "_a", "a.b", "a b", "", "..", ".", "/etc", "a/b", "../x", "a\0b", "é",
            "web\n", "-p",
        ] {
            assert!(!valid_name(name), "{name:?} should be invalid");
        }
    }

    #[test]
    fn a_stack_name_is_only_built_from_a_valid_name() {
        assert_eq!(StackName::parse("web").unwrap().as_str(), "web");
        assert_eq!(StackName::parse("../x"), Err(InvalidName("../x".into())));
        assert!(StackName::parse("/abs").is_err());
    }

    #[test]
    fn deserialising_a_stack_name_validates_it() {
        let ok: StackName = serde_json::from_str(r#""web""#).unwrap();
        assert_eq!(ok.as_str(), "web");
        assert!(serde_json::from_str::<StackName>(r#""..""#).is_err());
        assert!(serde_json::from_str::<StackName>(r#""-v""#).is_err());
    }

    #[test]
    fn service_names_cannot_read_as_options_or_paths() {
        for name in ["web", "Web", "db.primary", "a_b-1"] {
            assert!(valid_service_name(name), "{name:?} should be valid");
        }
        for name in ["", "-d", ".", "..", ".hidden", "a/b", "a b", "_x"] {
            assert!(!valid_service_name(name), "{name:?} should be invalid");
        }
        assert!(serde_json::from_str::<ServiceName>(r#""--rmi""#).is_err());
    }
}
