//! Dev-only WebSocket bridge to the Fathom backend, so Playwright can drive the real UI
//! against a real cluster from a plain browser. The protocol mirrors the Tauri bridge
//! (`send` returns a request id, `subscribe` streams event batches); see
//! [`fathom::protocol::DevBridgeClientMsg`] and [`fathom::protocol::DevBridgeServerMsg`].
//!
//! Safety fences: loopback-only listener, a required shared token, a local-origin
//! allow-list, a mandatory write guard confined to one context and one labelled sandbox
//! namespace, and the MCP server disabled.

pub mod auth;
pub mod guard;
pub mod sandbox;
pub mod server;

/// Default listen port.
pub const DEFAULT_PORT: u16 = 18290;
