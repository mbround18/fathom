//! The GitSource port's gix adapter: mirrors, pins, ancestry, packs and diffs.
//!
//! Spec 055 (Keel). Phase 1 scaffolding: modules are filled by later phases, tests first.

pub mod ancestry;
pub mod diff;
pub mod mirror;
pub mod pack;
pub mod pin;
