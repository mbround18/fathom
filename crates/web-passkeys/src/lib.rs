//! Passkeys for the suite's servers: store trait, ceremonies, recovery codes and a pure
//! `verify_assertion`.
//!
//! Spec 055 (Keel) T023; Depths (spec 054) uses the same crate.
//!
//! The `ceremonies` feature (default) adds `ceremony`, `store` and `recovery`, which need
//! webauthn-rs and so OpenSSL. Without it the crate is verify-only (`verify`, `cose`, `b64`), which
//! is all Keel's runner and relay link.
//!
//! - `ceremony`: registration and authentication over webauthn-rs, user verification required.
//! - `store`: the `PasskeyStore` trait. Applications bring the storage.
//! - `recovery`: single-use recovery codes, stored only as SHA-256.
//! - [`verify`]: [`verify::verify_assertion`], which needs only a pinned public key, so Keel's
//!   hosts can check override approvals without any server state.
//! - [`cose`]: COSE_Key bytes for pinning ES256 and EdDSA keys.
//! - [`b64`]: base64url without padding, as WebAuthn encodes ids and challenges.

pub mod b64;
#[cfg(feature = "ceremonies")]
pub mod ceremony;
pub mod cose;
#[cfg(feature = "ceremonies")]
pub mod recovery;
#[cfg(feature = "ceremonies")]
pub mod store;
pub mod verify;

#[cfg(feature = "ceremonies")]
pub use ceremony::{Ceremonies, CeremonyError, PasskeyUser};
pub use cose::CoseKey;
#[cfg(feature = "ceremonies")]
pub use store::{PasskeyStore, StoredPasskey};
pub use verify::{CoseAlg, VerifyError, verify_assertion};
