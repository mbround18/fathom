//! Cluster/context commands: connection lifecycle, context switching, namespace
//! filtering, nodes and cluster events.

use crossbeam_channel::Sender;
use std::time::Duration;

use crate::backend::state::BackendState;
use crate::backend::{
    context_status, daemon_ctl, deployments, events, federation, fleet, integrations, kube,
    metrics, namespaces, nodes, ns_scope, pods, sync,
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
                fleet::sync_pollers(state, sender);
            }
        }
        BackendCmd::StartFleetPolling => fleet::start_polling(state, sender),
        BackendCmd::StopFleetPolling => fleet::stop_polling(state),
        BackendCmd::FederateContext { context } => federation::federate(state, sender, context),
        BackendCmd::UnfederateContext { context } => {
            federation::unfederate(state, sender, &context);
        }
        BackendCmd::RequestFederationSnapshot => federation::snapshot(state, sender),
        // 036-incluster-daemon: optional daemon client (off by default; no requests when off).
        BackendCmd::SetDaemonPreference { context, mode } => {
            daemon_ctl::set_preference(state, sender, &context, mode);
            if state.cluster.current_context.as_deref() == Some(context.as_str()) {
                sync::respawn_pod_watch(state, sender);
            }
        }
        BackendCmd::RefreshDaemon { context } => daemon_ctl::refresh(state, sender, &context),
        BackendCmd::RequestDaemonTimeline { context, limit } => {
            daemon_ctl::request_timeline(state, sender, &context, limit);
        }
        BackendCmd::RequestDaemonTopology { context } => {
            daemon_ctl::request_topology(state, sender, &context);
        }
        BackendCmd::RequestDaemonMetrics {
            context,
            uid,
            metric,
            range_secs,
        } => daemon_ctl::request_metrics(state, sender, &context, uid, metric, range_secs),
        BackendCmd::SetContext { context } => set_context(state, sender, context).await?,
        BackendCmd::RefreshNamespaces => sync::refresh_namespaces(state, sender).await?,
        BackendCmd::RefreshNodes => nodes::refresh_nodes(state, sender).await,
        BackendCmd::SelectNamespace { name } => {
            namespaces::select_namespace(state, sender, &name).await?;
        }
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
        BackendCmd::RefreshAuditLog => refresh_audit_log(state, sender),
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

fn refresh_audit_log(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::global(section_keys::audit_log()),
        Correlation::None,
        BackendEvent::System(SystemEvent::AuditLogLoaded {
            entries: state.audit.load(state.identity.as_ref()),
        }),
    );
}

async fn initialize(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let local = state.access.allows_local_features();
    // Contexts can be loaded even before we connect. Locally this also classifies the kubeconfig
    // (missing, empty, malformed, no current context) for the first-run screen.
    let contexts_result = if local {
        let load = kube::load_and_send_contexts_full(
            state.cluster.seq.as_ref(),
            state.cluster.current_context.clone(),
            sender,
        );
        Ok((load.contexts, load.current))
    } else {
        kube::send_contexts_for(
            state.access.as_ref(),
            state.cluster.seq.as_ref(),
            state.cluster.current_context.clone(),
            sender,
        )
    };

    // 034: reconnect the persisted federated set (no-op when empty).
    federation::restore(state, sender);

    // The context that will end up active: locally, the explicit selection if the kubeconfig
    // still defines it, else kubeconfig's own `current-context`; hosted, the access's context.
    // The Connecting/Connected/Error bracket (012-incluster-daemon-federation P1, FR-002)
    // targets it even on first launch with no prior selection.
    let connecting_target = if local {
        let current = contexts_result
            .as_ref()
            .ok()
            .and_then(|(_, current)| current.clone());
        state.cluster.current_context = current.clone();
        current
    } else {
        state
            .cluster
            .current_context
            .clone()
            .or_else(kube::read_kubeconfig_current_context)
    };

    // First run: no kubeconfig, an empty or malformed one, or no context selected. There is
    // nothing to connect to, and the UI explains why from `KubeconfigStatus` (with "create a
    // local cluster" / "pick a context" actions). Not an error: no raw notification.
    if local && connecting_target.is_none() {
        state.cluster.client = None;
        emit_context_changed(state, sender);
        if let Ok((contexts, _)) = &contexts_result {
            context_status::spawn_status_probes(state, sender, contexts, None);
        }
        return Ok(());
    }

    events::notify_info(
        state.cluster.seq.as_ref(),
        sender,
        "Connecting to cluster...".to_owned(),
    );
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
            Ok(()) => {
                context_status::emit_status(
                    &state.cluster.seq,
                    sender,
                    target,
                    ContextConnectionStatus::Connected,
                    None,
                );
                context_status::emit_active_problem_value(&state.cluster.seq, sender, target, None);
            }
            Err(e) => context_status::emit_active_problem(&state.cluster.seq, sender, target, e),
        }
    }
    if let Err(e) = connect_result {
        if !local {
            return Err(e);
        }
        // The UI shows the classified problem with Retry / pick another context; the other
        // contexts still get probed so the picker shows which ones are reachable.
        emit_context_changed(state, sender);
        if let Ok((contexts, current)) = &contexts_result {
            context_status::spawn_status_probes(state, sender, contexts, current.as_deref());
        }
        return Ok(());
    }

    // Best-effort: if we didn't have an explicit context selection, treat kubeconfig's
    // current context as the backend-authoritative active context.
    if state.cluster.current_context.is_none() {
        state.cluster.current_context = kube::read_kubeconfig_current_context();
    }
    emit_context_changed(state, sender);
    daemon_ctl::sync_active(state, sender);

    if let Ok((contexts, current)) = &contexts_result {
        context_status::spawn_status_probes(state, sender, contexts, current.as_deref());
        fleet::sync_pollers(state, sender);
    }

    events::notify_info(state.cluster.seq.as_ref(), sender, "Connected.".to_owned());

    // Optional backing services (metrics API, Prometheus, saved port forwards) are discovered by
    // `post_connect` after this command releases the state lock.
    refresh_core_resources(state, sender).await
}

/// What [`post_connect`] should do after a successful connect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::backend) enum PostConnect {
    /// `Initialize`: probe metrics/Prometheus, restore saved forwards, auto-forward Prometheus.
    Initialize,
    /// `SetContext`: probe metrics/Prometheus for the new context.
    SetContext,
}

/// Optional integration discovery after `Initialize` / `SetContext`, run once the command has
/// finished and released the state lock.
///
/// On a cold cluster this work (metrics API probe, Prometheus discovery, port-forward restore,
/// Prometheus auto-forward) took over 20 s, and while `Initialize` held the lock through it
/// every pane refresh queued behind it and looked stuck on "Loading". Now each network call
/// runs unlocked and the lock is taken only briefly to record a result; if the active context
/// changes meanwhile, the stale results are dropped.
pub(in crate::backend) async fn post_connect(
    state: &tokio::sync::Mutex<BackendState>,
    sender: &Sender<BackendEventEnvelope>,
    kind: PostConnect,
) {
    let (client, context, epoch, secrets, probe) = {
        let s = state.lock().await;
        let Some(client) = s.cluster.client.clone() else {
            return;
        };
        let probe = s.metrics.availability == crate::backend::state::MetricsAvailability::Unknown;
        (
            client,
            s.cluster.current_context.clone(),
            s.cluster.connection_epoch,
            s.secrets.clone(),
            probe,
        )
    };
    let still_current = |s: &BackendState| {
        s.cluster.connection_epoch == epoch && s.cluster.current_context == context
    };

    if probe {
        let availability = metrics::detect_metrics_api(&client).await;
        let mut s = state.lock().await;
        if !still_current(&s) {
            return;
        }
        metrics::record_metrics_availability(&mut s, sender, availability);
    }

    if let Some(ctx) = &context {
        let _ = integrations::auto_discover_prometheus(&client, ctx, secrets.as_ref()).await;
    }

    if kind != PostConnect::Initialize {
        return;
    }

    {
        let mut s = state.lock().await;
        if !still_current(&s) {
            return;
        }
        // Restore saved port forwards from last session (local listeners; no cluster round trip).
        if let Err(e) = crate::backend::restore_saved_port_forwards(&mut s, sender).await {
            events::notify_info(
                s.cluster.seq.as_ref(),
                sender,
                format!("Note: Could not restore port forwards: {}", e),
            );
        }
    }

    integrations::ensure_metrics_port_forward(
        state,
        sender,
        &client,
        context.as_deref(),
        still_current,
    )
    .await;
}

async fn set_context(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: String,
) -> anyhow::Result<()> {
    // Stop background activity and reconnect with the new context.
    state.log_stream.stop_all();
    state.exec_session.stop();
    // 037: the node-shell pod lives in the old cluster; delete it with the client captured
    // at start, bounded, before the connection is replaced.
    crate::backend::node_shell::end_active(state, crate::backend::node_shell::CONTEXT_SWITCH_BOUND)
        .await;
    state.watch_tasks.abort_all();
    state.node_actions.abort_all();
    // The CRD catalog describes the previous cluster; `RefreshCrdInstances` resolves kinds it
    // lacks from the API server, so an empty catalog is safe until `RefreshCrds` runs again.
    state.cluster.crd_catalog.clear();
    // 019: the cached version describes the cluster being left behind.
    if let Ok(mut cached) = state.cluster.cluster_version.write() {
        *cached = None;
    }

    state.cluster.current_context = Some(context.clone());
    state.capabilities.invalidate_all();
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
        Ok(()) => {
            context_status::emit_status(
                &state.cluster.seq,
                sender,
                &context,
                ContextConnectionStatus::Connected,
                None,
            );
            context_status::emit_active_problem_value(&state.cluster.seq, sender, &context, None);
        }
        Err(e) => context_status::emit_active_problem(&state.cluster.seq, sender, &context, e),
    }
    if let Err(e) = connect_result {
        if !state.access.allows_local_features() {
            return Err(e);
        }
        // The context is selected but unusable: the UI shows the classified problem (Retry,
        // pick another context). No raw error notification and no pane refreshes against it.
        emit_context_changed(state, sender);
        if let Ok((contexts, current)) = kube::load_and_send_contexts(
            state.cluster.seq.as_ref(),
            state.cluster.current_context.clone(),
            sender,
        ) {
            context_status::spawn_status_probes(state, sender, &contexts, current.as_deref());
        }
        return Ok(());
    }
    emit_context_changed(state, sender);
    daemon_ctl::sync_active(state, sender);

    // Re-load contexts so UI reflects the selected one, and re-probe every other context's
    // status now that the active one has changed.
    if let Ok((contexts, current)) = kube::load_and_send_contexts(
        state.cluster.seq.as_ref(),
        state.cluster.current_context.clone(),
        sender,
    ) {
        context_status::spawn_status_probes(state, sender, &contexts, current.as_deref());
        fleet::sync_pollers(state, sender);
    }

    // Metrics API and Prometheus discovery run in `post_connect`, after the lock is released.
    refresh_core_resources(state, sender).await?;

    events::notify_info(
        state.cluster.seq.as_ref(),
        sender,
        "Context switched.".to_owned(),
    );
    Ok(())
}

/// Connect for the current context and store the client on the backend state.
///
/// The connection is checked with one bounded `GET /version` before it is shared: building a
/// client never touches the network, so an unreachable server or a rejected token would
/// otherwise count as connected. On failure the previous client is dropped too, so nothing keeps
/// talking to the context that was just switched away from.
async fn connect_and_share_client(state: &mut BackendState) -> anyhow::Result<()> {
    let checked = match kube::connect_client_for_current_context(state).await {
        Ok(client) => context_status::probe_client(&client).await.map(|()| client),
        Err(e) => Err(e),
    };
    let client = match checked {
        Ok(client) => client,
        Err(e) => {
            state.cluster.client = None;
            state.cluster.connection_epoch = state.cluster.connection_epoch.wrapping_add(1);
            state
                .mcp
                .cluster
                .publish(None, state.cluster.current_context.clone());
            return Err(e);
        }
    };
    crate::backend::first_run::set_active_down(false);
    state.cluster.client = Some(client);
    // 031: a new identity; whether it may list namespaces is learnt again.
    state.cluster.scope.reset();
    state.cluster.connection_epoch = state.cluster.connection_epoch.wrapping_add(1);
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

/// Refresh namespaces, pods and deployments, then make sure background sync runs.
async fn refresh_core_resources(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    sync::refresh_namespaces(state, sender).await?;
    // 031: a namespace-scoped identity may be denied even its own namespace's pods; that must
    // not stop the connect (background sync, pollers) from completing.
    tolerate_forbidden(
        state,
        sender,
        section_keys::workloads_pods(),
        pods::refresh_pods(state, sender).await,
    )?;
    tolerate_forbidden(
        state,
        sender,
        section_keys::workloads_deployments(),
        deployments::refresh_deployments(state, sender).await,
    )?;
    sync::ensure_background_sync(state, sender).await?;
    sync::ensure_flux_poll(state, sender).await;
    sync::ensure_knative_poll(state, sender).await;
    sync::ensure_cluster_version_poll(state, sender).await;
    sync::ensure_trivy_poll(state, sender).await;
    sync::ensure_keda_poll(state, sender).await;
    sync::ensure_hubble_poll(state, sender).await;
    sync::ensure_velero_poll(state, sender).await;
    sync::ensure_cert_manager_poll(state, sender).await;
    Ok(())
}

/// `Ok` for an RBAC denial, reported as `AccessDenied` on `section` so that pane shows
/// "Not permitted" rather than loading forever; the error otherwise.
fn tolerate_forbidden(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    section: crate::protocol::SectionKey,
    result: anyhow::Result<()>,
) -> anyhow::Result<()> {
    match result {
        Err(e) if ns_scope::anyhow_is_forbidden(&e) => {
            tracing::info!("not permitted: {e:#}");
            events::emit(
                state.cluster.seq.as_ref(),
                sender,
                Route::namespaced(section, state.cluster.current_context.clone(), None),
                Correlation::None,
                BackendEvent::System(SystemEvent::AccessDenied {
                    message: format!("{e:#}"),
                }),
            );
            Ok(())
        }
        other => other,
    }
}

async fn set_namespace_filter(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespaces: Vec<String>,
) -> anyhow::Result<()> {
    // 031: a namespace-scoped identity cannot list namespaces, so the picker lets the user type
    // one; remember it so it stays in the namespace list and in "all namespaces".
    if state.cluster.scope.remember(&namespaces) {
        sync::emit_namespace_names(
            state.cluster.seq.as_ref(),
            sender,
            state.cluster.current_context.clone(),
            state.cluster.scope.snapshot().reachable,
        );
    }
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
    tolerate_forbidden(
        state,
        sender,
        section_keys::workloads_pods(),
        pods::refresh_pods(state, sender).await,
    )?;
    tolerate_forbidden(
        state,
        sender,
        section_keys::workloads_deployments(),
        deployments::refresh_deployments(state, sender).await,
    )?;
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
    // 036 US4: the daemon's warm snapshot when a session runs for this context, else LIST.
    let listed = match daemon_ctl::snapshot_source(state) {
        Some(src) => {
            crate::backend::warm_lists::list_with_daemon::<Event>(client, Some(&src), "events")
                .await
                .map(|l| l.items)
                .map_err(anyhow::Error::from)
        }
        // 031: per namespace for a filter or a namespace-scoped identity.
        None => state.cluster.list_namespaced::<Event>().await,
    };
    match listed {
        Ok(items) => {
            let summaries: Vec<crate::protocol::v1::KubernetesEvent> = items
                .into_iter()
                .take(100) // Limit to last 100 events for performance
                .map(kubernetes_event_summary)
                .collect();

            emit_events_list(state.cluster.seq.as_ref(), sender, context, summaries);
        }
        Err(e) => {
            tracing::warn!("Failed to list events: {}", e);
            events::report_error(
                state.cluster.seq.as_ref(),
                sender,
                format!("Failed to list events: {e:#}"),
                ns_scope::anyhow_is_forbidden(&e),
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
            .map(|t| crate::backend::workload_common::kube_rfc3339(&t))
            .unwrap_or_default(),
        last_timestamp: event
            .last_timestamp
            .as_ref()
            .map(|t| crate::backend::workload_common::kube_rfc3339(&t))
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
