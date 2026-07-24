use crate::backend::metrics_provider::PrometheusClient;
use crate::backend::portforward::PortForwardManager;
use kube::Client;
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
    pub(super) next_stream_id: u64,
}

impl LogStreamState {
    pub(super) fn stop(&mut self) {
        abort_task(&mut self.log_task);
    }

    pub(super) fn next_id(&mut self) -> crate::protocol::StreamId {
        let id = crate::protocol::StreamId(self.next_stream_id);
        self.next_stream_id = self.next_stream_id.saturating_add(1);
        id
    }
}

/// Long-running background watcher tasks.
pub(super) struct WatchTasks {
    pub(super) pod_watch_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) namespace_sync_task: Option<tokio::task::JoinHandle<()>>,
}

impl WatchTasks {
    pub(super) fn abort_all(&mut self) {
        abort_task(&mut self.pod_watch_task);
        abort_task(&mut self.namespace_sync_task);
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

/// MCP HTTP server task, shared state, and configured port.
pub(super) struct McpState {
    pub(super) http_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) state: Option<std::sync::Arc<crate::mcp_server::McpAppState>>,
    pub(super) port: u16,
}

pub(super) struct BackendState {
    pub(super) cluster: ClusterContext,
    pub(super) log_stream: LogStreamState,
    pub(super) watch_tasks: WatchTasks,
    pub(super) metrics: MetricsState,
    pub(super) port_forward_manager: PortForwardManager,
    pub(super) mcp: McpState,
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
            },
            log_stream: LogStreamState {
                log_task: None,
                next_stream_id: 1,
            },
            watch_tasks: WatchTasks {
                pod_watch_task: None,
                namespace_sync_task: None,
            },
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
            mcp: McpState {
                http_task: None,
                state: None,
                port: 18288,
            },
        }
    }
}

pub(super) fn abort_task(task: &mut Option<tokio::task::JoinHandle<()>>) {
    if let Some(handle) = task.take() {
        handle.abort();
    }
}

pub(super) fn cleanup_all_tasks(state: &mut BackendState) {
    tracing::debug!("Aborting log task");
    state.log_stream.stop();
    tracing::debug!("Aborting watch tasks");
    state.watch_tasks.abort_all();
    tracing::debug!("Aborting metrics poll tasks");
    state.metrics.abort_tasks();

    tracing::debug!("Cleaning up port forward manager");
    state.port_forward_manager.cleanup_all();
    tracing::info!("All background tasks aborted and cleaned up");
}
