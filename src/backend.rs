use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, CmdRequest, Correlation, MutationTarget, Route,
    ScopeKind, SystemEvent, section_keys,
};
use crossbeam_channel::{Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Runtime;

use crate::config::Args;

mod agent;
mod applications;
mod apply;
mod cmd;
mod context_status;
mod crds;
mod cronjobs;
mod daemonsets;
mod deployments;
mod endpoints;
mod events;
mod exec;
mod flux;
mod helm;
mod hubble;
mod images;
mod ingressclasses;
mod ingresses;
pub mod integrations;
mod interactions;
mod jobs;
mod kind;
mod kube;
mod longhorn;
mod mcp;
mod metrics;
mod metrics_provider;
mod networkpolicies;
mod nodes;
pub mod persistence;
mod pods;
mod portforward;
mod related;
mod replicasets;
mod replicationcontrollers;
mod services;
mod state;
mod statefulsets;
mod sync;
mod team_store;
mod workload_common;

mod clusterrolebindings;
mod clusterroles;
mod configmaps;
mod horizontalpodautoscalers;
mod leases;
mod limitranges;
mod mutatingwebhooks;
mod persistentvolumeclaims;
mod persistentvolumes;
mod poddisruptionbudgets;
mod priorityclasses;
mod rbac_access;
mod resourcequotas;
mod rolebindings;
mod roles;
mod runtimeclasses;
mod secrets;
mod storageclasses;
mod validatingwebhooks;

use state::BackendState;

/// Run the backend on the current thread until `receiver` closes.
///
/// `C` is either a bare [`BackendCmd`] (uncorrelated: the egui app) or a [`CmdRequest`]
/// carrying the bridge-assigned request id.
pub fn init<C>(receiver: Receiver<C>, sender: Sender<BackendEventEnvelope>, args: Args)
where
    C: Into<CmdRequest>,
{
    let rt = Runtime::new().expect("Failed to create Tokio runtime");

    let mut state = BackendState::new();
    state.agent_conversations = agent::persistence::load_agent_conversations();
    // 015-mcp-agent-exposure: shells that must never expose an MCP endpoint (`fathom-devbridge`)
    // set `Args::disable_mcp`, and `McpState::start` then refuses before spawning anything. The
    // server is off regardless — this only makes it un-enableable for this process.
    state.mcp.disabled = args.disable_mcp;

    // Clear metrics history on startup to begin fresh collection
    if let Err(e) = persistence::clear_metrics_history() {
        tracing::warn!("Failed to clear metrics history on startup: {}", e);
    }

    events::emit(
        state.cluster.seq.as_ref(),
        &sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::BackendStarted),
    );
    events::notify_info(
        state.cluster.seq.as_ref(),
        &sender,
        "Backend thread started (tokio runtime ready).".to_owned(),
    );

    // 015-mcp-agent-exposure: bring the stdio MCP server up if the operator has enabled it, and
    // in `--only-mcp` mode run it *as* this process. `Done` means the whole backend is finished.
    if mcp::bootstrap(&rt, &mut state, &sender, &args) == mcp::Bootstrap::Done {
        return;
    }

    let state = Arc::new(tokio::sync::Mutex::new(state));

    // Use a channel to coordinate task completions for shutdown
    let (task_tx, mut task_rx) = tokio::sync::mpsc::unbounded_channel();

    // Receive commands until the channel closes
    while let Ok(request) = receiver.recv() {
        let request: CmdRequest = request.into();
        tracing::debug!(
            "Received command {:?}: {:?}",
            request.id,
            std::mem::discriminant(&request.cmd)
        );

        let state = state.clone();
        let sender = sender.clone();
        let task_tx = task_tx.clone();

        // Spawn command handler as a background task
        rt.spawn(async move {
            run_cmd(request, &state, &sender).await;
            // Signal task completion
            let _ = task_tx.send(());
        });
    }

    // Give spawned tasks a moment to finish
    rt.block_on(async {
        // Allow some tasks to complete
        for _ in 0..10 {
            tokio::select! {
                _ = task_rx.recv() => {
                    // Task completed
                }
                _ = tokio::time::sleep(Duration::from_millis(50)) => {
                    break;
                }
            }
        }
    });

    // Final cleanup on exit
    rt.block_on(async {
        let mut final_state = state.lock().await;
        state::cleanup_all_tasks(&mut final_state);
        events::notify_info(
            final_state.cluster.seq.as_ref(),
            &sender,
            "Backend thread stopped.".to_owned(),
        );
    });
}

/// Dispatch one command under the state lock, then emit its `CommandFinished`.
///
/// Runs inside [`events::in_cmd_scope`], so every envelope emitted while handling the
/// command defaults to its request correlation, and every error notification (from the
/// handler itself, or from a returned `Err`, reported here) marks it failed.
async fn run_cmd(
    request: CmdRequest,
    state: &tokio::sync::Mutex<BackendState>,
    sender: &Sender<BackendEventEnvelope>,
) {
    let CmdRequest { id, cmd } = request;
    events::in_cmd_scope(id, async move {
        let route = finished_route(&cmd);
        let mut state = state.lock().await;
        match cmd::dispatch_cmd(cmd, &mut state, sender).await {
            Ok(true) => tracing::debug!("Command signaled shutdown"),
            Ok(false) => {}
            // `{:#}` keeps the whole anyhow chain (e.g. the Kubernetes API message).
            Err(err) => {
                events::notify_error(state.cluster.seq.as_ref(), sender, format!("{err:#}"))
            }
        }
        let errors = events::take_cmd_errors();
        let (ok, error) = if errors.is_empty() {
            (true, None)
        } else {
            (false, Some(errors.join("\n")))
        };
        events::emit(
            state.cluster.seq.as_ref(),
            sender,
            route,
            Correlation::None, // → the command's Request correlation
            BackendEvent::CommandFinished { ok, error },
        );
    })
    .await;
}

/// Route of a command's `CommandFinished`: its section, namespaced to the target
/// namespace for single-namespace writes. `context` is left `None` on purpose so a UI
/// never discards the terminal event as stale after a context switch.
fn finished_route(cmd: &BackendCmd) -> Route {
    let Some(section) = cmd.section() else {
        return Route::global(section_keys::system());
    };
    match cmd.mutation_target() {
        MutationTarget::Namespaced { mut namespaces } if namespaces.len() == 1 => {
            Route::namespaced(section, None, namespaces.pop())
        }
        MutationTarget::ClusterScoped => Route::cluster(section, None),
        _ => Route {
            section,
            scope: ScopeKind::Namespaced,
            context: None,
            namespace: None,
        },
    }
}

/// Restore saved port forwards from previous session
async fn restore_saved_port_forwards(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let saved_ports = persistence::load_all_ports()?;
    if saved_ports.is_empty() {
        return Ok(());
    }

    let client = state
        .cluster
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No Kubernetes client"))?;
    let mut restored_count = 0;

    for port_config in saved_ports {
        // Only restore port forwards with auto_start enabled
        if !port_config.auto_start {
            continue;
        }

        match state
            .port_forward_manager
            .start(
                client,
                &port_config.namespace,
                &port_config.pod,
                port_config.local_port,
                port_config.remote_port,
                port_config.container.clone(),
            )
            .await
        {
            Ok(tunnel_id) => {
                // Update the tunnel with auto_start flag
                if let Some(tunnel) = state.port_forward_manager.tunnels.get_mut(&tunnel_id) {
                    tunnel.auto_start = true;
                }
                restored_count += 1;
                events::notify_info(
                    state.cluster.seq.as_ref(),
                    sender,
                    format!(
                        "Restored port forward: {}:{} -> {}:{}/{}",
                        port_config.pod,
                        port_config.local_port,
                        port_config.namespace,
                        port_config.pod,
                        port_config.remote_port
                    ),
                );
            }
            Err(e) => {
                events::notify_info(
                    state.cluster.seq.as_ref(),
                    sender,
                    format!(
                        "Failed to restore port forward for {}/{}: {}",
                        port_config.namespace, port_config.pod, e
                    ),
                );
            }
        }
    }

    if restored_count > 0 {
        events::notify_info(
            state.cluster.seq.as_ref(),
            sender,
            format!("Restored {} port forwards", restored_count),
        );
        // Emit updated list to UI
        crate::backend::portforward::emit_port_forward_list(state, sender);
    }

    Ok(())
}

// Ensure Prometheus metrics are accessible via port forward
// Moved to backend/integrations.rs: ensure_metrics_port_forward()
/*
async fn ensure_metrics_port_forward(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state
        .cluster
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No Kubernetes client"))?;

    // Load user-configured Prometheus first (per context), then fall back to common defaults
    let context = state
        .current_context
        .clone()
        .unwrap_or_else(|| "default".to_owned());

    let mut prometheus_configs: Vec<(String, String, u16)> = Vec::new();

    if let Ok(cfg) = IntegrationConfig::load(&context) {
        if let Some(prom) = cfg.prometheus.as_ref().filter(|p| p.enabled) {
            prometheus_configs.push((prom.namespace.clone(), prom.service.clone(), prom.port));
        }
    }

    // Try common Prometheus locations
    prometheus_configs.extend_from_slice(&[
        ("monitoring".to_owned(), "prometheus".to_owned(), 9090u16),
        ("prometheus".to_owned(), "prometheus".to_owned(), 9090),
        ("kube-monitoring".to_owned(), "prometheus".to_owned(), 9090),
    ]);

    for (namespace, service, port) in prometheus_configs {
        // Check if a pod matching the service exists
        match find_pod_for_service(client, &namespace, &service).await {
            Ok(Some(pod_name)) => {
                // Check if a port forward for this pod already exists
                let existing = state.port_forward_manager.list().iter().any(|pf| {
                    pf.namespace == namespace && pf.pod == pod_name && pf.local_port == port
                });

                if existing {
                    tracing::debug!(
                        "Port forward for {}/{} on port {} already exists, skipping",
                        namespace,
                        pod_name,
                        port
                    );
                    return Ok(());
                }

                match state
                    .port_forward_manager
                    .start(client, &namespace, &pod_name, port, port, None)
                    .await
                {
                    Ok(_) => {
                        events::notify_info(
                            state.cluster.seq.as_ref(),
                            sender,
                            format!(
                                "Auto-started Prometheus port forward: localhost:{} -> {}/{}:{}",
                                port, namespace, pod_name, port
                            ),
                        );
                        // Emit updated list and save to disk
                        crate::backend::portforward::emit_port_forward_list(state, sender);
                        crate::backend::portforward::save_port_forwards_to_disk(state);
                        return Ok(());
                    }
                    Err(e) => {
                        tracing::debug!(
                            "Failed to start port forward for {}/{}: {}",
                            namespace,
                            pod_name,
                            e
                        );
                    }
                }
            }
            Ok(None) => {
                tracing::debug!("No pod found for service {}/{}", namespace, service);
            }
            Err(e) => {
                tracing::debug!("Error finding pod for {}/{}: {}", namespace, service, e);
            }
        }
    }

    // Fallback: Check if port 9090 is available locally and try to find any Prometheus instance
    if crate::backend::portforward::is_local_port_available(9090) {
        // Try to find any Prometheus pod in common namespaces
        for namespace in &["monitoring", "prometheus", "kube-monitoring", "default"] {
            if let Ok(Some(pod_name)) = find_any_prometheus_pod(client, namespace).await {
                // Check if a port forward for this pod already exists
                let existing = state.port_forward_manager.list().iter().any(|pf| {
                    pf.namespace == *namespace && pf.pod == pod_name && pf.local_port == 9090
                });

                if existing {
                    tracing::debug!(
                        "Port forward for {}/{} on port 9090 already exists, skipping",
                        namespace,
                        pod_name
                    );
                    return Ok(());
                }

                match state
                    .port_forward_manager
                    .start(client, namespace, &pod_name, 9090, 9090, None)
                    .await
                {
                    Ok(_) => {
                        events::notify_info(
                            state.cluster.seq.as_ref(),
                            sender,
                            format!(
                                "Auto-discovered Prometheus port forward: localhost:9090 -> {}/{}:9090",
                                namespace, pod_name
                            ),
                        );
                        // Emit updated list and save to disk
                        crate::backend::portforward::emit_port_forward_list(state, sender);
                        crate::backend::portforward::save_port_forwards_to_disk(state);
                        return Ok(());
                    }
                    Err(e) => {
                        tracing::debug!(
                            "Failed to start port forward for {}/{}: {}",
                            namespace,
                            pod_name,
                            e
                        );
                    }
                }
            }
        }
    }

    events::notify_info(
        state.cluster.seq.as_ref(),
        sender,
        "Prometheus auto port-forward not started (no matching service/pod found). Configure a port forward under Network > Port Forwards or adjust Prometheus settings under Integrations.".to_owned(),
    );

    Ok(())
}
*/

// (moved to backend/integrations.rs)
/*
/// Find a pod backing a service by checking pod labels
async fn find_pod_for_service(
    client: &::kube::Client,
    namespace: &str,
    service: &str,
) -> anyhow::Result<Option<String>> {
    use ::kube::Api;
    use k8s_openapi::api::core::v1::Service;

    let svc_api = Api::<Service>::namespaced(client.clone(), namespace);

    match svc_api.get(service).await {
        Ok(svc) => {
            // First, try to resolve via Endpoints to honor any non-standard selectors/endpoints
            if let Some(pod_name) = find_pod_via_endpoints(client, namespace, service).await? {
                return Ok(Some(pod_name));
            }

            // Get the selector labels from the service
            if let Some(selector) = svc.spec.as_ref().and_then(|s| s.selector.as_ref()) {
                // Build a label selector string
                let mut label_query = String::new();
                for (i, (key, val)) in selector.iter().enumerate() {
                    if i > 0 {
                        label_query.push(',');
                    }
                    label_query.push_str(&format!("{}={}", key, val));
                }

                // Find a pod matching these labels
                use k8s_openapi::api::core::v1::Pod;
                let pod_api = Api::<Pod>::namespaced(client.clone(), namespace);
                let list_params = ::kube::api::ListParams::default().labels(&label_query);

                match pod_api.list(&list_params).await {
                    Ok(pods) => {
                        if let Some(pod) = pods.items.first() {
                            if let Some(name) = pod.metadata.name.as_ref() {
                                return Ok(Some(name.clone()));
                            }
                        }
                    }
                    Err(_) => {}
                }
            }
        }
        Err(_) => {}
    }

    Ok(None)
}
*/

// Prometheus helpers moved to backend/integrations.rs

// Port availability helper now provided by backend/portforward.rs

// Prometheus helpers moved to backend/integrations.rs

// Prometheus helpers moved to backend/integrations.rs

// Port forward helpers moved to backend/portforward.rs:
// - emit_port_forward_list(state, sender)
// - save_port_forwards_to_disk(state)
// Port check helper centralized in backend/portforward.rs
