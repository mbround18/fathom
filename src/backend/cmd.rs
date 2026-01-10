use crossbeam_channel::Sender;

use super::state::BackendState;
use super::{events, portforward};
use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, Correlation, Route, SystemEvent, section_keys,
};

/// Temporary dispatcher wrapper.
/// Handles port-forward related commands locally and delegates
/// the remainder to `backend::handle_cmd` until commands are
/// extracted into resource-specific handlers.
pub async fn dispatch_cmd(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<bool> {
    match cmd {
        BackendCmd::Shutdown => {
            tracing::info!("Shutdown command received, cleaning up all background tasks...");
            events::notify_info(
                state.seq.as_ref(),
                sender,
                "Shutting down backend...".to_owned(),
            );
            super::state::cleanup_all_tasks(state);
            tracing::info!("All background tasks cleaned up, exiting backend");
            return Ok(true); // Signal to exit loop
        }
        BackendCmd::StartPortForward {
            namespace,
            pod,
            local_port,
            remote_port,
            container,
        } => {
            if let Some(client) = &state.client {
                match state
                    .port_forward_manager
                    .start(client, &namespace, &pod, local_port, remote_port, container)
                    .await
                {
                    Ok(tunnel_id) => {
                        events::notify_info(
                            state.seq.as_ref(),
                            sender,
                            format!(
                                "Started port forward [{}]: {}/{} {}:{}",
                                tunnel_id, namespace, pod, local_port, remote_port
                            ),
                        );
                        // Emit updated list and persist
                        portforward::emit_port_forward_list(state, sender);
                        portforward::save_port_forwards_to_disk(state);
                    }
                    Err(e) => {
                        events::notify_error(
                            state.seq.as_ref(),
                            sender,
                            format!("Failed to start port forward: {}", e),
                        );
                    }
                }
            } else {
                events::notify_error(
                    state.seq.as_ref(),
                    sender,
                    "Not connected to cluster".to_owned(),
                );
            }
            Ok(false)
        }
        BackendCmd::StopPortForward { tunnel_id } => {
            match state.port_forward_manager.stop(&tunnel_id).await {
                Ok(_) => {
                    events::notify_info(
                        state.seq.as_ref(),
                        sender,
                        format!("Stopped port forward: {}", tunnel_id),
                    );
                    portforward::emit_port_forward_list(state, sender);
                    portforward::save_port_forwards_to_disk(state);
                }
                Err(e) => {
                    events::notify_error(
                        state.seq.as_ref(),
                        sender,
                        format!("Failed to stop port forward: {}", e),
                    );
                }
            }
            Ok(false)
        }
        BackendCmd::DeletePortForward { tunnel_id } => {
            let _ = state.port_forward_manager.stop(&tunnel_id).await;
            state.port_forward_manager.delete(&tunnel_id);

            events::notify_info(
                state.seq.as_ref(),
                sender,
                format!("Deleted port forward: {}", tunnel_id),
            );
            portforward::emit_port_forward_list(state, sender);
            portforward::save_port_forwards_to_disk(state);
            Ok(false)
        }
        BackendCmd::UpdatePortForward {
            tunnel_id,
            local_port,
            remote_port,
            auto_start,
        } => {
            if let Some(pf) = state.port_forward_manager.tunnels.get_mut(&tunnel_id) {
                pf.local_port = local_port;
                pf.remote_port = remote_port;
                pf.auto_start = auto_start;

                events::notify_info(
                    state.seq.as_ref(),
                    sender,
                    format!("Updated port forward: {}", tunnel_id),
                );
                portforward::emit_port_forward_list(state, sender);
                portforward::save_port_forwards_to_disk(state);
            } else {
                events::notify_error(
                    state.seq.as_ref(),
                    sender,
                    format!("Port forward not found: {}", tunnel_id),
                );
            }
            Ok(false)
        }
        BackendCmd::RefreshPortForwards => {
            portforward::emit_port_forward_list(state, sender);
            events::notify_info(
                state.seq.as_ref(),
                sender,
                "Port forwards refreshed".to_owned(),
            );
            Ok(false)
        }
        BackendCmd::CheckPortAvailability { port } => {
            let available = portforward::is_local_port_available(port);
            let event = BackendEvent::System(SystemEvent::PortAvailability { port, available });
            events::emit(
                state.seq.as_ref(),
                sender,
                Route::global(section_keys::network_portforwards()),
                Correlation::None,
                event,
            );
            Ok(false)
        }
        other => {
            super::handle_cmd(other, state, sender).await?;
            Ok(false)
        }
    }
}
