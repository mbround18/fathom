//! Keel's versioned command/event protocol, defined in Rust with generated TypeScript.
//!
//! Spec 055 (Keel). Wire-only types; `tests/export_bindings.rs` writes the TypeScript bindings to
//! `apps/keel/web/src/protocol/generated/`, and `tests/roundtrip.rs` round-trips every variant.

pub mod binding;
pub mod cmd;
pub mod event;
pub mod passkey;
pub mod types;

pub use binding::{
    ActiveStatus, BindingInfo, BindingSpec, BindingUpdate, BranchName, CredentialGrantInfo,
    CredentialInfo, CredentialKind, DeliveryDecision, DeliveryInfo, DeliveryRefusal, Instant,
    JobSummary, KnownHost, ManifestProblem, ObjectName, ReceiverInfo, ReceiverProvider, RepoUrl,
    SecretValue, SyncState, Transport,
};
pub use cmd::{Cmd, CmdEnvelope, Hello, TOKEN_ACTIONS};
pub use event::{Event, EventEnvelope};
pub use passkey::{
    AssertionCredential, AssertionResponse, AttestationCredential, AttestationResponse, CeremonyId,
    CredProps, MAX_PASSKEY_NAME_CHARS, PasskeyAssertion, PasskeyCredential, PasskeyExtensions,
    PasskeyInfo, PasskeyName, PasskeyPurpose, PasskeyRefusedReason, PasskeyRequest,
    PasskeyResponse, PasskeyTransport, PublicKeyType, RecoveryCode, Verification,
};
pub use types::{
    B64Url, ClientName, CsrfToken, InstanceRole, Me, MeGrant, Preview, PreviewId, RequestId,
    TeamRole,
};

// Inventories (US4)
pub mod inventory;
pub use inventory::{
    InventoryChange, InventoryChangeKind, InventoryHostInfo, InventoryInfo, InventoryProblem,
    InventoryRevisionInfo, InventoryRevisionSummary, InventorySource, InventoryStatus,
    InventoryText, MAX_INVENTORY_TEXT_BYTES, RevisionStatus,
};

// Governance (US6)
pub mod governance;
pub use governance::{
    ActionName, ApiTokenInfo, GrantInfo, GrantProfiles, Label, MAX_GRANT_PROFILES, MAX_LABEL_CHARS,
    MAX_TOKEN_ACTIONS, MAX_TOKEN_DAYS, ProfileName, ProjectInfo, Slug, TeamInfo, TeamMemberInfo,
    TokenActions, TokenSecret, UserInfo, WorkspaceInfo,
};

// Runs (US2)
pub mod run;
pub use run::{
    FirstFailure, HostPhase, HostStats, JobInfo, LogStream, Pattern, PolicyCheck, PolicyInfo,
    PolicySpec, RunHostInfo, RunInfo, RunPhase, RunReason, RunSummary, RunTrigger,
};

/// The version a client states in `Hello.protocol_version`.
pub mod host;
pub use host::{
    EnrollToken, HostCertInfo, HostDetail, HostInfo, HostName, HostProfiles, HostStatus,
    MAX_HOST_PROFILES, SshCaInfo, SshCaKind, SshCaState,
};

pub const PROTOCOL_VERSION: u32 = 1;

// Signed mode: review and signers (US8)
pub mod review;
pub use review::{
    BaselineHosts, CommitSha, DiffFile, DiffPage, DiffStatus, MAX_BASELINE_HOSTS,
    MAX_REPO_PATH_BYTES, MAX_REVIEW_COMMENT_CHARS, MAX_SIGNER_KEY_BYTES, OverrideChallenge,
    RepoPath, ReviewComment, ReviewCommit, ReviewDecision, ReviewFlag, ReviewFlagKind, ReviewInfo,
    ReviewReason, ReviewState, SignatureStatus, SignerInfo, SignerKey, SignerKind,
};
