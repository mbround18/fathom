//! Pure trust checks shared by the server and the executor: commit signatures, override challenges, the ledger. No I/O.
//!
//! Spec 055 (Keel). Phase 1 scaffolding: modules are filled by later phases, tests first.

pub mod commit_check;
pub mod challenge;
pub mod override_verify;
pub mod ledger;
