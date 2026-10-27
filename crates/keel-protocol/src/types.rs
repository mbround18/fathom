//! Shared wire types.
//!
//! Everything a client sends is parsed from a hostile network: structs deny unknown fields, enums
//! are tagged, and client strings are bounded, checked newtypes.

use std::fmt;

use keel_core::uuid::Uuid;
use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

/// Longest CSRF token accepted.
pub const MAX_CSRF_TOKEN_BYTES: usize = 128;
/// Longest client name in `Hello`.
pub const MAX_CLIENT_BYTES: usize = 64;
/// Longest base64url blob (a WebAuthn field) accepted.
pub const MAX_B64URL_BYTES: usize = 16 * 1024;

fn is_b64url(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

macro_rules! checked_string {
    ($(#[$doc:meta])* $name:ident, $what:literal, |$v:ident| $check:expr) => {
        $(#[$doc])*
        #[derive(Clone, PartialEq, Eq, Hash, Serialize, TS)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Checks `value`.
            pub fn new(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                let check = |$v: &str| -> Result<(), &'static str> { $check };
                match check(&value) {
                    Ok(()) => Ok(Self(value)),
                    Err(why) => Err(format!("invalid {}: {why}", $what)),
                }
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                Self::new(String::deserialize(d)?).map_err(serde::de::Error::custom)
            }
        }
    };
}
pub(crate) use checked_string;

checked_string!(
    /// The session's CSRF token (URL-safe base64), at most 128 bytes. Never logged.
    CsrfToken,
    "csrf token",
    |v| {
        if v.is_empty() || v.len() > MAX_CSRF_TOKEN_BYTES {
            return Err("length");
        }
        if v.bytes().all(is_b64url) { Ok(()) } else { Err("bad character") }
    }
);

impl fmt::Debug for CsrfToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CsrfToken(..)")
    }
}

checked_string!(
    /// Free-form name of the client (`keel-web/0.1.0`), printable ASCII, at most 64 bytes.
    ClientName,
    "client name",
    |v| {
        if v.is_empty() || v.len() > MAX_CLIENT_BYTES {
            return Err("length");
        }
        if v.bytes().all(|b| b.is_ascii_graphic() || b == b' ') { Ok(()) } else { Err("bad character") }
    }
);

impl fmt::Debug for ClientName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ClientName({:?})", self.0)
    }
}

checked_string!(
    /// Unpadded base64url, at most 16 KiB (WebAuthn binary fields).
    B64Url,
    "base64url",
    |v| {
        if v.len() > MAX_B64URL_BYTES {
            return Err("too long");
        }
        if v.bytes().all(is_b64url) { Ok(()) } else { Err("bad character") }
    }
);

impl fmt::Debug for B64Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "B64Url({} bytes)", self.0.len())
    }
}

/// A command's id, echoed as `reply_to` on the events it produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(transparent)]
pub struct RequestId(#[ts(type = "number")] pub u64);

/// A pending two-step mutation. Single-use, bound to the session that made it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(transparent)]
pub struct PreviewId(#[ts(type = "string")] pub Uuid);

/// A user's instance-wide role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum InstanceRole {
    Owner,
    Admin,
    User,
}

impl From<keel_core::InstanceRole> for InstanceRole {
    fn from(r: keel_core::InstanceRole) -> Self {
        match r {
            keel_core::InstanceRole::Owner => Self::Owner,
            keel_core::InstanceRole::Admin => Self::Admin,
            keel_core::InstanceRole::User => Self::User,
        }
    }
}

/// A team's role on a workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TeamRole {
    Viewer,
    Operator,
    Approver,
    WsAdmin,
}

impl From<keel_core::TeamRole> for TeamRole {
    fn from(r: keel_core::TeamRole) -> Self {
        match r {
            keel_core::TeamRole::Viewer => Self::Viewer,
            keel_core::TeamRole::Operator => Self::Operator,
            keel_core::TeamRole::Approver => Self::Approver,
            keel_core::TeamRole::WsAdmin => Self::WsAdmin,
        }
    }
}

/// One of the user's effective grants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MeGrant {
    #[ts(type = "string")]
    pub workspace: Uuid,
    pub workspace_slug: String,
    pub role: TeamRole,
    /// Empty: the whole workspace.
    pub profiles: Vec<String>,
}

/// The signed-in user, as `GetMe` returns it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Me {
    #[ts(type = "string")]
    pub id: Uuid,
    pub display_name: String,
    pub email: Option<String>,
    pub instance_role: InstanceRole,
    pub grants: Vec<MeGrant>,
    pub verification: crate::passkey::Verification,
}

/// A two-step mutation awaiting `Confirm`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Preview {
    pub preview_id: PreviewId,
    /// The action name, as audited (`workspace_create`, `run_approve`, ...).
    pub action: String,
    /// One line for the confirm dialog.
    pub summary: String,
    /// Every affected object (hosts, bindings, users), human-readable.
    pub affected: Vec<String>,
    /// `Confirm` must carry a passkey assertion (unless the session verified within the window).
    pub step_up: bool,
    /// RFC 3339 UTC.
    pub expires_at: String,
}
