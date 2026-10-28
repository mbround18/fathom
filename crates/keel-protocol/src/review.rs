//! Signed-mode review, reviewed overrides and trusted signers (spec 055 US8,
//! contracts/protocol.md "Review (signed mode)" and "Signers", contracts/signed-mode.md).
//!
//! Keel's signature check is advisory: it decides whether a run needs review. Hosts verify the
//! commits and every override themselves, against their own pinned signers and approvers.

use std::fmt;

use keel_core::uuid::Uuid;
use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

use crate::governance::bounded_list;
use crate::host::HostName;
use crate::passkey::PasskeyRequest;
use crate::types::{PreviewId, checked_string};

/// The longest review comment, in characters.
pub const MAX_REVIEW_COMMENT_CHARS: usize = 2000;
/// The most hosts one baseline may name.
pub const MAX_BASELINE_HOSTS: usize = 256;
/// The longest public key accepted for a signer, in bytes.
pub const MAX_SIGNER_KEY_BYTES: usize = 65_536;
/// The longest repository path accepted, in bytes.
pub const MAX_REPO_PATH_BYTES: usize = 4096;

checked_string!(
    /// Why a reviewer overrides or denies: 1 to 2000 characters, not only spaces. Newlines and
    /// tabs are allowed; other control characters are not. Required (R20).
    ReviewComment,
    "comment",
    |v| {
        let n = v.chars().count();
        if n == 0 || n > MAX_REVIEW_COMMENT_CHARS {
            return Err("must be 1 to 2000 characters");
        }
        if v.trim().is_empty() {
            return Err("must not be blank");
        }
        if v.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
            return Err("control character");
        }
        Ok(())
    }
);

impl fmt::Debug for ReviewComment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ReviewComment({:?})", self.0)
    }
}

checked_string!(
    /// A full commit id: 40 (SHA-1) or 64 (SHA-256) lowercase hex digits.
    CommitSha,
    "commit",
    |v| {
        let ok = (v.len() == 40 || v.len() == 64)
            && v.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if ok { Ok(()) } else { Err("must be 40 or 64 lowercase hex digits") }
    }
);

impl fmt::Debug for CommitSha {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CommitSha({})", self.0)
    }
}

checked_string!(
    /// A path inside the repository: 1 to 4096 bytes, no control characters.
    RepoPath,
    "path",
    |v| {
        if v.is_empty() || v.len() > MAX_REPO_PATH_BYTES {
            return Err("length");
        }
        if v.chars().any(char::is_control) { Err("control character") } else { Ok(()) }
    }
);

impl fmt::Debug for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RepoPath({:?})", self.0)
    }
}

checked_string!(
    /// A signer's public key: one SSH `allowed_signers` key (`ssh-ed25519 AAAA…`) or one
    /// ASCII-armored OpenPGP public key block. 1 to 64 KiB. The server parses it.
    SignerKey,
    "public key",
    |v| {
        if v.trim().is_empty() || v.len() > MAX_SIGNER_KEY_BYTES {
            return Err("length");
        }
        Ok(())
    }
);

impl fmt::Debug for SignerKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SignerKey({} bytes)", self.0.len())
    }
}

bounded_list!(
    /// The hosts a baseline covers (1 to 256, no duplicates).
    BaselineHosts,
    HostName,
    MAX_BASELINE_HOSTS
);

/// Where a review group stands (data-model "review_groups").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    Open,
    Approved,
    Denied,
    Expired,
}

impl ReviewState {
    /// The stored name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Expired => "expired",
        }
    }
}

/// Why a group needs review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReviewReason {
    /// At least one new commit is not signed by a trusted signer.
    Unsigned,
    /// The target is not a descendant of the hosts' base.
    Downgrade,
    /// The hosts have no base yet: only a baseline approval can authorize them.
    Baseline,
    /// Keel could not run its check (missing objects, a store error). Fails closed.
    CheckFailed,
}

/// One commit's signature, as Keel's early check saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SignatureStatus {
    /// Signed by a trusted signer.
    Good,
    /// No signature.
    Missing,
    Unparseable,
    /// Signed, but by no key Keel trusts.
    UnknownKey,
    BadSignature,
    Expired,
    Revoked,
    NotSigningCapable,
    KeyInvalid,
    WrongNamespace,
    WeakHash,
    /// Keel did not check it (the check could not run).
    Unchecked,
}

/// One commit in a review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ReviewCommit {
    pub id: String,
    /// `Name <email>`.
    pub author: String,
    /// `Name <email>`.
    pub committer: String,
    /// The first line of the message.
    pub summary: String,
    pub status: SignatureStatus,
    /// The trusted signer's id, when `status` is `good`.
    pub signer: Option<String>,
}

/// What kind of trust or targeting change a review flags (R20).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReviewFlagKind {
    /// A change under `.keel/`.
    KeelConfig,
    /// A change to an inventory file the manifest names.
    Inventory,
    /// A change that touches a `keel_runner_*` variable.
    RunnerVars,
}

/// One flagged change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ReviewFlag {
    pub kind: ReviewFlagKind,
    pub path: String,
}

/// One review group in a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ReviewInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    #[ts(type = "string")]
    pub run: Uuid,
    #[ts(type = "string")]
    pub workspace: Uuid,
    #[ts(type = "string | null")]
    pub binding: Option<Uuid>,
    pub job_name: String,
    /// `None` for a baseline.
    pub base: Option<String>,
    pub target: String,
    pub hosts: Vec<String>,
    pub reason: ReviewReason,
    pub state: ReviewState,
    /// Distinct approvers so far.
    pub approvals: u32,
    /// Distinct approvers needed.
    pub quorum: u32,
    /// The approver may not be an author of a failing commit.
    pub four_eyes: bool,
    pub flagged: u32,
    /// RFC 3339 UTC.
    pub created_at: String,
}

/// Someone's decision on a review group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ReviewDecision {
    pub approver: String,
    pub approve: bool,
    pub comment: Option<String>,
    /// RFC 3339 UTC.
    pub at: String,
    /// When the override lapses on the hosts, RFC 3339 UTC.
    pub expires_at: Option<String>,
}

/// How a file changed between the base and the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DiffStatus {
    Added,
    Modified,
    Deleted,
}

/// One file of a review's diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DiffFile {
    pub path: String,
    pub status: DiffStatus,
    /// Flagged as a trust or targeting change.
    pub flags: Vec<ReviewFlagKind>,
}

/// A trusted signer in Keel's early-check list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SignerInfo {
    #[ts(type = "string")]
    pub id: Uuid,
    pub kind: SignerKind,
    /// The id the signature check reports: the SSH principal, or the OpenPGP fingerprint.
    pub fingerprint: String,
    pub label: String,
    /// The person whose key this is (for four-eyes).
    pub user: Option<String>,
    pub added_by: Option<String>,
    /// RFC 3339 UTC.
    pub added_at: String,
}

/// A signer's key format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SignerKind {
    Openpgp,
    Ssh,
}

impl SignerKind {
    /// The stored name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Openpgp => "openpgp",
            Self::Ssh => "ssh",
        }
    }
}

/// The passkey ceremony an override or baseline is confirmed with: the browser signs the
/// canonical challenge, and `Confirm.step_up` carries the assertion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct OverrideChallenge {
    pub preview_id: PreviewId,
    pub request: PasskeyRequest,
    /// The RFC 8785 JSON whose SHA-256 is the WebAuthn challenge, shown to the approver.
    pub canonical: String,
    /// When the override lapses on the hosts, RFC 3339 UTC (whole seconds).
    pub expires_at: String,
}

/// A page of one file's diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DiffPage {
    pub path: String,
    /// 0-based.
    pub page: u32,
    pub pages: u32,
    /// Unified diff lines of this page (`+`, `-` or ` ` prefixed; `@@` hunk headers).
    pub lines: Vec<String>,
    /// The file is binary; `lines` is empty.
    pub binary: bool,
}
