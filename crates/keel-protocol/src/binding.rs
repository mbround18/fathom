//! Repository bindings, receivers, deliveries and credentials (spec 055 US1, contracts/protocol.md
//! "Bindings", "Receivers", "Credentials").

use std::fmt;

use keel_core::uuid::Uuid;
use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

use crate::types::checked_string;

/// Longest repository URL accepted.
pub const MAX_URL_BYTES: usize = 2048;
/// Longest pinned `known_hosts` line accepted.
pub const MAX_KNOWN_HOST_BYTES: usize = 8 * 1024;
/// Largest credential value accepted.
pub const MAX_SECRET_BYTES: usize = 64 * 1024;

fn printable_line(v: &str) -> bool {
    v.chars().all(|c| !c.is_control())
}

checked_string!(
    /// A repository URL as the operator typed it (`https://…`, `ssh://…` or `git@host:path`).
    /// The git adapter decides which schemes it fetches.
    RepoUrl,
    "repository url",
    |v| {
        if v.is_empty() || v.len() > MAX_URL_BYTES {
            return Err("length");
        }
        if v.chars().any(|c| c.is_whitespace() || c.is_control()) || v.starts_with('-') {
            return Err("bad character");
        }
        Ok(())
    }
);

impl fmt::Debug for RepoUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RepoUrl({:?})", self.0)
    }
}

checked_string!(
    /// A branch name, without `refs/heads/`.
    BranchName,
    "branch",
    |v| {
        if v.is_empty() || v.len() > 255 {
            return Err("length");
        }
        if v.chars().any(|c| c.is_whitespace() || c.is_control()) || v.starts_with('-') {
            return Err("bad character");
        }
        Ok(())
    }
);

impl fmt::Debug for BranchName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BranchName({:?})", self.0)
    }
}

checked_string!(
    /// The pinned host key, as one `known_hosts` line (`host key-type base64`).
    KnownHost,
    "known host",
    |v| {
        if v.is_empty() || v.len() > MAX_KNOWN_HOST_BYTES {
            return Err("length");
        }
        if printable_line(v) { Ok(()) } else { Err("must be one line") }
    }
);

impl fmt::Debug for KnownHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KnownHost({:?})", self.0)
    }
}

checked_string!(
    /// A receiver, credential or profile name: `a-z`, `0-9`, `-`, `_`, at most 63.
    ObjectName,
    "name",
    |v| {
        let b = v.as_bytes();
        if b.is_empty() || b.len() > 63 {
            return Err("length");
        }
        if !(b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) {
            return Err("must start with a letter or digit");
        }
        if b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-' || *c == b'_') {
            Ok(())
        } else {
            Err("bad character")
        }
    }
);

impl fmt::Debug for ObjectName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ObjectName({:?})", self.0)
    }
}

checked_string!(
    /// An RFC 3339 instant as a filter bound; the server parses it.
    Instant,
    "instant",
    |v| {
        if v.is_empty() || v.len() > 40 {
            return Err("length");
        }
        if v.bytes().all(|b| b.is_ascii_graphic()) { Ok(()) } else { Err("bad character") }
    }
);

impl fmt::Debug for Instant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Instant({:?})", self.0)
    }
}

checked_string!(
    /// A credential value on its way in. Sealed at rest, never returned, never logged.
    SecretValue,
    "credential value",
    |v| {
        if v.is_empty() || v.len() > MAX_SECRET_BYTES {
            return Err("length");
        }
        Ok(())
    }
);

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue(..)")
    }
}

/// How Keel reaches the repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Https,
    Ssh,
}

/// `active | disabled`, for bindings and receivers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ActiveStatus {
    Active,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReceiverProvider {
    Github,
    Gitlab,
    Forgejo,
    Generic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    VaultPassword,
    Env,
    SecretVars,
    GitHttpsToken,
    GitSshKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryDecision {
    Accepted,
    Refused,
}

/// Why a delivery was refused (data-model `deliveries.refusal`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryRefusal {
    BadSignature,
    UnknownReceiver,
    ReceiverDisabled,
    NoMatchingBinding,
    BindingDisabled,
    RefNotBranch,
    PayloadInvalid,
}

/// What came of fetching after a delivery or a sync (data-model `deliveries.sync_state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    Syncing,
    Synced,
    Duplicate,
    Stale,
    HistoryRewritten,
    SyncFailed,
}

/// `PreviewBindingCreate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BindingSpec {
    #[ts(type = "string")]
    pub project: Uuid,
    pub url: RepoUrl,
    pub branch: BranchName,
    pub transport: Transport,
    /// A `git_https_token` or `git_ssh_key` credential in the same workspace.
    #[serde(default)]
    #[ts(type = "string | null")]
    pub credential: Option<Uuid>,
    /// Required for SSH.
    #[serde(default)]
    pub known_host: Option<KnownHost>,
    /// Seconds between polls; `None` means webhook (or manual) only.
    #[serde(default)]
    pub poll_interval: Option<u32>,
    pub allow_force_push: bool,
    pub signed_mode: bool,
    pub override_quorum: u32,
}

/// `PreviewBindingUpdate`: the binding's mutable settings, replaced whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BindingUpdate {
    #[ts(type = "string")]
    pub binding: Uuid,
    pub branch: BranchName,
    #[serde(default)]
    #[ts(type = "string | null")]
    pub credential: Option<Uuid>,
    #[serde(default)]
    pub known_host: Option<KnownHost>,
    #[serde(default)]
    pub poll_interval: Option<u32>,
    pub allow_force_push: bool,
    pub override_quorum: u32,
}

/// One manifest problem, as the binding shows it under `manifest_invalid`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ManifestProblem {
    pub at: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct BindingInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    #[ts(type = "string")]
    pub project: Uuid,
    #[ts(type = "string")]
    pub workspace: Uuid,
    pub url: String,
    pub branch: String,
    pub transport: Transport,
    #[ts(type = "string | null")]
    pub credential: Option<Uuid>,
    pub known_host: Option<String>,
    pub poll_interval: Option<u32>,
    pub allow_force_push: bool,
    pub signed_mode: bool,
    pub override_quorum: u32,
    pub status: ActiveStatus,
    pub last_commit: Option<String>,
    /// RFC 3339 UTC.
    pub last_synced_at: Option<String>,
    /// A stable code (`manifest_invalid`, `sync_failed`, `history_rewritten`, ...).
    pub last_sync_error: Option<String>,
    pub manifest_problems: Vec<ManifestProblem>,
}

/// A job as the binding page lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct JobSummary {
    pub name: String,
    pub playbook: String,
    pub profiles: Vec<String>,
    pub inventory: String,
    /// `credential_unavailable` when a named credential is not granted.
    pub blocked: Option<String>,
    pub defined_at_commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ReceiverInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    #[ts(type = "string")]
    pub workspace: Uuid,
    pub name: String,
    pub provider: ReceiverProvider,
    pub status: ActiveStatus,
    /// The URL to configure at the source-control host.
    pub url: String,
    /// While set and in the future, the previous secret still verifies.
    pub previous_expires_at: Option<String>,
    #[ts(type = "Array<string>")]
    pub bindings: Vec<Uuid>,
    pub created_at: String,
    pub rotated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DeliveryInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    #[ts(type = "string")]
    pub receiver: Uuid,
    #[ts(type = "string | null")]
    pub binding: Option<Uuid>,
    /// The provider's delivery id.
    pub provider_delivery_id: String,
    pub received_at: String,
    pub decision: DeliveryDecision,
    pub refusal: Option<DeliveryRefusal>,
    pub sync_state: Option<SyncState>,
    pub commit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CredentialGrantInfo {
    #[ts(type = "string")]
    pub binding: Uuid,
    /// Empty: every profile.
    pub profiles: Vec<String>,
}

/// A credential, without its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CredentialInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    #[ts(type = "string")]
    pub workspace: Uuid,
    pub name: String,
    pub kind: CredentialKind,
    /// Truncated SHA-256 of the value, so operators can tell values apart.
    pub fingerprint: String,
    pub created_at: String,
    pub rotated_at: Option<String>,
    pub grants: Vec<CredentialGrantInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_strings() {
        assert!(RepoUrl::new("https://git.example.com/ops/infra.git").is_ok());
        assert!(RepoUrl::new("-uhttps://x").is_err());
        assert!(RepoUrl::new("https://x/a b").is_err());
        assert!(BranchName::new("main").is_ok());
        assert!(BranchName::new("--upload-pack=x").is_err());
        assert!(KnownHost::new("git.example.com ssh-ed25519 AAAA").is_ok());
        assert!(KnownHost::new("a\nb").is_err());
        assert!(ObjectName::new("prod-vault").is_ok());
        assert!(ObjectName::new("Prod").is_err());
        assert!(SecretValue::new("").is_err());
        assert_eq!(
            format!("{:?}", SecretValue::new("hunter2").unwrap()),
            "SecretValue(..)"
        );
    }
}
