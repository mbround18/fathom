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

pub(super) struct BackendState {
    pub(super) client: Option<Client>,
    pub(super) current_context: Option<String>,
    pub(super) namespace_filter: Vec<String>,
    pub(super) namespace_filter_shared: Arc<RwLock<Vec<String>>>,
    pub(super) log_task: Option<tokio::task::JoinHandle<()>>,

    pub(super) seq: Arc<AtomicU64>,
    pub(super) next_stream_id: u64,

    pub(super) pod_watch_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) namespace_sync_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) deployment_watch_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) daemonset_watch_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) statefulset_watch_task: Option<tokio::task::JoinHandle<()>>,

    pub(super) metrics_availability: MetricsAvailability,
    pub(super) metrics_unavailable_notified: bool,
    pub(super) metrics_forbidden_notified: bool,
    pub(super) metrics_poll_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) workload_metrics_poll_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) prometheus_client: Option<PrometheusClient>,

    pub(super) port_forward_manager: PortForwardManager,
}

impl BackendState {
    pub(super) fn new() -> Self {
        Self {
            client: None,
            current_context: None,
            namespace_filter: Vec::new(),
            namespace_filter_shared: Arc::new(RwLock::new(Vec::new())),
            log_task: None,
            seq: Arc::new(AtomicU64::new(0)),
            next_stream_id: 1,
            pod_watch_task: None,
            namespace_sync_task: None,
            deployment_watch_task: None,
            daemonset_watch_task: None,
            statefulset_watch_task: None,
            metrics_availability: MetricsAvailability::Unknown,
            metrics_unavailable_notified: false,
            metrics_forbidden_notified: false,
            metrics_poll_task: None,
            workload_metrics_poll_task: None,
            prometheus_client: None,
            port_forward_manager: PortForwardManager::new(),
        }
    }

    pub(super) fn reset_metrics_probe(&mut self) {
        self.metrics_availability = MetricsAvailability::Unknown;
        self.metrics_unavailable_notified = false;
        self.metrics_forbidden_notified = false;
    }

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

pub(super) fn stop_logs(state: &mut BackendState) {
    if let Some(handle) = state.log_task.take() {
        handle.abort();
    }
}

pub(super) fn cleanup_all_tasks(state: &mut BackendState) {
    abort_task(&mut state.log_task);
    abort_task(&mut state.pod_watch_task);
    abort_task(&mut state.namespace_sync_task);
    abort_task(&mut state.deployment_watch_task);
    abort_task(&mut state.daemonset_watch_task);
    abort_task(&mut state.statefulset_watch_task);
    state.port_forward_manager.cleanup_all();
}

pub(super) fn client_or_err(state: &BackendState) -> anyhow::Result<Client> {
    state
        .client
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Not connected yet."))
}
