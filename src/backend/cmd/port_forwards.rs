//! Port-forward commands.

use crossbeam_channel::Sender;

use crate::backend::state::BackendState;
use crate::backend::{events, portforward};
use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, Correlation, Route, SystemEvent, section_keys,
};

/// Handles port-forward commands; returns the command back if it belongs to another family.
pub(super) async fn handle(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> Option<BackendCmd> {
    match cmd {
        BackendCmd::StartPortForward {
            namespace,
            pod,
            local_port,
            remote_port,
            container,
        } => {
            start_port_forward(
                state,
                sender,
                &namespace,
                &pod,
                local_port,
                remote_port,
                container,
            )
            .await;
        }
        BackendCmd::StopPortForward { tunnel_id } => {
            stop_port_forward(state, sender, &tunnel_id).await;
        }
        BackendCmd::DeletePortForward { tunnel_id } => {
            let _ = state.port_forward_manager.stop(&tunnel_id).await;
            state.port_forward_manager.delete(&tunnel_id);

            events::notify_info(
                state.cluster.seq.as_ref(),
                sender,
                format!("Deleted port forward: {}", tunnel_id),
            );
            publish_port_forwards(state, sender);
        }
        BackendCmd::UpdatePortForward {
            tunnel_id,
            local_port,
            remote_port,
            auto_start,
        } => update_port_forward(
            state,
            sender,
            &tunnel_id,
            local_port,
            remote_port,
            auto_start,
        ),
        BackendCmd::RefreshPortForwards => {
            portforward::emit_port_forward_list(state, sender);
            events::notify_info(
                state.cluster.seq.as_ref(),
                sender,
                "Port forwards refreshed".to_owned(),
            );
        }
        BackendCmd::CheckPortAvailability { port } => {
            let available = portforward::is_local_port_available(port);
            let event = BackendEvent::System(SystemEvent::PortAvailability { port, available });
            events::emit(
                state.cluster.seq.as_ref(),
                sender,
                Route::global(section_keys::network_portforwards()),
                Correlation::None,
                event,
            );
        }
        other => return Some(other),
    }
    None
}

/// Emit the updated port-forward list and persist it.
fn publish_port_forwards(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    portforward::emit_port_forward_list(state, sender);
    portforward::save_port_forwards_to_disk(state);
}

async fn start_port_forward(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    pod: &str,
    local_port: u16,
    remote_port: u16,
    container: Option<String>,
) {
    let Some(client) = &state.cluster.client else {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            "Not connected to cluster".to_owned(),
        );
        return;
    };
    match state
        .port_forward_manager
        .start(client, namespace, pod, local_port, remote_port, container)
        .await
    {
        Ok(tunnel_id) => {
            events::notify_info(
                state.cluster.seq.as_ref(),
                sender,
                format!(
                    "Started port forward [{}]: {}/{} {}:{}",
                    tunnel_id, namespace, pod, local_port, remote_port
                ),
            );
            publish_port_forwards(state, sender);
        }
        Err(e) => {
            events::notify_error(
                state.cluster.seq.as_ref(),
                sender,
                format!("Failed to start port forward: {}", e),
            );
        }
    }
}

async fn stop_port_forward(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    tunnel_id: &str,
) {
    match state.port_forward_manager.stop(tunnel_id).await {
        Ok(_) => {
            events::notify_info(
                state.cluster.seq.as_ref(),
                sender,
                format!("Stopped port forward: {}", tunnel_id),
            );
            publish_port_forwards(state, sender);
        }
        Err(e) => {
            events::notify_error(
                state.cluster.seq.as_ref(),
                sender,
                format!("Failed to stop port forward: {}", e),
            );
        }
    }
}

fn update_port_forward(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    tunnel_id: &str,
    local_port: u16,
    remote_port: u16,
    auto_start: bool,
) {
    let Some(pf) = state.port_forward_manager.tunnels.get_mut(tunnel_id) else {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            format!("Port forward not found: {}", tunnel_id),
        );
        return;
    };
    pf.local_port = local_port;
    pf.remote_port = remote_port;
    pf.auto_start = auto_start;

    events::notify_info(
        state.cluster.seq.as_ref(),
        sender,
        format!("Updated port forward: {}", tunnel_id),
    );
    publish_port_forwards(state, sender);
}
