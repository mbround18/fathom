use super::{
    ApplicationDetails, ApplicationSummary, ClusterRoleBindingDetails, ClusterRoleBindingSummary,
    ClusterRoleDetails, ClusterRoleSummary, ConfigMapDetails, ConfigMapSummary, Correlation,
    CronJobDetails, CronJobSummary, DaemonSetDetails, DaemonSetSummary, DeploymentDetails,
    DeploymentSummary, EndpointDetails, EndpointSummary, EventsSummary, FluxOverview,
    FluxResourceSummary, HelmReleaseDetails, HelmReleaseSummary,
    HorizontalPodAutoscalerDetails, HorizontalPodAutoscalerSummary, HubbleTrafficSummary,
    IngressClassDetails, IngressClassSummary, IngressDetails, IngressSummary, JobDetails,
    JobSummary, LeaseDetails, LeaseSummary, LimitRangeDetails, LimitRangeSummary,
    MutatingWebhookConfigurationDetails, MutatingWebhookConfigurationSummary, NamespaceDetails,
    NamespaceSummary, NetworkPolicyDetails, NetworkPolicySummary, NodeDetails, NodeSummary,
    PersistentVolumeClaimDetails, PersistentVolumeClaimSummary, PersistentVolumeDetails,
    PersistentVolumeSummary, PodDetails, PodDisruptionBudgetDetails, PodDisruptionBudgetSummary,
    PodMetricsHistory, PodMetricsSummary, PodSummary, PriorityClassDetails, PriorityClassSummary,
    PrometheusMetrics, RelatedResourceRef, ReplicaSetSummary, ReplicationControllerDetails,
    ReplicationControllerSummary, ResourceQuotaDetails, ResourceQuotaSummary, RoleBindingDetails,
    RoleBindingSummary, RoleDetails, RoleSummary, Route, RuntimeClassDetails, RuntimeClassSummary,
    SecretDetails, SecretSummary, SectionKey, ServiceDetails, ServiceSummary, StatefulSetDetails,
    StatefulSetSummary, StorageClassDetails, StorageClassSummary, StreamId, UiConfig,
    ValidatingWebhookConfigurationDetails, ValidatingWebhookConfigurationSummary,
    WorkloadMetricsAggregate,
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

    /// Hubble/Cilium traffic summary, sampled from a bounded `GetFlows` poll.
    HubbleTrafficUpdated(HubbleTrafficSummary),

    /// Reply to `BackendCmd::LoadUiConfig`.
    /// `ui` is `None` when absent or invalid (the invalid blob is kept under `ui_invalid`).
    UiConfigLoaded {
        ui: Option<UiConfig>,
        expanded_groups: Vec<String>,
        mcp_auto_start: bool,
    },

    /// Full CRD catalog, refreshed via `RefreshCrds` or on context switch.
    CrdCatalogUpdated {
        entries: Vec<CrdCatalogEntry>,
    },

    /// Instances of one CRD kind, refreshed via `RefreshCrdInstances`. Each entry is the
    /// object's raw JSON (same `raw_json`-as-`String` convention every built-in kind's
    /// details type already uses), so it flows through the existing Inspector unchanged.
    CrdInstancesUpdated {
        group: String,
        kind: String,
        version: String,
        namespace: Option<String>,
        instances: Vec<String>,
    },
}

/// One installed CustomResourceDefinition, discovered via `kube::discovery`. See
/// data-model.md "Custom Resource Definition Catalog Entry".
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CrdCatalogEntry {
    pub group: String,
    pub kind: String,
    pub plural: String,
    pub versions: Vec<String>,
    pub scope: CrdScope,
    pub permission_denied: bool,
}

/// One custom resource instance's details, generic across every discovered CRD kind (no
/// per-kind struct, unlike built-in kinds' `<Kind>Details`) — the same `raw_json`-as-`String`
/// convention every built-in kind's details type already uses, so it flows through the existing
/// Details/YAML/Events Inspector unchanged. See data-model.md "Custom Resource Instance".
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CrdInstanceDetails {
    pub group: String,
    pub kind: String,
    pub version: String,
    pub namespace: Option<String>,
    pub name: String,
    pub resource_version: String,
    pub raw_json: String,
}

/// Fieldless enum: serialized as a plain string (`"Namespaced" | "Cluster"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum CrdScope {
    Namespaced,
    Cluster,
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
    ApplicationsUpdated(Vec<ApplicationSummary>),
    HelmReleasesUpdated(Vec<HelmReleaseSummary>),
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
    CrdInstanceDetailsUpdated(CrdInstanceDetails),
    ApplicationDetailsUpdated(ApplicationDetails),
    ApplicationMetricsUpdated(WorkloadMetricsAggregate),
    RelatedResourcesUpdated {
        section: SectionKey,
        kind: String,
        namespace: Option<String>,
        name: String,
        related: Vec<RelatedResourceRef>,
    },
    HelmReleaseDetailsUpdated(HelmReleaseDetails),
    FluxResourceDetailsUpdated(FluxResourceSummary),
    FluxResourceNotFound {
        kind: String,
        namespace: String,
        name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum StreamEvent {
    PodLogsChunk {
        stream_id: StreamId,
        lines: Vec<String>,
    },
    /// Emitted once a `StartPodExec` attach succeeds, letting the frontend flip from
    /// "attaching" to "active" (FR-009/FR-013).
    PodExecReady {
        stream_id: StreamId,
    },
    /// Base64-encoded exec session output (see research.md R4).
    PodExecOutputChunk {
        stream_id: StreamId,
        data: String,
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
    /// Emitted once an `ApplyResourceYaml` succeeds (before `CommandFinished`).
    ResourceApplied {
        request_id: String,
        resource_version: String,
    },
    /// Incremental progress for an in-flight `DrainNode`, keyed by `request_id`.
    DrainProgress {
        request_id: String,
        total_pods: u32,
        evicted: Vec<String>,
        blocked_on: Vec<(String, String)>,
        ignored_daemonset_pods: Vec<String>,
    },
    /// Terminal event for a node action (cordon/uncordon/drain/taint), keyed by `request_id`.
    NodeActionFinished {
        request_id: String,
        ok: bool,
        error: Option<String>,
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
    /// Terminal event of one command: emitted exactly once, after its handler returns,
    /// with the command's `Correlation::Request` (or `None` for uncorrelated commands).
    ///
    /// `ok` is false when the handler failed or reported an error while running (for
    /// `BulkDelete`: when any item failed). `error` is then the verbatim backend /
    /// Kubernetes error text (several failures are joined with newlines); it is `None`
    /// when `ok` is true.
    ///
    /// Its route is the command's section (namespaced to the target namespace for
    /// single-namespace writes) with `context: None`, so a UI never drops it as stale;
    /// commands without a section use the global `system` route.
    CommandFinished {
        ok: bool,
        error: Option<String>,
    },
}

pub type BackendEvent = BackendEventV1;

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(event: StreamEvent) {
        let json = serde_json::to_string(&event).unwrap();
        let back: StreamEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(format!("{back:?}"), format!("{event:?}"));
    }

    #[test]
    fn pod_exec_ready_roundtrip() {
        roundtrip(StreamEvent::PodExecReady {
            stream_id: StreamId(1),
        });
    }

    #[test]
    fn pod_exec_output_chunk_roundtrip() {
        roundtrip(StreamEvent::PodExecOutputChunk {
            stream_id: StreamId(1),
            data: "aGVsbG8=".to_owned(),
        });
    }

    #[test]
    fn resource_applied_roundtrip() {
        roundtrip(StreamEvent::ResourceApplied {
            request_id: "req-1".to_owned(),
            resource_version: "12345".to_owned(),
        });
    }

    #[test]
    fn drain_progress_roundtrip() {
        roundtrip(StreamEvent::DrainProgress {
            request_id: "req-1".to_owned(),
            total_pods: 3,
            evicted: vec!["pod-a".to_owned()],
            blocked_on: vec![(
                "pod-b".to_owned(),
                "Cannot evict pod as it would violate the pod's disruption budget.".to_owned(),
            )],
            ignored_daemonset_pods: vec!["pod-c".to_owned()],
        });
    }

    #[test]
    fn node_action_finished_roundtrip() {
        roundtrip(StreamEvent::NodeActionFinished {
            request_id: "req-1".to_owned(),
            ok: true,
            error: None,
        });
        roundtrip(StreamEvent::NodeActionFinished {
            request_id: "req-1".to_owned(),
            ok: false,
            error: Some("node not found".to_owned()),
        });
    }

    fn system_roundtrip(event: SystemEvent) {
        let json = serde_json::to_string(&event).unwrap();
        let back: SystemEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(format!("{back:?}"), format!("{event:?}"));
    }

    #[test]
    fn crd_catalog_updated_roundtrip() {
        system_roundtrip(SystemEvent::CrdCatalogUpdated {
            entries: vec![CrdCatalogEntry {
                group: "source.toolkit.fluxcd.io".to_owned(),
                kind: "GitRepository".to_owned(),
                plural: "gitrepositories".to_owned(),
                versions: vec!["v1".to_owned()],
                scope: CrdScope::Namespaced,
                permission_denied: false,
            }],
        });
    }

    #[test]
    fn crd_instances_updated_roundtrip() {
        system_roundtrip(SystemEvent::CrdInstancesUpdated {
            group: "source.toolkit.fluxcd.io".to_owned(),
            kind: "GitRepository".to_owned(),
            version: "v1".to_owned(),
            namespace: Some("flux-system".to_owned()),
            instances: vec![serde_json::json!({"metadata": {"name": "podinfo"}}).to_string()],
        });
    }
}
