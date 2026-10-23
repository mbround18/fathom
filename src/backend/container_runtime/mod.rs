//! Local container-runtime management (spec 048).
//!
//! The runtime itself — the CLI runner, inventory, grouping, previews, mutations, streams — lives
//! in the shared `container-runtime` crate (spec 050), re-exported here under the paths Fathom has
//! always used. What stays is Fathom's: the adoption store's location and the command gate, the
//! kind cluster identity of a node container ([`inventory`]'s post-pass and [`cluster_link`]), and
//! the log follow's bridge onto Fathom's stream registry ([`logs`]).

pub mod adoption;
pub mod cluster_link;
pub mod inventory;
pub mod logs;

pub use ::container_runtime::cli::Cli;
pub use ::container_runtime::{compose, grouping, mutate, preview, probe, session, stats, stream};
