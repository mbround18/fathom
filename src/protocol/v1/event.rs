use super::AuditLogEntry;
use super::{
    AdoptedConnectionProfile, ApiResourceCatalog, ApplicationDetails, ApplicationSummary,
    CertManagerOverview, CertificateChain, ClusterRoleBindingDetails, ClusterRoleBindingSummary,
    ClusterRoleDetails, ClusterRoleSummary, ClusterSettings, ClusterVersionSummary,
    ConfigMapDetails, ConfigMapSummary, ConnectionProfile, Correlation, CronJobDetails,
    CronJobSummary, DaemonSetDetails, DaemonSetSummary, DeploymentDetails, DeploymentSummary,
    EndpointDetails, EndpointSummary, EventsSummary, FluxOverview, FluxResourceSummary,
    GatekeeperOverview, HelmMutationApplied, HelmMutationFailed, HelmMutationPreviewReady,
    HelmReleaseDetails, HelmReleaseSummary, HorizontalPodAutoscalerDetails,
    HorizontalPodAutoscalerSummary, HubbleTrafficSummary, IngressClassDetails, IngressClassSummary,
    IngressDetails, IngressSummary, JobDetails, JobSummary, KedaOverview, KedaResourceDetails,
    KnativeOverview, KubeadmDetectionResult, LeaseDetails, LeaseSummary, LimitRangeDetails,
    LimitRangeSummary, MutatingWebhookConfigurationDetails, MutatingWebhookConfigurationSummary,
    NamespaceAccessResolved, NamespaceDetails, NamespaceSummary, NetworkPolicyDetails,
    NetworkPolicySummary, NodeDetails, NodeMetricsSummary, NodeShellAccessCheck, NodeShellPhase,
    NodeShellSettings, NodeSummary, OrphanedNodeShellPod, PersistentVolumeClaimDetails,
    PersistentVolumeClaimSummary, PersistentVolumeDetails, PersistentVolumeSummary, PodDetails,
    PodDisruptionBudgetDetails, PodDisruptionBudgetSummary, PodMetricsHistory, PodMetricsSummary,
    PodSummary, PriorityClassDetails, PriorityClassSummary, PrometheusMetrics, RbacAccessResult,
    RelatedResourceRef, ReplicaSetSummary, ReplicationControllerDetails,
    ReplicationControllerSummary, ResourceQuotaDetails, ResourceQuotaSummary, RoleBindingDetails,
    RoleBindingSummary, RoleDetails, RoleSummary, Route, RuntimeClassDetails, RuntimeClassSummary,
    SecretDetails, SecretSummary, SecretValueReveal, SectionKey, ServiceDetails, ServiceSummary,
    StatefulSetDetails, StatefulSetSummary, StorageClassDetails, StorageClassSummary, StreamId,
    UiConfig, ValidatingWebhookConfigurationDetails, ValidatingWebhookConfigurationSummary,
    VeleroOverview, VerticalPodAutoscalerDetails, VerticalPodAutoscalerSummary,
    WorkloadMetricsAggregate,
};
use super::{
    CapabilityQuery, CapabilityResult, CapabilitySet, IdpDirectorySnapshot, OidcConfig,
    OidcIdentity, OidcSignInStage, OrgClusterSettings, OrphanedSubject, TeamLink,
    TeamPublishRejection, TeamStoreStatus,
};
use super::{KindClusterSummary, KindPrerequisites};
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

/// Why the kubeconfig gave Fathom nothing to connect to (first-run UX). Fieldless enum,
/// serialized as a plain string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum KubeconfigIssueKind {
    /// No kubeconfig file exists at any searched path.
    Missing,
    /// A kubeconfig exists but defines no contexts (for example an empty file).
    Empty,
    /// A kubeconfig file exists but cannot be parsed.
    Malformed,
    /// A kubeconfig file exists but cannot be read (permissions, not a regular file).
    Unreadable,
    /// Contexts exist but none is selected, or `current-context` names a context that does not
    /// exist.
    NoCurrentContext,
}

/// A kubeconfig problem the UI should explain instead of a raw error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct KubeconfigIssue {
    pub kind: KubeconfigIssueKind,
    /// The files involved: the searched paths (`Missing`) or the offending file.
    pub paths: Vec<String>,
    /// Short, human-readable detail (parser message, missing context name). May be empty.
    pub detail: String,
}

/// Classified reason the active context's connection failed. Fieldless enum, serialized as a
/// plain string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum ConnectionProblemKind {
    /// Nothing is listening at the server address (connection refused, host down).
    Unreachable,
    /// The server host name does not resolve.
    Dns,
    /// The server did not answer in time.
    Timeout,
    /// TLS failed (untrusted or mismatched certificate).
    Tls,
    /// The server rejected the credentials (HTTP 401): expired or invalid token/certificate.
    Unauthorized,
    /// The credentials are valid but not allowed to read the API (HTTP 403).
    Forbidden,
    /// Anything else.
    Other,
}

/// A classified connection failure for one context (first-run UX).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ConnectionProblem {
    pub kind: ConnectionProblemKind,
    /// The server URL from the kubeconfig, when known.
    pub server: Option<String>,
    /// Verbatim backend error (Principle I), kept for a details disclosure.
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum SystemEvent {
    BackendStarted,

    /// The hosted session ended; the UI shows a sign-in prompt (031-hosted-server-mode).
    SessionEnded {
        reason: super::SessionEndReason,
    },

    /// A per-user limit refused the request (031-hosted-server-mode).
    LimitReached {
        kind: super::LimitKind,
        max: u32,
    },

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

    /// First-run UX: the kubeconfig problem found while (re)loading contexts, or `None` when the
    /// kubeconfig is usable. Emitted right after every `ContextsUpdated` from a kubeconfig load.
    KubeconfigStatus {
        issue: Option<KubeconfigIssue>,
    },

    /// First-run UX: the active context's connection check failed (`problem`), or recovered
    /// (`None`). Emitted by `Initialize`/`SetContext` after a bounded `GET /version`, and by the
    /// background watchers when the cluster goes away mid-session or comes back.
    ActiveConnectionProblem {
        context: String,
        problem: Option<ConnectionProblem>,
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
        /// 035: typed cloud-auth plugin failure behind `error`, when that is what failed.
        plugin_error: Option<crate::protocol::v1::PluginError>,
    },

    /// 035-cloud-auth-plugins: per-context exec plugin detection (no spawn), emitted on
    /// `Initialize`/`RefreshContexts` and after `SetExecPolicy`.
    CloudAuthDetected {
        infos: Vec<crate::protocol::v1::CloudAuthInfo>,
    },

    NamespaceFilterChanged {
        namespaces: Vec<String>,
    },

    /// 031: the section command on this route was denied by RBAC (`403 Forbidden`). The pane
    /// shows "Not permitted" with `message`; no error notification is raised.
    AccessDenied {
        message: String,
    },

    /// 031: whether the identity may list namespaces. When `restricted`, `NamespacesUpdated`
    /// carries only the context's namespace plus any the user typed, and the picker accepts a
    /// typed namespace name.
    NamespaceAccess {
        restricted: bool,
    },

    PrometheusConnectionTest {
        success: bool,
        message: String,
    },

    GrafanaConnectionTest {
        success: bool,
        message: String,
    },

    /// 039-observability-settings: saved non-secret settings for `context`. There is
    /// deliberately no field that can carry the Grafana API key, only `key_set`.
    ObservabilitySettings {
        context: String,
        prometheus: Option<PrometheusSettings>,
        grafana: Option<GrafanaSettings>,
    },

    MetricsStackScanned(crate::protocol::MetricsStackScanResult),

    PortAvailability {
        port: u16,
        available: bool,
    },

    /// 041: a `StartServicePortForward` tunnel is up (or an existing one was reused):
    /// `service`:`port` in `namespace` answers on `localhost:local_port`.
    ServicePortForwardReady {
        namespace: String,
        service: String,
        port: u16,
        local_port: u16,
        tunnel_id: String,
    },

    /// Disk usage metrics discovered via CRDs or Prometheus
    DiskUsageUpdated(Vec<DiskUsage>),

    /// Flux GitOps Toolkit state (sources, Kustomizations, HelmReleases)
    FluxOverviewUpdated(FluxOverview),

    /// Knative Serving state (Services, Revisions, Routes) — 028-knative-pane.
    KnativeOverviewUpdated(KnativeOverview),

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
    /// KEDA ScaledObjects/ScaledJobs (029-keda-autoscaler-pane).
    KedaOverviewUpdated(KedaOverview),
    /// KEDA single-resource detail for the Inspector.
    KedaResourceDetailsUpdated(KedaResourceDetails),
    /// The selected KEDA resource no longer exists.
    KedaResourceNotFound {
        kind: String,
        namespace: String,
        name: String,
    },

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
        /// Why the list could not be read; `None` when `instances` is the real (possibly empty)
        /// list. Typed so the UI can say "forbidden" or "not found" rather than show an empty
        /// list or spin forever.
        failure: Option<CrdListFailure>,
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

    // 038-sdk-login-ollama. None of these carry a secret.
    /// Single shared provider-status stream (also consumed by Available Agents).
    AiProviderStatus {
        statuses: Vec<super::ProviderStatus>,
        active: Option<super::Provider>,
        offline: bool,
        /// Model selected for Ollama, so consumers need no second event for the summary.
        ollama_model: Option<String>,
    },
    AiLoginFlowUpdate {
        flow: super::LoginFlow,
    },
    AiOllamaState {
        state: super::OllamaState,
        selected_model: Option<String>,
    },
    AiCredentialMigration {
        outcome: super::CredentialMigrationOutcome,
    },
    /// `message_verbatim` is the provider/OS error text unmodified (Principle I).
    AiError {
        code: String,
        message_verbatim: String,
        hint: String,
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
    // 033-rauthy-oidc-teams-users. No variant carries a token, refresh token, client secret
    // or admin API key. Stale events (superseded request, previous context) are discarded.
    /// Reply to `LoadOidcConfig` and broadcast after any config change.
    OidcConfigLoaded {
        config: OidcConfig,
        /// Current identity per signed-in issuer (empty when nobody is signed in).
        identities: Vec<OidcIdentity>,
    },
    OidcSignInProgress {
        issuer_id: String,
        stage: OidcSignInStage,
    },
    /// `None` = signed out / no identity for the issuer.
    OidcIdentityUpdated {
        issuer_id: String,
        identity: Option<OidcIdentity>,
    },
    /// The session ended (refresh failed, revoked). `reason` is the verbatim IdP error.
    OidcSessionEnded {
        issuer_id: String,
        reason: String,
    },
    /// Verbatim failure at `stage` (`"discovery"`, `"authorize"`, `"token"`, `"validate"`, ...).
    OidcError {
        issuer_id: String,
        stage: String,
        message: String,
    },
    /// Whether an admin API key exists in the keyring (never the key itself).
    IdpAdminKeyStatus {
        issuer_id: String,
        present: bool,
    },
    IdpDirectoryUpdated(IdpDirectorySnapshot),
    /// Advisory group -> shared-profile links from the team store (033 US3, R8).
    TeamLinksUpdated {
        links: Vec<TeamLink>,
    },
    CapabilitiesResolved(CapabilitySet),
    CapabilityChecked {
        query: CapabilityQuery,
        result: CapabilityResult,
    },
    /// Reply to `FetchManifests` (040). Every requested index appears in exactly one of
    /// `manifests` or `errors`, except an over-cap request (one error, index 0).
    ManifestsFetched {
        request_id: String,
        manifests: Vec<super::FetchedManifest>,
        errors: Vec<super::ManifestFetchError>,
    },
    /// Reply to `PreviewKubeconfigExport` (040). Contains no credential value.
    KubeconfigExportPreviewed(super::KubeconfigExportPreview),
    /// Result of `ExportKubeconfig`, or a failed preview (040). `message` never carries a
    /// credential value.
    KubeconfigExportFinished {
        context: String,
        ok: bool,
        path: Option<String>,
        embedded_credentials_written: bool,
        message: Option<String>,
    },
    RoleAssignmentPreview {
        yaml: String,
        rendered_subject_name: String,
        warnings: Vec<String>,
    },
    OrphanedSubjectsUpdated {
        orphans: Vec<OrphanedSubject>,
    },
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

    /// 027-fleet-rollup-dashboard: one context's fleet summary card, emitted once per poll
    /// attempt (success or failure) while that context's fleet poller is running. Routed
    /// globally, matching `ContextStatusUpdated`'s pattern — the frontend keys its own map by
    /// `.context` on receipt.
    FleetSummaryUpdated(crate::protocol::v1::FleetContextSummary),

    /// 034-simultaneous-federation: the federated set changed (or a snapshot was requested).
    FederationStateUpdated {
        federated: Vec<String>,
        soft_cap: u32,
        over_cap: bool,
    },
    /// 034-simultaneous-federation: one federated context's counters changed (debounced),
    /// with the recomputed combined rollup.
    FederationRollupUpdated {
        context: crate::protocol::v1::FederationContextRollup,
        combined: crate::protocol::v1::FederationCombinedRollup,
    },

    /// 036-incluster-daemon: per-context daemon status, routed globally like `ContextStatusUpdated`.
    DaemonStatusUpdated {
        context: String,
        status: crate::protocol::v1::DaemonStatus,
    },
    /// 036: batched (<= 200 / 250 ms) timeline entries tagged with the stream session so
    /// superseded sessions are discarded.
    DaemonEventsBatch {
        context: String,
        session: u64,
        events: Vec<crate::protocol::v1::DaemonTimelineEntry>,
        truncated: Option<crate::protocol::v1::TruncationMarker>,
    },
    /// 036: topology snapshot/delta from the daemon.
    DaemonTopologyUpdated {
        context: String,
        session: u64,
        delta: crate::protocol::v1::TopologyDelta,
    },
    /// 036: metric series from the daemon.
    DaemonMetricsUpdated {
        context: String,
        session: u64,
        series: Vec<crate::protocol::v1::MetricSeries>,
    },

    /// 037-node-shell: reply to `CheckNodeShellAccess`. Overall access is "every check allowed".
    NodeShellAccessChecked {
        namespace: String,
        checks: Vec<NodeShellAccessCheck>,
    },
    /// 037-node-shell: emitted on every session transition, reflecting real cluster state.
    /// `session_id` is a generation token; the UI drops events older than the current session.
    NodeShellStatus {
        session_id: u64,
        phase: NodeShellPhase,
        node: String,
        namespace: String,
        pod: String,
        /// Verbatim API error, pod condition or waiting reason.
        detail: Option<String>,
        /// True when deleting the pod failed and it still exists.
        leftover: bool,
    },
    /// 037-node-shell: reply to `ListNodeShellOrphans` / after `DeleteNodeShellOrphans`.
    NodeShellOrphans {
        pods: Vec<OrphanedNodeShellPod>,
    },
    /// 037-node-shell: reply to `GetNodeShellSettings`, and after a successful set.
    NodeShellSettingsLoaded {
        context: String,
        settings: NodeShellSettings,
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

/// Why one CRD kind's instance list could not be read (`CrdInstancesUpdated::failure`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum CrdListFailure {
    /// The API server refused the list (HTTP 403) for the current identity.
    Forbidden { message: String },
    /// The kind or the requested version is not served (HTTP 404), e.g. the CRD was removed.
    NotFound { message: String },
    /// Any other failure (timeout, conversion webhook error, server error).
    Failed { message: String },
}

/// 039: saved Prometheus endpoint for a context (non-secret).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PrometheusSettings {
    pub namespace: String,
    pub service: String,
    pub port: u16,
}

/// 039: saved Grafana settings for a context. The API key is never included, only whether one
/// is stored in the credential store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct GrafanaSettings {
    pub base_url: String,
    pub key_set: bool,
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
    VerticalPodAutoscalersUpdated(Vec<VerticalPodAutoscalerSummary>),
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
    NodeMetricsUpdated(NodeMetricsSummary),
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
    /// Reply to `RevealSecretValue` only (never unprompted, never logged): one key's value
    /// or a typed reason it could not be read. See [`SecretValueReveal`].
    SecretValueRevealed(SecretValueReveal),
    ResourceQuotaDetailsUpdated(ResourceQuotaDetails),
    LimitRangeDetailsUpdated(LimitRangeDetails),
    HorizontalPodAutoscalerDetailsUpdated(HorizontalPodAutoscalerDetails),
    VerticalPodAutoscalerDetailsUpdated(VerticalPodAutoscalerDetails),
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
    /// Reply for `CheckResourceMutationAccess` (018-rbac-aware-mutation-ui). Read-only.
    ResourceMutationAccessChecked {
        api_version: String,
        kind: String,
        result: RbacAccessResult,
    },
    /// Reply for `ResolveNamespaceAccess` (011-rbac-oidc-administration, Story 4). Read-only.
    NamespaceAccessResolved(NamespaceAccessResolved),
    /// Reply for `RefreshKubeadmDetection` (011-rbac-oidc-administration, Story 5). Never an
    /// error: an unreadable/absent ConfigMap simply yields `checked: false`.
    KubeadmDetectionUpdated(KubeadmDetectionResult),
    /// Reply for `RefreshApiResources` (042-role-create): API discovery flattened for the
    /// Role/ClusterRole rules editor's API group and resource pickers. Read-only.
    ApiResourcesUpdated(ApiResourceCatalog),
    /// Reply for `CheckPodExecAccess` (024-pod-exec-terminal): a live, proactive
    /// `pods/exec` `SelfSubjectAccessReview` result, checked before the exec-session
    /// confirmation is shown. `reason` MUST be `Some` when the underlying review call
    /// itself errored; it MAY be `None` for a plain server-side deny with no explanation.
    PodExecAccessChecked {
        namespace: String,
        name: String,
        container: Option<String>,
        allowed: bool,
        reason: Option<String>,
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
    /// 030: host readiness for local kind provisioning.
    KindPrerequisitesUpdated {
        prerequisites: KindPrerequisites,
    },
    /// 030: current local kind clusters.
    KindClustersUpdated {
        clusters: Vec<KindClusterSummary>,
    },
    /// 030: one stage line from an in-flight kind create/delete, keyed by `request_id`.
    KindProvisionProgress {
        request_id: String,
        stage: String,
    },
    /// 030: terminal event for a kind create/delete, keyed by `request_id`.
    KindClusterActionFinished {
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
    fn node_shell_events_roundtrip() {
        fn rt(e: SystemEvent) {
            let json = serde_json::to_string(&e).unwrap();
            let back: SystemEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(format!("{back:?}"), format!("{e:?}"));
        }
        use crate::protocol::v1::NodeShellCheckOutcome;
        rt(SystemEvent::NodeShellAccessChecked {
            namespace: "default".to_owned(),
            checks: vec![
                NodeShellAccessCheck {
                    verb: "create".to_owned(),
                    resource: "pods".to_owned(),
                    subresource: Some("exec".to_owned()),
                    namespace: Some("default".to_owned()),
                    outcome: NodeShellCheckOutcome::Denied {
                        reason: Some("no".to_owned()),
                    },
                },
                NodeShellAccessCheck {
                    verb: "get".to_owned(),
                    resource: "nodes".to_owned(),
                    subresource: None,
                    namespace: None,
                    outcome: NodeShellCheckOutcome::Error {
                        message: "boom".to_owned(),
                    },
                },
            ],
        });
        for phase in [
            NodeShellPhase::Creating,
            NodeShellPhase::Pending,
            NodeShellPhase::Running,
            NodeShellPhase::Attached,
            NodeShellPhase::Terminating,
            NodeShellPhase::Removed,
            NodeShellPhase::Failed,
        ] {
            rt(SystemEvent::NodeShellStatus {
                session_id: 2,
                phase,
                node: "n1".to_owned(),
                namespace: "default".to_owned(),
                pod: "p".to_owned(),
                detail: Some("ImagePullBackOff".to_owned()),
                leftover: phase == NodeShellPhase::Failed,
            });
        }
        rt(SystemEvent::NodeShellOrphans {
            pods: vec![OrphanedNodeShellPod {
                namespace: "default".to_owned(),
                name: "p".to_owned(),
                node: Some("n1".to_owned()),
                phase: Some("Failed".to_owned()),
                created_at: None,
            }],
        });
        rt(SystemEvent::NodeShellSettingsLoaded {
            context: "kind".to_owned(),
            settings: NodeShellSettings::default(),
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

    #[test]
    fn kind_events_roundtrip() {
        roundtrip(StreamEvent::KindPrerequisitesUpdated {
            prerequisites: KindPrerequisites {
                kind_installed: false,
                kind_version: None,
                runtime_installed: false,
                runtime_kind: None,
                runtime_running: false,
                ready: false,
            },
        });
        roundtrip(StreamEvent::KindClustersUpdated { clusters: vec![] });
        roundtrip(StreamEvent::KindProvisionProgress {
            request_id: "r".to_owned(),
            stage: "Preparing nodes".to_owned(),
        });
        roundtrip(StreamEvent::KindClusterActionFinished {
            request_id: "r".to_owned(),
            ok: false,
            error: Some("boom".to_owned()),
        });
    }

    #[test]
    fn keda_events_roundtrip() {
        use super::super::{KedaResourceSummary, ScaleReasoning, ScaleReasoningState};
        system_roundtrip(SystemEvent::KedaOverviewUpdated(KedaOverview::default()));
        let summary = KedaResourceSummary {
            kind: "ScaledObject".to_owned(),
            name: "queue-worker".to_owned(),
            namespace: "default".to_owned(),
            target_kind: "Deployment".to_owned(),
            target_name: "queue-worker".to_owned(),
            target_found: true,
            current_replicas: 4,
            min_replicas: 1,
            max_replicas: 10,
            paused: false,
            ready: true,
            trigger_types: vec!["prometheus".to_owned()],
            age: "3d".to_owned(),
        };
        system_roundtrip(SystemEvent::KedaOverviewUpdated(KedaOverview {
            installed: true,
            scaled_objects: vec![summary.clone()],
            scaled_jobs: vec![],
        }));
        system_roundtrip(SystemEvent::KedaResourceDetailsUpdated(
            KedaResourceDetails {
                summary,
                triggers: vec![],
                reasoning: ScaleReasoning {
                    state: ScaleReasoningState::MidRange,
                    message: "ok".to_owned(),
                },
            },
        ));
        system_roundtrip(SystemEvent::KedaResourceNotFound {
            kind: "ScaledObject".to_owned(),
            namespace: "default".to_owned(),
            name: "gone".to_owned(),
        });
    }

    fn system_roundtrip(event: SystemEvent) {
        let json = serde_json::to_string(&event).unwrap();
        let back: SystemEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(format!("{back:?}"), format!("{event:?}"));
    }

    #[test]
    fn manifest_export_events_roundtrip() {
        use crate::protocol::v1::{
            FetchedManifest, KubeconfigCredentialKind, KubeconfigExportPreview, ManifestFetchError,
        };
        system_roundtrip(SystemEvent::ManifestsFetched {
            request_id: "exp-1".to_owned(),
            manifests: vec![FetchedManifest {
                index: 0,
                raw_json: "{\"kind\":\"Deployment\"}".to_owned(),
            }],
            errors: vec![ManifestFetchError {
                index: 1,
                message: "not found".to_owned(),
            }],
        });
        system_roundtrip(SystemEvent::KubeconfigExportPreviewed(
            KubeconfigExportPreview {
                context: "c".to_owned(),
                redacted_yaml: "apiVersion: v1\n".to_owned(),
                credential: KubeconfigCredentialKind::Exec {
                    command: "kubelogin".to_owned(),
                },
                embedded_secret_fields: vec!["token".to_owned()],
                file_references: vec![],
                warnings: vec![],
                suggested_path: "/home/u/.kube/fathom-c.yaml".to_owned(),
                source_path: "/home/u/.kube/config".to_owned(),
            },
        ));
        system_roundtrip(SystemEvent::KubeconfigExportFinished {
            context: "c".to_owned(),
            ok: true,
            path: Some("/home/u/.kube/fathom-c.yaml".to_owned()),
            embedded_credentials_written: false,
            message: None,
        });
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
            failure: None,
        });
        system_roundtrip(SystemEvent::CrdInstancesUpdated {
            group: "acme.cert-manager.io".to_owned(),
            kind: "Order".to_owned(),
            version: "v1".to_owned(),
            namespace: None,
            instances: vec![],
            failure: Some(CrdListFailure::Forbidden {
                message: "orders is forbidden".to_owned(),
            }),
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

    #[test]
    fn ai_events_roundtrip() {
        use super::super::*;
        let status = ProviderStatus {
            provider: Provider::Github,
            state: ProviderState::Connected,
            account_label: Some("octocat".to_owned()),
            last_checked_at: Some("2026-01-01T00:00:00Z".to_owned()),
        };
        let ev = SystemEvent::AiProviderStatus {
            statuses: vec![status],
            active: Some(Provider::Github),
            offline: false,
            ollama_model: None,
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"AiProviderStatus\""), "{json}");
        system_roundtrip(ev);
        system_roundtrip(SystemEvent::AiLoginFlowUpdate {
            flow: LoginFlow {
                provider: Provider::Github,
                kind: LoginFlowKind::DeviceCode,
                user_code: Some("ABCD-1234".to_owned()),
                verification_uri: Some("https://github.com/login/device".to_owned()),
                expires_at: None,
                interval_s: Some(5),
                phase: LoginPhase::Pending,
            },
        });
        system_roundtrip(SystemEvent::AiOllamaState {
            state: OllamaState {
                endpoint: DEFAULT_OLLAMA_ENDPOINT.to_owned(),
                reachable: false,
                version: None,
                installed_hint: InstalledHint::NotFound,
                models: Vec::new(),
                checked_at: "2026-01-01T00:00:00Z".to_owned(),
                error: Some("connection refused".to_owned()),
            },
            selected_model: Some("llama3.2".to_owned()),
        });
        for outcome in [
            CredentialMigrationOutcome::Migrated,
            CredentialMigrationOutcome::NothingToMigrate,
            CredentialMigrationOutcome::Failed {
                message: "locked".to_owned(),
            },
        ] {
            system_roundtrip(SystemEvent::AiCredentialMigration { outcome });
        }
        system_roundtrip(SystemEvent::AiError {
            code: "offline_blocked".to_owned(),
            message_verbatim: "x".to_owned(),
            hint: "y".to_owned(),
        });
    }

    fn resource_read_roundtrip(event: ResourceReadEvent) {
        let json = serde_json::to_string(&event).unwrap();
        let back: ResourceReadEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(format!("{back:?}"), format!("{event:?}"));
    }

    #[test]
    fn api_resources_updated_roundtrip() {
        use super::super::{ApiGroupResources, ApiResourceName};
        resource_read_roundtrip(ResourceReadEvent::ApiResourcesUpdated(ApiResourceCatalog {
            groups: vec![ApiGroupResources {
                group: String::new(),
                resources: vec![ApiResourceName {
                    name: "pods/log".to_owned(),
                    namespaced: true,
                    verbs: vec!["get".to_owned()],
                }],
            }],
            failed_groups: vec!["metrics.k8s.io".to_owned()],
        }));
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
    fn pod_exec_access_checked_roundtrip() {
        resource_read_roundtrip(ResourceReadEvent::PodExecAccessChecked {
            namespace: "default".to_owned(),
            name: "web-0".to_owned(),
            container: Some("app".to_owned()),
            allowed: true,
            reason: None,
        });
        resource_read_roundtrip(ResourceReadEvent::PodExecAccessChecked {
            namespace: "default".to_owned(),
            name: "web-0".to_owned(),
            container: None,
            allowed: false,
            reason: Some("forbidden: user cannot create pods/exec".to_owned()),
        });
    }

    #[test]
    fn context_status_updated_roundtrip() {
        system_roundtrip(SystemEvent::ContextStatusUpdated {
            context: "prod".to_owned(),
            status: ContextConnectionStatus::Connected,
            error: None,
            plugin_error: None,
        });
        system_roundtrip(SystemEvent::ContextStatusUpdated {
            context: "prod".to_owned(),
            status: ContextConnectionStatus::Connecting,
            error: None,
            plugin_error: None,
        });
        system_roundtrip(SystemEvent::ContextStatusUpdated {
            context: "prod".to_owned(),
            status: ContextConnectionStatus::NotConnected,
            error: None,
            plugin_error: None,
        });
        system_roundtrip(SystemEvent::ContextStatusUpdated {
            context: "prod".to_owned(),
            status: ContextConnectionStatus::Error,
            error: Some("connection refused".to_owned()),
            plugin_error: None,
        });
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn oidc_events_roundtrip() {
        use super::super::{
            CapabilityRule, CapabilitySource, ContextAuthMode, IdpGroup, IdpUser, OidcIdentityState,
        };
        let identity = OidcIdentity {
            issuer_id: "rauthy".to_owned(),
            sub: "u1".to_owned(),
            username: "alice".to_owned(),
            email: None,
            groups: vec!["fathom-viewers".to_owned()],
            expires_at: 1,
            state: OidcIdentityState::Refreshing,
        };
        system_roundtrip(SystemEvent::OidcConfigLoaded {
            config: OidcConfig::default(),
            identities: vec![identity.clone()],
        });
        for stage in [
            OidcSignInStage::Awaiting,
            OidcSignInStage::DeviceCode {
                user_code: "AB-12".to_owned(),
                verification_uri: "https://x/device".to_owned(),
                expires_at: 5,
            },
            OidcSignInStage::Validating,
            OidcSignInStage::Done,
        ] {
            system_roundtrip(SystemEvent::OidcSignInProgress {
                issuer_id: "rauthy".to_owned(),
                stage,
            });
        }
        system_roundtrip(SystemEvent::OidcIdentityUpdated {
            issuer_id: "rauthy".to_owned(),
            identity: Some(identity),
        });
        system_roundtrip(SystemEvent::OidcIdentityUpdated {
            issuer_id: "rauthy".to_owned(),
            identity: None,
        });
        system_roundtrip(SystemEvent::OidcSessionEnded {
            issuer_id: "rauthy".to_owned(),
            reason: "invalid_grant".to_owned(),
        });
        system_roundtrip(SystemEvent::OidcError {
            issuer_id: "rauthy".to_owned(),
            stage: "discovery".to_owned(),
            message: "connection refused".to_owned(),
        });
        system_roundtrip(SystemEvent::IdpAdminKeyStatus {
            issuer_id: "rauthy".to_owned(),
            present: true,
        });
        system_roundtrip(SystemEvent::IdpDirectoryUpdated(IdpDirectorySnapshot {
            issuer_id: "rauthy".to_owned(),
            users: vec![IdpUser {
                sub: "u1".to_owned(),
                username: "alice".to_owned(),
                email: Some("a@x".to_owned()),
                groups: vec![],
                enabled: true,
            }],
            groups: vec![IdpGroup {
                name: "g".to_owned(),
                member_count: 2,
            }],
            fetched_at: 3,
            stale: false,
            error: None,
        }));
        system_roundtrip(SystemEvent::CapabilitiesResolved(CapabilitySet {
            context: "prod".to_owned(),
            namespace: Some("default".to_owned()),
            identity_sub: "u1".to_owned(),
            rules: vec![CapabilityRule::default()],
            incomplete: true,
            fetched_at: 4,
        }));
        system_roundtrip(SystemEvent::CapabilityChecked {
            query: CapabilityQuery {
                context: "prod".to_owned(),
                namespace: None,
                verb: "get".to_owned(),
                resource: "nodes".to_owned(),
                api_group: None,
                subresource: None,
            },
            result: CapabilityResult {
                allowed: true,
                reason: None,
                source: CapabilitySource::Rules,
                verified: true,
            },
        });
        system_roundtrip(SystemEvent::RoleAssignmentPreview {
            yaml: "kind: RoleBinding\n".to_owned(),
            rendered_subject_name: "oidc:dev".to_owned(),
            warnings: vec!["no groups prefix configured".to_owned()],
        });
        system_roundtrip(SystemEvent::OrphanedSubjectsUpdated {
            orphans: vec![OrphanedSubject {
                binding: "ClusterRoleBinding/x".to_owned(),
                subject_name: "oidc:gone".to_owned(),
            }],
        });
        let _ = ContextAuthMode::Oidc;
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

    #[test]
    fn knative_overview_updated_roundtrip() {
        system_roundtrip(SystemEvent::KnativeOverviewUpdated(
            KnativeOverview::default(),
        ));
        system_roundtrip(SystemEvent::KnativeOverviewUpdated(KnativeOverview {
            installed: true,
            services: vec![super::super::KnativeServiceSummary {
                name: "hello".to_owned(),
                namespace: "default".to_owned(),
                ready: super::super::KnativeReadyState::Ready,
                latest_ready_revision: Some("hello-00001".to_owned()),
                latest_created_revision: Some("hello-00001".to_owned()),
            }],
            revisions: vec![super::super::KnativeRevisionSummary {
                name: "hello-00001".to_owned(),
                namespace: "default".to_owned(),
                service_name: "hello".to_owned(),
                ready: super::super::KnativeReadyState::Ready,
                actual_replicas: 0,
                desired_replicas: 0,
                scale_reason: super::super::KnativeScaleReason::ScaledToZero {
                    idle_since: Some("2026-09-23T00:00:00Z".to_owned()),
                },
                traffic_percent: 100,
            }],
            routes: vec![super::super::KnativeRouteSummary {
                name: "hello".to_owned(),
                namespace: "default".to_owned(),
                service_name: "hello".to_owned(),
                ready: super::super::KnativeReadyState::Ready,
                url: Some("http://hello.default.example.com".to_owned()),
                traffic: vec![super::super::KnativeRouteTrafficTarget {
                    revision_name: "hello-00001".to_owned(),
                    percent: 100,
                    tag: None,
                    revision_missing: false,
                }],
            }],
        }));
    }
}
