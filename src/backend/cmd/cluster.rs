//! Cluster/context commands: connection lifecycle, context switching, namespace
//! filtering, nodes and cluster events.

use crossbeam_channel::Sender;
use std::time::Duration;

use crate::backend::state::BackendState;
use crate::backend::{
    context_status, deployments, events, integrations, kube, metrics, nodes, pods, sync,
};
use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, ContextConnectionStatus, Correlation,
    ResourceListEvent, Route, SystemEvent, section_keys,
};

/// Handles cluster-level commands; returns the command back if it belongs to another family.
pub(super) async fn handle(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::Initialize => initialize(state, sender).await?,
        BackendCmd::RefreshContexts => {
            if let Ok((contexts, current)) = kube::load_and_send_contexts(
                state.cluster.seq.as_ref(),
                state.cluster.current_context.clone(),
                sender,
            ) {
                context_status::spawn_status_probes(state, sender, &contexts, current.as_deref());
            }
        }
        BackendCmd::SetContext { context } => set_context(state, sender, context).await?,
        BackendCmd::RefreshNamespaces => sync::refresh_namespaces(state, sender).await?,
        BackendCmd::RefreshNodes => nodes::refresh_nodes(state, sender).await,
        BackendCmd::SelectNode { name } => nodes::select_node(state, sender, &name).await?,
        BackendCmd::CordonNode { name } => nodes::cordon_node(state, sender, &name).await?,
        BackendCmd::UncordonNode { name } => nodes::uncordon_node(state, sender, &name).await?,
        BackendCmd::DrainNode {
            name,
            ignore_daemonsets,
            force_delete_empty_dir,
        } => {
            nodes::drain_node(
                state,
                sender,
                name,
                ignore_daemonsets,
                force_delete_empty_dir,
            )
            .await?;
        }
        BackendCmd::CancelDrainNode { name, request_id } => {
            nodes::cancel_drain_node(state, sender, &name, &request_id);
        }
        BackendCmd::TaintNode {
            name,
            op,
            key,
            value,
            effect,
        } => nodes::taint_node(state, sender, &name, op, key, value, effect).await?,
        BackendCmd::RefreshAuditLog => {
            events::emit(
                state.cluster.seq.as_ref(),
                sender,
                Route::global(section_keys::audit_log()),
                Correlation::None,
                BackendEvent::System(SystemEvent::AuditLogLoaded {
                    entries: crate::backend::audit_log::load_entries(),
                }),
            );
        }
        BackendCmd::RefreshEvents => refresh_events(state, sender).await,
        BackendCmd::SetNamespaceFilter { namespaces } => {
            set_namespace_filter(state, sender, namespaces).await?;
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
        BackendCmd::SetActiveSection { section } => {
            // Light prefetch: keep current section fresh.
            if section == section_keys::workloads_pods() {
                pods::refresh_pods(state, sender).await?;
            } else if section == section_keys::workloads_deployments() {
                deployments::refresh_deployments(state, sender).await?;
            }
        }
        other => return Ok(Some(other)),
    }
    Ok(None)
}

async fn initialize(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    // Contexts can be loaded even before we connect.
    let contexts_result = kube::load_and_send_contexts(
        state.cluster.seq.as_ref(),
        state.cluster.current_context.clone(),
        sender,
    );

    events::notify_info(
        state.cluster.seq.as_ref(),
        sender,
        "Connecting to cluster (kubeconfig/in-cluster)...".to_owned(),
    );

    // The context that will end up active is either the explicit selection, or (best-effort)
    // kubeconfig's own `current-context` — mirror the fallback below so the Connecting/
    // Connected/Error bracket (012-incluster-daemon-federation P1, FR-002) targets the right
    // context even on first launch with no prior selection.
    let connecting_target = state
        .cluster
        .current_context
        .clone()
        .or_else(kube::read_kubeconfig_current_context);
    if let Some(target) = &connecting_target {
        context_status::emit_status(
            &state.cluster.seq,
            sender,
            target,
            ContextConnectionStatus::Connecting,
            None,
        );
    }

    let connect_result = connect_and_share_client(state).await;
    if let Some(target) = &connecting_target {
        match &connect_result {
            Ok(()) => context_status::emit_status(
                &state.cluster.seq,
                sender,
                target,
                ContextConnectionStatus::Connected,
                None,
            ),
            Err(e) => context_status::emit_status(
                &state.cluster.seq,
                sender,
                target,
                ContextConnectionStatus::Error,
                Some(format!("{e:#}")),
            ),
        }
    }
    connect_result?;

    // Best-effort: if we didn't have an explicit context selection, treat kubeconfig's
    // current context as the backend-authoritative active context.
    if state.cluster.current_context.is_none() {
        state.cluster.current_context = kube::read_kubeconfig_current_context();
    }
    emit_context_changed(state, sender);

    if let Ok((contexts, current)) = &contexts_result {
        context_status::spawn_status_probes(state, sender, contexts, current.as_deref());
    }

    events::notify_info(state.cluster.seq.as_ref(), sender, "Connected.".to_owned());

    // Discover optional backing services.
    metrics::probe_metrics_api(state, sender).await?;
    auto_discover_prometheus(state).await;

    // Restore saved port forwards from last session
    if let Err(e) = crate::backend::restore_saved_port_forwards(state, sender).await {
        events::notify_info(
            state.cluster.seq.as_ref(),
            sender,
            format!("Note: Could not restore port forwards: {}", e),
        );
    }

    // Ensure metrics are accessible via port forward
    if let Err(e) = integrations::ensure_metrics_port_forward(state, sender).await {
        events::notify_info(
            state.cluster.seq.as_ref(),
            sender,
            format!("Note: Could not auto-setup metrics port forward: {}", e),
        );
    }

    refresh_core_resources(state, sender).await
}

async fn set_context(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: String,
) -> anyhow::Result<()> {
    // Stop background activity and reconnect with the new context.
    state.log_stream.stop();
    state.exec_session.stop();
    state.watch_tasks.abort_all();
    state.node_actions.abort_all();

    state.cluster.current_context = Some(context.clone());
    state.metrics.reset_probe();

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

    context_status::emit_status(
        &state.cluster.seq,
        sender,
        &context,
        ContextConnectionStatus::Connecting,
        None,
    );
    let connect_result = connect_and_share_client(state).await;
    match &connect_result {
        Ok(()) => context_status::emit_status(
            &state.cluster.seq,
            sender,
            &context,
            ContextConnectionStatus::Connected,
            None,
        ),
        Err(e) => context_status::emit_status(
            &state.cluster.seq,
            sender,
            &context,
            ContextConnectionStatus::Error,
            Some(format!("{e:#}")),
        ),
    }
    connect_result?;
    emit_context_changed(state, sender);

    // Re-load contexts so UI reflects the selected one, and re-probe every other context's
    // status now that the active one has changed.
    if let Ok((contexts, current)) = kube::load_and_send_contexts(
        state.cluster.seq.as_ref(),
        state.cluster.current_context.clone(),
        sender,
    ) {
        context_status::spawn_status_probes(state, sender, &contexts, current.as_deref());
    }

    metrics::probe_metrics_api(state, sender).await?;
    auto_discover_prometheus(state).await;

    refresh_core_resources(state, sender).await?;

    events::notify_info(
        state.cluster.seq.as_ref(),
        sender,
        "Context switched.".to_owned(),
    );
    Ok(())
}

/// Connect for the current context and store the client on the backend state.
async fn connect_and_share_client(state: &mut BackendState) -> anyhow::Result<()> {
    let client = kube::connect_client_for_current_context(state).await?;
    state.cluster.client = Some(client);
    // 015-mcp-agent-exposure: keep whatever stdio MCP server is running pointed at the cluster
    // Fathom itself is on. Published unconditionally — the handle exists whether or not the
    // server is enabled, so this never starts anything (FR-002).
    state.mcp.cluster.publish(
        state.cluster.client.clone(),
        state.cluster.current_context.clone(),
    );
    Ok(())
}

fn emit_context_changed(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::ContextChanged {
            current: state.cluster.current_context.clone(),
        }),
    );
}

/// Auto-discover Prometheus services in the monitoring namespace (best-effort).
async fn auto_discover_prometheus(state: &BackendState) {
    if let Some(client) = &state.cluster.client
        && let Some(ctx) = &state.cluster.current_context
    {
        let _ = integrations::auto_discover_prometheus(client, ctx).await;
    }
}

/// Refresh namespaces, pods and deployments, then make sure background sync runs.
async fn refresh_core_resources(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    sync::refresh_namespaces(state, sender).await?;
    pods::refresh_pods(state, sender).await?;
    deployments::refresh_deployments(state, sender).await?;
    sync::ensure_background_sync(state, sender).await?;
    sync::ensure_flux_poll(state, sender).await;
    sync::ensure_knative_poll(state, sender).await;
    sync::ensure_cluster_version_poll(state, sender).await;
    sync::ensure_trivy_poll(state, sender).await;
    sync::ensure_hubble_poll(state, sender).await;
    sync::ensure_velero_poll(state, sender).await;
    sync::ensure_cert_manager_poll(state, sender).await;
    Ok(())
}

async fn set_namespace_filter(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespaces: Vec<String>,
) -> anyhow::Result<()> {
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
    Ok(())
}

/// Best-effort: fetch recent events and emit.
async fn refresh_events(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    let Some(client) = &state.cluster.client else {
        return;
    };
    // Capture the context at request start so a late result keeps its origin (FR-016).
    let context = state.cluster.current_context.clone();
    use k8s_openapi::api::core::v1::Event;
    let api = ::kube::Api::<Event>::all(client.clone());
    match api.list(&Default::default()).await {
        Ok(event_list) => {
            let summaries: Vec<crate::protocol::v1::KubernetesEvent> = event_list
                .items
                .into_iter()
                .take(100) // Limit to last 100 events for performance
                .map(kubernetes_event_summary)
                .collect();

            emit_events_list(state.cluster.seq.as_ref(), sender, context, summaries);
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

/// Route for the cluster-wide Kubernetes events list fetched from `context`.
fn events_route(context: Option<String>) -> Route {
    Route::cluster(section_keys::events(), context)
}

/// Emit `EventsUpdated` tagged with the context the events were listed from.
fn emit_events_list(
    seq: &std::sync::atomic::AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    context: Option<String>,
    summaries: Vec<crate::protocol::v1::KubernetesEvent>,
) {
    let total_count = summaries.len() as u32;
    events::emit(
        seq,
        sender,
        events_route(context),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::EventsUpdated(
            crate::protocol::v1::EventsSummary {
                events: summaries,
                total_count,
            },
        )),
    );
}

fn kubernetes_event_summary(
    event: k8s_openapi::api::core::v1::Event,
) -> crate::protocol::v1::KubernetesEvent {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ScopeKind;
    use std::sync::atomic::AtomicU64;

    #[test]
    fn events_list_carries_originating_context() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let seq = AtomicU64::new(0);
        emit_events_list(&seq, &tx, Some("kind-a".to_owned()), Vec::new());

        let env = rx.try_recv().expect("an EventsUpdated envelope");
        assert_eq!(env.route.context.as_deref(), Some("kind-a"));
        assert_eq!(env.route.scope, ScopeKind::Cluster);
        assert_eq!(env.route.section, section_keys::events());
        assert!(matches!(
            env.event,
            BackendEvent::ResourceList(ResourceListEvent::EventsUpdated(_))
        ));
    }
}
