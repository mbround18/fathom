//! Keel's agnostic domain: policy, runs, approvals, rollback, inventory and ports. No I/O.
//!
//! Spec 055 (Keel). Modules not yet filled are placeholders for later phases.

pub mod approval;
pub mod authz;
pub mod domain;
pub mod inventory;
pub mod manifest;
pub mod migration;
pub mod policy;
pub mod ports;
pub mod rollback;
pub mod run;

pub use domain::{
    ApiTokenId, AuditEventId, GrantId, HostId, InstanceRole, Origin, ParseError, ProjectId, RunId,
    Slug, TeamId, TeamRole, UserId, WorkspaceId,
};
