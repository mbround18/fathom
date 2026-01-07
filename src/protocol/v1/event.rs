use super::{
    Correlation, CronJobDetails, CronJobSummary, DaemonSetDetails, DaemonSetSummary,
    DeploymentDetails, DeploymentSummary, EventsSummary, JobDetails, JobSummary, NamespaceDetails,
    NamespaceSummary, NodeDetails, NodeSummary, PodDetails, PodMetricsHistory, PodMetricsSummary,
    PodSummary, PrometheusMetrics, ReplicaSetDetails, ReplicaSetSummary,
    ReplicationControllerDetails, ReplicationControllerSummary, Route, StatefulSetDetails,
    StatefulSetSummary, StreamId, WorkloadMetricsAggregate,
};

#[derive(Debug, Clone)]
pub struct BackendEventEnvelopeV1 {
    pub version: u32,
    pub seq: u64,
    pub route: Route,
    pub correlation: Correlation,
    pub event: BackendEventV1,
}

impl BackendEventEnvelopeV1 {
    pub fn new(seq: u64, route: Route, correlation: Correlation, event: BackendEventV1) -> Self {
        Self {
            version: 1,
            seq,
            route,
            correlation,
            event,
        }
    }
}

pub type BackendEventEnvelope = BackendEventEnvelopeV1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub enum SystemEvent {
    BackendStarted,
    Notification {
        level: NotificationLevel,
        message: String,
    },

    ContextsUpdated {
        contexts: Vec<String>,
        current: Option<String>,
    },

    ContextChanged {
        current: Option<String>,
    },

    NamespaceFilterChanged {
        namespaces: Vec<String>,
    },

    PrometheusConnectionTest {
        success: bool,
        message: String,
    },

    GrafanaConnectionTest {
        success: bool,
        message: String,
    },

    McpServerStatus {
        running: bool,
        message: String,
    },

    PortAvailability {
        port: u16,
        available: bool,
    },
}

#[derive(Debug, Clone)]
pub enum ResourceListEvent {
    NamespacesUpdated(Vec<String>),
    NodesUpdated(Vec<NodeSummary>),
    NamespaceSummariesUpdated(Vec<NamespaceSummary>),
    PodsUpdated(Vec<PodSummary>),
    DeploymentsUpdated(Vec<DeploymentSummary>),
    DaemonSetsUpdated(Vec<DaemonSetSummary>),
    StatefulSetsUpdated(Vec<StatefulSetSummary>),
    ReplicaSetsUpdated(Vec<ReplicaSetSummary>),
    ReplicationControllersUpdated(Vec<ReplicationControllerSummary>),
    JobsUpdated(Vec<JobSummary>),
    CronJobsUpdated(Vec<CronJobSummary>),
    EventsUpdated(EventsSummary),
    PortForwardsUpdated(Vec<crate::sections::network::PortForward>),
}

#[derive(Debug, Clone)]
pub enum ResourceReadEvent {
    NodeDetailsUpdated(NodeDetails),
    NamespaceDetailsUpdated(NamespaceDetails),
    PodDetailsUpdated(PodDetails),
    PodMetricsUpdated(PodMetricsSummary),
    PodMetricsHistoryUpdated(PodMetricsHistory),
    PrometheusMetricsUpdated(PrometheusMetrics),
    DeploymentDetailsUpdated(DeploymentDetails),
    DeploymentMetricsUpdated(WorkloadMetricsAggregate),
    DaemonSetDetailsUpdated(DaemonSetDetails),
    DaemonSetMetricsUpdated(WorkloadMetricsAggregate),
    StatefulSetDetailsUpdated(StatefulSetDetails),
    StatefulSetMetricsUpdated(WorkloadMetricsAggregate),
    ReplicaSetDetailsUpdated(ReplicaSetDetails),
    ReplicationControllerDetailsUpdated(ReplicationControllerDetails),
    JobDetailsUpdated(JobDetails),
    CronJobDetailsUpdated(CronJobDetails),
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    PodLogsChunk {
        stream_id: StreamId,
        lines: Vec<String>,
    },
    PortForwardTunnel {
        tunnel_id: String,
        status: PortForwardStatus,
        local_endpoint: Option<String>,
    },
    StreamClosed {
        stream_id: StreamId,
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortForwardStatus {
    Connecting,
    Active,
    Error,
    Closed,
}

#[derive(Debug, Clone)]
pub enum BackendEventV1 {
    System(SystemEvent),
    ResourceList(ResourceListEvent),
    ResourceRead(ResourceReadEvent),
    Stream(StreamEvent),
}

pub type BackendEvent = BackendEventV1;
