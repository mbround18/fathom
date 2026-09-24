//! 035-cloud-auth-plugins command family.

use crossbeam_channel::Sender;

use crate::backend::state::BackendState;
use crate::backend::{cloud_auth, persistence};
use crate::protocol::{BackendCmd, BackendEventEnvelope};

pub(super) fn handle(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::SetExecPolicy { context, policy } => {
            persistence::save_exec_policy(&context, policy)?;
            // Re-emit detection so the UI reflects the new policy; a prior blocked failure is
            // retried by the next probe/poll (the policy check precedes the cache).
            cloud_auth::emit_detection(&state.cluster.seq, sender);
        }
        other => return Ok(Some(other)),
    }
    Ok(None)
}
