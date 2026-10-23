//! `container-runtime`: Docker and Compose runtime shared by the suite: CLI runner, argv previews, streams and wire types (spec 050).
//!
//! Everything here reaches the runtime through its command line, using the mockable
//! [`runner::Runner`]. That is a requirement, not a convenience: a mutation preview must show the
//! operation in a form the operator could reproduce themselves, and an argv is reproducible where
//! a socket call is not. No dependency on any app crate, `kube` or `tauri` (checked in CI).

pub mod runner;
