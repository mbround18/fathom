//! `oci-registry`: a HEAD-first OCI distribution client, shared by Keel (spec 061) and Depths
//! (spec 062).
//!
//! # What it is
//!
//! An adapter between the suite's servers and container registries (Docker Hub, GHCR, any
//! registry speaking the OCI distribution API). It answers two questions:
//!
//! - **What does this tag point at now?** [`Client::head_digest`] returns the digest an
//!   [`ImageRef`] resolves to.
//! - **What is this image?** [`Client::resolve`] reads one image, by digest, for one
//!   [`Platform`]: its manifest digests, its config labels, user and creation time
//!   ([`Resolved`]).
//!
//! It holds no Keel or Depths types, and must not depend on any app crate. Each app maps the
//! results into its own model, names its own metrics, and decides what an update means. The
//! contract, including the exact list of HTTP requests the crate may make, is
//! `specs/061-keel-update-notice/contracts/registry.md`. A request not on that list is a bug.
//!
//! # The HEAD-first rule
//!
//! Registries count manifest `GET`s against pull limits, and do not count `HEAD`s. So:
//!
//! - [`Client::head_digest`] uses `HEAD` only, plus the token request a `401` asks for. It never
//!   falls back to `GET`. Checking an unchanged tag therefore costs no pulls.
//! - [`Client::resolve`] is the only call that `GET`s a manifest, and it takes a digest, not a tag
//!   ([`RegistryError::NotByDigest`] otherwise, before any request). Callers resolve only a digest
//!   they haven't seen, so a newly published image costs at most one pull.
//! - Every call returns [`Counted`]: its result and the [`RequestCounts`] it spent (`head`, `get`,
//!   and `manifest_get`, the ones that count as pulls). Apps feed these into their telemetry, so
//!   an operator can check the pull budget from their own metrics.
//!
//! Credentials are [`Credentials::Anonymous`] or [`Credentials::Basic`], with the password held
//! as a `secrecy::SecretString`. They are never logged. Bearer tokens are cached per registry,
//! scope and credentials. Only `https` registries are accepted.
//!
//! # Feature `test-util`
//!
//! For tests only; no application enables it outside its dev-dependencies. It adds:
//!
//! - `testing::FakeRegistry`: an in-process registry on `127.0.0.1:0`, with tag, manifest,
//!   index and blob fixtures, scripted error statuses, optional Basic auth, and a log of every
//!   request it served. Tests assert the request list against that log.
//! - `Client::with_base_url`: sends every registry and token request to the fake, plain `http`
//!   allowed.
//!
//! Nothing in the crate's tests, or the apps' tests that use the fake, reaches the network.
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
