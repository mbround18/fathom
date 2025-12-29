use super::{
    Correlation, DaemonSetDetails, DaemonSetSummary, DeploymentDetails, DeploymentSummary,
    PodDetails, PodMetricsSummary, PodSummary, ReplicaSetDetails, ReplicaSetSummary,
    ReplicationControllerDetails, ReplicationControllerSummary, Route, StatefulSetDetails,
    StatefulSetSummary, StreamId,
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
}

#[derive(Debug, Clone)]
pub enum ResourceListEvent {
    NamespacesUpdated(Vec<String>),
    PodsUpdated(Vec<PodSummary>),
    DeploymentsUpdated(Vec<DeploymentSummary>),
    DaemonSetsUpdated(Vec<DaemonSetSummary>),
    StatefulSetsUpdated(Vec<StatefulSetSummary>),
    ReplicaSetsUpdated(Vec<ReplicaSetSummary>),
    ReplicationControllersUpdated(Vec<ReplicationControllerSummary>),
}

#[derive(Debug, Clone)]
pub enum ResourceReadEvent {
    PodDetailsUpdated(PodDetails),
    PodMetricsUpdated(PodMetricsSummary),
    DeploymentDetailsUpdated(DeploymentDetails),
    DaemonSetDetailsUpdated(DaemonSetDetails),
    StatefulSetDetailsUpdated(StatefulSetDetails),
    ReplicaSetDetailsUpdated(ReplicaSetDetails),
    ReplicationControllerDetailsUpdated(ReplicationControllerDetails),
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
