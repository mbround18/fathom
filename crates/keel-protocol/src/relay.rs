//! Zones, relays, runner paths and host queries (spec 067, contracts/protocol.md).

use keel_core::uuid::Uuid;
use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

use crate::types::checked_string;

/// The longest zone description, in characters.
pub const MAX_ZONE_DESCRIPTION_CHARS: usize = 500;

checked_string!(
    /// A zone or relay name: `^[a-z0-9][a-z0-9-]{0,62}$`, and not `direct` (the implicit zone of
    /// hosts with no zone).
    ZoneName,
    "zone name",
    |v| {
        let b = v.as_bytes();
        if b.is_empty() || b.len() > 63 {
            return Err("length");
        }
        if !(b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) {
            return Err("must start with a lowercase letter or digit");
        }
        if !b
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
        {
            return Err("only a-z, 0-9 and -");
        }
        if v == "direct" {
            return Err("`direct` is reserved");
        }
        Ok(())
    }
);

impl std::fmt::Debug for ZoneName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ZoneName({:?})", self.0)
    }
}

checked_string!(
    /// A zone's description: at most 500 characters.
    ZoneDescription,
    "zone description",
    |v| {
        if v.chars().count() > MAX_ZONE_DESCRIPTION_CHARS {
            Err("longer than 500 characters")
        } else {
            Ok(())
        }
    }
);

impl std::fmt::Debug for ZoneDescription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ZoneDescription({:?})", self.0)
    }
}

/// A zone: hosts that sit behind the same relays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ZoneInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    #[ts(type = "string")]
    pub workspace: Uuid,
    pub name: ZoneName,
    pub description: String,
    /// Hosts in the zone.
    pub hosts: u32,
    /// Relays serving the zone, revoked ones excluded.
    pub relays: u32,
}

/// A relay's state as Keel sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RelayState {
    Connecting,
    Connected,
    Unreachable,
    /// Keel stopped dialling it after a protocol violation; an owner release clears it.
    Quarantined,
    Revoked,
}

/// A relay Keel dials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct RelayInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    #[ts(type = "string")]
    pub workspace: Uuid,
    #[ts(type = "string")]
    pub zone: Uuid,
    pub name: ZoneName,
    pub dial_address: String,
    /// `SHA256:…` of the pinned host key.
    pub host_key_fingerprint: String,
    /// The enrolled host the relay runs on, if any.
    #[ts(type = "string | null")]
    pub host: Option<Uuid>,
    pub state: RelayState,
    /// When the state began.
    pub since: Option<String>,
    pub rtt_ms: Option<u32>,
    pub version: Option<String>,
    /// Runner streams it carries.
    pub streams: Option<u32>,
    pub last_error: Option<String>,
    pub revoked_at: Option<String>,
}

/// How a runner's session reaches Keel. The relay-claimed peer is advisory and not exposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunnerPath {
    Direct,
    Relay {
        #[ts(type = "string")]
        relay: Uuid,
    },
}

/// A host's derived reach (data-model.md, "Derived reach").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostReach {
    Connected {
        path: RunnerPath,
    },
    /// Not connected, but a path exists: the zone is direct or one of its relays is connected.
    #[default]
    Disconnected,
    /// Not connected, and every non-revoked relay of its zone is unreachable.
    RelayUnreachable {
        #[ts(type = "Array<string>")]
        relays: Vec<Uuid>,
    },
    /// Its last session came over a relay that is now revoked.
    RelayRevoked {
        #[ts(type = "string")]
        relay: Uuid,
    },
    /// Its zone has no relay left.
    NoPath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RelayLinkKeyState {
    Pending,
    Active,
    Retired,
}

/// Keel's key for dialling relays. Public half only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct RelayLinkKey {
    /// `SHA256:…`.
    pub fingerprint: String,
    /// The `ssh-ed25519 AAAA… keel-relay-link` line a relay pins.
    pub public_line: String,
    pub state: RelayLinkKeyState,
}

/// A read-only question to a host's runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum QueryKind {
    Facts,
    AppliedRevision,
    UnitState,
    DiskHealth,
    PathFree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum QueryOutcome {
    Answered,
    Refused,
    Timeout,
    NoCapability,
    Disconnected,
}

/// One host query and, once it has one, its outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct HostQuery {
    #[ts(type = "string")]
    pub id: Uuid,
    #[ts(type = "string")]
    pub host: Uuid,
    pub kind: QueryKind,
    #[ts(type = "unknown")]
    pub params: serde_json::Value,
    pub asked_by: String,
    pub asked_at: String,
    pub outcome: Option<QueryOutcome>,
    #[ts(type = "unknown")]
    pub answer: Option<serde_json::Value>,
    pub refusal: Option<String>,
    pub answered_at: Option<String>,
}
