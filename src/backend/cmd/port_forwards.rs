//! Port-forward commands.

use crossbeam_channel::Sender;

use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;

use crate::backend::state::BackendState;
use crate::backend::{events, integrations, portforward};
use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, Correlation, Route, SystemEvent, section_keys,
};
use crate::sections::network::PortForwardStatus;

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
        BackendCmd::StartServicePortForward {
            namespace,
            service,
            port,
        } => start_service_port_forward(state, sender, &namespace, &service, port).await,
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

/// 041: forward to a Service's pod through the regular [`portforward::PortForwardManager`], so the
/// tunnel shows in the Port forwards list. Reuses an active tunnel to the same pod and port.
async fn start_service_port_forward(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    service: &str,
    port: u16,
) {
    let seq = state.cluster.seq.clone();
    let fail = |msg: String| events::notify_error(seq.as_ref(), sender, msg);
    let Some(client) = state.cluster.client.clone() else {
        fail("Not connected to cluster".to_owned());
        return;
    };
    let pod = match integrations::find_pod_for_service(&client, namespace, service).await {
        Ok(Some(pod)) => pod,
        Ok(None) => {
            fail(format!(
                "No running pod behind Service {namespace}/{service} to forward to"
            ));
            return;
        }
        Err(e) => {
            fail(format!(
                "Could not resolve Service {namespace}/{service}: {e}"
            ));
            return;
        }
    };
    let remote_port = match service_target_port(&client, namespace, service, &pod, port).await {
        Ok(p) => p,
        Err(e) => {
            fail(e);
            return;
        }
    };

    let existing = state.port_forward_manager.list().into_iter().find(|pf| {
        pf.namespace == namespace
            && pf.pod == pod
            && pf.remote_port == remote_port
            && matches!(
                pf.status,
                PortForwardStatus::Active | PortForwardStatus::Connecting
            )
    });
    let (tunnel_id, local_port) = if let Some(pf) = existing {
        (pf.id, pf.local_port)
    } else {
        let Some(local_port) = pick_local_port(port, portforward::is_local_port_available) else {
            fail(format!(
                "No free local port (10000 and up) for {namespace}/{service}:{port}"
            ));
            return;
        };
        match state
            .port_forward_manager
            .start(&client, namespace, &pod, local_port, remote_port, None)
            .await
        {
            Ok(id) => (id, local_port),
            Err(e) => {
                fail(format!(
                    "Failed to forward {namespace}/{service}:{port}: {e}"
                ));
                return;
            }
        }
    };

    publish_port_forwards(state, sender);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::global(section_keys::network_portforwards()),
        Correlation::None,
        BackendEvent::System(SystemEvent::ServicePortForwardReady {
            namespace: namespace.to_owned(),
            service: service.to_owned(),
            port,
            local_port,
            tunnel_id: tunnel_id.clone(),
        }),
    );
    events::notify_info(
        state.cluster.seq.as_ref(),
        sender,
        format!("Forwarding {namespace}/{service}:{port} on localhost:{local_port} [{tunnel_id}]"),
    );
}

/// The container port behind `service`:`port` on `pod` (numeric or named targetPort; defaults to
/// `port` itself, as Kubernetes does).
async fn service_target_port(
    client: &kube::Client,
    namespace: &str,
    service: &str,
    pod: &str,
    port: u16,
) -> Result<u16, String> {
    use k8s_openapi::api::core::v1::{Pod, Service};
    let svc = kube::Api::<Service>::namespaced(client.clone(), namespace)
        .get(service)
        .await
        .map_err(|e| format!("Could not read Service {namespace}/{service}: {e}"))?;
    let ports = svc.spec.and_then(|s| s.ports).unwrap_or_default();
    let Some(sp) = ports.iter().find(|p| p.port == i32::from(port)) else {
        return Err(format!("Service {namespace}/{service} has no port {port}"));
    };
    let pod = match &sp.target_port {
        Some(IntOrString::String(_)) => kube::Api::<Pod>::namespaced(client.clone(), namespace)
            .get(pod)
            .await
            .ok(),
        _ => None,
    };
    target_port_for(sp, pod.as_ref())
        .ok_or_else(|| format!("Could not map {namespace}/{service}:{port} to a container port"))
}

/// Pure half of [`service_target_port`].
fn target_port_for(
    sp: &k8s_openapi::api::core::v1::ServicePort,
    pod: Option<&k8s_openapi::api::core::v1::Pod>,
) -> Option<u16> {
    match &sp.target_port {
        None => u16::try_from(sp.port).ok(),
        Some(IntOrString::Int(n)) => u16::try_from(*n).ok(),
        Some(IntOrString::String(name)) => pod?
            .spec
            .as_ref()?
            .containers
            .iter()
            .flat_map(|c| c.ports.iter().flatten())
            .find(|p| p.name.as_deref() == Some(name.as_str()))
            .and_then(|p| u16::try_from(p.container_port).ok()),
    }
}

/// Lowest local port a forward is auto-assigned (feedback 2026-09-27 #27): high enough to stay
/// clear of well-known and commonly used development ports.
const AUTO_LOCAL_PORT_MIN: u16 = 10_000;

/// Local port for an auto-assigned forward: `preferred` when it is at least
/// [`AUTO_LOCAL_PORT_MIN`] and free, else the next free port above the start, wrapping around to
/// [`AUTO_LOCAL_PORT_MIN`]. `is_free` should really bind the port
/// ([`portforward::is_local_port_available`]). Ports the user types in are not routed through
/// here; they are used as given.
fn pick_local_port(preferred: u16, is_free: impl Fn(u16) -> bool) -> Option<u16> {
    let start = preferred.max(AUTO_LOCAL_PORT_MIN);
    (start..=u16::MAX)
        .chain(AUTO_LOCAL_PORT_MIN..start)
        .find(|p| is_free(*p))
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

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{Container, ContainerPort, Pod, PodSpec, ServicePort};

    fn sp(port: i32, target: Option<IntOrString>) -> ServicePort {
        ServicePort {
            port,
            target_port: target,
            ..Default::default()
        }
    }

    #[test]
    fn target_port_numeric_named_and_default() {
        assert_eq!(target_port_for(&sp(80, None), None), Some(80));
        assert_eq!(
            target_port_for(&sp(80, Some(IntOrString::Int(3000))), None),
            Some(3000)
        );
        let pod = Pod {
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "grafana".into(),
                    ports: Some(vec![ContainerPort {
                        name: Some("http-web".into()),
                        container_port: 3000,
                        ..Default::default()
                    }]),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let named = sp(80, Some(IntOrString::String("http-web".into())));
        assert_eq!(target_port_for(&named, Some(&pod)), Some(3000));
        assert_eq!(target_port_for(&named, None), None);
    }

    #[test]
    fn local_port_is_auto_assigned_from_10000_up() {
        // Low service ports (privileged or common dev ports) map to 10000 and up.
        assert_eq!(pick_local_port(80, |_| true), Some(10_000));
        assert_eq!(pick_local_port(3000, |_| true), Some(10_000));
        assert_eq!(pick_local_port(3000, |p| p > 10_001), Some(10_002));
        // A service port already in the auto range is kept when free.
        assert_eq!(pick_local_port(30_000, |_| true), Some(30_000));
        assert_eq!(pick_local_port(30_000, |p| p != 30_000), Some(30_001));
        // The search wraps around to the bottom of the range.
        assert_eq!(pick_local_port(u16::MAX, |p| p == 10_005), Some(10_005));
        assert_eq!(pick_local_port(3000, |_| false), None);
        // Never below the range, whatever is free.
        assert_eq!(
            pick_local_port(80, |p| p < 10_000 || p == 12_345),
            Some(12_345)
        );
    }

    #[test]
    fn local_port_skips_ports_that_are_really_bound() {
        use std::net::{Ipv4Addr, TcpListener};

        // Hold a port in the auto range and check the real bind probe steps over it.
        let held = (AUTO_LOCAL_PORT_MIN..=u16::MAX)
            .find_map(|p| TcpListener::bind((Ipv4Addr::LOCALHOST, p)).ok())
            .expect("a bindable port in the auto range");
        let port = held.local_addr().unwrap().port();
        assert!(!portforward::is_local_port_available(port));
        let picked = pick_local_port(port, portforward::is_local_port_available).unwrap();
        assert_ne!(picked, port);
        assert!(picked >= AUTO_LOCAL_PORT_MIN);
    }
}
