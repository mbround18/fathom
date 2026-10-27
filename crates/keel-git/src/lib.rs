//! The GitSource port's gix adapter: mirrors, pins, ancestry, packs and diffs.
//!
//! Spec 055 (Keel).

pub mod ancestry;
pub mod diff;
pub mod mirror;
pub mod pack;
pub mod pin;

pub use ancestry::Ancestry;
pub use mirror::{Auth, CommitInfo, EntryKind, FetchRequest, Mirror, MirrorError};
