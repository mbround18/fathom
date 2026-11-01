//! Approving and denying runs held by an approval policy (spec 064).

use keel_protocol::{Cmd, Event};

use crate::bindings::op_error;
use crate::dispatch::Request;
use crate::serve::AppState;

/// Handles the run-approval commands.
pub async fn handle(_app: &AppState, _req: &Request<'_>, _cmd: Cmd) -> Vec<Event> {
    op_error("unsupported", "Run approval is not available yet.")
}
