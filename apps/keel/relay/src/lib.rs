//! Keel relay (spec 067): sits in a spoke network Keel cannot reach. Keel dials it over SSH
//! (`link_listen`); runners dial its `runner_listen` port, and each runner TCP connection is
//! handed to Keel as a `forwarded-tcpip` channel. The relay never terminates a runner session:
//! it only copies ciphertext (contracts/relay.md).
//!
//! - [`config`], `/etc/keel-relay/relay.toml`.
//! - [`key`], the relay's Ed25519 host key and the `hub_keys` file.
//! - [`link`], the SSH server Keel dials, and the `keel-relay-v1` control channel.
//! - [`forward`], the runner listener and the byte copy.
//! - [`cache`], the optional runner release cache.

pub mod cache;
pub mod config;
pub mod forward;
pub mod key;
pub mod link;
