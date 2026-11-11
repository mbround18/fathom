//! A workspace's primary repository: its catalog, job drafts, the forge connection and the
//! changes Keel proposes (spec 066).
//!
//! Foundation only (T003): every command is routed here and refused `not_implemented` until its
//! story lands.

use keel_protocol::{Cmd, Event};

use crate::dispatch::Request;
use crate::serve::AppState;

/// Handles the spec 066 commands.
pub async fn handle(_app: &AppState, _req: &Request<'_>, cmd: Cmd) -> Vec<Event> {
    vec![Event::op_error(
        "not_implemented",
        format!("{} is not available yet.", cmd.name()),
    )]
}
