//! `depths-protocol`: Depths versioned command/event protocol, defined in Rust with generated TypeScript (spec 050).
//!
//! The wire contract is `specs/050-depths-stack-manager/contracts/protocol.md`. The TypeScript in
//! `apps/depths/web/src/protocol/generated/` is generated from these types by
//! `cargo test -p depths-protocol --test export_bindings` and drift-checked in `scripts/ci.sh`.
//!
//! - `GET /api/v1/ws`: the client sends [`Hello`] first, then [`CmdEnvelope`]s; the server sends
//!   [`EventEnvelope`]s.
//! - Container-level types are the 048 wire types from `container_runtime::wire`, re-exported
//!   unchanged as [`wire`] and at the crate root.
//!
//! Wire-only: this crate depends on no Depths domain or server crate.

pub mod cmd;
pub mod event;
pub mod types;

/// The container-runtime wire types (spec 048), shared with Fathom.
pub use container_runtime::wire;
pub use container_runtime::wire::*;

pub use cmd::{Cmd, CmdEnvelope, Hello};
pub use event::{Event, EventEnvelope};
pub use types::*;

/// The protocol version a client states in [`Hello::protocol_version`].
pub const PROTOCOL_VERSION: u32 = 1;

/// WebSocket close codes (same as Fathom hosted).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum CloseCode {
    /// No session, or it expired. Also a socket that sends no valid `Hello` in time.
    Unauthenticated = 4401,
    /// Wrong CSRF token, or the person was removed or lost their last stack.
    Forbidden = 4403,
    /// Too many sockets for this session or IP, or too many commands per second.
    TooMany = 4429,
}

impl CloseCode {
    pub const ALL: [CloseCode; 3] = [Self::Unauthenticated, Self::Forbidden, Self::TooMany];

    pub fn code(self) -> u16 {
        self as u16
    }

    /// The name the TypeScript constants use.
    pub fn name(self) -> &'static str {
        match self {
            Self::Unauthenticated => "Unauthenticated",
            Self::Forbidden => "Forbidden",
            Self::TooMany => "TooMany",
        }
    }
}
