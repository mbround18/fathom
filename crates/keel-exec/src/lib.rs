//! Keel's runner channel: frames, codec, SSH CA and KRL, SSH configs, pack builder, keel_events parser.
//!
//! Spec 055 (Keel). Phase 1 scaffolding: modules are filled by later phases, tests first.

pub mod ca;
pub mod client_config;
pub mod codec;
pub mod events;
#[cfg(feature = "fake")]
pub mod fake;
pub mod frames;
pub mod krl;
pub mod pack_builder;
pub mod server_config;
