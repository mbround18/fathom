//! Payload types for local container-runtime management (spec 048).
//!
//! Defined once in the shared `container-runtime` crate (spec 050) and re-exported here, so the
//! serde shape and the generated bindings in `ui/src/protocol/generated/` are unchanged.

pub use container_runtime::wire::*;
