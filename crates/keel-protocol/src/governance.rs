//! Workspaces, projects, teams, grants, users and API tokens (spec 055 US6, T035).
//!
//! Every name a client sends is a checked newtype, so the server never sees an over-long or
//! malformed one. Ids travel as UUID strings.

use std::fmt;

use keel_core::uuid::Uuid;
use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

use crate::types::{InstanceRole, TeamRole, checked_string};

/// Longest display name (workspace, project, team, token) in characters.
pub const MAX_LABEL_CHARS: usize = 64;
/// Most profiles one grant may name.
pub const MAX_GRANT_PROFILES: usize = 32;
/// Most actions one API token may name.
pub const MAX_TOKEN_ACTIONS: usize = 64;
/// Longest an API token may live, in days.
pub const MAX_TOKEN_DAYS: u16 = 366;

checked_string!(
    /// A workspace or project slug: `^[a-z0-9][a-z0-9-]{0,62}$`.
    Slug,
    "slug",
    |v| {
        if keel_core::Slug::parse(v).is_ok() {
            Ok(())
        } else {
            Err("must match ^[a-z0-9][a-z0-9-]{0,62}$")
        }
    }
);

impl fmt::Debug for Slug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Slug({:?})", self.0)
    }
}

checked_string!(
    /// A human name (workspace, project, team or token): 1 to 64 characters, trimmed, no
    /// control characters.
    Label,
    "name",
    |v| {
        let n = v.chars().count();
        if n == 0 || n > MAX_LABEL_CHARS {
            return Err("must be 1 to 64 characters");
        }
        if v.trim() != v {
            return Err("leading or trailing space");
        }
        if v.chars().any(char::is_control) { Err("control character") } else { Ok(()) }
    }
);

impl fmt::Debug for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Label({:?})", self.0)
    }
}

checked_string!(
    /// A host profile name a grant is scoped to: `^[a-z0-9][a-z0-9_-]{0,62}$`.
    ProfileName,
    "profile",
    |v| {
        let b = v.as_bytes();
        let ok = !b.is_empty()
            && b.len() <= 63
            && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
            && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-' || *c == b'_');
        if ok { Ok(()) } else { Err("must match ^[a-z0-9][a-z0-9_-]{0,62}$") }
    }
);

impl fmt::Debug for ProfileName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ProfileName({:?})", self.0)
    }
}

checked_string!(
    /// A command name an API token may run (`Cmd::name`): `^[a-z][a-z0-9_]{0,63}$`.
    ActionName,
    "action",
    |v| {
        let b = v.as_bytes();
        let ok = !b.is_empty()
            && b.len() <= 64
            && b[0].is_ascii_lowercase()
            && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_');
        if ok { Ok(()) } else { Err("must match ^[a-z][a-z0-9_]{0,63}$") }
    }
);

impl fmt::Debug for ActionName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ActionName({:?})", self.0)
    }
}

macro_rules! bounded_list {
    ($(#[$doc:meta])* $name:ident, $item:ty, $max:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
        #[serde(transparent)]
        pub struct $name(pub Vec<$item>);

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let v = Vec::<$item>::deserialize(d)?;
                if v.len() > $max {
                    return Err(serde::de::Error::custom(format!("at most {} items", $max)));
                }
                for (i, item) in v.iter().enumerate() {
                    if v[..i].contains(item) {
                        return Err(serde::de::Error::custom("duplicate item"));
                    }
                }
                Ok(Self(v))
            }
        }
    };
}

bounded_list!(
    /// The profiles a grant is scoped to (at most 32, no duplicates); empty means the whole
    /// workspace.
    GrantProfiles,
    ProfileName,
    MAX_GRANT_PROFILES
);

bounded_list!(
    /// The commands an API token may run (at most 64, no duplicates).
    TokenActions,
    ActionName,
    MAX_TOKEN_ACTIONS
);

/// A project: groups bindings and jobs inside a workspace, for display only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ProjectInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    pub slug: String,
    pub name: String,
}

/// A workspace: the authorization boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct WorkspaceInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    /// RFC 3339 UTC.
    pub created: String,
    pub archived: bool,
    pub projects: Vec<ProjectInfo>,
}

/// A team member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TeamMemberInfo {
    #[ts(type = "string")]
    pub user: Uuid,
    pub display_name: String,
}

/// A team's role on a workspace, optionally scoped to host profiles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct GrantInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    #[ts(type = "string")]
    pub workspace: Uuid,
    pub workspace_slug: String,
    pub role: TeamRole,
    /// Empty: the whole workspace.
    pub profiles: Vec<String>,
}

/// A team, with its members and grants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TeamInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    pub name: String,
    pub members: Vec<TeamMemberInfo>,
    pub grants: Vec<GrantInfo>,
}

/// A user, as instance administrators see them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct UserInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    pub display_name: String,
    pub email: Option<String>,
    pub instance_role: InstanceRole,
    /// Removed users stay listed (the audit log names them) but cannot sign in.
    pub disabled: bool,
    /// Names of the teams the user is in.
    pub teams: Vec<String>,
    /// RFC 3339 UTC.
    pub created: String,
}

/// An API token, without its secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ApiTokenInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    pub name: String,
    #[ts(type = "string")]
    pub owner: Uuid,
    #[ts(type = "string")]
    pub workspace: Uuid,
    pub workspace_slug: String,
    pub actions: Vec<String>,
    /// RFC 3339 UTC.
    pub expires_at: String,
    pub last_used_at: Option<String>,
    pub revoked: bool,
}

/// An API token's secret, shown once on creation. Never logged.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(transparent)]
pub struct TokenSecret(pub String);

impl fmt::Debug for TokenSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TokenSecret(..)")
    }
}
