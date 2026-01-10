use super::{
    Correlation, CronJobDetails, CronJobSummary, DaemonSetDetails, DaemonSetSummary,
    DeploymentDetails, DeploymentSummary, DiskUsage, EndpointDetails, EndpointSummary,
    IngressClassDetails, IngressClassSummary, IngressDetails, IngressSummary, JobDetails,
    JobSummary, NetworkPolicyDetails, NetworkPolicySummary, PodDetails, PodMetricsHistory,
    PodMetricsSummary, PodSummary, PrometheusMetrics, ReplicationControllerDetails,
    ReplicationControllerSummary, Route, ServiceDetails, ServiceSummary, StatefulSetDetails,
    StatefulSetSummary, StreamId, WorkloadDetails, WorkloadMetricsAggregate, WorkloadSummary,
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

    DiskUsageUpdated(Vec<DiskUsage>),
}

#[derive(Debug, Clone)]
pub enum ResourceListEvent {
    NamespacesUpdated(Vec<String>),
    PodsUpdated(Vec<PodSummary>),
    DeploymentsUpdated(Vec<DeploymentSummary>),
    DaemonSetsUpdated(Vec<DaemonSetSummary>),
    StatefulSetsUpdated(Vec<StatefulSetSummary>),
    CronJobsUpdated(Vec<CronJobSummary>),
    JobsUpdated(Vec<JobSummary>),
    EndpointsUpdated(Vec<EndpointSummary>),
    ServicesUpdated(Vec<ServiceSummary>),
    IngressesUpdated(Vec<IngressSummary>),
    IngressClassesUpdated(Vec<IngressClassSummary>),
    NetworkPoliciesUpdated(Vec<NetworkPolicySummary>),
    ReplicationControllersUpdated(Vec<ReplicationControllerSummary>),
    ReplicaSetsUpdated(Vec<WorkloadSummary>),
    PortForwardsUpdated(Vec<String>), // Simplified: using Vec<String> for now, can be enhanced
}

#[derive(Debug, Clone)]
pub enum ResourceReadEvent {
    PodDetailsUpdated(PodDetails),
    PodMetricsUpdated(PodMetricsSummary),
    PodMetricsHistoryUpdated(PodMetricsHistory),
    DeploymentDetailsUpdated(DeploymentDetails),
    DeploymentMetricsUpdated(WorkloadMetricsAggregate),
    DaemonSetDetailsUpdated(DaemonSetDetails),
    DaemonSetMetricsUpdated(WorkloadMetricsAggregate),
    StatefulSetDetailsUpdated(StatefulSetDetails),
    StatefulSetMetricsUpdated(WorkloadMetricsAggregate),
    CronJobDetailsUpdated(CronJobDetails),
    JobDetailsUpdated(JobDetails),
    EndpointDetailsUpdated(EndpointDetails),
    ServiceDetailsUpdated(ServiceDetails),
    IngressDetailsUpdated(IngressDetails),
    IngressClassDetailsUpdated(IngressClassDetails),
    NetworkPolicyDetailsUpdated(NetworkPolicyDetails),
    ReplicationControllerDetailsUpdated(ReplicationControllerDetails),
    ReplicaSetDetailsUpdated(WorkloadDetails),
    PrometheusMetricsUpdated(PrometheusMetrics),
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    PodLogsChunk {
        stream_id: StreamId,
        lines: Vec<String>,
    },
    StreamClosed {
        stream_id: StreamId,
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub enum BackendEventV1 {
    System(SystemEvent),
    ResourceList(ResourceListEvent),
    ResourceRead(ResourceReadEvent),
    Stream(StreamEvent),
}

pub type BackendEvent = BackendEventV1;
