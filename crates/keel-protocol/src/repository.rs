//! A workspace's primary repository: its catalog, job drafts, the forge connection and the
//! changes Keel proposes to it (spec 066, contracts/protocol.md).
//!
//! The catalog types mirror `keel_core::catalog::Catalog`, which the server stores as JSON. They
//! never carry a sops value: only file paths, key paths and recipients (FR-015).

use std::fmt;

use keel_core::uuid::Uuid;
use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

use crate::binding::{BranchName, KnownHost, RepoUrl, Transport};
use crate::governance::bounded_list;
use crate::types::checked_string;

/// Most inventory groups one `PreviewHostAdd` may name.
pub const MAX_HOST_GROUPS: usize = 16;
/// Longest forge API base URL accepted.
pub const MAX_API_BASE_BYTES: usize = 2048;

checked_string!(
    /// An Ansible inventory group name: 1 to 255 bytes of ASCII letters, digits, `_`, `-` and
    /// `.`, starting with a letter, a digit or `_`.
    GroupName,
    "group name",
    |v| {
        let b = v.as_bytes();
        if b.is_empty() || b.len() > 255 {
            return Err("length");
        }
        if !(b[0].is_ascii_alphanumeric() || b[0] == b'_') {
            return Err("must start with a letter, a digit or _");
        }
        if b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.')) {
            Ok(())
        } else {
            Err("bad character")
        }
    }
);

impl fmt::Debug for GroupName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GroupName({:?})", self.0)
    }
}

bounded_list!(
    /// The inventory groups a new host joins (at most 16, no duplicates).
    HostGroups,
    GroupName,
    MAX_HOST_GROUPS
);

checked_string!(
    /// A forge API base URL (`https://git.example/api/v1`): `http://` or `https://`, at most
    /// 2048 bytes, no whitespace.
    ApiBase,
    "api base",
    |v| {
        if v.len() < 8 || v.len() > MAX_API_BASE_BYTES {
            return Err("length");
        }
        if !(v.starts_with("https://") || v.starts_with("http://")) {
            return Err("must start with https:// or http://");
        }
        if v.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err("bad character");
        }
        Ok(())
    }
);

impl fmt::Debug for ApiBase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ApiBase({:?})", self.0)
    }
}

/// `PreviewWorkspaceRepository`: a `BindingSpec` without `project`, since the binding goes on the
/// workspace's `default` project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RepositorySpec {
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

/// Which forge API Keel speaks to open pull requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ForgeProvider {
    Forgejo,
    Gitea,
    Github,
}

/// A workspace's forge connection. The token itself is a `forge_token` credential and never
/// travels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ForgeConnectionInfo {
    #[ts(type = "string")]
    pub workspace: Uuid,
    pub provider: ForgeProvider,
    pub api_base: String,
    #[ts(type = "string")]
    pub credential: Uuid,
    /// RFC 3339 UTC of the last probe.
    pub probed_at: Option<String>,
    /// What the last probe refused with, if it did.
    pub probe_error: Option<String>,
    /// RFC 3339 UTC.
    pub created_at: String,
}

/// What came of a change Keel proposed to the repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChangeOutcome {
    /// A pull request is open on a new branch.
    Opened { url: String, branch: String },
    /// No forge (or writes are off, or the layout is unsupported): apply this diff by hand.
    Manual { diff: String, reason: String },
    /// The forge refused; the diff is here to apply by hand.
    Failed { diff: String, error: String },
}

/// The enrolment `PreviewHostAdd` issued: the same fields as `Event::HostEnrollment`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct EnrollmentIssued {
    #[ts(type = "string")]
    pub id: Uuid,
    #[ts(type = "string")]
    pub workspace: Uuid,
    pub inventory_name: String,
    pub profiles: Vec<String>,
    pub expires_at: String,
    pub token: crate::host::EnrollToken,
    /// The enrolment template rendered with this token.
    pub snippet: String,
    /// The enrolment instructions, raw Markdown; empty when unset.
    pub instructions: String,
    /// The settings could not be read, so the defaults were used.
    pub instructions_unavailable: bool,
}

/// Whether the primary catalog's inventories list a host (data-model "host inventory status").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum HostInventoryStatus {
    Listed,
    PendingInventory,
    /// There is no primary catalog.
    Unknown,
}

/// Reply to `GetCatalog`: the stored catalog plus decrypt state computed at read time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CatalogView {
    #[ts(type = "string")]
    pub binding: Uuid,
    /// The commit the catalog was built from.
    pub commit: String,
    /// RFC 3339 UTC.
    pub built_at: String,
    /// The last build failed; this is the last good catalog.
    pub stale: bool,
    pub error: Option<String>,
    pub playbooks: Vec<CatalogPlaybook>,
    pub roles: Vec<CatalogRole>,
    pub inventories: Vec<CatalogInventory>,
    pub secrets: CatalogSecrets,
    pub problems: Vec<CatalogProblem>,
    pub skipped: CatalogSkipped,
    /// Every host in the workspace, with what its age key can decrypt.
    pub hosts: Vec<HostDecrypt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CatalogPlaybook {
    pub path: String,
    pub plays: Vec<CatalogPlay>,
    /// Roles the playbook reaches, directly or through dependencies.
    pub roles: Vec<String>,
    pub imports: Vec<String>,
    pub vars_files: Vec<String>,
    /// References Keel could not follow statically (templated names, for example).
    pub unresolved: Vec<String>,
    /// Declared jobs that run this playbook.
    pub jobs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CatalogPlay {
    pub name: Option<String>,
    /// The play's `hosts:` pattern.
    pub hosts: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CatalogRole {
    pub name: String,
    pub path: String,
    pub description: Option<String>,
    /// Variable names from `defaults/main.yml`; never their values.
    pub defaults: Vec<String>,
    pub dependencies: Vec<String>,
    pub reached_by: Vec<String>,
    /// A collection role, listed and not followed.
    pub external: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CatalogInventoryKind {
    Yaml,
    Ini,
    Dir,
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CatalogInventory {
    pub path: String,
    pub kind: CatalogInventoryKind,
    pub groups: Vec<CatalogGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CatalogGroup {
    pub name: String,
    pub parent: Option<String>,
    pub hosts: Vec<String>,
    pub children: Vec<String>,
    pub vars_file: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CatalogSecrets {
    /// `.sops.yaml`, when there is one.
    pub rules_file: Option<String>,
    pub rules: Vec<SopsRule>,
    pub files: Vec<SopsFile>,
}

/// One `.sops.yaml` creation rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SopsRule {
    pub index: u32,
    pub path_regex: String,
    pub age: Vec<String>,
    pub pgp: Vec<String>,
}

/// One sops-encrypted file: metadata and key paths only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SopsFile {
    pub path: String,
    /// The index of the creation rule that covers it, first match.
    pub rule: Option<u32>,
    pub age: Vec<String>,
    pub pgp: Vec<String>,
    pub lastmodified: Option<String>,
    /// Dotted key paths; never values.
    pub keys: Vec<String>,
    /// Recipients that differ from the rule's; `None` when there is no rule to compare with.
    pub drift: Option<SopsDrift>,
    pub problem: Option<String>,
    /// Declared jobs that read this file.
    pub read_by: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SopsDrift {
    /// In the rule, not in the file: `sops updatekeys` would add them.
    pub missing: Vec<String>,
    /// In the file, not in the rule.
    pub extra: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CatalogProblem {
    pub path: String,
    pub line: Option<u32>,
    pub message: String,
}

/// What the build left out for size.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CatalogSkipped {
    pub files: u32,
    #[ts(type = "number")]
    pub bytes: u64,
    pub reason: Option<String>,
}

/// One host's decrypt state, computed when the catalog is read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct HostDecrypt {
    #[ts(type = "string")]
    pub host: Uuid,
    pub inventory_name: String,
    /// The age public key its runner last reported.
    pub age_recipient: Option<String>,
    /// Paths of the sops files its key is a recipient of.
    pub decrypts: Vec<String>,
}
