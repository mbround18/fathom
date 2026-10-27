//! Keel's agnostic domain: policy, runs, approvals, rollback, inventory and ports. No I/O.
//!
//! Spec 055 (Keel). Phase 1 scaffolding: modules are filled by later phases, tests first.

pub mod policy;
pub mod run;
pub mod approval;
pub mod rollback;
pub mod inventory;
pub mod manifest;
pub mod migration;
pub mod authz;
pub mod ports;
