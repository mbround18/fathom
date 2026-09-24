use crate::backend::metrics_provider::PrometheusClient;
use crate::backend::portforward::PortForwardManager;
use crate::protocol::CrdCatalogEntry;
use kube::Client;
use std::collections::HashMap;
use std::sync::{Arc, RwLock, atomic::AtomicU64};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MetricsAvailability {
    Unknown,
    Available,
    Unavailable,
    Forbidden,
}

/// Client connection, active context, and namespace scope shared by every
/// resource module for building Routes and fetching from the cluster.
pub(super) struct ClusterContext {
    pub(super) client: Option<Client>,
    pub(super) current_context: Option<String>,
    pub(super) namespace_filter: Vec<String>,
    pub(super) namespace_filter_shared: Arc<RwLock<Vec<String>>>,
    pub(super) seq: Arc<AtomicU64>,
    /// Last catalog discovered by `RefreshCrds` (US3), consulted by `RefreshCrdInstances`/
    /// `SelectCrdInstance` to resolve a CRD's `plural`/`Scope` without re-listing every CRD.
    pub(super) crd_catalog: Vec<CrdCatalogEntry>,
}

impl ClusterContext {
    pub(super) fn client_or_err(&self) -> anyhow::Result<Client> {
        self.client
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Not connected yet."))
    }
}

/// Pod log-tailing session state.
pub(super) struct LogStreamState {
    pub(super) log_task: Option<tokio::task::JoinHandle<()>>,
}

impl LogStreamState {
    pub(super) fn stop(&mut self) {
        abort_task(&mut self.log_task);
    }
}

/// Interactive `kube exec` shell session state (one active application-wide, FR-012).
pub(super) struct ExecSessionState {
    /// Held for the lifetime of the session so `.abort()` can be called on stop; dropping
    /// it early would leave the crate's internal message-loop task undriveable.
    pub(super) attached: Option<kube::api::AttachedProcess>,
    /// Owns the stdout-read relay loop that emits `StreamEvent::PodExecOutputChunk`.
    pub(super) relay_task: Option<tokio::task::JoinHandle<()>>,
    /// Written to directly by the `SendPodExecInput` handler.
    ///
    /// `+ Sync` (beyond research.md R2's minimal `Send`) is required so `&BackendState`
    /// itself stays `Send` across the many `.await` points that hold a `&BackendState`
    /// borrow elsewhere in the backend (e.g. `sync.rs`'s `tokio::join!`).
    pub(super) stdin_writer: Option<Box<dyn tokio::io::AsyncWrite + Unpin + Send + Sync>>,
    /// Sent to directly by the `ResizePodExec` handler.
    pub(super) resize_tx: Option<futures::channel::mpsc::Sender<kube::api::TerminalSize>>,
}

impl ExecSessionState {
    pub(super) fn stop(&mut self) {
        if let Some(p) = self.attached.take() {
            p.abort();
        }
        abort_task(&mut self.relay_task);
        self.stdin_writer = None;
        self.resize_tx = None;
    }
}

/// Long-running background watcher tasks.
pub(super) struct WatchTasks {
    pub(super) pod_watch_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) deployment_watch_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) namespace_sync_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) hubble_poll_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) flux_poll_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) cluster_version_poll_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) gatekeeper_poll_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) velero_poll_task: Option<tokio::task::JoinHandle<()>>,
    /// 023-cert-manager-certificates-pane: background poll of the Certificates overview.
    pub(super) cert_manager_poll_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) trivy_poll_task: Option<tokio::task::JoinHandle<()>>,
}

impl WatchTasks {
    pub(super) fn abort_all(&mut self) {
        abort_task(&mut self.pod_watch_task);
        abort_task(&mut self.deployment_watch_task);
        abort_task(&mut self.namespace_sync_task);
        abort_task(&mut self.hubble_poll_task);
        abort_task(&mut self.flux_poll_task);
        abort_task(&mut self.cluster_version_poll_task);
        abort_task(&mut self.gatekeeper_poll_task);
        abort_task(&mut self.velero_poll_task);
        abort_task(&mut self.cert_manager_poll_task);
        abort_task(&mut self.trivy_poll_task);
    }
}

/// In-flight `DrainNode` background tasks, keyed by the draining command's own request id
/// (stringified), mirroring `LogStreamState`/`ExecSessionState`'s task-lifecycle pattern so a
/// `CancelDrainNode` (or context switch / shutdown) can abort the task cleanly, preserving
/// whatever partial `DrainProgress` was already reported.
#[derive(Default)]
pub(super) struct NodeActionState {
    pub(super) drain_tasks: HashMap<String, tokio::task::JoinHandle<()>>,
}

impl NodeActionState {
    /// Abort and drop one in-flight drain by its request id, if still running.
    pub(super) fn cancel(&mut self, request_id: &str) -> bool {
        if let Some(handle) = self.drain_tasks.remove(request_id) {
            handle.abort();
            true
        } else {
            false
        }
    }

    pub(super) fn abort_all(&mut self) {
        for (_, handle) in self.drain_tasks.drain() {
            handle.abort();
        }
    }
}

/// Metrics probe state and background polling tasks.
pub(super) struct MetricsState {
    pub(super) availability: MetricsAvailability,
    pub(super) unavailable_notified: bool,
    pub(super) forbidden_notified: bool,
    pub(super) poll_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) polling_notified: Option<bool>,
    pub(super) workload_poll_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) prometheus_client: Option<PrometheusClient>,
}

impl MetricsState {
    pub(super) fn reset_probe(&mut self) {
        self.availability = MetricsAvailability::Unknown;
        self.unavailable_notified = false;
        self.forbidden_notified = false;
    }

    pub(super) fn abort_tasks(&mut self) {
        abort_task(&mut self.poll_task);
        abort_task(&mut self.workload_poll_task);
    }
}

pub(super) struct BackendState {
    pub(super) cluster: ClusterContext,
    pub(super) log_stream: LogStreamState,
    pub(super) exec_session: ExecSessionState,
    pub(super) watch_tasks: WatchTasks,
    pub(super) node_actions: NodeActionState,
    pub(super) metrics: MetricsState,
    pub(super) port_forward_manager: PortForwardManager,
    /// Session-scoped image-inspection cache, keyed by resolved manifest digest
    /// (010-container-image-inspection, FR-008/data-model.md). In-memory only, never
    /// persisted to `~/.fathom/` (Constitution VI) or shared across cluster connections.
    pub(super) image_inspect_cache: HashMap<String, crate::protocol::ImageInspection>,
    /// 021-trivy-vulnerability-pane: per-Pod vulnerability inspections keyed `namespace/pod`.
    pub(super) pod_vuln_inspect_cache: HashMap<String, crate::protocol::PodVulnerabilityInspection>,
    /// 013-agent-sdk-ops: per-object assistant conversations, keyed by
    /// `"<kind>/<namespace>/<name>"`. Session-scoped for now (Phase 6/T044 adds
    /// `~/.fathom/agent_conversations.json` persistence across restarts); never
    /// shared across cluster connections.
    pub(super) agent_conversations: HashMap<String, crate::protocol::v1::AgentConversation>,
    /// 015-mcp-agent-exposure: the stdio MCP server's lifecycle. Default-constructed holds no
    /// task and no cancellation token, so a Fathom process that never receives
    /// `SetMcpServerEnabled { enabled: true }` never spawns a server (FR-002).
    pub(super) mcp: crate::backend::mcp::McpState,
    /// Shared by `LogStreamState` and `ExecSessionState` so `StreamId`s stay unique
    /// across both kinds of session (research.md R2).
    next_stream_id: u64,
}

impl BackendState {
    pub(super) fn new() -> Self {
        Self {
            cluster: ClusterContext {
                client: None,
                current_context: None,
                namespace_filter: Vec::new(),
                namespace_filter_shared: Arc::new(RwLock::new(Vec::new())),
                seq: Arc::new(AtomicU64::new(0)),
                crd_catalog: Vec::new(),
            },
            log_stream: LogStreamState { log_task: None },
            exec_session: ExecSessionState {
                attached: None,
                relay_task: None,
                stdin_writer: None,
                resize_tx: None,
            },
            next_stream_id: 1,
            watch_tasks: WatchTasks {
                pod_watch_task: None,
                deployment_watch_task: None,
                namespace_sync_task: None,
                hubble_poll_task: None,
                flux_poll_task: None,
                cluster_version_poll_task: None,
                gatekeeper_poll_task: None,
                velero_poll_task: None,
                cert_manager_poll_task: None,
                trivy_poll_task: None,
            },
            node_actions: NodeActionState::default(),
            metrics: MetricsState {
                availability: MetricsAvailability::Unknown,
                unavailable_notified: false,
                forbidden_notified: false,
                poll_task: None,
                polling_notified: None,
                workload_poll_task: None,
                prometheus_client: None,
            },
            port_forward_manager: PortForwardManager::new(),
            image_inspect_cache: HashMap::new(),
            pod_vuln_inspect_cache: HashMap::new(),
            agent_conversations: HashMap::new(),
            mcp: crate::backend::mcp::McpState::default(),
        }
    }

    /// Assigns the next `StreamId`, shared by log-tailing and exec sessions alike
    /// (research.md R2) so a single stream_id key space is safe to use in `store.streams`.
    pub(super) fn next_stream_id(&mut self) -> crate::protocol::StreamId {
        let id = crate::protocol::StreamId(self.next_stream_id);
        self.next_stream_id = self.next_stream_id.saturating_add(1);
        id
    }
}

pub(super) fn abort_task(task: &mut Option<tokio::task::JoinHandle<()>>) {
    if let Some(handle) = task.take() {
        handle.abort();
    }
}

pub(super) fn cleanup_all_tasks(state: &mut BackendState) {
    abort_background_tasks(state);

    tracing::debug!("Cleaning up port forward manager");
    state.port_forward_manager.cleanup_all();
    tracing::info!("All background tasks aborted and cleaned up");
}

/// Abort log streaming, exec, watch and metrics-polling tasks.
// Cognitive-complexity is inflated by `tracing`'s level-check macro expansion, not real
// branching; this function is a flat sequence of independent stop calls.
#[allow(clippy::cognitive_complexity)]
fn abort_background_tasks(state: &mut BackendState) {
    tracing::debug!("Aborting log task");
    state.log_stream.stop();
    tracing::debug!("Aborting exec session");
    state.exec_session.stop();
    tracing::debug!("Aborting watch tasks");
    state.watch_tasks.abort_all();
    tracing::debug!("Aborting node action tasks");
    state.node_actions.abort_all();
    tracing::debug!("Aborting metrics poll tasks");
    state.metrics.abort_tasks();
    tracing::debug!("Aborting MCP stdio server");
    state.mcp.abort();
}
