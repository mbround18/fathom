//! 037-node-shell command family.

use crossbeam_channel::Sender;

use crate::backend::node_shell::{self, StartParams};
use crate::backend::state::BackendState;
use crate::protocol::{BackendCmd, BackendEventEnvelope, NodeShellSettings};

pub(super) async fn handle(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::CheckNodeShellAccess { namespace } => {
            node_shell::handle_check_access(state, sender, namespace).await?;
        }
        BackendCmd::StartNodeShell {
            node,
            namespace,
            image,
            image_pull_secret,
            deadline_seconds,
        } => {
            // Validate through the same rules as saved settings.
            let checked = NodeShellSettings {
                image,
                namespace,
                image_pull_secret,
                deadline_seconds: deadline_seconds
                    .unwrap_or(crate::protocol::NODE_SHELL_DEFAULT_DEADLINE_SECONDS),
            }
            .normalized()
            .map_err(anyhow::Error::msg)?;
            node_shell::start(
                state,
                sender,
                StartParams {
                    node,
                    namespace: checked.namespace,
                    image: checked.image,
                    image_pull_secret: checked.image_pull_secret,
                    deadline_seconds: checked.deadline_seconds,
                },
            )
            .await?;
        }
        BackendCmd::StopNodeShell { session_id, .. } => {
            node_shell::stop(state, session_id).await;
        }
        BackendCmd::ListNodeShellOrphans { namespace } => {
            node_shell::handle_list_orphans(state, sender, namespace).await?;
        }
        BackendCmd::DeleteNodeShellOrphans { pods } => {
            node_shell::handle_delete_orphans(state, sender, pods).await?;
        }
        BackendCmd::GetNodeShellSettings => node_shell::handle_get_settings(state, sender),
        BackendCmd::SetNodeShellSettings { settings } => {
            node_shell::handle_set_settings(state, sender, settings)?;
        }
        other => return Ok(Some(other)),
    }
    Ok(None)
}
