//! Passkeys for the suite's servers: store trait, ceremonies, recovery codes and a pure
//! `verify_assertion`.
//!
//! Spec 055 (Keel) T023; Depths (spec 054) uses the same crate.
//!
//! - [`ceremony`]: registration and authentication over webauthn-rs, user verification required.
//! - [`store`]: the [`store::PasskeyStore`] trait. Applications bring the storage.
//! - [`recovery`]: single-use recovery codes, stored only as SHA-256.
//! - [`verify`]: [`verify::verify_assertion`], which needs only a pinned public key, so Keel's
//!   hosts can check override approvals without any server state.
//! - [`cose`]: COSE_Key bytes for pinning ES256 and EdDSA keys.

pub mod ceremony;
pub mod cose;
pub mod recovery;
pub mod store;
pub mod verify;

pub use ceremony::{Ceremonies, CeremonyError, PasskeyUser};
pub use cose::CoseKey;
pub use store::{PasskeyStore, StoredPasskey};
pub use verify::{CoseAlg, VerifyError, verify_assertion};
