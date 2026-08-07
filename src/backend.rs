use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, Correlation, ResourceListEvent, Route,
    SystemEvent, section_keys,
};
use crossbeam_channel::{Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Runtime;

use crate::backend::integrations::{IntegrationConfig, PrometheusConfig};
use crate::config::Args;

mod cmd;
mod cronjobs;
mod daemonsets;
mod deployments;
mod endpoints;
mod events;
mod flux;
mod ingressclasses;
mod ingresses;
pub mod integrations;
mod interactions;
mod jobs;
mod kube;
mod longhorn;
mod metrics;
mod metrics_provider;
mod networkpolicies;
mod nodes;
pub mod persistence;
mod pods;
mod portforward;
mod replicasets;
mod replicationcontrollers;
mod services;
mod state;
mod statefulsets;
mod sync;
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
mod resourcequotas;
mod rolebindings;
mod roles;
mod runtimeclasses;
mod secrets;
mod storageclasses;
mod validatingwebhooks;

use state::BackendState;

pub fn init(receiver: Receiver<BackendCmd>, sender: Sender<BackendEventEnvelope>, args: Args) {
    let rt = Runtime::new().expect("Failed to create Tokio runtime");

    let mut state = BackendState::new();

    // Clear metrics history on startup to begin fresh collection
    if let Err(e) = persistence::clear_metrics_history() {
        tracing::warn!("Failed to clear metrics history on startup: {}", e);
    }

    // Create MCP state and persist port
    let mcp_state = std::sync::Arc::new(crate::mcp_server::McpAppState::new());
    state.mcp.state = Some(mcp_state.clone());
    state.mcp.port = args.mcp_port;

    // Load auto-start configuration
    let auto_start_config = persistence::AutoStartConfig::load().unwrap_or_default();

    // Spawn MCP HTTP server if enabled via CLI args, only_mcp mode, or auto-start config
    // Port 18288: 1.8288 meters ≈ 1 fathom (unit of depth measurement)
    let initial_mcp_http_handle = if args.mcp_http || args.only_mcp || auto_start_config.mcp_http {
        tracing::info!(
            "Starting MCP HTTP server on http://127.0.0.1:{} (18288 = 1.8288m ≈ 1 fathom)",
            args.mcp_port
        );
        Some(rt.spawn({
            let s = mcp_state.clone();
            let p = args.mcp_port;
            async move {
                let _ = crate::mcp_server::run_http_mcp_server(s, p).await;
            }
        }))
    } else {
        None
    };

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

    // Wrap state in Arc<Mutex> and store initial MCP handle if present
    let state = Arc::new(tokio::sync::Mutex::new(state));
    if let Some(handle) = initial_mcp_http_handle {
        let state_clone = state.clone();
        rt.block_on(async move {
            let mut s = state_clone.lock().await;
            s.mcp.http_task = Some(handle);
        });
    }

    // Use a channel to coordinate task completions for shutdown
    let (task_tx, mut task_rx) = tokio::sync::mpsc::unbounded_channel();

    loop {
        // Try to receive a command (non-blocking check for shutdown signal first)
        let cmd = match receiver.recv() {
            Ok(cmd) => cmd,
            Err(_) => break,
        };

        tracing::debug!("Received command: {:?}", std::mem::discriminant(&cmd));

        let state = state.clone();
        let sender = sender.clone();
        let task_tx = task_tx.clone();

        // Spawn command handler as a background task
        rt.spawn(async move {
            let mut state = state.lock().await;
            let result = cmd::dispatch_cmd(cmd, &mut state, &sender).await;
            match result {
                Ok(should_exit) if should_exit => {
                    tracing::debug!("Command signaled shutdown");
                }
                Err(err) => {
                    events::notify_error(state.cluster.seq.as_ref(), &sender, err.to_string())
                }
                _ => {}
            }
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

    // Cleanup: abort MCP HTTP server if running
    {
        let state_arc = state.clone();
        rt.block_on(async move {
            let mut s = state_arc.lock().await;
            state::abort_task(&mut s.mcp.http_task);
        });
    }

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

async fn handle_cmd(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    match cmd {
        BackendCmd::Initialize => {
            // Contexts can be loaded even before we connect.
            let _ = kube::load_and_send_contexts(
                state.cluster.seq.as_ref(),
                state.cluster.current_context.clone(),
                sender,
            );

            events::notify_info(
                state.cluster.seq.as_ref(),
                sender,
                "Connecting to cluster (kubeconfig/in-cluster)...".to_owned(),
            );

            let client = kube::connect_client_for_current_context(state).await?;
            state.cluster.client = Some(client.clone());

            // Share client with MCP server for direct K8s tool calls
            if let Some(mcp) = &state.mcp.state {
                *mcp.kube_client.write().await = Some(client);
            }

            // Best-effort: if we didn't have an explicit context selection, treat kubeconfig's
            // current context as the backend-authoritative active context.
            if state.cluster.current_context.is_none() {
                state.cluster.current_context = kube::read_kubeconfig_current_context();
            }
            if let (Some(mcp), Some(ctx)) = (&state.mcp.state, &state.cluster.current_context) {
                mcp.set_context(ctx.clone()).await;
            }
            events::emit(
                state.cluster.seq.as_ref(),
                sender,
                Route::global(section_keys::system()),
                Correlation::None,
                BackendEvent::System(SystemEvent::ContextChanged {
                    current: state.cluster.current_context.clone(),
                }),
            );

            events::notify_info(state.cluster.seq.as_ref(), sender, "Connected.".to_owned());

            // Discover optional backing services.
            metrics::probe_metrics_api(state, sender).await?;

            // Auto-discover Prometheus services in the monitoring namespace
            if let Some(client) = &state.cluster.client
                && let Some(ctx) = &state.cluster.current_context
            {
                let _ = crate::backend::integrations::auto_discover_prometheus(client, ctx).await;
            }

            // Restore saved port forwards from last session
            if let Err(e) = restore_saved_port_forwards(state, sender).await {
                events::notify_info(
                    state.cluster.seq.as_ref(),
                    sender,
                    format!("Note: Could not restore port forwards: {}", e),
                );
            }

            // Ensure metrics are accessible via port forward
            if let Err(e) =
                crate::backend::integrations::ensure_metrics_port_forward(state, sender).await
            {
                events::notify_info(
                    state.cluster.seq.as_ref(),
                    sender,
                    format!("Note: Could not auto-setup metrics port forward: {}", e),
                );
            }

            sync::refresh_namespaces(state, sender).await?;
            pods::refresh_pods(state, sender).await?;
            deployments::refresh_deployments(state, sender).await?;

            sync::ensure_background_sync(state, sender).await?;
        }

        BackendCmd::RefreshContexts => {
            let _ = kube::load_and_send_contexts(
                state.cluster.seq.as_ref(),
                state.cluster.current_context.clone(),
                sender,
            );
        }

        BackendCmd::SetContext { context } => {
            // Stop background activity and reconnect with the new context.
            state.log_stream.stop();
            state::abort_task(&mut state.watch_tasks.pod_watch_task);
            state::abort_task(&mut state.watch_tasks.namespace_sync_task);

            state.cluster.current_context = Some(context.clone());
            state.metrics.reset_probe();
            if let Some(mcp) = &state.mcp.state {
                mcp.set_context(context.clone()).await;
            }

            // Context switch invalidates namespace filters; reset to "all" on the backend.
            state.cluster.namespace_filter.clear();
            if let Ok(mut shared) = state.cluster.namespace_filter_shared.write() {
                shared.clear();
            }
            events::emit(
                state.cluster.seq.as_ref(),
                sender,
                Route::global(section_keys::system()),
                Correlation::None,
                BackendEvent::System(SystemEvent::NamespaceFilterChanged {
                    namespaces: Vec::new(),
                }),
            );

            events::notify_info(
                state.cluster.seq.as_ref(),
                sender,
                format!("Switching context to '{}'...", context),
            );

            let client = kube::connect_client_for_current_context(state).await?;
            state.cluster.client = Some(client.clone());

            // Share new client with MCP server
            if let Some(mcp) = &state.mcp.state {
                *mcp.kube_client.write().await = Some(client);
            }

            events::emit(
                state.cluster.seq.as_ref(),
                sender,
                Route::global(section_keys::system()),
                Correlation::None,
                BackendEvent::System(SystemEvent::ContextChanged {
                    current: state.cluster.current_context.clone(),
                }),
            );

            // Re-load contexts so UI reflects the selected one.
            let _ = kube::load_and_send_contexts(
                state.cluster.seq.as_ref(),
                state.cluster.current_context.clone(),
                sender,
            );

            metrics::probe_metrics_api(state, sender).await?;

            // Auto-discover Prometheus services in the monitoring namespace
            if let Some(client) = &state.cluster.client
                && let Some(ctx) = &state.cluster.current_context
            {
                let _ = crate::backend::integrations::auto_discover_prometheus(client, ctx).await;
            }

            sync::refresh_namespaces(state, sender).await?;
            pods::refresh_pods(state, sender).await?;
            deployments::refresh_deployments(state, sender).await?;
            sync::ensure_background_sync(state, sender).await?;

            events::notify_info(
                state.cluster.seq.as_ref(),
                sender,
                "Context switched.".to_owned(),
            );
        }

        BackendCmd::RefreshNamespaces => {
            sync::refresh_namespaces(state, sender).await?;
        }

        BackendCmd::RefreshNodes => {
            tracing::debug!("RefreshNodes command received");
            // Best-effort: fetch node list and emit event
            if let Some(client) = &state.cluster.client {
                use k8s_openapi::api::core::v1::{Node, Pod};
                let node_api = ::kube::Api::<Node>::all(client.clone());
                let pod_api = ::kube::Api::<Pod>::all(client.clone());
                // Fetch nodes and pods concurrently for pod_count per node
                let lp = ::kube::api::ListParams::default();
                let (nodes_result, pods_result) =
                    tokio::join!(node_api.list(&lp), pod_api.list(&lp),);
                if let Ok(nodes) = nodes_result {
                    // Build a map of node_name -> pod count from running pods
                    let pod_counts: std::collections::HashMap<String, u32> = pods_result
                        .map(|pods| {
                            let mut counts: std::collections::HashMap<String, u32> =
                                std::collections::HashMap::new();
                            for pod in pods.items {
                                if let Some(node_name) =
                                    pod.spec.as_ref().and_then(|s| s.node_name.clone())
                                {
                                    *counts.entry(node_name).or_default() += 1;
                                }
                            }
                            counts
                        })
                        .unwrap_or_default();

                    tracing::debug!("Fetched {} nodes from API", nodes.items.len());
                    let summaries: Vec<crate::protocol::v1::NodeSummary> = nodes
                        .items
                        .into_iter()
                        .map(|node| {
                            let meta = &node.metadata;
                            let status = &node.status;
                            let node_name = meta.name.clone().unwrap_or_default();
                            let allocatable = status
                                .as_ref()
                                .and_then(|s| s.allocatable.as_ref())
                                .cloned()
                                .unwrap_or_default();

                            // Parse CPU quantity: handles "4" (cores), "500m" (millicores), "1.5" (cores)
                            let cpu_millicores = allocatable
                                .get("cpu")
                                .map(|q| parse_cpu_to_millicores(&q.0))
                                .unwrap_or(0);

                            // Parse memory quantity: handles Ki, Mi, Gi, M, G
                            let mem_bytes = allocatable
                                .get("memory")
                                .map(|q| parse_memory_to_bytes(&q.0))
                                .unwrap_or(0);

                            // Parse node Ready status from conditions
                            let node_status = status
                                .as_ref()
                                .and_then(|s| s.conditions.as_ref())
                                .and_then(|conds| {
                                    conds.iter().find(|c| c.type_ == "Ready").map(|c| {
                                        if c.status == "True" {
                                            "Ready"
                                        } else {
                                            "NotReady"
                                        }
                                    })
                                })
                                .unwrap_or("Unknown")
                                .to_string();

                            let pod_count = pod_counts.get(&node_name).copied().unwrap_or(0);

                            // Parse pod capacity from allocatable.pods
                            let pod_capacity = allocatable
                                .get("pods")
                                .and_then(|q| q.0.parse::<u32>().ok())
                                .unwrap_or(110); // 110 is the Kubernetes default

                            crate::protocol::v1::NodeSummary {
                                name: node_name,
                                status: node_status,
                                role: meta
                                    .labels
                                    .as_ref()
                                    .and_then(|l| l.get("node-role.kubernetes.io/master"))
                                    .or_else(|| {
                                        meta.labels.as_ref().and_then(|l| {
                                            l.get("node-role.kubernetes.io/control-plane")
                                        })
                                    })
                                    .map(|_| "control-plane".to_string())
                                    .unwrap_or_else(|| "worker".to_string()),
                                pod_count,
                                pod_capacity,
                                cpu_allocatable_millicores: cpu_millicores,
                                memory_allocatable_bytes: mem_bytes,
                            }
                        })
                        .collect();

                    tracing::debug!("Emitting NodesUpdated event with {} nodes", summaries.len());
                    events::emit(
                        state.cluster.seq.as_ref(),
                        sender,
                        Route::global(section_keys::nodes()),
                        Correlation::None,
                        BackendEvent::ResourceList(ResourceListEvent::NodesUpdated(summaries)),
                    );
                } else if let Err(e) = nodes_result {
                    tracing::warn!("Failed to list nodes: {}", e);
                    events::notify_error(
                        state.cluster.seq.as_ref(),
                        sender,
                        format!("Failed to list nodes: {}", e),
                    );
                }
            }
        }

        BackendCmd::SelectNode { name } => {
            tracing::debug!("SelectNode command received for: {}", name);
            // Fetch node details
            if let Some(client) = &state.cluster.client {
                use k8s_openapi::api::core::v1::Node;
                let api = ::kube::Api::<Node>::all(client.clone());
                match api.get(&name).await {
                    Ok(node) => {
                        let details = crate::backend::nodes::extract_node_details(&node)?;
                        events::emit(
                            state.cluster.seq.as_ref(),
                            sender,
                            Route::global(section_keys::nodes()),
                            Correlation::None,
                            BackendEvent::ResourceRead(
                                crate::protocol::ResourceReadEvent::NodeDetailsUpdated(details),
                            ),
                        );
                    }
                    Err(e) => {
                        tracing::warn!("Failed to get node details: {}", e);
                        events::notify_error(
                            state.cluster.seq.as_ref(),
                            sender,
                            format!("Failed to get node details: {}", e),
                        );
                    }
                }
            }
        }

        BackendCmd::RefreshEvents => {
            // Best-effort: fetch recent events and emit
            if let Some(client) = &state.cluster.client {
                use k8s_openapi::api::core::v1::Event;
                let api = ::kube::Api::<Event>::all(client.clone());
                match api.list(&Default::default()).await {
                    Ok(event_list) => {
                        let summaries: Vec<crate::protocol::v1::KubernetesEvent> = event_list
                            .items
                            .into_iter()
                            .take(100) // Limit to last 100 events for performance
                            .map(|event| {
                                let involved = &event.involved_object;
                                crate::protocol::v1::KubernetesEvent {
                                    namespace: involved.namespace.clone().unwrap_or_default(),
                                    name: event.metadata.name.clone().unwrap_or_default(),
                                    event_type: event.type_.clone().unwrap_or_default(),
                                    involved_object_kind: involved.kind.clone().unwrap_or_default(),
                                    involved_object_name: involved.name.clone().unwrap_or_default(),
                                    reason: event.reason.clone().unwrap_or_default(),
                                    message: event.message.clone().unwrap_or_default(),
                                    count: event.count.unwrap_or(1) as u32,
                                    first_timestamp: event
                                        .first_timestamp
                                        .as_ref()
                                        .map(|t| t.0.to_rfc3339())
                                        .unwrap_or_default(),
                                    last_timestamp: event
                                        .last_timestamp
                                        .as_ref()
                                        .map(|t| t.0.to_rfc3339())
                                        .unwrap_or_default(),
                                    source: event
                                        .source
                                        .as_ref()
                                        .and_then(|s| s.component.clone())
                                        .unwrap_or_default(),
                                }
                            })
                            .collect();

                        let total_count = summaries.len() as u32;
                        events::emit(
                            state.cluster.seq.as_ref(),
                            sender,
                            Route::global(section_keys::events()),
                            Correlation::None,
                            BackendEvent::ResourceList(ResourceListEvent::EventsUpdated(
                                crate::protocol::v1::EventsSummary {
                                    events: summaries,
                                    total_count,
                                },
                            )),
                        );
                    }
                    Err(e) => {
                        tracing::warn!("Failed to list events: {}", e);
                        events::notify_error(
                            state.cluster.seq.as_ref(),
                            sender,
                            format!("Failed to list events: {}", e),
                        );
                    }
                }
            }
        }

        BackendCmd::SetNamespaceFilter { namespaces } => {
            state.cluster.namespace_filter = namespaces;
            if let Ok(mut shared) = state.cluster.namespace_filter_shared.write() {
                *shared = state.cluster.namespace_filter.clone();
            }

            events::emit(
                state.cluster.seq.as_ref(),
                sender,
                Route::global(section_keys::system()),
                Correlation::None,
                BackendEvent::System(SystemEvent::NamespaceFilterChanged {
                    namespaces: state.cluster.namespace_filter.clone(),
                }),
            );
            pods::refresh_pods(state, sender).await?;
            deployments::refresh_deployments(state, sender).await?;
        }

        BackendCmd::RefreshPods => {
            pods::refresh_pods(state, sender).await?;
        }

        BackendCmd::DeletePod { namespace, name } => {
            pods::delete_pod(state, sender, &namespace, &name).await?;
            pods::refresh_pods(state, sender).await?;
        }

        BackendCmd::SelectPod { namespace, name } => {
            pods::fetch_pod_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::StartPodLogs {
            namespace,
            name,
            container,
        } => {
            pods::start_logs(state, sender, namespace, name, container).await?;
        }

        BackendCmd::StopPodLogs => {
            state.log_stream.stop();
        }

        BackendCmd::FetchPodMetrics { namespace, name } => {
            metrics::fetch_metrics(state, sender, &namespace, &name).await?;
        }

        BackendCmd::StartMetricsPolling {
            namespace,
            name,
            interval_secs,
        } => {
            // Fetch pod to get resource specs
            use k8s_openapi::api::core::v1::Pod;
            let client = match &state.cluster.client {
                Some(c) => c,
                None => anyhow::bail!("No Kubernetes client available"),
            };
            let pod_api = ::kube::Api::<Pod>::namespaced(client.clone(), &namespace);
            let pod = pod_api.get(&name).await?;

            // Extract resource specs (reuse logic from pods module)
            let resource_specs = pods::extract_resource_specs(&pod);

            metrics::start_metrics_polling(
                state,
                sender,
                namespace,
                name,
                resource_specs,
                interval_secs,
            )
            .await?;
        }

        BackendCmd::StopMetricsPolling => {
            metrics::stop_metrics_polling(state);
        }

        BackendCmd::StartWorkloadMetricsPolling {
            namespace,
            workload_name,
            workload_kind,
            pod_selector,
            interval_secs,
        } => {
            metrics::start_workload_metrics_polling(
                state,
                sender,
                namespace,
                workload_name,
                workload_kind,
                pod_selector,
                interval_secs,
            )
            .await?;
        }

        BackendCmd::StopWorkloadMetricsPolling => {
            metrics::stop_workload_metrics_polling(state);
        }

        BackendCmd::RefreshDeployments => {
            deployments::refresh_deployments(state, sender).await?;
        }

        BackendCmd::SelectDeployment { namespace, name } => {
            deployments::fetch_deployment_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::ScaleDeployment {
            namespace,
            name,
            replicas,
        } => {
            deployments::scale_deployment(state, sender, &namespace, &name, replicas).await?;
            deployments::refresh_deployments(state, sender).await?;
        }

        BackendCmd::RestartDeployment { namespace, name } => {
            deployments::restart_deployment(state, sender, &namespace, &name).await?;
            deployments::refresh_deployments(state, sender).await?;
        }

        BackendCmd::DeleteDeployment { namespace, name } => {
            deployments::delete_deployment(state, sender, &namespace, &name).await?;
            deployments::refresh_deployments(state, sender).await?;
        }

        // Workloads / DaemonSets section
        BackendCmd::RefreshDaemonSets => {
            daemonsets::refresh_daemonsets(state, sender).await?;
        }

        BackendCmd::SelectDaemonSet { namespace, name } => {
            daemonsets::fetch_daemonset_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::RestartDaemonSet { namespace, name } => {
            daemonsets::restart_daemonset(state, sender, &namespace, &name).await?;
            daemonsets::refresh_daemonsets(state, sender).await?;
        }

        BackendCmd::DeleteDaemonSet { namespace, name } => {
            daemonsets::delete_daemonset(state, sender, &namespace, &name).await?;
            daemonsets::refresh_daemonsets(state, sender).await?;
        }

        // Workloads / StatefulSets section
        BackendCmd::RefreshStatefulSets => {
            statefulsets::refresh_statefulsets(state, sender).await?;
        }

        BackendCmd::SelectStatefulSet { namespace, name } => {
            statefulsets::fetch_statefulset_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::ScaleStatefulSet {
            namespace,
            name,
            replicas,
        } => {
            statefulsets::scale_statefulset(state, sender, &namespace, &name, replicas).await?;
            statefulsets::refresh_statefulsets(state, sender).await?;
        }

        BackendCmd::RestartStatefulSet { namespace, name } => {
            statefulsets::restart_statefulset(state, sender, &namespace, &name).await?;
            statefulsets::refresh_statefulsets(state, sender).await?;
        }

        BackendCmd::DeleteStatefulSet { namespace, name } => {
            statefulsets::delete_statefulset(state, sender, &namespace, &name).await?;
            statefulsets::refresh_statefulsets(state, sender).await?;
        }

        // Workloads / ReplicaSets section
        BackendCmd::RefreshReplicaSets => {
            replicasets::refresh_replicasets(state, sender).await?;
        }

        BackendCmd::SelectReplicaSet { namespace, name } => {
            replicasets::fetch_replicaset_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::ScaleReplicaSet {
            namespace,
            name,
            replicas,
        } => {
            replicasets::scale_replicaset(state, sender, &namespace, &name, replicas).await?;
            replicasets::refresh_replicasets(state, sender).await?;
        }

        BackendCmd::RestartReplicaSet { namespace, name } => {
            replicasets::restart_replicaset(state, sender, &namespace, &name).await?;
            replicasets::refresh_replicasets(state, sender).await?;
        }

        BackendCmd::DeleteReplicaSet { namespace, name } => {
            replicasets::delete_replicaset(state, sender, &namespace, &name).await?;
            replicasets::refresh_replicasets(state, sender).await?;
        }

        // Workloads / ReplicationControllers section
        BackendCmd::RefreshReplicationControllers => {
            replicationcontrollers::refresh_replicationcontrollers(state, sender).await?;
        }

        BackendCmd::SelectReplicationController { namespace, name } => {
            replicationcontrollers::fetch_replicationcontroller_details(
                state, sender, &namespace, &name,
            )
            .await?;
        }

        BackendCmd::ScaleReplicationController {
            namespace,
            name,
            replicas,
        } => {
            replicationcontrollers::scale_replicationcontroller(
                state, sender, &namespace, &name, replicas,
            )
            .await?;
            replicationcontrollers::refresh_replicationcontrollers(state, sender).await?;
        }

        BackendCmd::RestartReplicationController { namespace, name } => {
            replicationcontrollers::restart_replicationcontroller(state, sender, &namespace, &name)
                .await?;
            replicationcontrollers::refresh_replicationcontrollers(state, sender).await?;
        }

        BackendCmd::DeleteReplicationController { namespace, name } => {
            replicationcontrollers::delete_replicationcontroller(state, sender, &namespace, &name)
                .await?;
            replicationcontrollers::refresh_replicationcontrollers(state, sender).await?;
        }

        // Workloads / Jobs section
        BackendCmd::RefreshJobs => {
            jobs::refresh_jobs(state, sender).await?;
        }

        BackendCmd::SelectJob { namespace, name } => {
            jobs::fetch_job_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::DeleteJob { namespace, name } => {
            jobs::delete_job(state, sender, &namespace, &name).await?;
            jobs::refresh_jobs(state, sender).await?;
        }

        // Workloads / CronJobs section
        BackendCmd::RefreshCronJobs => {
            cronjobs::refresh_cronjobs(state, sender).await?;
        }

        BackendCmd::SelectCronJob { namespace, name } => {
            cronjobs::fetch_cronjob_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::SuspendCronJob {
            namespace,
            name,
            suspend,
        } => {
            cronjobs::suspend_cronjob(state, sender, &namespace, &name, suspend).await?;
            cronjobs::refresh_cronjobs(state, sender).await?;
        }

        BackendCmd::TriggerCronJob { namespace, name } => {
            cronjobs::trigger_cronjob(state, sender, &namespace, &name).await?;
        }

        BackendCmd::DeleteCronJob { namespace, name } => {
            cronjobs::delete_cronjob(state, sender, &namespace, &name).await?;
            cronjobs::refresh_cronjobs(state, sender).await?;
        }

        BackendCmd::SimulateSlowCall => {
            events::notify_info(
                state.cluster.seq.as_ref(),
                sender,
                "Simulating slow call...".into(),
            );
            tokio::time::sleep(Duration::from_secs(2)).await;
            events::notify_info(state.cluster.seq.as_ref(), sender, "Slow call done.".into());
        }

        BackendCmd::BulkDelete { items } => {
            let mut success_count = 0;
            let mut failed_items = Vec::new();

            for (namespace, name) in items {
                if state.cluster.client.is_some() {
                    match pods::delete_pod(state, sender, &namespace, &name).await {
                        Ok(_) => success_count += 1,
                        Err(e) => {
                            tracing::warn!(namespace = %namespace, name = %name, "Bulk delete failed: {}", e);
                            failed_items.push(format!("{}/{}", namespace, name));
                        }
                    }
                } else {
                    failed_items.push(format!("{}/{}", namespace, name));
                }
            }

            if success_count > 0 {
                events::notify_info(
                    state.cluster.seq.as_ref(),
                    sender,
                    format!("Deleted {} item(s)", success_count),
                );
            }

            if !failed_items.is_empty() {
                events::notify_error(
                    state.cluster.seq.as_ref(),
                    sender,
                    format!("Failed to delete: {}", failed_items.join(", ")),
                );
            }

            // Refresh the relevant sections
            pods::refresh_pods(state, sender).await?;
        }

        BackendCmd::SetActiveSection { section } => {
            // Light prefetch: keep current section fresh.
            if section == section_keys::workloads_pods() {
                pods::refresh_pods(state, sender).await?;
            } else if section == section_keys::workloads_deployments() {
                deployments::refresh_deployments(state, sender).await?;
            }
        }

        // Network / Services section
        BackendCmd::RefreshServices => {
            services::refresh_services(state, sender).await?;
        }

        BackendCmd::SelectService { namespace, name } => {
            services::fetch_service_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::DeleteService { namespace, name } => {
            services::delete_service(state, sender, &namespace, &name).await?;
        }

        // Network / Endpoints section
        BackendCmd::RefreshEndpoints => {
            endpoints::refresh_endpoints(state, sender).await?;
        }

        BackendCmd::SelectEndpoint { namespace, name } => {
            endpoints::fetch_endpoint_details(state, sender, &namespace, &name).await?;
        }

        // Network / Ingresses section
        BackendCmd::RefreshIngresses => {
            ingresses::refresh_ingresses(state, sender).await?;
        }

        BackendCmd::SelectIngress { namespace, name } => {
            ingresses::fetch_ingress_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::DeleteIngress { namespace, name } => {
            ingresses::delete_ingress(state, sender, &namespace, &name).await?;
        }

        // Network / Ingress Classes section
        BackendCmd::RefreshIngressClasses => {
            ingressclasses::refresh_ingress_classes(state, sender).await?;
        }

        BackendCmd::SelectIngressClass { name } => {
            ingressclasses::fetch_ingress_class_details(state, sender, &name).await?;
        }

        // Network / Network Policies section
        BackendCmd::RefreshNetworkPolicies => {
            networkpolicies::refresh_network_policies(state, sender).await?;
        }

        BackendCmd::SelectNetworkPolicy { namespace, name } => {
            networkpolicies::fetch_network_policy_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::DeleteNetworkPolicy { namespace, name } => {
            networkpolicies::delete_network_policy(state, sender, &namespace, &name).await?;
        }

        BackendCmd::ConfigurePrometheus {
            namespace,
            service,
            port,
        } => {
            let context = state
                .cluster
                .current_context
                .clone()
                .unwrap_or_else(|| "default".to_owned());

            let mut cfg = IntegrationConfig::load(&context).unwrap_or(IntegrationConfig {
                context: context.clone(),
                prometheus: None,
                grafana: None,
                argo: None,
                flux: None,
            });

            cfg.context = context.clone();
            cfg.prometheus = Some(PrometheusConfig {
                enabled: true,
                namespace: namespace.clone(),
                service: service.clone(),
                port,
            });

            if let Err(e) = cfg.save() {
                events::notify_error(
                    state.cluster.seq.as_ref(),
                    sender,
                    format!("Failed to save Prometheus config: {}", e),
                );
            } else {
                // Start port-forward to Prometheus service
                let client = state.cluster.client_or_err()?;

                // Use port-forward manager to create a tunnel to the Prometheus service
                // Note: Prometheus typically runs in a StatefulSet, we'll target the service pod
                let pf_result = state
                    .port_forward_manager
                    .start(&client, &namespace, &service, port, 9090, None)
                    .await;

                match pf_result {
                    Ok(_tunnel_id) => {
                        // Create and store PrometheusClient
                        let url = format!("http://localhost:{}", port);
                        state.metrics.prometheus_client =
                            Some(metrics_provider::PrometheusClient::new(url));

                        events::notify_info(
                            state.cluster.seq.as_ref(),
                            sender,
                            format!(
                                "Configured Prometheus: {}/{} port {} (context: {}) - port-forward active",
                                namespace, service, port, context
                            ),
                        );
                    }
                    Err(e) => {
                        // Port-forward failed, but still configure the client (assumes manual port-forward)
                        let url = format!("http://localhost:{}", port);
                        state.metrics.prometheus_client =
                            Some(metrics_provider::PrometheusClient::new(url));

                        events::notify_info(
                            state.cluster.seq.as_ref(),
                            sender,
                            format!(
                                "Configured Prometheus: {}/{} port {} (context: {}) - port-forward failed, ensure manual tunnel is active: {}",
                                namespace, service, port, context, e
                            ),
                        );
                    }
                }
            }
        }
        BackendCmd::TestPrometheusConnection {
            namespace: _,
            service: _,
            port,
        } => {
            // Test Prometheus connection via localhost (assumes port-forward is established)
            let url = format!("http://localhost:{}/api/v1/query?query=up", port);
            match reqwest::Client::new().get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    // Connection successful - create and store client for future use
                    let client_url = format!("http://localhost:{}", port);
                    state.metrics.prometheus_client =
                        Some(metrics_provider::PrometheusClient::new(client_url));

                    events::emit(
                        state.cluster.seq.as_ref(),
                        sender,
                        Route::global(section_keys::integrations_metrics()),
                        Correlation::None,
                        BackendEvent::System(SystemEvent::PrometheusConnectionTest {
                            success: true,
                            message: "Connection successful".to_owned(),
                        }),
                    );
                }
                Ok(resp) => {
                    events::emit(
                        state.cluster.seq.as_ref(),
                        sender,
                        Route::global(section_keys::integrations_metrics()),
                        Correlation::None,
                        BackendEvent::System(SystemEvent::PrometheusConnectionTest {
                            success: false,
                            message: format!("HTTP {}", resp.status()),
                        }),
                    );
                }
                Err(e) => {
                    events::emit(
                        state.cluster.seq.as_ref(),
                        sender,
                        Route::global(section_keys::integrations_metrics()),
                        Correlation::None,
                        BackendEvent::System(SystemEvent::PrometheusConnectionTest {
                            success: false,
                            message: format!(
                                "Connection failed (ensure port-forward is active): {}",
                                e
                            ),
                        }),
                    );
                }
            }
        }
        BackendCmd::ConfigureGrafana {
            base_url,
            api_key: _,
        } => {
            // TODO: Configure Grafana integration
            events::notify_info(
                state.cluster.seq.as_ref(),
                sender,
                format!("Configured Grafana: {}", base_url),
            );
        }
        BackendCmd::TestGrafanaConnection { base_url, api_key } => {
            // Test Grafana connection via the provided base URL
            let client = reqwest::Client::new();
            let url = format!("{}/api/health", base_url);
            match client
                .get(&url)
                .header("Authorization", format!("Bearer {}", api_key))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    events::emit(
                        state.cluster.seq.as_ref(),
                        sender,
                        Route::global(section_keys::integrations_grafana()),
                        Correlation::None,
                        BackendEvent::System(SystemEvent::GrafanaConnectionTest {
                            success: true,
                            message: "Connection successful".to_owned(),
                        }),
                    );
                }
                Ok(resp) => {
                    events::emit(
                        state.cluster.seq.as_ref(),
                        sender,
                        Route::global(section_keys::integrations_grafana()),
                        Correlation::None,
                        BackendEvent::System(SystemEvent::GrafanaConnectionTest {
                            success: false,
                            message: format!("HTTP {}", resp.status()),
                        }),
                    );
                }
                Err(e) => {
                    events::emit(
                        state.cluster.seq.as_ref(),
                        sender,
                        Route::global(section_keys::integrations_grafana()),
                        Correlation::None,
                        BackendEvent::System(SystemEvent::GrafanaConnectionTest {
                            success: false,
                            message: e.to_string(),
                        }),
                    );
                }
            }
        }
        BackendCmd::RefreshIntegrations => match &state.cluster.client {
            Some(client) => match crate::backend::integrations::scan_metrics_stack(client).await {
                Ok(result) => {
                    events::emit(
                        state.cluster.seq.as_ref(),
                        sender,
                        Route::global(section_keys::integrations_metrics()),
                        Correlation::None,
                        BackendEvent::System(SystemEvent::MetricsStackScanned(result)),
                    );
                }
                Err(e) => {
                    // IntegrationsView's "scanning" flag (and the disabled
                    // Scan button / stuck "Scanning..." label it drives) only
                    // ever clears on a MetricsStackScanned event - it used to
                    // never arrive on this branch, leaving the button
                    // permanently disabled until the user switched cluster
                    // context. Emit an empty result alongside the error so
                    // the UI unblocks, in addition to the error notification.
                    events::notify_error(
                        state.cluster.seq.as_ref(),
                        sender,
                        format!("Failed to scan cluster for metrics stack: {}", e),
                    );
                    events::emit(
                        state.cluster.seq.as_ref(),
                        sender,
                        Route::global(section_keys::integrations_metrics()),
                        Correlation::None,
                        BackendEvent::System(SystemEvent::MetricsStackScanned(
                            crate::protocol::MetricsStackScanResult {
                                tier: crate::protocol::MetricsStackTier::None,
                                components: Vec::new(),
                                suggested_endpoint: None,
                            },
                        )),
                    );
                }
            },
            None => {
                events::notify_error(
                    state.cluster.seq.as_ref(),
                    sender,
                    "Not connected to a cluster.".to_owned(),
                );
                events::emit(
                    state.cluster.seq.as_ref(),
                    sender,
                    Route::global(section_keys::integrations_metrics()),
                    Correlation::None,
                    BackendEvent::System(SystemEvent::MetricsStackScanned(
                        crate::protocol::MetricsStackScanResult {
                            tier: crate::protocol::MetricsStackTier::None,
                            components: Vec::new(),
                            suggested_endpoint: None,
                        },
                    )),
                );
            }
        },
        BackendCmd::RefreshFlux => match &state.cluster.client {
            Some(client) => match crate::backend::flux::scan_flux(client).await {
                Ok(overview) => {
                    events::emit(
                        state.cluster.seq.as_ref(),
                        sender,
                        Route::global(section_keys::integrations_flux()),
                        Correlation::None,
                        BackendEvent::System(SystemEvent::FluxOverviewUpdated(overview)),
                    );
                }
                Err(e) => {
                    events::notify_error(
                        state.cluster.seq.as_ref(),
                        sender,
                        format!("Failed to scan cluster for Flux resources: {}", e),
                    );
                }
            },
            None => {
                events::notify_error(
                    state.cluster.seq.as_ref(),
                    sender,
                    "Not connected to a cluster.".to_owned(),
                );
            }
        },
        BackendCmd::OpenMcpConfigDir => match crate::mcp_server::mcp_dir() {
            Ok(path) => {
                if let Err(e) = std::fs::create_dir_all(&path) {
                    events::notify_error(
                        state.cluster.seq.as_ref(),
                        sender,
                        format!("Failed to create MCP dir: {}", e),
                    );
                } else {
                    let cmd = if cfg!(target_os = "windows") {
                        "explorer"
                    } else if cfg!(target_os = "macos") {
                        "open"
                    } else {
                        "xdg-open"
                    };

                    match tokio::process::Command::new(cmd).arg(&path).spawn() {
                        Ok(_) => {
                            events::notify_info(
                                state.cluster.seq.as_ref(),
                                sender,
                                format!("Opening MCP config directory: {}", path.display()),
                            );
                        }
                        Err(e) => {
                            events::notify_error(
                                state.cluster.seq.as_ref(),
                                sender,
                                format!("Failed to open MCP directory: {}", e),
                            );
                        }
                    }
                }
            }
            Err(e) => {
                events::notify_error(
                    state.cluster.seq.as_ref(),
                    sender,
                    format!("Failed to resolve MCP dir: {}", e),
                );
            }
        },
        BackendCmd::StartMcpServer => {
            // If already running, correct the UI rather than silently no-op'ing -
            // if it thought the server was stopped (e.g. after a missed status
            // update), a click here should still leave it showing "Running".
            if state.mcp.http_task.is_some() {
                events::emit(
                    state.cluster.seq.as_ref(),
                    sender,
                    Route::global(section_keys::integrations_mcp()),
                    Correlation::None,
                    BackendEvent::System(SystemEvent::McpServerStatus {
                        running: true,
                        message: "MCP server already running".to_owned(),
                    }),
                );
                return Ok(());
            }

            // Ensure state exists
            let mcp_state = match &state.mcp.state {
                Some(s) => s.clone(),
                None => {
                    let s = std::sync::Arc::new(crate::mcp_server::McpAppState::new());
                    state.mcp.state = Some(s.clone());
                    s
                }
            };

            // Initialize config files (best-effort)
            tokio::spawn(async move {
                let _ = crate::mcp_server::initialize_mcp_server().await;
            });

            // Bind first, *then* announce - a failed bind (e.g. port already
            // in use by something else) must not leave the UI stuck claiming
            // the server is running when nothing is actually listening.
            let port = state.mcp.port;
            match crate::mcp_server::bind_mcp_listener(port).await {
                Ok(listener) => {
                    let handle = tokio::spawn({
                        let s = mcp_state.clone();
                        async move {
                            let _ = crate::mcp_server::serve_mcp_http(s, listener).await;
                        }
                    });
                    state.mcp.http_task = Some(handle);

                    events::emit(
                        state.cluster.seq.as_ref(),
                        sender,
                        Route::global(section_keys::integrations_mcp()),
                        Correlation::None,
                        BackendEvent::System(SystemEvent::McpServerStatus {
                            running: true,
                            message: format!(
                                "MCP HTTP server starting on http://127.0.0.1:{}",
                                port
                            ),
                        }),
                    );
                    events::notify_info(
                        state.cluster.seq.as_ref(),
                        sender,
                        format!(
                            "MCP HTTP server starting on http://127.0.0.1:{} (1 fathom ≈ 1.8288m)",
                            port
                        ),
                    );
                }
                Err(e) => {
                    events::emit(
                        state.cluster.seq.as_ref(),
                        sender,
                        Route::global(section_keys::integrations_mcp()),
                        Correlation::None,
                        BackendEvent::System(SystemEvent::McpServerStatus {
                            running: false,
                            message: format!("Failed to start MCP server on port {}: {}", port, e),
                        }),
                    );
                }
            }
        }
        BackendCmd::RefreshMcpStatus => {
            let running = match &state.mcp.state {
                Some(mcp_state) => mcp_state.is_running_status().await,
                None => false,
            };
            events::emit(
                state.cluster.seq.as_ref(),
                sender,
                Route::global(section_keys::integrations_mcp()),
                Correlation::None,
                BackendEvent::System(SystemEvent::McpServerStatus {
                    running,
                    message: if running {
                        "MCP server is running".to_owned()
                    } else {
                        "MCP server is stopped".to_owned()
                    },
                }),
            );
        }
        BackendCmd::StopMcpServer => {
            // Abort HTTP task if running
            state::abort_task(&mut state.mcp.http_task);

            // Emit MCP server stopped event
            events::emit(
                state.cluster.seq.as_ref(),
                sender,
                Route::global(section_keys::integrations_mcp()),
                Correlation::None,
                BackendEvent::System(SystemEvent::McpServerStatus {
                    running: false,
                    message: "MCP server stopped".to_owned(),
                }),
            );
            events::notify_info(
                state.cluster.seq.as_ref(),
                sender,
                "MCP server stopped".to_owned(),
            );
        }

        // Storage / Config / Access resources
        BackendCmd::RefreshPersistentVolumes => {
            persistentvolumes::refresh_persistentvolumes(state, sender).await?;
        }

        BackendCmd::SelectPersistentVolume { name } => {
            persistentvolumes::fetch_persistentvolume_details(state, sender, &name).await?;
        }

        BackendCmd::DeletePersistentVolume { name } => {
            persistentvolumes::delete_persistentvolume(state, sender, &name).await?;
            persistentvolumes::refresh_persistentvolumes(state, sender).await?;
        }

        BackendCmd::RefreshPersistentVolumeClaims => {
            persistentvolumeclaims::refresh_persistentvolumeclaims(state, sender).await?;
        }

        BackendCmd::SelectPersistentVolumeClaim { namespace, name } => {
            persistentvolumeclaims::fetch_persistentvolumeclaim_details(
                state, sender, &namespace, &name,
            )
            .await?;
        }

        BackendCmd::DeletePersistentVolumeClaim { namespace, name } => {
            persistentvolumeclaims::delete_persistentvolumeclaim(state, sender, &namespace, &name)
                .await?;
            persistentvolumeclaims::refresh_persistentvolumeclaims(state, sender).await?;
        }

        BackendCmd::RefreshStorageClasses => {
            storageclasses::refresh_storageclasses(state, sender).await?;
        }

        BackendCmd::SelectStorageClass { name } => {
            storageclasses::fetch_storageclass_details(state, sender, &name).await?;
        }

        BackendCmd::DeleteStorageClass { name } => {
            storageclasses::delete_storageclass(state, sender, &name).await?;
            storageclasses::refresh_storageclasses(state, sender).await?;
        }

        BackendCmd::RefreshRoles => {
            roles::refresh_roles(state, sender).await?;
        }

        BackendCmd::SelectRole { namespace, name } => {
            roles::fetch_role_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::DeleteRole { namespace, name } => {
            roles::delete_role(state, sender, &namespace, &name).await?;
            roles::refresh_roles(state, sender).await?;
        }

        BackendCmd::RefreshRoleBindings => {
            rolebindings::refresh_role_bindings(state, sender).await?;
        }

        BackendCmd::SelectRoleBinding { namespace, name } => {
            rolebindings::fetch_role_binding_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::DeleteRoleBinding { namespace, name } => {
            rolebindings::delete_role_binding(state, sender, &namespace, &name).await?;
            rolebindings::refresh_role_bindings(state, sender).await?;
        }

        BackendCmd::RefreshClusterRoles => {
            clusterroles::refresh_cluster_roles(state, sender).await?;
        }

        BackendCmd::SelectClusterRole { name } => {
            clusterroles::fetch_cluster_role_details(state, sender, &name).await?;
        }

        BackendCmd::DeleteClusterRole { name } => {
            clusterroles::delete_cluster_role(state, sender, &name).await?;
            clusterroles::refresh_cluster_roles(state, sender).await?;
        }

        BackendCmd::RefreshClusterRoleBindings => {
            clusterrolebindings::refresh_cluster_role_bindings(state, sender).await?;
        }

        BackendCmd::SelectClusterRoleBinding { name } => {
            clusterrolebindings::fetch_cluster_role_binding_details(state, sender, &name).await?;
        }

        BackendCmd::DeleteClusterRoleBinding { name } => {
            clusterrolebindings::delete_cluster_role_binding(state, sender, &name).await?;
            clusterrolebindings::refresh_cluster_role_bindings(state, sender).await?;
        }

        BackendCmd::RefreshConfigMaps => {
            configmaps::refresh_configmaps(state, sender).await?;
        }

        BackendCmd::SelectConfigMap { namespace, name } => {
            configmaps::fetch_configmap_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::DeleteConfigMap { namespace, name } => {
            configmaps::delete_configmap(state, sender, &namespace, &name).await?;
            configmaps::refresh_configmaps(state, sender).await?;
        }

        BackendCmd::RefreshSecrets => {
            secrets::refresh_secrets(state, sender).await?;
        }

        BackendCmd::SelectSecret { namespace, name } => {
            secrets::fetch_secret_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::DeleteSecret { namespace, name } => {
            secrets::delete_secret(state, sender, &namespace, &name).await?;
            secrets::refresh_secrets(state, sender).await?;
        }

        BackendCmd::RefreshResourceQuotas => {
            resourcequotas::refresh_resourcequotas(state, sender).await?;
        }

        BackendCmd::SelectResourceQuota { namespace, name } => {
            resourcequotas::fetch_resourcequota_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::DeleteResourceQuota { namespace, name } => {
            resourcequotas::delete_resourcequota(state, sender, &namespace, &name).await?;
            resourcequotas::refresh_resourcequotas(state, sender).await?;
        }

        BackendCmd::RefreshLimitRanges => {
            limitranges::refresh_limitranges(state, sender).await?;
        }

        BackendCmd::SelectLimitRange { namespace, name } => {
            limitranges::fetch_limitrange_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::DeleteLimitRange { namespace, name } => {
            limitranges::delete_limitrange(state, sender, &namespace, &name).await?;
            limitranges::refresh_limitranges(state, sender).await?;
        }

        BackendCmd::RefreshHorizontalPodAutoscalers => {
            horizontalpodautoscalers::refresh_horizontalpodautoscalers(state, sender).await?;
        }

        BackendCmd::SelectHorizontalPodAutoscaler { namespace, name } => {
            horizontalpodautoscalers::fetch_horizontalpodautoscaler_details(
                state, sender, &namespace, &name,
            )
            .await?;
        }

        BackendCmd::DeleteHorizontalPodAutoscaler { namespace, name } => {
            horizontalpodautoscalers::delete_horizontalpodautoscaler(
                state, sender, &namespace, &name,
            )
            .await?;
            horizontalpodautoscalers::refresh_horizontalpodautoscalers(state, sender).await?;
        }

        BackendCmd::RefreshPodDisruptionBudgets => {
            poddisruptionbudgets::refresh_poddisruptionbudgets(state, sender).await?;
        }

        BackendCmd::SelectPodDisruptionBudget { namespace, name } => {
            poddisruptionbudgets::fetch_poddisruptionbudget_details(
                state, sender, &namespace, &name,
            )
            .await?;
        }

        BackendCmd::DeletePodDisruptionBudget { namespace, name } => {
            poddisruptionbudgets::delete_poddisruptionbudget(state, sender, &namespace, &name)
                .await?;
            poddisruptionbudgets::refresh_poddisruptionbudgets(state, sender).await?;
        }

        BackendCmd::RefreshPriorityClasses => {
            priorityclasses::refresh_priorityclasses(state, sender).await?;
        }

        BackendCmd::SelectPriorityClass { name } => {
            priorityclasses::fetch_priorityclass_details(state, sender, &name).await?;
        }

        BackendCmd::DeletePriorityClass { name } => {
            priorityclasses::delete_priorityclass(state, sender, &name).await?;
            priorityclasses::refresh_priorityclasses(state, sender).await?;
        }

        BackendCmd::RefreshRuntimeClasses => {
            runtimeclasses::refresh_runtimeclasses(state, sender).await?;
        }

        BackendCmd::SelectRuntimeClass { name } => {
            runtimeclasses::fetch_runtimeclass_details(state, sender, &name).await?;
        }

        BackendCmd::DeleteRuntimeClass { name } => {
            runtimeclasses::delete_runtimeclass(state, sender, &name).await?;
            runtimeclasses::refresh_runtimeclasses(state, sender).await?;
        }

        BackendCmd::RefreshLeases => {
            leases::refresh_leases(state, sender).await?;
        }

        BackendCmd::SelectLease { namespace, name } => {
            leases::fetch_lease_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::DeleteLease { namespace, name } => {
            leases::delete_lease(state, sender, &namespace, &name).await?;
            leases::refresh_leases(state, sender).await?;
        }

        BackendCmd::RefreshMutatingWebhookConfigurations => {
            mutatingwebhooks::refresh_mutatingwebhooks(state, sender).await?;
        }

        BackendCmd::SelectMutatingWebhookConfiguration { name } => {
            mutatingwebhooks::fetch_mutatingwebhook_details(state, sender, &name).await?;
        }

        BackendCmd::DeleteMutatingWebhookConfiguration { name } => {
            mutatingwebhooks::delete_mutatingwebhook(state, sender, &name).await?;
            mutatingwebhooks::refresh_mutatingwebhooks(state, sender).await?;
        }

        BackendCmd::RefreshValidatingWebhookConfigurations => {
            validatingwebhooks::refresh_validatingwebhooks(state, sender).await?;
        }

        BackendCmd::SelectValidatingWebhookConfiguration { name } => {
            validatingwebhooks::fetch_validatingwebhook_details(state, sender, &name).await?;
        }

        BackendCmd::DeleteValidatingWebhookConfiguration { name } => {
            validatingwebhooks::delete_validatingwebhook(state, sender, &name).await?;
            validatingwebhooks::refresh_validatingwebhooks(state, sender).await?;
        }
        _ => {
            // This command is not handled in the core `handle_cmd` implementation.
            // Some commands are handled upstream in `cmd::dispatch_cmd` (e.g., port-forward related commands).
        }
    }

    Ok(())
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

/// Parse a Kubernetes CPU quantity string into millicores.
/// Handles: "4" (cores), "4000m" (millicores), "1.5" (cores as float)
fn parse_cpu_to_millicores(q: &str) -> i64 {
    if let Some(stripped) = q.strip_suffix('m') {
        // Already in millicores: "500m" -> 500
        stripped.parse::<i64>().unwrap_or(0)
    } else if let Ok(cores) = q.parse::<f64>() {
        // Whole or fractional cores: "4" -> 4000, "1.5" -> 1500
        (cores * 1000.0) as i64
    } else {
        0
    }
}

/// Parse a Kubernetes memory quantity string into bytes.
/// Handles: "2Gi", "2048Mi", "1984568Ki", "2G", "2M", "2K"
fn parse_memory_to_bytes(q: &str) -> i64 {
    let suffixes: &[(&str, i64)] = &[
        ("Gi", 1 << 30),
        ("Mi", 1 << 20),
        ("Ki", 1 << 10),
        ("G", 1_000_000_000),
        ("M", 1_000_000),
        ("K", 1_000),
    ];
    for (suffix, multiplier) in suffixes {
        if let Some(stripped) = q.strip_suffix(suffix) {
            return stripped.parse::<i64>().unwrap_or(0) * multiplier;
        }
    }
    q.parse::<i64>().unwrap_or(0)
}
