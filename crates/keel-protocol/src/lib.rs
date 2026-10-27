//! Keel's versioned command/event protocol, defined in Rust with generated TypeScript.
//!
//! Spec 055 (Keel). Wire-only types; `tests/export_bindings.rs` writes the TypeScript bindings to
//! `apps/keel/web/src/protocol/generated/`. Phase 1 scaffolding: `Cmd` and `Event` arrive with
//! their round-trip tests in later phases.

pub mod cmd;
pub mod event;
pub mod types;

pub use types::Hello;

/// The version a client states in `Hello.protocol_version`.
pub const PROTOCOL_VERSION: u32 = 1;
