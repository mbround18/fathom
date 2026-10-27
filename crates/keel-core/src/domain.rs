//! Domain types shared across Keel (spec 055, T017): ids, roles, slugs and origins.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! id_newtype {
    ($(#[$doc:meta] $name:ident),* $(,)?) => {$(
        #[$doc]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            /// A fresh time-ordered (UUIDv7) id.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self)
            }
        }

        impl From<Uuid> for $name {
            fn from(id: Uuid) -> Self {
                Self(id)
            }
        }
    )*};
}

id_newtype!(
    /// A user account.
    UserId,
    /// A workspace: the unit of isolation for bindings, inventory, runs and grants.
    WorkspaceId,
    /// A project inside a workspace.
    ProjectId,
    /// A team.
    TeamId,
    /// A team's grant on a workspace.
    GrantId,
    /// An API token.
    ApiTokenId,
    /// An audit event.
    AuditEventId,
    /// A run.
    RunId,
    /// A managed host.
    HostId,
);

/// Error for an enum or slug that does not parse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind}: {value:?}")]
pub struct ParseError {
    pub kind: &'static str,
    pub value: String,
}

macro_rules! str_enum {
    ($(#[$doc:meta])* $name:ident, $kind:literal { $($variant:ident => $text:literal),* $(,)? }) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),* }

        impl $name {
            /// Every variant, in declaration order.
            pub const ALL: &'static [$name] = &[$($name::$variant),*];

            /// The stored and wire spelling.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self { $($name::$variant => $text),* }
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = $crate::domain::ParseError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($text => Ok($name::$variant),)*
                    _ => Err($crate::domain::ParseError { kind: $kind, value: s.to_owned() }),
                }
            }
        }
    };
}
pub(crate) use str_enum;

str_enum!(
    /// A user's instance-wide role. `owner` and `admin` administer the whole instance.
    InstanceRole, "instance role" {
        Owner => "owner",
        Admin => "admin",
        User => "user",
    }
);

str_enum!(
    /// A team's role on a workspace, through a grant.
    TeamRole, "team role" {
        Viewer => "viewer",
        Operator => "operator",
        Approver => "approver",
        WsAdmin => "ws_admin",
    }
);

str_enum!(
    /// Where a team, membership or grant comes from. Standalone Keel only knows `local`; suite
    /// mode adds directory-synced origins later.
    Origin, "origin" {
        Local => "local",
    }
);

/// A URL-safe name: `^[a-z0-9][a-z0-9-]{0,62}$`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Slug(String);

impl Slug {
    /// Validates `value` as a slug.
    pub fn parse(value: &str) -> Result<Self, ParseError> {
        let bytes = value.as_bytes();
        let ok = !bytes.is_empty()
            && bytes.len() <= 63
            && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
            && bytes
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-');
        if ok {
            Ok(Self(value.to_owned()))
        } else {
            Err(ParseError {
                kind: "slug",
                value: value.to_owned(),
            })
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Slug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Slug {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl<'de> Deserialize<'de> for Slug {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        for ok in ["a", "0", "web-1", "a".repeat(63).as_str()] {
            assert!(Slug::parse(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "-a",
            "A",
            "a_b",
            "a.b",
            "a b",
            "é",
            "a".repeat(64).as_str(),
        ] {
            assert!(Slug::parse(bad).is_err(), "{bad}");
        }
        assert!(serde_json::from_str::<Slug>("\"Bad\"").is_err());
        assert_eq!(
            serde_json::from_str::<Slug>("\"ok\"").unwrap().as_str(),
            "ok"
        );
    }

    #[test]
    fn enums_round_trip_their_spelling() {
        for r in TeamRole::ALL {
            assert_eq!(r.as_str().parse::<TeamRole>().unwrap(), *r);
            assert_eq!(
                serde_json::to_string(r).unwrap(),
                format!("\"{}\"", r.as_str())
            );
        }
        for r in InstanceRole::ALL {
            assert_eq!(r.as_str().parse::<InstanceRole>().unwrap(), *r);
        }
        assert_eq!("local".parse::<Origin>().unwrap(), Origin::Local);
        assert!("ldap".parse::<Origin>().is_err());
    }

    #[test]
    fn ids_are_v7() {
        let a = UserId::new();
        assert_eq!(a.0.get_version_num(), 7);
        assert_eq!(a.to_string().parse::<UserId>().unwrap(), a);
    }
}
