//! Keel's versioned command/event protocol, defined in Rust with generated TypeScript.
//!
//! Spec 055 (Keel). Wire-only types; `tests/export_bindings.rs` writes the TypeScript bindings to
//! `apps/keel/web/src/protocol/generated/`, and `tests/roundtrip.rs` round-trips every variant.

pub mod cmd;
pub mod event;
pub mod passkey;
pub mod types;

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

// Governance (US6)
pub mod governance;
pub use governance::{
    ActionName, ApiTokenInfo, GrantInfo, GrantProfiles, Label, MAX_GRANT_PROFILES, MAX_LABEL_CHARS,
    MAX_TOKEN_ACTIONS, MAX_TOKEN_DAYS, ProfileName, ProjectInfo, Slug, TeamInfo, TeamMemberInfo,
    TokenActions, TokenSecret, UserInfo, WorkspaceInfo,
};

/// The version a client states in `Hello.protocol_version`.
pub const PROTOCOL_VERSION: u32 = 1;
