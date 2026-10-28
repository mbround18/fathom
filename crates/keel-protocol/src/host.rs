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
