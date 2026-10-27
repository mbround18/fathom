//! Keel's versioned command/event protocol, defined in Rust with generated TypeScript.
//!
//! Spec 055 (Keel). Wire-only types; `tests/export_bindings.rs` writes the TypeScript bindings to
//! `apps/keel/web/src/protocol/generated/`, and `tests/roundtrip.rs` round-trips every variant.

pub mod cmd;
pub mod event;
pub mod passkey;
pub mod types;

pub use cmd::{Cmd, CmdEnvelope, Hello};
pub use event::{Event, EventEnvelope};
pub use passkey::{
    AssertionCredential, AssertionResponse, CeremonyId, CredProps, PasskeyAssertion,
    PasskeyExtensions, PublicKeyType, Verification,
};
pub use types::{
    B64Url, ClientName, CsrfToken, InstanceRole, Me, MeGrant, Preview, PreviewId, RequestId,
    TeamRole,
};

/// The version a client states in `Hello.protocol_version`.
pub const PROTOCOL_VERSION: u32 = 1;
