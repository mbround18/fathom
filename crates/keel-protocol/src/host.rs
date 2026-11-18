//! Hosts, enrollment and the SSH CAs (spec 055 US7, contracts/protocol.md "Hosts and profiles",
//! "SSH CA"; contracts/runner.md).

use std::fmt;

use keel_core::uuid::Uuid;
use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

use crate::governance::{ProfileName, bounded_list};
use crate::types::checked_string;

/// Most profiles one enrollment may assign.
pub const MAX_HOST_PROFILES: usize = 32;

checked_string!(
    /// A host's inventory name (Ansible `inventory_hostname`): 1 to 253 characters of `A-Z`,
    /// `a-z`, `0-9`, `.`, `_`, `-`, starting with a letter or digit.
    HostName,
    "host name",
    |v| {
        let b = v.as_bytes();
        if b.is_empty() || b.len() > 253 {
            return Err("length");
        }
        if !b[0].is_ascii_alphanumeric() {
            return Err("must start with a letter or digit");
        }
        if b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-')) {
            Ok(())
        } else {
            Err("bad character")
        }
    }
);

impl fmt::Debug for HostName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HostName({:?})", self.0)
    }
}

bounded_list!(
    /// The profiles an enrollment assigns (at most 32, no duplicates).
    HostProfiles,
    ProfileName,
    MAX_HOST_PROFILES
);

/// A host's connection state (data-model "hosts").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum HostStatus {
    Online,
    Offline,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SshCaKind {
    /// Signs Keel's listener host certificate; runners pin it.
    Host,
    /// Signs runner certificates.
    Runner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SshCaState {
    Active,
    Retiring,
    Retired,
}

/// An enrolment token's state, derived in this order: used, revoked, burned, expired, pending
/// (spec 060 research R4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum EnrollmentState {
    Pending,
    Used,
    Revoked,
    Burned,
    Expired,
}

/// An enrolment token, without the token (spec 060, US3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct EnrollmentInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    #[ts(type = "string")]
    pub workspace: Uuid,
    pub workspace_slug: String,
    pub inventory_name: String,
    pub profiles: Vec<String>,
    pub state: EnrollmentState,
    /// The creator's display name, while the user exists.
    pub created_by: Option<String>,
    /// RFC 3339 UTC.
    pub created_at: String,
    pub expires_at: String,
    /// The host that used the token.
    #[ts(type = "string | null")]
    pub host: Option<Uuid>,
    /// The token this one replaced, when it was rolled.
    #[ts(type = "string | null")]
    pub rolled_from: Option<Uuid>,
    /// The host is converged to its profiles once it enrolls (spec 068 US2).
    pub auto_converge: bool,
}

/// A one-time enrollment token, `keel-enroll-v1.<secret>.<host CA fingerprint>`. Shown once;
/// Keel keeps only its hash. Never logged.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(transparent)]
pub struct EnrollToken(pub String);

impl fmt::Debug for EnrollToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EnrollToken(..)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct HostInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    #[ts(type = "string")]
    pub workspace: Uuid,
    pub inventory_name: String,
    pub state: HostStatus,
    /// Profiles Keel assigned.
    pub profiles: Vec<String>,
    /// The host's own ceiling (`keel_runner_allowed_profiles`).
    pub allowed_profiles: Vec<String>,
    pub require_signed: bool,
    pub runner_version: Option<String>,
    pub ansible_version: Option<String>,
    pub enrolled_at: String,
    pub last_seen_at: Option<String>,
    pub revoked_at: Option<String>,
    /// The host reported an `ansible-playbook` run outside Keel (not `--check`) that no baseline
    /// approval or later run of its jobs has covered yet (spec 058 amendment A2).
    pub changed_outside: bool,
    /// The age public key its runner last reported (spec 066).
    pub age_recipient: Option<String>,
    /// Whether the primary repository's inventory lists it yet (spec 066).
    pub inventory_status: crate::repository::HostInventoryStatus,
    /// Its zone; `None` is the implicit `direct` zone (spec 067).
    #[serde(default)]
    #[ts(type = "string | null")]
    pub zone: Option<Uuid>,
    /// Derived reach (spec 067, data-model.md).
    #[serde(default)]
    pub reach: crate::relay::HostReach,
    /// The host's current converge, if one is open (spec 068).
    #[serde(default)]
    pub converge: Option<crate::converge::ConvergeHostSummary>,
    /// Open profile suggestions from the inventory's `groups:` (spec 068).
    #[serde(default)]
    pub suggestions: u16,
}

/// One run outside Keel, as the host reported it: the host's claim, not something Keel checked
/// (spec 058 amendment A2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct OutsideRunInfo {
    /// When the host says it ran.
    pub at: String,
    pub user: String,
    pub playbook: String,
    /// 40 hex characters, or `None` when the checkout had no commit.
    pub commit: Option<String>,
    pub dirty: bool,
    /// `--check`: a dry run.
    pub check: bool,
    /// When Keel received it.
    pub received_at: String,
}

/// A runner certificate Keel issued to a host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct HostCertInfo {
    /// The u64 serial in decimal (JSON numbers lose precision past 2^53).
    pub serial: String,
    pub principals: Vec<String>,
    pub valid_after: String,
    pub valid_before: String,
    pub revoked_at: Option<String>,
}

/// Reply to `GetHost`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct HostDetail {
    pub host: HostInfo,
    pub python_version: Option<String>,
    pub override_quorum: Option<u32>,
    pub ledger_head: Option<String>,
    /// Newest first.
    pub certs: Vec<HostCertInfo>,
    /// The newest 20 runs outside Keel the host reported, newest first.
    pub outside_runs: Vec<OutsideRunInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SshCaInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    pub kind: SshCaKind,
    pub state: SshCaState,
    /// `SHA256:…`.
    pub fingerprint: String,
    /// `ssh-ed25519 AAAA… keel-<kind>-ca`.
    pub public_key: String,
    /// A retiring CA is trusted until then.
    pub not_after: Option<String>,
    pub created_at: String,
}

// ---- join tokens and join requests (contracts/join.md) -------------------------------------

/// The longest lifetime a join token may be issued with, in days.
pub const MAX_JOIN_TOKEN_DAYS: u16 = 365;
/// The lifetime a join token is issued with unless the operator picks another, in days.
pub const DEFAULT_JOIN_TOKEN_DAYS: u16 = 30;

checked_string!(
    /// An operator's note on a rejected join request: at most 500 characters, no control
    /// characters but newlines.
    JoinComment,
    "join comment",
    |v| {
        if v.chars().count() > 500 {
            return Err("length");
        }
        if v.chars().any(|c| c.is_control() && c != '\n') {
            return Err("control character");
        }
        Ok(())
    }
);

impl fmt::Debug for JoinComment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JoinComment({:?})", self.0)
    }
}

/// Where a join request stands (data-model "join_requests").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum JoinRequestState {
    Pending,
    Accepted,
    Rejected,
    Expired,
}

/// A host knocking with a join token. Everything but `fingerprint` is the host's own claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct JoinRequestInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    #[ts(type = "string")]
    pub workspace: Uuid,
    /// `SHA256:…` of the host key: compare it with `keel-runner fingerprint` on the host.
    pub fingerprint: String,
    /// The inventory name the host suggests (its `policy.toml`, or the token's).
    pub inventory_name: String,
    /// The host's own ceiling.
    pub allowed_profiles: Vec<String>,
    /// Profiles the token suggests, within `allowed_profiles`: the accept form's prefill.
    pub suggested_profiles: Vec<String>,
    pub require_signed: bool,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub runner_version: String,
    /// Where the request came from: an address, or the relay that carried it.
    pub source: String,
    pub state: JoinRequestState,
    /// Why it was rejected or expired (`rejected`, `token_rotated`, `expired`, …).
    pub reason: Option<String>,
    /// The operator's note on a rejection.
    pub comment: Option<String>,
    /// The join token it came with.
    pub jti: String,
    pub first_seen_at: String,
    pub last_seen_at: String,
    /// The operator who accepted or rejected it, while the user exists.
    pub decided_by: Option<String>,
    pub decided_at: Option<String>,
    /// The host an accepted request created.
    #[ts(type = "string | null")]
    pub host: Option<Uuid>,
}

/// A workspace's join token, without the token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct JoinTokenInfo {
    #[ts(type = "string")]
    pub workspace: Uuid,
    /// Whether the workspace has an active token. None is the default.
    pub active: bool,
    pub jti: Option<String>,
    pub expires_at: Option<String>,
    pub created_at: Option<String>,
    /// The issuer's display name, while the user exists.
    pub created_by: Option<String>,
    pub inventory_name: Option<String>,
    pub profiles: Vec<String>,
}

/// A join token, `<header>.<claims>.<signature>`. Shown once on issue or rotate; Keel keeps
/// only its `jti` and hash. Never logged.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(transparent)]
pub struct JoinTokenSecret(pub String);

impl fmt::Debug for JoinTokenSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JoinTokenSecret(..)")
    }
}
