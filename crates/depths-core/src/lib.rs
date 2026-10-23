//! `depths-core`: Depths domain: stacks, status, roles and operation planning, with no I/O (spec 050).
//!
//! Everything here is a pure function of its inputs. The filesystem and the container runtime are
//! reached only through the traits in [`ports`]; `depths-stacks-fs` and the server's runtime
//! adapter implement them. No dependency on `kube`, `tauri` or any Fathom crate.

pub mod discovery;
pub mod name;
pub mod status;

pub use name::{InvalidName, ServiceName, StackName, valid_name, valid_service_name};
