//! `oci-registry`: a HEAD-first OCI distribution client shared by Keel (spec 061) and Depths
//! (spec 062). See `specs/061-keel-update-notice/contracts/registry.md`.
//!
//! An adapter: it holds no Keel or Depths types. `head_digest` learns a tag's digest with HEAD
//! requests only (which registries do not count as pulls); `resolve` reads one image by digest.
//! Feature `test-util` adds [`testing::FakeRegistry`] and `Client::with_base_url`.
pub mod auth;
pub mod client;
pub mod error;
pub mod manifest;
pub mod reference;
#[cfg(feature = "test-util")]
pub mod testing;

pub use auth::Credentials;
pub use client::{Client, Counted, RequestCounts, Resolved};
pub use error::RegistryError;
pub use manifest::Platform;
pub use reference::{Digest, ImageRef, Reference};
