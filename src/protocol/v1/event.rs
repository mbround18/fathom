use super::AuditLogEntry;
use super::{
    AdoptedConnectionProfile, ApplicationDetails, ApplicationSummary, CertManagerOverview,
    CertificateChain, ClusterRoleBindingDetails, ClusterRoleBindingSummary, ClusterRoleDetails,
    ClusterRoleSummary, ClusterSettings, ClusterVersionSummary, ConfigMapDetails, ConfigMapSummary,
    ConnectionProfile, Correlation, CronJobDetails, CronJobSummary, DaemonSetDetails,
    DaemonSetSummary, DeploymentDetails, DeploymentSummary, EndpointDetails, EndpointSummary,
    EventsSummary, FluxOverview, FluxResourceSummary, GatekeeperOverview, HelmMutationApplied,
    HelmMutationFailed, HelmMutationPreviewReady, HelmReleaseDetails, HelmReleaseSummary,
    HorizontalPodAutoscalerDetails, HorizontalPodAutoscalerSummary, HubbleTrafficSummary,
    IngressClassDetails, IngressClassSummary, IngressDetails, IngressSummary, JobDetails,
    JobSummary, KubeadmDetectionResult, LeaseDetails, LeaseSummary, LimitRangeDetails,
    LimitRangeSummary, MutatingWebhookConfigurationDetails, MutatingWebhookConfigurationSummary,
    NamespaceAccessResolved, NamespaceDetails, NamespaceSummary, NetworkPolicyDetails,
    NetworkPolicySummary, NodeDetails, NodeSummary, PersistentVolumeClaimDetails,
    PersistentVolumeClaimSummary, PersistentVolumeDetails, PersistentVolumeSummary, PodDetails,
    PodDisruptionBudgetDetails, PodDisruptionBudgetSummary, PodMetricsHistory, PodMetricsSummary,
    PodSummary, PriorityClassDetails, PriorityClassSummary, PrometheusMetrics, RbacAccessResult,
    RelatedResourceRef, ReplicaSetSummary, ReplicationControllerDetails,
    ReplicationControllerSummary, ResourceQuotaDetails, ResourceQuotaSummary, RoleBindingDetails,
    RoleBindingSummary, RoleDetails, RoleSummary, Route, RuntimeClassDetails, RuntimeClassSummary,
    SecretDetails, SecretSummary, SectionKey, ServiceDetails, ServiceSummary, StatefulSetDetails,
    StatefulSetSummary, StorageClassDetails, StorageClassSummary, StreamId, UiConfig,
    ValidatingWebhookConfigurationDetails, ValidatingWebhookConfigurationSummary, VeleroOverview,
    WorkloadMetricsAggregate,
};
use super::{OrgClusterSettings, TeamPublishRejection, TeamStoreStatus};
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

/// Live reachability/connection status for one kubeconfig context
/// (012-incluster-daemon-federation P1, FR-002). Fieldless enum: serialized as a plain string
/// (`"Connected" | "Connecting" | "Error" | "NotConnected"`), matching `NotificationLevel`'s
/// convention. See `data-model.md` for the state-transition diagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum ContextConnectionStatus {
    /// This is the active context and its live client is connected.
    Connected,
    /// A connect attempt (active context) or reachability probe (other context) is in flight.
    Connecting,
    /// Last connect attempt or probe failed; the sibling `error` field carries the detail.
    Error,
    /// Reachability probe succeeded but this is not the active context (no live client).
    NotConnected,
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

    /// Live per-context connection status (012-incluster-daemon-federation P1, FR-002/FR-004).
    /// Emitted once per non-active context by a bounded background reachability probe, and for
    /// the active context by the actual connect/switch attempt in `cmd/cluster.rs` — never both
    /// for the same context (research.md R3), so the UI can apply these last-write-wins per
    /// `context` with no reconciliation.
    ContextStatusUpdated {
        context: String,
        status: ContextConnectionStatus,
        /// Verbatim backend error (Principle I), present only when `status == Error`.
        error: Option<String>,
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

    PortAvailability {
        port: u16,
        available: bool,
    },

    /// Disk usage metrics discovered via CRDs or Prometheus
    DiskUsageUpdated(Vec<DiskUsage>),

    /// Flux GitOps Toolkit state (sources, Kustomizations, HelmReleases)
    FluxOverviewUpdated(FluxOverview),

    /// Cluster + node Kubernetes version snapshot, refreshed periodically (019).
    ClusterVersionUpdated(ClusterVersionSummary),
    /// Velero backup state (Backups, Schedules, Restores, BackupStorageLocations; 022-velero-backup-pane)
    VeleroOverviewUpdated(VeleroOverview),
    /// cert-manager CRD detection + Certificate list (dashboard aggregate).
    CertManagerOverviewUpdated(CertManagerOverview),

    /// Resolved lifecycle chain for one Certificate.
    CertificateChainUpdated(CertificateChain),
    /// Aqua Trivy Operator vulnerability state, cluster-wide (021-trivy-vulnerability-pane).
    TrivyOverviewUpdated(super::TrivyOverview),

    /// Hubble/Cilium traffic summary, sampled from a bounded `GetFlows` poll.
    HubbleTrafficUpdated(HubbleTrafficSummary),

    /// OPA Gatekeeper constraint compliance state (020-gatekeeper-policy-pane).
    GatekeeperOverviewUpdated(GatekeeperOverview),

    /// A rollback/upgrade diff-preview was computed; nothing has been mutated yet (006).
    HelmMutationPreviewReady(HelmMutationPreviewReady),

    /// A rollback/upgrade/uninstall mutation succeeded (006).
    HelmMutationApplied(HelmMutationApplied),

    /// A preview or apply stage of a Helm mutate flow failed (006).
    HelmMutationFailed(HelmMutationFailed),

    /// Reply to `BackendCmd::LoadUiConfig`.
    /// `ui` is `None` when absent or invalid (the invalid blob is kept under `ui_invalid`).
    UiConfigLoaded {
        ui: Option<UiConfig>,
        expanded_groups: Vec<String>,
        /// `config.json → auto_start.mcp_stdio`: whether the stdio MCP server
        /// (015-mcp-agent-exposure) is enabled. Off by default (FR-002).
        mcp_stdio_enabled: bool,
    },

    /// Reply to `BackendCmd::RefreshAuditLog` (026-mutation-audit-log-viewer): the local,
    /// bounded mutation history, oldest first.
    AuditLogLoaded {
        entries: Vec<AuditLogEntry>,
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

    /// Reply to `AgentGetAuthMode`, and broadcast after `AgentSetAuthMode` (013).
    AgentAuthModeUpdated {
        mode: crate::protocol::v1::AgentAuthMode,
    },
    /// Reply to `AgentRevealKey` only — never sent unprompted, never logged (FR-004,
    /// Principle VI).
    AgentKeyRevealed {
        key: String,
    },

    /// 014-teams-organizations: reply to `LoadClusterSettings`, and broadcast after
    /// `SaveClusterSettings`/`AcceptOrgClusterSettings`/`DismissOrgClusterSettings`.
    ClusterSettingsLoaded {
        context: String,
        settings: ClusterSettings,
    },
    /// `SaveClusterSettings`/`AcceptOrgClusterSettings` failed local validation or the
    /// config write itself; the caller's prior local settings are left untouched
    /// (FR-009, Constitution Principle VI).
    ClusterSettingsError {
        context: String,
        reason: String,
    },
    /// Reply to `RefreshTeamStoreStatus`, and broadcast whenever reachability changes
    /// (FR-005).
    TeamStoreStatusUpdated(TeamStoreStatus),
    /// Reply to `RefreshConnectionProfiles`, and broadcast after publish/adopt/remove.
    ConnectionProfilesUpdated {
        /// Profiles this operator has adopted locally (still-shared or not).
        adopted: Vec<AdoptedConnectionProfile>,
        /// Profiles published by teammates that this operator has not adopted yet.
        /// Empty when the shared store is unreachable or unconfigured (FR-005).
        available: Vec<ConnectionProfile>,
    },
    /// A newer org-level Cluster Settings default is available for `context` and has
    /// not yet been accepted or dismissed by this operator (FR-007).
    OrgClusterSettingsAvailable {
        context: String,
        settings: OrgClusterSettings,
    },
    /// `PublishConnectionProfile`/`PublishOrgClusterSettings` was refused: a suspected
    /// secret, a store-write conflict, or an unreachable store (FR-002, FR-012).
    TeamPublishRejected {
        rejection: TeamPublishRejection,
    },

    /// 015-mcp-agent-exposure: reply to `LoadMcpServerStatus`, and broadcast after every
    /// `SetMcpServerEnabled`. `running` is never `true` while `enabled` is `false` — the
    /// stdio server task only ever exists inside the `enabled: true` path (FR-002/FR-013).
    McpServerStatusChanged {
        enabled: bool,
        running: bool,
        /// The literal command an external MCP client should launch to attach to this
        /// Fathom installation over stdio (FR-007).
        connection_command: String,
    },

    // 015-mcp-agent-exposure US4: the "Launch example chat" subprocess (`src/backend/mcp/
    // demo_chat.rs`). `session_id` counts up from 1 per Fathom process, so the terminal view
    // can tell its own session's output from a previous one's trailing chunks — the same
    // "track the highest id you started" convention `PodTerminalTab` uses for stream ids.
    /// The `claude` subprocess started and its stdin/stdout are attached.
    McpDemoChatReady {
        session_id: u32,
    },
    /// Base64-encoded subprocess output (stdout and stderr interleaved), same shape as
    /// `StreamEvent::PodExecOutputChunk`.
    McpDemoChatOutputChunk {
        session_id: u32,
        data: String,
    },
    /// The subprocess exited, or was stopped. `reason` is shown verbatim.
    McpDemoChatEnded {
        session_id: u32,
        reason: String,
    },
    /// No `claude` executable was found on `PATH`. Emitted instead of — never alongside — a
    /// spawn, so the affordance can never present a fabricated session (FR-009/FR-011).
    /// `instructions` is the operator-facing setup text.
    McpDemoChatNotDetected {
        instructions: String,
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
    /// Success reply for `InspectContainerImage` (010-container-image-inspection).
    ContainerImageInspected {
        namespace: String,
        pod: String,
        container: String,
        inspection: super::ImageInspection,
    },
    /// Failure reply for `InspectContainerImage`: an explicit, distinguishable error state
    /// (FR-009), never a blank/generic panel.
    ContainerImageInspectFailed {
        namespace: String,
        pod: String,
        container: String,
        error: super::ImageInspectError,
    },
    /// Success reply for `InspectPodVulnerabilities` (021-trivy-vulnerability-pane).
    PodVulnerabilitiesInspected {
        namespace: String,
        pod: String,
        inspection: super::PodVulnerabilityInspection,
    },
    /// Failure reply for `InspectPodVulnerabilities`: an explicit, distinguishable error state
    /// (FR-007/FR-008/FR-013), never a blank/generic panel.
    PodVulnerabilitiesInspectFailed {
        namespace: String,
        pod: String,
        error: super::VulnInspectError,
    },
    /// Full conversation state for one object, sent after `AgentOpenConversation`,
    /// `AgentAsk`, `AgentClearConversation`, `AgentReviewSuggestedAction`,
    /// `AgentDeclineSuggestedAction`, and `AgentRecordExecutedAction` (013). Always the
    /// full conversation, never a partial diff — the same "resend the whole detail"
    /// convention every other `*DetailsUpdated` event already uses.
    AgentConversationUpdated {
        object: super::AgentObjectRef,
        conversation: super::AgentConversation,
    },
    /// Reply for `RefreshRbacSelfAccess` (011-rbac-oidc-administration, FR-007 gating).
    RbacSelfAccessUpdated {
        results: Vec<RbacAccessResult>,
    },
    /// Reply for `ResolveNamespaceAccess` (011-rbac-oidc-administration, Story 4). Read-only.
    NamespaceAccessResolved(NamespaceAccessResolved),
    /// Reply for `RefreshKubeadmDetection` (011-rbac-oidc-administration, Story 5). Never an
    /// error: an unreadable/absent ConfigMap simply yields `checked: false`.
    KubeadmDetectionUpdated(KubeadmDetectionResult),
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
    use crate::protocol::{
        GatekeeperConstraintSummary, GatekeeperInstallState, GatekeeperViolation,
    };

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

    #[test]
    fn velero_overview_updated_roundtrip() {
        system_roundtrip(SystemEvent::VeleroOverviewUpdated(VeleroOverview {
            installed: true,
            permission_denied: false,
            backups: vec![super::super::VeleroBackupSummary {
                name: "nightly-20260101".to_owned(),
                namespace: "velero".to_owned(),
                phase: super::super::VeleroPhase::Completed,
                schedule_name: Some("nightly".to_owned()),
                start_timestamp: "2026-01-01T00:00:00Z".to_owned(),
                completion_timestamp: "2026-01-01T00:05:00Z".to_owned(),
                errors: 0,
                warnings: 0,
            }],
            schedules: vec![],
            restores: vec![],
            storage_locations: vec![],
        }));
    }

    #[test]
    fn agent_auth_mode_updated_roundtrip() {
        system_roundtrip(SystemEvent::AgentAuthModeUpdated {
            mode: super::super::AgentAuthMode::Unconfigured,
        });
        system_roundtrip(SystemEvent::AgentAuthModeUpdated {
            mode: super::super::AgentAuthMode::Anthropic {
                key_configured: true,
            },
        });
    }

    #[test]
    fn agent_key_revealed_roundtrip() {
        system_roundtrip(SystemEvent::AgentKeyRevealed {
            key: "sk-ant-test".to_owned(),
        });
    }

    fn resource_read_roundtrip(event: ResourceReadEvent) {
        let json = serde_json::to_string(&event).unwrap();
        let back: ResourceReadEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(format!("{back:?}"), format!("{event:?}"));
    }

    #[test]
    fn agent_conversation_updated_roundtrip() {
        resource_read_roundtrip(ResourceReadEvent::AgentConversationUpdated {
            object: super::super::AgentObjectRef {
                kind: "Pod".to_owned(),
                namespace: "default".to_owned(),
                name: "api-0".to_owned(),
            },
            conversation: super::super::AgentConversation::default(),
        });
    }

    #[test]
    fn context_status_updated_roundtrip() {
        system_roundtrip(SystemEvent::ContextStatusUpdated {
            context: "prod".to_owned(),
            status: ContextConnectionStatus::Connected,
            error: None,
        });
        system_roundtrip(SystemEvent::ContextStatusUpdated {
            context: "prod".to_owned(),
            status: ContextConnectionStatus::Connecting,
            error: None,
        });
        system_roundtrip(SystemEvent::ContextStatusUpdated {
            context: "prod".to_owned(),
            status: ContextConnectionStatus::NotConnected,
            error: None,
        });
        system_roundtrip(SystemEvent::ContextStatusUpdated {
            context: "prod".to_owned(),
            status: ContextConnectionStatus::Error,
            error: Some("connection refused".to_owned()),
        });
    }

    #[test]
    fn team_events_roundtrip() {
        system_roundtrip(SystemEvent::ClusterSettingsLoaded {
            context: "prod".to_owned(),
            settings: super::super::ClusterSettings::default(),
        });
        system_roundtrip(SystemEvent::ClusterSettingsError {
            context: "prod".to_owned(),
            reason: "unsupported clusterSettings.schema 2 (expected 1)".to_owned(),
        });
        system_roundtrip(SystemEvent::TeamStoreStatusUpdated(TeamStoreStatus {
            store_path: Some("/mnt/team-fathom".to_owned()),
            reachable: true,
            error: None,
        }));
        system_roundtrip(SystemEvent::ConnectionProfilesUpdated {
            adopted: vec![],
            available: vec![],
        });
        system_roundtrip(SystemEvent::OrgClusterSettingsAvailable {
            context: "prod".to_owned(),
            settings: OrgClusterSettings {
                context: "prod".to_owned(),
                rev: 2,
                published_by: "alice".to_owned(),
                published_at: "2026-09-23T00:00:00Z".to_owned(),
                metrics_source: super::super::MetricsSource::Prometheus,
                prometheus_path: "/metrics".to_owned(),
                https: true,
                path_prefix: String::new(),
                hidden_metrics: vec![],
            },
        });
        system_roundtrip(SystemEvent::TeamPublishRejected {
            rejection: TeamPublishRejection::SuspectedSecret {
                field: "apiServer".to_owned(),
                reason: "looks like it contains credential/secret material".to_owned(),
            },
        });
        system_roundtrip(SystemEvent::TeamPublishRejected {
            rejection: TeamPublishRejection::Conflict {
                expected_rev: 1,
                current_rev: 2,
            },
        });
    }

    #[test]
    fn gatekeeper_overview_updated_roundtrip() {
        system_roundtrip(SystemEvent::GatekeeperOverviewUpdated(GatekeeperOverview {
            state: GatekeeperInstallState::Active,
            error: String::new(),
            constraints: vec![GatekeeperConstraintSummary {
                kind: "K8sRequiredLabels".to_owned(),
                name: "require-team-label".to_owned(),
                enforcement_action: "deny".to_owned(),
                total_violations: Some(1),
                violations: vec![GatekeeperViolation {
                    section: SectionKey::new("workloads/pods"),
                    kind: "Pod".to_owned(),
                    namespace: "default".to_owned(),
                    name: "nginx".to_owned(),
                    enforcement_action: "deny".to_owned(),
                    message: "missing required label \"team\"".to_owned(),
                }],
                violations_truncated: false,
                template_missing: false,
            }],
        }));
        system_roundtrip(SystemEvent::GatekeeperOverviewUpdated(GatekeeperOverview {
            state: GatekeeperInstallState::NotInstalled,
            error: String::new(),
            constraints: Vec::new(),
        }));
    }
}
