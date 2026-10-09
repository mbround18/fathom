use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, CmdRequest, Correlation, MutationTarget, Route,
    ScopeKind, SystemEvent, section_keys,
};
use crossbeam_channel::{Receiver, Sender};
use futures::FutureExt;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Runtime;

use crate::config::Args;

pub mod access;
mod agent;
mod ai;
mod apiserver_auth;
mod applications;
mod apply;
pub mod audit_log;
mod cert_manager;
pub mod cloud_auth;
mod cluster_version;
mod cmd;
mod container_runtime;
mod context_status;
mod crds;
mod cronjobs;
mod daemon_client;
mod daemon_ctl;
mod daemonsets;
mod deployments;
mod endpoints;
mod events;
mod exec;
mod federation;
mod first_run;
mod fleet;
mod fleet_model;
mod flux;
mod gatekeeper;
mod helm;
mod http_client;
mod hubble;
mod idp_directory;
mod images;
mod ingressclasses;
mod ingresses;
pub mod integrations;
mod interactions;
mod jobs;
mod keda;
mod kind;
mod knative;
pub mod kube;
mod kubeconfig_export;
mod log_streams;
mod longhorn;
mod manifest_export;
mod mcp;
mod metrics;
mod metrics_provider;
mod namespaces;
mod networkpolicies;
mod node_shell;
mod nodes;
mod ns_scope;
pub mod oidc;
pub mod persistence;
mod pods;
mod portforward;
#[cfg(test)]
mod post_connect_tests;
mod prom_discovery;
mod related;
mod replicasets;
mod replicationcontrollers;
#[cfg(test)]
mod run_cmd_tests;
pub mod secret_store;
mod served_crd;
mod services;
mod state;
mod statefulsets;
mod sync;
mod team_store;
mod trivy;
mod warm_lists;
mod workload_common;

mod capabilities;
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
pub mod rbac_access;
mod rbac_discovery;
mod resourcequotas;
mod role_assignment;
mod rolebindings;
mod roles;
mod runtimeclasses;
mod secrets;
mod storageclasses;
mod validatingwebhooks;
mod velero;
mod verticalpodautoscalers;

use state::BackendState;

/// Run the backend on the current thread until `receiver` closes.
///
/// `C` is either a bare [`BackendCmd`] (uncorrelated: the egui app) or a [`CmdRequest`]
/// carrying the bridge-assigned request id.
pub fn init<C>(receiver: Receiver<C>, sender: Sender<BackendEventEnvelope>, args: Args)
where
    C: Into<CmdRequest>,
{
    init_with(
        receiver,
        sender,
        args,
        Arc::new(access::KubeconfigAccess),
        Arc::new(audit_log::LocalFileSink),
        None,
        None,
    );
}

/// [`init`] with explicit seams (031-hosted-server-mode): where clients come from, where
/// audit entries go, the signed-in identity, and an optional Tokio runtime (the hosted
/// server gives each user a small runtime; `None` builds the default multi-thread one).
pub fn init_with<C>(
    receiver: Receiver<C>,
    sender: Sender<BackendEventEnvelope>,
    args: Args,
    access: Arc<dyn access::ClusterAccess>,
    audit: Arc<dyn audit_log::AuditSink>,
    identity: Option<access::Identity>,
    runtime: Option<Runtime>,
) where
    C: Into<CmdRequest>,
{
    crate::startup::install_panic_hook();
    // 031: a hosted backend's state root is scoped to its own threads (this one and its
    // runtime's), never process-wide: several users' backends share the process. A caller
    // that supplies its own runtime scopes it with `persistence::scope_runtime_to_state_root`.
    let state_root = access.state_root();
    let rt = runtime.unwrap_or_else(|| {
        let mut builder = tokio::runtime::Builder::new_multi_thread();
        builder.enable_all();
        if let Some(root) = state_root.clone() {
            persistence::scope_runtime_to_state_root(&mut builder, root);
        }
        builder.build().expect("Failed to create Tokio runtime")
    });
    if let Some(root) = state_root {
        persistence::set_thread_state_root(root);
    }
    let local_features = access.allows_local_features();
    let home_context = access.home_context();

    let mut state = BackendState::with_seams(access, audit, identity);
    state.cluster.current_context = home_context;
    state.agent_conversations = agent::persistence::load_agent_conversations();
    // 038: load AI-assist settings, migrate any legacy plaintext credential into the OS
    // credential store (never losing it), and learn which providers are connected.
    rt.block_on(
        state
            .ai
            .bootstrap(&ai::Emitter::new(state.cluster.seq.clone(), sender.clone())),
    );
    // 015-mcp-agent-exposure: shells that must never expose an MCP endpoint (`fathom-devbridge`)
    // set `Args::disable_mcp`, and `McpState::start` then refuses before spawning anything. The
    // server is off regardless — this only makes it un-enableable for this process.
    state.mcp.disabled = args.disable_mcp || !local_features;

    // Clear metrics history on startup to begin fresh collection. Skipped for hosted access:
    // a user's backend is re-created on every sign-in after idle teardown, and history is
    // kept across those restarts.
    if local_features && let Err(e) = persistence::clear_metrics_history() {
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

    // Terminal commands run one at a time, in arrival order, on a single task. Spawning a task
    // per command (below) lets them race for the state lock, which reordered keystrokes typed
    // into a pod terminal ("echo" reached the shell as "ehco").
    let (ordered_tx, mut ordered_rx) = tokio::sync::mpsc::unbounded_channel::<CmdRequest>();
    {
        let state = state.clone();
        let sender = sender.clone();
        rt.spawn(async move {
            while let Some(request) = ordered_rx.recv().await {
                run_cmd(request, &state, &sender).await;
            }
        });
    }

    // Receive commands until the channel closes
    while let Ok(request) = receiver.recv() {
        let request: CmdRequest = request.into();
        tracing::debug!(
            "Received command {:?}: {:?}",
            request.id,
            std::mem::discriminant(&request.cmd)
        );

        if is_order_sensitive(&request.cmd) {
            let _ = ordered_tx.send(request);
            continue;
        }

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
        node_shell::end_active(&mut final_state, node_shell::APP_EXIT_BOUND).await;
        state::cleanup_all_tasks(&mut final_state);
        events::notify_info(
            final_state.cluster.seq.as_ref(),
            &sender,
            "Backend thread stopped.".to_owned(),
        );
    });
}

/// Commands whose relative order matters: a pod terminal's session start, keystrokes, resizes
/// and stop must reach the shell exactly in the order they were sent, and a log
/// stream's stop/start pair under one id (React StrictMode's mount/unmount/mount) must not swap
/// (a late `StopPodLogStream` would kill the stream that was just restarted).
fn is_order_sensitive(cmd: &BackendCmd) -> bool {
    matches!(
        cmd,
        BackendCmd::StartPodLogs { .. }
            | BackendCmd::StopPodLogs
            | BackendCmd::StopPodLogStream { .. }
            | BackendCmd::StartPodExec { .. }
            | BackendCmd::SendPodExecInput { .. }
            | BackendCmd::ResizePodExec { .. }
            | BackendCmd::StopPodExec
    )
}

/// How long an ordinary command may hold the state lock: `FATHOM_COMMAND_TIMEOUT_SECS`, or 30s.
///
/// Every command runs under the one state mutex, so a single stalled kube request (default
/// read timeout: 295s) used to block every later click, including `Shutdown`. Read once; an
/// unparsable or zero value falls back to the default.
fn default_command_timeout() -> Duration {
    static LIMIT: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *LIMIT.get_or_init(|| {
        std::env::var("FATHOM_COMMAND_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|secs| *secs > 0)
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_COMMAND_TIMEOUT)
    })
}

const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// The bound for commands that legitimately run long under the lock: a `helm upgrade` or
/// `kind delete` subprocess, an image build, or an AI turn (which has its own 120s HTTP bound).
const LONG_COMMAND_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// The time budget for one command's dispatch, or `None` for a command that must never be cut
/// off. Most handlers answer in milliseconds or spawn their long work (kind create, drain,
/// node shell wait, OIDC flows, streams) and return at once, so they get the default. The
/// exceptions are listed here, by name, so a new long-running inline handler is a visible
/// decision rather than a surprise 30s failure.
fn command_timeout(cmd: &BackendCmd) -> Option<Duration> {
    match cmd {
        // Shutdown is the command that must get through; its own steps are bounded
        // (`node_shell::APP_EXIT_BOUND`) and cutting it short would skip cleanup.
        BackendCmd::Shutdown => None,
        // helm subprocesses run inline (helm's own default wait is 5m).
        BackendCmd::ApplyHelmUpgrade { .. }
        | BackendCmd::ApplyHelmRollback { .. }
        | BackendCmd::PreviewHelmUpgrade { .. }
        | BackendCmd::PreviewHelmRollback { .. }
        | BackendCmd::UninstallHelmRelease { .. }
        // `kind delete cluster` runs inline so the active-context fallback can mutate state.
        | BackendCmd::DeleteKindCluster { .. }
        // Image builds/pulls/removals run on a blocking thread but are awaited inline.
        | BackendCmd::IssueContainerOperation { .. }
        // One LLM turn, grounded against the cluster.
        | BackendCmd::AgentAsk { .. }
        // One delete per selected pod, sequentially.
        | BackendCmd::BulkDelete { .. } => Some(LONG_COMMAND_TIMEOUT),
        _ => Some(default_command_timeout()),
    }
}

/// Run one command, containing any panic in its handler.
///
/// A handler that panics must not take its task with it: on the ordered lane that task is
/// the only one, and every later terminal/log command would vanish into a closed channel.
/// The panic is caught here and reported like any other failure (`CommandFinished { ok: false }`
/// plus an error notification), and the task goes on to the next command. The state lock is
/// released by unwinding (`tokio::sync::Mutex` does not poison), so the state stays usable.
async fn run_cmd(
    request: CmdRequest,
    state: &tokio::sync::Mutex<BackendState>,
    sender: &Sender<BackendEventEnvelope>,
) {
    let id = request.id;
    let route = finished_route(&request.cmd);
    let kind = std::mem::discriminant(&request.cmd);
    let Err(payload) = AssertUnwindSafe(run_cmd_unguarded(request, state, sender))
        .catch_unwind()
        .await
    else {
        return;
    };
    let message = panic_message(payload.as_ref());
    tracing::error!(request = ?id, command = ?kind, "command handler panicked: {message}");
    let state = state.lock().await;
    events::in_cmd_scope(id, None, async {
        let error = format!("internal error: {message}");
        events::notify_error(state.cluster.seq.as_ref(), sender, error.clone());
        events::emit(
            state.cluster.seq.as_ref(),
            sender,
            route,
            Correlation::None, // → the command's Request correlation
            BackendEvent::CommandFinished {
                ok: false,
                error: Some(error),
            },
        );
    })
    .await;
}

/// The text of a panic payload: what `panic!` was given, or a placeholder for a typed payload.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

/// Fault injection for `run_cmd` tests, keyed by request id so parallel tests never see each
/// other's hooks and the protocol enum needs no test-only variant.
#[cfg(test)]
pub(super) mod test_hooks {
    use crate::protocol::RequestId;
    use std::sync::Mutex;
    use std::time::Duration;

    static PANIC_ON: Mutex<Vec<u64>> = Mutex::new(Vec::new());
    static TIMEOUTS: Mutex<Vec<(u64, Duration)>> = Mutex::new(Vec::new());

    /// Replace the command timeout for request id `id`.
    pub(super) fn timeout_for(id: u64, limit: Duration) {
        TIMEOUTS.lock().unwrap().push((id, limit));
    }

    pub(super) fn timeout_override(id: Option<RequestId>) -> Option<Duration> {
        let RequestId(id) = id?;
        let mut entries = TIMEOUTS.lock().unwrap();
        let pos = entries.iter().position(|(x, _)| *x == id)?;
        Some(entries.swap_remove(pos).1)
    }

    /// Make the command with request id `id` panic inside the state lock.
    pub(super) fn panic_on(id: u64) {
        PANIC_ON.lock().unwrap().push(id);
    }

    pub(super) fn maybe_panic(id: Option<RequestId>) {
        let Some(RequestId(id)) = id else { return };
        let mut ids = PANIC_ON.lock().unwrap();
        if let Some(pos) = ids.iter().position(|x| *x == id) {
            ids.swap_remove(pos);
            drop(ids);
            panic!("injected panic for request {id}");
        }
    }
}

/// Dispatch one command under the state lock, then emit its `CommandFinished`.
///
/// Runs inside [`events::in_cmd_scope`], so every envelope emitted while handling the
/// command defaults to its request correlation, and every error notification (from the
/// handler itself, or from a returned `Err`, reported here) marks it failed.
async fn run_cmd_unguarded(
    request: CmdRequest,
    state: &tokio::sync::Mutex<BackendState>,
    sender: &Sender<BackendEventEnvelope>,
) {
    let CmdRequest { id, cmd } = request;
    let route = finished_route(&cmd);
    let scope_route = cmd.section().map(|_| route.clone());
    let post_connect = match &cmd {
        BackendCmd::Initialize => Some(cmd::PostConnect::Initialize),
        BackendCmd::SetContext { .. } => Some(cmd::PostConnect::SetContext),
        _ => None,
    };
    let ok = events::in_cmd_scope(id, scope_route, async move {
        // 026: summarize before `cmd` is consumed by dispatch.
        let audit_summary = cmd.audit_summary();
        let request_id = events::current_request_id_string();
        let mut state = state.lock().await;
        #[cfg(test)]
        test_hooks::maybe_panic(id);
        let limit = command_timeout(&cmd);
        #[cfg(test)]
        let limit = test_hooks::timeout_override(id).or(limit);
        let dispatch = cmd::dispatch_cmd(cmd, &mut state, sender);
        // Dropping the timed-out dispatch future releases every borrow it held, so the lock
        // (released below with `state`) is free for the next command instead of waiting out a
        // kube read timeout (295s by default) or a hung subprocess.
        let result = match limit {
            Some(limit) => match tokio::time::timeout(limit, dispatch).await {
                Ok(result) => result,
                Err(_) => Err(anyhow::anyhow!("timed out after {}s", limit.as_secs())),
            },
            None => dispatch.await,
        };
        match result {
            Ok(true) => tracing::debug!("Command signaled shutdown"),
            Ok(false) => {}
            // `{:#}` keeps the whole anyhow chain (e.g. the Kubernetes API message).
            Err(err) => events::report_error(
                state.cluster.seq.as_ref(),
                sender,
                format!("{err:#}"),
                ns_scope::anyhow_is_forbidden(&err),
            ),
        }
        let errors = events::take_cmd_errors();
        let (ok, error) = if errors.is_empty() {
            (true, None)
        } else {
            (false, Some(errors.join("\n")))
        };
        if let Some(summary) = audit_summary {
            let context = state.cluster.current_context.clone().unwrap_or_default();
            let actor = state.identity.as_ref().map(access::Identity::audit_actor);
            state
                .audit
                .record(summary, context, request_id, ok, error.as_deref(), actor);
        }
        events::emit(
            state.cluster.seq.as_ref(),
            sender,
            route,
            Correlation::None, // → the command's Request correlation
            BackendEvent::CommandFinished { ok, error },
        );
        ok
    })
    .await;
    // Optional integration discovery runs after the lock is released (see `post_connect`).
    if let (true, Some(kind)) = (ok, post_connect) {
        cmd::post_connect(state, sender, kind).await;
    }
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
    if !state.access.allows_local_features() {
        return Ok(());
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_commands_take_the_ordered_lane() {
        assert!(is_order_sensitive(&BackendCmd::SendPodExecInput {
            data: "ZQ==".to_owned()
        }));
        assert!(is_order_sensitive(&BackendCmd::ResizePodExec {
            cols: 80,
            rows: 24
        }));
        assert!(is_order_sensitive(&BackendCmd::StopPodExec));
        assert!(is_order_sensitive(&BackendCmd::StopPodLogs));
        assert!(is_order_sensitive(&BackendCmd::StopPodLogStream {
            stream_id: crate::protocol::StreamId(1 << 32)
        }));
        assert!(!is_order_sensitive(&BackendCmd::StopMetricsPolling));
    }

    #[test]
    fn command_timeouts_are_default_except_for_named_long_runners() {
        assert_eq!(command_timeout(&BackendCmd::Shutdown), None);
        for long in [
            BackendCmd::DeleteKindCluster {
                request_id: "r".into(),
                name: "c".into(),
            },
            BackendCmd::UninstallHelmRelease {
                namespace: "ns".into(),
                release: "r".into(),
                base_revision: 1,
            },
            BackendCmd::BulkDelete { items: Vec::new() },
        ] {
            assert_eq!(
                command_timeout(&long),
                Some(LONG_COMMAND_TIMEOUT),
                "{long:?}"
            );
        }
        for ordinary in [
            BackendCmd::Initialize,
            BackendCmd::RefreshPods,
            BackendCmd::SimulateSlowCall,
            BackendCmd::StopPodExec,
            BackendCmd::SetContext {
                context: "c".into(),
            },
        ] {
            assert_eq!(
                command_timeout(&ordinary),
                Some(default_command_timeout()),
                "{ordinary:?}"
            );
        }
        if std::env::var_os("FATHOM_COMMAND_TIMEOUT_SECS").is_none() {
            assert_eq!(default_command_timeout(), DEFAULT_COMMAND_TIMEOUT);
        }
    }
}
