use super::{
    ClusterRoleBindingDetails, ClusterRoleBindingSummary, ClusterRoleDetails, ClusterRoleSummary,
    ConfigMapDetails, ConfigMapSummary, Correlation, CronJobDetails, CronJobSummary,
    DaemonSetDetails, DaemonSetSummary, DeploymentDetails, DeploymentSummary, EndpointDetails,
    EndpointSummary, EventsSummary, FluxOverview, HorizontalPodAutoscalerDetails,
    HorizontalPodAutoscalerSummary, IngressClassDetails, IngressClassSummary, IngressDetails,
    IngressSummary, JobDetails, JobSummary, LeaseDetails, LeaseSummary, LimitRangeDetails,
    LimitRangeSummary, MutatingWebhookConfigurationDetails, MutatingWebhookConfigurationSummary,
    NamespaceDetails, NamespaceSummary, NetworkPolicyDetails, NetworkPolicySummary, NodeDetails,
    NodeSummary, PersistentVolumeClaimDetails, PersistentVolumeClaimSummary,
    PersistentVolumeDetails, PersistentVolumeSummary, PodDetails, PodDisruptionBudgetDetails,
    PodDisruptionBudgetSummary, PodMetricsHistory, PodMetricsSummary, PodSummary,
    PriorityClassDetails, PriorityClassSummary, PrometheusMetrics, ReplicaSetSummary,
    ReplicationControllerDetails, ReplicationControllerSummary, ResourceQuotaDetails,
    ResourceQuotaSummary, RoleBindingDetails, RoleBindingSummary, RoleDetails, RoleSummary, Route,
    RuntimeClassDetails, RuntimeClassSummary, SecretDetails, SecretSummary, ServiceDetails,
    ServiceSummary, StatefulSetDetails, StatefulSetSummary, StorageClassDetails,
    StorageClassSummary, StreamId, UiConfig, ValidatingWebhookConfigurationDetails,
    ValidatingWebhookConfigurationSummary, WorkloadMetricsAggregate,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct BackendEventEnvelopeV1 {
    pub version: u32,
    #[ts(type = "number")]
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

/// Fieldless enum: serialized as a plain string (`"Info" | "Warn" | "Error"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum NotificationLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
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

    MetricsStackScanned(crate::protocol::MetricsStackScanResult),

    McpServerStatus {
        running: bool,
        message: String,
    },

    PortAvailability {
        port: u16,
        available: bool,
    },

    /// Disk usage metrics discovered via CRDs or Prometheus
    DiskUsageUpdated(Vec<DiskUsage>),

    /// Flux GitOps Toolkit state (sources, Kustomizations, HelmReleases)
    FluxOverviewUpdated(FluxOverview),

    /// Reply to `BackendCmd::LoadUiConfig`.
    /// `ui` is `None` when absent or invalid (the invalid blob is kept under `ui_invalid`).
    UiConfigLoaded {
        ui: Option<UiConfig>,
        expanded_groups: Vec<String>,
        mcp_auto_start: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DiskUsage {
    pub namespace: String,
    pub name: String,
    pub provisioned_bytes: u64,
    pub used_bytes: Option<u64>,
    pub state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
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
    ServicesUpdated(Vec<ServiceSummary>),
    EndpointsUpdated(Vec<EndpointSummary>),
    IngressesUpdated(Vec<IngressSummary>),
    IngressClassesUpdated(Vec<IngressClassSummary>),
    NetworkPoliciesUpdated(Vec<NetworkPolicySummary>),
    PersistentVolumesUpdated(Vec<PersistentVolumeSummary>),
    PersistentVolumeClaimsUpdated(Vec<PersistentVolumeClaimSummary>),
    StorageClassesUpdated(Vec<StorageClassSummary>),
    RolesUpdated(Vec<RoleSummary>),
    RoleBindingsUpdated(Vec<RoleBindingSummary>),
    ClusterRolesUpdated(Vec<ClusterRoleSummary>),
    ClusterRoleBindingsUpdated(Vec<ClusterRoleBindingSummary>),
    ConfigMapsUpdated(Vec<ConfigMapSummary>),
    SecretsUpdated(Vec<SecretSummary>),
    ResourceQuotasUpdated(Vec<ResourceQuotaSummary>),
    LimitRangesUpdated(Vec<LimitRangeSummary>),
    HorizontalPodAutoscalersUpdated(Vec<HorizontalPodAutoscalerSummary>),
    PodDisruptionBudgetsUpdated(Vec<PodDisruptionBudgetSummary>),
    PriorityClassesUpdated(Vec<PriorityClassSummary>),
    RuntimeClassesUpdated(Vec<RuntimeClassSummary>),
    LeasesUpdated(Vec<LeaseSummary>),
    MutatingWebhookConfigurationsUpdated(Vec<MutatingWebhookConfigurationSummary>),
    ValidatingWebhookConfigurationsUpdated(Vec<ValidatingWebhookConfigurationSummary>),
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
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
    ReplicaSetDetailsUpdated(super::replicasets::ReplicaSetDetails),
    ReplicationControllerDetailsUpdated(ReplicationControllerDetails),
    JobDetailsUpdated(JobDetails),
    CronJobDetailsUpdated(CronJobDetails),
    ServiceDetailsUpdated(ServiceDetails),
    EndpointDetailsUpdated(EndpointDetails),
    IngressDetailsUpdated(IngressDetails),
    IngressClassDetailsUpdated(IngressClassDetails),
    NetworkPolicyDetailsUpdated(NetworkPolicyDetails),
    PersistentVolumeDetailsUpdated(PersistentVolumeDetails),
    PersistentVolumeClaimDetailsUpdated(PersistentVolumeClaimDetails),
    StorageClassDetailsUpdated(StorageClassDetails),
    RoleDetailsUpdated(RoleDetails),
    RoleBindingDetailsUpdated(RoleBindingDetails),
    ClusterRoleDetailsUpdated(ClusterRoleDetails),
    ClusterRoleBindingDetailsUpdated(ClusterRoleBindingDetails),
    ConfigMapDetailsUpdated(ConfigMapDetails),
    SecretDetailsUpdated(SecretDetails),
    ResourceQuotaDetailsUpdated(ResourceQuotaDetails),
    LimitRangeDetailsUpdated(LimitRangeDetails),
    HorizontalPodAutoscalerDetailsUpdated(HorizontalPodAutoscalerDetails),
    PodDisruptionBudgetDetailsUpdated(PodDisruptionBudgetDetails),
    PriorityClassDetailsUpdated(PriorityClassDetails),
    RuntimeClassDetailsUpdated(RuntimeClassDetails),
    LeaseDetailsUpdated(LeaseDetails),
    MutatingWebhookConfigurationDetailsUpdated(MutatingWebhookConfigurationDetails),
    ValidatingWebhookConfigurationDetailsUpdated(ValidatingWebhookConfigurationDetails),
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
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

/// Tunnel status carried by `StreamEvent::PortForwardTunnel`.
///
/// Fieldless enum: serialized as a plain string. Exported to TypeScript as
/// `TunnelStatus` because `sections::network::PortForwardStatus` (persisted in
/// `ports.json`) already owns the `PortForwardStatus` name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(rename = "TunnelStatus")]
pub enum PortForwardStatus {
    Connecting,
    Active,
    Error,
    Closed,
}

// Wire-protocol enum: boxing `ResourceRead` would ripple through ~78 construct/match
// sites and nested patterns; each event is moved once per emission, so size is moot.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum BackendEventV1 {
    System(SystemEvent),
    ResourceList(ResourceListEvent),
    ResourceRead(ResourceReadEvent),
    Stream(StreamEvent),
}

pub type BackendEvent = BackendEventV1;
