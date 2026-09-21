//! Cluster/context commands: connection lifecycle, context switching, namespace
//! filtering, nodes and cluster events.

use crossbeam_channel::Sender;
use std::time::Duration;

use crate::backend::state::{self, BackendState};
use crate::backend::{deployments, events, integrations, kube, metrics, nodes, pods, sync};
use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, Correlation, ResourceListEvent, Route,
    SystemEvent, section_keys,
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
            let _ = kube::load_and_send_contexts(
                state.cluster.seq.as_ref(),
                state.cluster.current_context.clone(),
                sender,
            );
        }
        BackendCmd::SetContext { context } => set_context(state, sender, context).await?,
        BackendCmd::RefreshNamespaces => sync::refresh_namespaces(state, sender).await?,
        BackendCmd::RefreshNodes => refresh_nodes(state, sender).await,
        BackendCmd::SelectNode { name } => select_node(state, sender, &name).await?,
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

    connect_and_share_client(state).await?;

    // Best-effort: if we didn't have an explicit context selection, treat kubeconfig's
    // current context as the backend-authoritative active context.
    if state.cluster.current_context.is_none() {
        state.cluster.current_context = kube::read_kubeconfig_current_context();
    }
    if let (Some(mcp), Some(ctx)) = (&state.mcp.state, &state.cluster.current_context) {
        mcp.set_context(ctx.clone()).await;
    }
    emit_context_changed(state, sender);

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

    connect_and_share_client(state).await?;
    emit_context_changed(state, sender);

    // Re-load contexts so UI reflects the selected one.
    let _ = kube::load_and_send_contexts(
        state.cluster.seq.as_ref(),
        state.cluster.current_context.clone(),
        sender,
    );

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

/// Connect for the current context and share the client with the MCP server
/// for direct K8s tool calls.
async fn connect_and_share_client(state: &mut BackendState) -> anyhow::Result<()> {
    let client = kube::connect_client_for_current_context(state).await?;
    state.cluster.client = Some(client.clone());

    if let Some(mcp) = &state.mcp.state {
        *mcp.kube_client.write().await = Some(client);
    }
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
    sync::ensure_background_sync(state, sender).await
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

/// Best-effort: fetch node list and emit event.
async fn refresh_nodes(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    tracing::debug!("RefreshNodes command received");
    let Some(client) = &state.cluster.client else {
        return;
    };
    // Capture the context at request start so a late result keeps its origin (FR-016).
    let context = state.cluster.current_context.clone();
    match fetch_node_summaries(client).await {
        Ok(summaries) => {
            tracing::debug!("Emitting NodesUpdated event with {} nodes", summaries.len());
            events::emit(
                state.cluster.seq.as_ref(),
                sender,
                nodes_route(context),
                Correlation::None,
                BackendEvent::ResourceList(ResourceListEvent::NodesUpdated(summaries)),
            );
        }
        Err(e) => {
            tracing::warn!("Failed to list nodes: {}", e);
            events::notify_error(
                state.cluster.seq.as_ref(),
                sender,
                format!("Failed to list nodes: {}", e),
            );
        }
    }
}

/// List nodes and pods concurrently and build per-node summaries (with pod counts).
async fn fetch_node_summaries(
    client: &::kube::Client,
) -> Result<Vec<crate::protocol::v1::NodeSummary>, ::kube::Error> {
    use k8s_openapi::api::core::v1::{Node, Pod};
    let node_api = ::kube::Api::<Node>::all(client.clone());
    let pod_api = ::kube::Api::<Pod>::all(client.clone());
    // Fetch nodes and pods concurrently for pod_count per node
    let lp = ::kube::api::ListParams::default();
    let (nodes_result, pods_result) = tokio::join!(node_api.list(&lp), pod_api.list(&lp),);
    let nodes = nodes_result?;

    // Build a map of node_name -> pod count from running pods
    let pod_counts = pods_result
        .map(|pods| nodes::pod_counts_by_node(pods.items))
        .unwrap_or_default();

    tracing::debug!("Fetched {} nodes from API", nodes.items.len());
    Ok(nodes
        .items
        .into_iter()
        .map(|node| nodes::node_summary(node, &pod_counts))
        .collect())
}

/// Fetch node details.
async fn select_node(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
) -> anyhow::Result<()> {
    tracing::debug!("SelectNode command received for: {}", name);
    let Some(client) = &state.cluster.client else {
        return Ok(());
    };
    let context = state.cluster.current_context.clone();
    use k8s_openapi::api::core::v1::Node;
    let api = ::kube::Api::<Node>::all(client.clone());
    match api.get(name).await {
        Ok(node) => {
            let details = nodes::extract_node_details(&node)?;
            events::emit(
                state.cluster.seq.as_ref(),
                sender,
                nodes_route(context),
                Correlation::None,
                BackendEvent::ResourceRead(crate::protocol::ResourceReadEvent::NodeDetailsUpdated(
                    details,
                )),
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

/// Route for cluster-wide node data fetched from `context`.
fn nodes_route(context: Option<String>) -> Route {
    Route::cluster(section_keys::nodes(), context)
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

    #[test]
    fn nodes_route_carries_originating_context() {
        let route = nodes_route(Some("kind-b".to_owned()));
        assert_eq!(route.context.as_deref(), Some("kind-b"));
        assert_eq!(route.scope, ScopeKind::Cluster);
        assert_eq!(route.section, section_keys::nodes());
    }
}
