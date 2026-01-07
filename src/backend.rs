use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, Correlation, ResourceListEvent, Route,
    SystemEvent, section_keys,
};
use crossbeam_channel::{Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Runtime;

use crate::backend::integrations::{IntegrationConfig, PrometheusConfig};

mod cronjobs;
mod daemonsets;
mod deployments;
mod events;
mod integrations;
mod interactions;
mod jobs;
mod kube;
mod metrics;
mod metrics_provider;
mod nodes;
pub mod persistence;
mod pods;
mod portforward;
mod replicasets;
mod replicationcontrollers;
mod state;
mod statefulsets;
mod sync;
mod workload_common;

use state::BackendState;
use state::client_or_err;

pub fn init(
    receiver: Receiver<BackendCmd>,
    sender: Sender<BackendEventEnvelope>,
    args: crate::Args,
) {
    let rt = Runtime::new().expect("Failed to create Tokio runtime");

    let state = BackendState::new();

    // Clear metrics history on startup to begin fresh collection
    if let Err(e) = persistence::clear_metrics_history() {
        tracing::warn!("Failed to clear metrics history on startup: {}", e);
    }

    // Create MCP state
    let mcp_state = std::sync::Arc::new(crate::mcp_server::McpAppState::new());

    // Load auto-start configuration
    let auto_start_config = persistence::AutoStartConfig::load().unwrap_or_default();

    // Spawn MCP HTTP server if enabled via CLI args, only_mcp mode, or auto-start config
    // Port 18288: 1.8288 meters ≈ 1 fathom (unit of depth measurement)
    let mcp_http_handle = if args.mcp_http || args.only_mcp || auto_start_config.mcp_http {
        tracing::info!(
            "Starting MCP HTTP server on http://127.0.0.1:{} (18288 = 1.8288m ≈ 1 fathom)",
            args.mcp_port
        );
        Some(rt.spawn(crate::mcp_server::run_http_mcp_server(
            mcp_state.clone(),
            args.mcp_port,
        )))
    } else {
        None
    };

    events::emit(
        state.seq.as_ref(),
        &sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::BackendStarted),
    );
    events::notify_info(
        state.seq.as_ref(),
        &sender,
        "Backend thread started (tokio runtime ready).".to_owned(),
    );

    let state = Arc::new(tokio::sync::Mutex::new(state));

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
            let result = handle_cmd(cmd, &mut state, &sender).await;
            match result {
                Ok(should_exit) if should_exit => {
                    tracing::debug!("Command signaled shutdown");
                }
                Err(err) => events::notify_error(state.seq.as_ref(), &sender, err.to_string()),
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
    if let Some(handle) = mcp_http_handle {
        handle.abort();
    }

    // Final cleanup on exit
    rt.block_on(async {
        let mut final_state = state.lock().await;
        state::cleanup_all_tasks(&mut final_state);
        events::notify_info(
            final_state.seq.as_ref(),
            &sender,
            "Backend thread stopped.".to_owned(),
        );
    });
}

async fn handle_cmd(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<bool> {
    match cmd {
        BackendCmd::Shutdown => {
            events::notify_info(
                state.seq.as_ref(),
                sender,
                "Shutting down backend...".to_owned(),
            );
            state::cleanup_all_tasks(state);
            return Ok(true); // Signal to exit loop
        }
        BackendCmd::Initialize => {
            // Contexts can be loaded even before we connect.
            let _ = kube::load_and_send_contexts(
                state.seq.as_ref(),
                state.current_context.clone(),
                sender,
            );

            events::notify_info(
                state.seq.as_ref(),
                sender,
                "Connecting to cluster (kubeconfig/in-cluster)...".to_owned(),
            );

            let client = kube::connect_client_for_current_context(state).await?;
            state.client = Some(client);

            // Best-effort: if we didn't have an explicit context selection, treat kubeconfig's
            // current context as the backend-authoritative active context.
            if state.current_context.is_none() {
                state.current_context = kube::read_kubeconfig_current_context();
            }
            events::emit(
                state.seq.as_ref(),
                sender,
                Route::global(section_keys::system()),
                Correlation::None,
                BackendEvent::System(SystemEvent::ContextChanged {
                    current: state.current_context.clone(),
                }),
            );

            events::notify_info(state.seq.as_ref(), sender, "Connected.".to_owned());

            // Discover optional backing services.
            metrics::probe_metrics_api(state, sender).await?;

            // Auto-discover Prometheus services in the monitoring namespace
            if let Some(client) = &state.client {
                if let Some(ctx) = &state.current_context {
                    let _ = auto_discover_prometheus(client, ctx).await;
                }
            }

            // Restore saved port forwards from last session
            if let Err(e) = restore_saved_port_forwards(state, sender).await {
                events::notify_info(
                    state.seq.as_ref(),
                    sender,
                    format!("Note: Could not restore port forwards: {}", e),
                );
            }

            // Ensure metrics are accessible via port forward
            if let Err(e) = ensure_metrics_port_forward(state, sender).await {
                events::notify_info(
                    state.seq.as_ref(),
                    sender,
                    format!("Note: Could not auto-setup metrics port forward: {}", e),
                );
            }

            sync::refresh_namespaces(state, sender).await?;
            pods::refresh_pods(state, sender).await?;
            deployments::refresh_deployments(state, sender).await?;
            daemonsets::refresh_daemonsets(state, sender).await?;
            statefulsets::refresh_statefulsets(state, sender).await?;
            replicasets::refresh_replicasets(state, sender).await?;
            replicationcontrollers::refresh_replicationcontrollers(state, sender).await?;
            jobs::refresh_jobs(state, sender).await?;
            cronjobs::refresh_cronjobs(state, sender).await?;

            sync::ensure_background_sync(state, sender).await?;
        }

        BackendCmd::RefreshContexts => {
            let _ = kube::load_and_send_contexts(
                state.seq.as_ref(),
                state.current_context.clone(),
                sender,
            );
        }

        BackendCmd::SetContext { context } => {
            // Stop background activity and reconnect with the new context.
            state::stop_logs(state);
            state::abort_task(&mut state.pod_watch_task);
            state::abort_task(&mut state.namespace_sync_task);

            state.current_context = Some(context.clone());
            state.reset_metrics_probe();

            // Context switch invalidates namespace filters; reset to "all" on the backend.
            state.namespace_filter.clear();
            if let Ok(mut shared) = state.namespace_filter_shared.write() {
                shared.clear();
            }
            events::emit(
                state.seq.as_ref(),
                sender,
                Route::global(section_keys::system()),
                Correlation::None,
                BackendEvent::System(SystemEvent::NamespaceFilterChanged {
                    namespaces: Vec::new(),
                }),
            );

            events::notify_info(
                state.seq.as_ref(),
                sender,
                format!("Switching context to '{}'...", context),
            );

            let client = kube::connect_client_for_current_context(state).await?;
            state.client = Some(client);

            events::emit(
                state.seq.as_ref(),
                sender,
                Route::global(section_keys::system()),
                Correlation::None,
                BackendEvent::System(SystemEvent::ContextChanged {
                    current: state.current_context.clone(),
                }),
            );

            // Re-load contexts so UI reflects the selected one.
            let _ = kube::load_and_send_contexts(
                state.seq.as_ref(),
                state.current_context.clone(),
                sender,
            );

            metrics::probe_metrics_api(state, sender).await?;

            // Auto-discover Prometheus services in the monitoring namespace
            if let Some(client) = &state.client {
                if let Some(ctx) = &state.current_context {
                    let _ = auto_discover_prometheus(client, ctx).await;
                }
            }

            sync::refresh_namespaces(state, sender).await?;
            pods::refresh_pods(state, sender).await?;
            deployments::refresh_deployments(state, sender).await?;
            daemonsets::refresh_daemonsets(state, sender).await?;
            statefulsets::refresh_statefulsets(state, sender).await?;
            replicasets::refresh_replicasets(state, sender).await?;
            replicationcontrollers::refresh_replicationcontrollers(state, sender).await?;
            sync::ensure_background_sync(state, sender).await?;

            events::notify_info(state.seq.as_ref(), sender, "Context switched.".to_owned());
        }

        BackendCmd::RefreshNamespaces => {
            sync::refresh_namespaces(state, sender).await?;
        }

        BackendCmd::RefreshNodes => {
            tracing::debug!("RefreshNodes command received");
            // Best-effort: fetch node list and emit event
            if let Some(client) = &state.client {
                use k8s_openapi::api::core::v1::Node;
                let api = ::kube::Api::<Node>::all(client.clone());
                match api.list(&Default::default()).await {
                    Ok(nodes) => {
                        tracing::debug!("Fetched {} nodes from API", nodes.items.len());
                        let summaries: Vec<crate::protocol::v1::NodeSummary> = nodes
                            .items
                            .into_iter()
                            .map(|node| {
                                let meta = &node.metadata;
                                let status = &node.status;
                                let allocatable = status
                                    .as_ref()
                                    .and_then(|s| s.allocatable.as_ref())
                                    .map(|a| a.clone())
                                    .unwrap_or_default();
                                let cpu_str = allocatable
                                    .get("cpu")
                                    .and_then(|q| q.0.parse::<i64>().ok())
                                    .unwrap_or(0);
                                let mem_str = allocatable
                                    .get("memory")
                                    .and_then(|q| {
                                        q.0.trim_end_matches("Ki")
                                            .parse::<i64>()
                                            .ok()
                                            .map(|m| m * 1024)
                                    })
                                    .unwrap_or(0);

                                crate::protocol::v1::NodeSummary {
                                    name: meta.name.clone().unwrap_or_default(),
                                    status: "Ready".to_string(), // TODO: parse from node conditions
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
                                    pod_count: 0, // TODO: count pods on this node
                                    cpu_allocatable_millicores: cpu_str * 1000,
                                    memory_allocatable_bytes: mem_str,
                                }
                            })
                            .collect();

                        tracing::debug!(
                            "Emitting NodesUpdated event with {} nodes",
                            summaries.len()
                        );
                        events::emit(
                            state.seq.as_ref(),
                            sender,
                            Route::global(section_keys::nodes()),
                            Correlation::None,
                            BackendEvent::ResourceList(ResourceListEvent::NodesUpdated(summaries)),
                        );
                    }
                    Err(e) => {
                        tracing::warn!("Failed to list nodes: {}", e);
                        events::notify_error(
                            state.seq.as_ref(),
                            sender,
                            format!("Failed to list nodes: {}", e),
                        );
                    }
                }
            }
        }

        BackendCmd::SelectNode { name } => {
            tracing::debug!("SelectNode command received for: {}", name);
            // Fetch node details
            if let Some(client) = &state.client {
                use k8s_openapi::api::core::v1::Node;
                let api = ::kube::Api::<Node>::all(client.clone());
                match api.get(&name).await {
                    Ok(node) => {
                        let details = crate::backend::nodes::extract_node_details(&node)?;
                        events::emit(
                            state.seq.as_ref(),
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
                            state.seq.as_ref(),
                            sender,
                            format!("Failed to get node details: {}", e),
                        );
                    }
                }
            }
        }

        BackendCmd::RefreshEvents => {
            // Best-effort: fetch recent events and emit
            if let Some(client) = &state.client {
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
                            state.seq.as_ref(),
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
                            state.seq.as_ref(),
                            sender,
                            format!("Failed to list events: {}", e),
                        );
                    }
                }
            }
        }

        BackendCmd::SetNamespaceFilter { namespaces } => {
            state.namespace_filter = namespaces;
            if let Ok(mut shared) = state.namespace_filter_shared.write() {
                *shared = state.namespace_filter.clone();
            }

            events::emit(
                state.seq.as_ref(),
                sender,
                Route::global(section_keys::system()),
                Correlation::None,
                BackendEvent::System(SystemEvent::NamespaceFilterChanged {
                    namespaces: state.namespace_filter.clone(),
                }),
            );
            pods::refresh_pods(state, sender).await?;
            deployments::refresh_deployments(state, sender).await?;
            daemonsets::refresh_daemonsets(state, sender).await?;
            statefulsets::refresh_statefulsets(state, sender).await?;
            replicasets::refresh_replicasets(state, sender).await?;
            replicationcontrollers::refresh_replicationcontrollers(state, sender).await?;
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
            state::stop_logs(state);
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
            let client = match &state.client {
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
            interval_secs,
        } => {
            // TODO: Implement workload metrics polling
            // This would start a background task that polls Prometheus for aggregated metrics
            // across all pods matching the workload's selector
            metrics::start_workload_metrics_polling(
                state,
                sender,
                namespace,
                workload_name,
                workload_kind,
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
            cronjobs::fetch_cronjob_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::TriggerCronJob { namespace, name } => {
            cronjobs::trigger_cronjob(state, sender, &namespace, &name).await?;
            cronjobs::refresh_cronjobs(state, sender).await?;
        }

        BackendCmd::DeleteCronJob { namespace, name } => {
            cronjobs::delete_cronjob(state, sender, &namespace, &name).await?;
            cronjobs::refresh_cronjobs(state, sender).await?;
        }

        BackendCmd::SimulateSlowCall => {
            events::notify_info(state.seq.as_ref(), sender, "Simulating slow call...".into());
            tokio::time::sleep(Duration::from_secs(2)).await;
            events::notify_info(state.seq.as_ref(), sender, "Slow call done.".into());
        }

        BackendCmd::BulkDelete { items } => {
            let mut success_count = 0;
            let mut failed_items = Vec::new();

            for (namespace, name) in items {
                // Try to delete as pod first
                if state.client.is_some() {
                    pods::delete_pod(state, sender, &namespace, &name)
                        .await
                        .ok();
                    success_count += 1;
                } else {
                    failed_items.push((namespace.clone(), name.clone()));
                }

                // If not a pod, we'd need to try other resource types
                // For now, log the failure
                failed_items.push((namespace.clone(), name.clone()));
                tracing::warn!(
                    namespace = %namespace,
                    name = %name,
                    "Failed to delete item in bulk delete"
                );
            }

            if success_count > 0 {
                events::notify_info(
                    state.seq.as_ref(),
                    sender,
                    format!("Deleted {} item(s)", success_count),
                );
            }

            if !failed_items.is_empty() {
                events::notify_error(
                    state.seq.as_ref(),
                    sender,
                    format!(
                        "Failed to delete {} item(s): {}",
                        failed_items.len(),
                        failed_items
                            .iter()
                            .map(|(ns, name)| format!("{}/{}", ns, name))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
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
            } else if section == section_keys::workloads_daemonsets() {
                daemonsets::refresh_daemonsets(state, sender).await?;
            } else if section == section_keys::workloads_statefulsets() {
                statefulsets::refresh_statefulsets(state, sender).await?;
            } else if section == section_keys::workloads_replicasets() {
                replicasets::refresh_replicasets(state, sender).await?;
            } else if section == section_keys::workloads_replicationcontrollers() {
                replicationcontrollers::refresh_replicationcontrollers(state, sender).await?;
            } else if section == section_keys::workloads_jobs() {
                jobs::refresh_jobs(state, sender).await?;
            } else if section == section_keys::workloads_cronjobs() {
                cronjobs::refresh_cronjobs(state, sender).await?;
            }
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
                        // Emit updated list
                        emit_port_forward_list(state, sender);
                        // Persist to disk
                        save_port_forwards_to_disk(state);
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
        }
        BackendCmd::StopPortForward { tunnel_id } => {
            match state.port_forward_manager.stop(&tunnel_id).await {
                Ok(_) => {
                    events::notify_info(
                        state.seq.as_ref(),
                        sender,
                        format!("Stopped port forward: {}", tunnel_id),
                    );
                    // Emit updated list
                    emit_port_forward_list(state, sender);
                    // Persist to disk
                    save_port_forwards_to_disk(state);
                }
                Err(e) => {
                    events::notify_error(
                        state.seq.as_ref(),
                        sender,
                        format!("Failed to stop port forward: {}", e),
                    );
                }
            }
        }
        BackendCmd::DeletePortForward { tunnel_id } => {
            // First stop if active, then remove from manager
            let _ = state.port_forward_manager.stop(&tunnel_id).await;
            state.port_forward_manager.delete(&tunnel_id);

            events::notify_info(
                state.seq.as_ref(),
                sender,
                format!("Deleted port forward: {}", tunnel_id),
            );

            // Emit updated list
            emit_port_forward_list(state, sender);
            // Persist to disk
            save_port_forwards_to_disk(state);
        }
        BackendCmd::UpdatePortForward {
            tunnel_id,
            local_port,
            remote_port,
            auto_start,
        } => {
            // Update the port forward configuration
            if let Some(pf) = state.port_forward_manager.tunnels.get_mut(&tunnel_id) {
                pf.local_port = local_port;
                pf.remote_port = remote_port;
                pf.auto_start = auto_start;

                events::notify_info(
                    state.seq.as_ref(),
                    sender,
                    format!("Updated port forward: {}", tunnel_id),
                );

                // Emit updated list
                emit_port_forward_list(state, sender);
                // Persist to disk
                save_port_forwards_to_disk(state);
            } else {
                events::notify_error(
                    state.seq.as_ref(),
                    sender,
                    format!("Port forward not found: {}", tunnel_id),
                );
            }
        }
        BackendCmd::RefreshPortForwards => {
            // Emit current port forward list
            emit_port_forward_list(state, sender);
            events::notify_info(
                state.seq.as_ref(),
                sender,
                "Port forwards refreshed".to_owned(),
            );
        }
        BackendCmd::CheckPortAvailability { port } => {
            // Check if local port is available for port forwarding
            let available = check_port_available(port);
            let event = BackendEvent::System(SystemEvent::PortAvailability { port, available });
            events::emit(
                state.seq.as_ref(),
                sender,
                Route::global(section_keys::network_portforwards()),
                Correlation::None,
                event,
            );
        }
        BackendCmd::ConfigurePrometheus {
            namespace,
            service,
            port,
        } => {
            let context = state
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
                    state.seq.as_ref(),
                    sender,
                    format!("Failed to save Prometheus config: {}", e),
                );
            } else {
                // Start port-forward to Prometheus service
                let client = client_or_err(state)?;

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
                        state.prometheus_client =
                            Some(metrics_provider::PrometheusClient::new(url));

                        events::notify_info(
                            state.seq.as_ref(),
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
                        state.prometheus_client =
                            Some(metrics_provider::PrometheusClient::new(url));

                        events::notify_info(
                            state.seq.as_ref(),
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
                    state.prometheus_client =
                        Some(metrics_provider::PrometheusClient::new(client_url));

                    events::emit(
                        state.seq.as_ref(),
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
                        state.seq.as_ref(),
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
                        state.seq.as_ref(),
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
                state.seq.as_ref(),
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
                        state.seq.as_ref(),
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
                        state.seq.as_ref(),
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
                        state.seq.as_ref(),
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
        BackendCmd::RefreshIntegrations => {
            // TODO: Refresh integrations status
            events::notify_info(
                state.seq.as_ref(),
                sender,
                "Integrations refreshed".to_owned(),
            );
        }
        BackendCmd::OpenMcpConfigDir => match crate::mcp_server::mcp_dir() {
            Ok(path) => {
                if let Err(e) = std::fs::create_dir_all(&path) {
                    events::notify_error(
                        state.seq.as_ref(),
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
                                state.seq.as_ref(),
                                sender,
                                format!("Opening MCP config directory: {}", path.display()),
                            );
                        }
                        Err(e) => {
                            events::notify_error(
                                state.seq.as_ref(),
                                sender,
                                format!("Failed to open MCP directory: {}", e),
                            );
                        }
                    }
                }
            }
            Err(e) => {
                events::notify_error(
                    state.seq.as_ref(),
                    sender,
                    format!("Failed to resolve MCP dir: {}", e),
                );
            }
        },
        BackendCmd::StartMcpServer => {
            // Initialize MCP server with current state
            let mcp_state = crate::mcp_server::McpAppState::new();
            let seq = state.seq.clone();
            let event_sender = sender.clone();

            // Spawn async task to initialize server
            tokio::spawn(async move {
                match crate::mcp_server::initialize_mcp_server(mcp_state).await {
                    Ok(_) => {
                        // Emit MCP server status event
                        events::emit(
                            seq.as_ref(),
                            &event_sender,
                            Route::global(section_keys::integrations_mcp()),
                            Correlation::None,
                            BackendEvent::System(SystemEvent::McpServerStatus {
                                running: true,
                                message: "MCP server started successfully".to_owned(),
                            }),
                        );
                        events::notify_info(
                            seq.as_ref(),
                            &event_sender,
                            "MCP server started successfully".to_owned(),
                        );
                    }
                    Err(e) => {
                        // Emit failure status event
                        events::emit(
                            seq.as_ref(),
                            &event_sender,
                            Route::global(section_keys::integrations_mcp()),
                            Correlation::None,
                            BackendEvent::System(SystemEvent::McpServerStatus {
                                running: false,
                                message: format!("Failed to start: {}", e),
                            }),
                        );
                        events::notify_error(
                            seq.as_ref(),
                            &event_sender,
                            format!("Failed to start MCP server: {}", e),
                        );
                    }
                }
            });
        }
        BackendCmd::StopMcpServer => {
            // Emit MCP server stopped event
            events::emit(
                state.seq.as_ref(),
                sender,
                Route::global(section_keys::integrations_mcp()),
                Correlation::None,
                BackendEvent::System(SystemEvent::McpServerStatus {
                    running: false,
                    message: "MCP server stopped".to_owned(),
                }),
            );
            events::notify_info(state.seq.as_ref(), sender, "MCP server stopped".to_owned());
        }
    }

    Ok(false)
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
                    state.seq.as_ref(),
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
                    state.seq.as_ref(),
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
            state.seq.as_ref(),
            sender,
            format!("Restored {} port forwards", restored_count),
        );
        // Emit updated list to UI
        emit_port_forward_list(state, sender);
    }

    Ok(())
}

/// Ensure Prometheus metrics are accessible via port forward
async fn ensure_metrics_port_forward(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state
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
                match state
                    .port_forward_manager
                    .start(client, &namespace, &pod_name, port, port, None)
                    .await
                {
                    Ok(_) => {
                        events::notify_info(
                            state.seq.as_ref(),
                            sender,
                            format!(
                                "Auto-started Prometheus port forward: localhost:{} -> {}/{}:{}",
                                port, namespace, pod_name, port
                            ),
                        );
                        // Emit updated list and save to disk
                        emit_port_forward_list(state, sender);
                        save_port_forwards_to_disk(state);
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
    if is_port_available(9090) {
        // Try to find any Prometheus pod in common namespaces
        for namespace in &["monitoring", "prometheus", "kube-monitoring", "default"] {
            if let Ok(Some(pod_name)) = find_any_prometheus_pod(client, namespace).await {
                match state
                    .port_forward_manager
                    .start(client, namespace, &pod_name, 9090, 9090, None)
                    .await
                {
                    Ok(_) => {
                        events::notify_info(
                            state.seq.as_ref(),
                            sender,
                            format!(
                                "Auto-discovered Prometheus port forward: localhost:9090 -> {}/{}:9090",
                                namespace, pod_name
                            ),
                        );
                        // Emit updated list and save to disk
                        emit_port_forward_list(state, sender);
                        save_port_forwards_to_disk(state);
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
        state.seq.as_ref(),
        sender,
        "Prometheus auto port-forward not started (no matching service/pod found). Configure a port forward under Network > Port Forwards or adjust Prometheus settings under Integrations.".to_owned(),
    );

    Ok(())
}

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

/// Resolve a pod backing a service using Endpoints (prefers ready addresses).
async fn find_pod_via_endpoints(
    client: &::kube::Client,
    namespace: &str,
    service: &str,
) -> anyhow::Result<Option<String>> {
    use ::kube::Api;
    use k8s_openapi::api::core::v1::Endpoints;

    let ep_api = Api::<Endpoints>::namespaced(client.clone(), namespace);
    if let Ok(eps) = ep_api.get(service).await {
        if let Some(subsets) = eps.subsets {
            for subset in subsets {
                // Prefer ready addresses, fallback to not_ready_addresses
                let candidates = subset
                    .addresses
                    .or(subset.not_ready_addresses)
                    .unwrap_or_default();
                for addr in candidates {
                    if let Some(target_ref) = addr.target_ref {
                        if target_ref.kind.as_deref() == Some("Pod") {
                            if let Some(name) = target_ref.name {
                                return Ok(Some(name));
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(None)
}

/// Check if a local port is available
fn is_port_available(port: u16) -> bool {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};

    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    match TcpListener::bind(addr) {
        Ok(_) => true,
        Err(_) => false,
    }
}

/// Find any Prometheus pod in a namespace by looking for pods with "prometheus" in the name
async fn find_any_prometheus_pod(
    client: &::kube::Client,
    namespace: &str,
) -> anyhow::Result<Option<String>> {
    use ::kube::Api;
    use k8s_openapi::api::core::v1::Pod;

    let pod_api = Api::<Pod>::namespaced(client.clone(), namespace);
    match pod_api.list(&Default::default()).await {
        Ok(pods) => {
            // Find first pod with "prometheus" in the name
            for pod in pods.items {
                if let Some(name) = pod.metadata.name.as_ref() {
                    if name.to_lowercase().contains("prometheus") {
                        return Ok(Some(name.clone()));
                    }
                }
            }
            Ok(None)
        }
        Err(_) => Ok(None),
    }
}

/// Auto-discover Prometheus services in the monitoring namespace and configure them
async fn auto_discover_prometheus(client: &::kube::Client, context: &str) -> anyhow::Result<()> {
    use ::kube::Api;
    use k8s_openapi::api::core::v1::Service;

    let svc_api = Api::<Service>::namespaced(client.clone(), "monitoring");
    match svc_api.list(&Default::default()).await {
        Ok(services) => {
            // Look for prometheus-k8s or prometheus-stack-kube-prom-prometheus
            for svc in services.items {
                if let Some(name) = svc.metadata.name.as_ref() {
                    if name.contains("prometheus")
                        && !name.contains("operator")
                        && !name.contains("adapter")
                    {
                        // Found a viable Prometheus service
                        tracing::info!("Auto-discovered Prometheus service: monitoring/{}", name);

                        // Update or create integration config
                        if let Ok(mut cfg) = IntegrationConfig::load(context) {
                            cfg.prometheus = Some(PrometheusConfig {
                                enabled: true,
                                namespace: "monitoring".to_string(),
                                service: name.clone(),
                                port: 9090,
                            });
                            let _ = cfg.save();
                            return Ok(());
                        }
                    }
                }
            }
        }
        Err(e) => {
            tracing::debug!("Failed to list services in monitoring namespace: {}", e);
        }
    }
    Ok(())
}

/// Emit current port forward list to UI
fn emit_port_forward_list(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    let list = state.port_forward_manager.list();
    events::emit(
        state.seq.as_ref(),
        sender,
        Route::global(section_keys::network_portforwards()),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::PortForwardsUpdated(list)),
    );
}

/// Save active port forwards to disk
fn save_port_forwards_to_disk(state: &BackendState) {
    let list = state.port_forward_manager.list();
    if let Err(e) = persistence::save_all_ports(&list) {
        tracing::warn!("Failed to save port forwards to disk: {}", e);
    }
}

/// Check if a local port is available (not in use)
fn check_port_available(port: u16) -> bool {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};

    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    TcpListener::bind(addr).is_ok()
}
