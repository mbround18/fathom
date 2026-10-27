//! Passkeys for the suite's servers: store trait, ceremonies, recovery codes and a pure verify_assertion.
//!
//! Spec 055 (Keel). Phase 1 scaffolding: modules are filled by later phases, tests first.

pub mod store;
pub mod ceremony;
pub mod recovery;
pub mod verify;
