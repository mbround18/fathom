use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{RequestId, SectionKey, UiConfig};

/// Commands sent from a UI client to the backend. Fire-and-forget: results arrive as events.
///
/// Serialized adjacently tagged: `{ "type": "RefreshPods" }`,
/// `{ "type": "SetContext", "data": { "context": "kind-a" } }`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum BackendCmdV1 {
    // Global / shell-owned
    Initialize,
    Shutdown,
    RefreshContexts,
    SetContext {
        context: String,
    },
    RefreshNamespaces,
    SetNamespaceFilter {
        namespaces: Vec<String>,
    },
    RefreshNodes,
    SelectNode {
        name: String,
    },
    RefreshEvents,

    // Workloads / Pods section
    RefreshPods,
    SelectPod {
        namespace: String,
        name: String,
    },
    DeletePod {
        namespace: String,
        name: String,
    },
    StartPodLogs {
        namespace: String,
        name: String,
        container: Option<String>,
    },
    StopPodLogs,
    FetchPodMetrics {
        namespace: String,
        name: String,
    },

    // Workloads / Deployments section
    RefreshDeployments,
    SelectDeployment {
        namespace: String,
        name: String,
    },
    ScaleDeployment {
        namespace: String,
        name: String,
        replicas: i32,
    },
    RestartDeployment {
        namespace: String,
        name: String,
    },
    DeleteDeployment {
        namespace: String,
        name: String,
    },

    // Workloads / DaemonSets section
    RefreshDaemonSets,
    SelectDaemonSet {
        namespace: String,
        name: String,
    },
    RestartDaemonSet {
        namespace: String,
        name: String,
    },
    DeleteDaemonSet {
        namespace: String,
        name: String,
    },

    // Workloads / StatefulSets section
    RefreshStatefulSets,
    SelectStatefulSet {
        namespace: String,
        name: String,
    },
    ScaleStatefulSet {
        namespace: String,
        name: String,
        replicas: i32,
    },
    RestartStatefulSet {
        namespace: String,
        name: String,
    },
    DeleteStatefulSet {
        namespace: String,
        name: String,
    },

    // Workloads / ReplicaSets section
    RefreshReplicaSets,
    SelectReplicaSet {
        namespace: String,
        name: String,
    },
    ScaleReplicaSet {
        namespace: String,
        name: String,
        replicas: i32,
    },
    RestartReplicaSet {
        namespace: String,
        name: String,
    },
    DeleteReplicaSet {
        namespace: String,
        name: String,
    },

    // Workloads / ReplicationControllers section
    RefreshReplicationControllers,
    SelectReplicationController {
        namespace: String,
        name: String,
    },
    ScaleReplicationController {
        namespace: String,
        name: String,
        replicas: i32,
    },
    RestartReplicationController {
        namespace: String,
        name: String,
    },
    DeleteReplicationController {
        namespace: String,
        name: String,
    },
    // Workloads / Jobs section
    RefreshJobs,
    SelectJob {
        namespace: String,
        name: String,
    },
    DeleteJob {
        namespace: String,
        name: String,
    },

    // Workloads / CronJobs section
    RefreshCronJobs,
    SelectCronJob {
        namespace: String,
        name: String,
    },
    SuspendCronJob {
        namespace: String,
        name: String,
        suspend: bool,
    },
    TriggerCronJob {
        namespace: String,
        name: String,
    },
    DeleteCronJob {
        namespace: String,
        name: String,
    },
    // Network / Port Forwards section
    CheckPortAvailability {
        port: u16,
    },
    StartPortForward {
        namespace: String,
        pod: String,
        local_port: u16,
        remote_port: u16,
        container: Option<String>,
    },
    StopPortForward {
        tunnel_id: String,
    },
    DeletePortForward {
        tunnel_id: String,
    },
    UpdatePortForward {
        tunnel_id: String,
        local_port: u16,
        remote_port: u16,
        auto_start: bool,
    },
    RefreshPortForwards,

    // Network / Services section
    RefreshServices,
    SelectService {
        namespace: String,
        name: String,
    },
    DeleteService {
        namespace: String,
        name: String,
    },

    // Network / Endpoints section
    RefreshEndpoints,
    SelectEndpoint {
        namespace: String,
        name: String,
    },

    // Network / Ingresses section
    RefreshIngresses,
    SelectIngress {
        namespace: String,
        name: String,
    },
    DeleteIngress {
        namespace: String,
        name: String,
    },

    // Network / Ingress Classes section
    RefreshIngressClasses,
    SelectIngressClass {
        name: String,
    },

    // Network / Network Policies section
    RefreshNetworkPolicies,
    SelectNetworkPolicy {
        namespace: String,
        name: String,
    },
    DeleteNetworkPolicy {
        namespace: String,
        name: String,
    },

    // Integrations / Metrics section
    RefreshIntegrations,
    StartMetricsPolling {
        namespace: String,
        name: String,
        interval_secs: u64,
    },
    StopMetricsPolling,
    StartWorkloadMetricsPolling {
        namespace: String,
        workload_name: String,
        workload_kind: String,
        pod_selector: String,
        interval_secs: u64,
    },
    StopWorkloadMetricsPolling,
    ConfigurePrometheus {
        namespace: String,
        service: String,
        port: u16,
    },
    TestPrometheusConnection {
        namespace: String,
        service: String,
        port: u16,
    },
    ConfigureGrafana {
        base_url: String,
        api_key: String,
    },
    TestGrafanaConnection {
        base_url: String,
        api_key: String,
    },
    RefreshFlux,
    RefreshHubble,
    // MCP Server integration
    StartMcpServer,
    StopMcpServer,
    RefreshMcpStatus,
    OpenMcpConfigDir,
    // Keeps the original Day-1 “proof” button around.
    SimulateSlowCall,
    // Bulk operations
    BulkDelete {
        items: Vec<(String, String)>, // (namespace, name)
    },
    // Future hook: allow UI to hint active section for prefetching (optional).
    SetActiveSection {
        section: SectionKey,
    },

    // Storage / Config / Access resources
    // storage_pv
    RefreshPersistentVolumes,
    SelectPersistentVolume {
        name: String,
    },
    DeletePersistentVolume {
        name: String,
    },

    // storage_pvc
    RefreshPersistentVolumeClaims,
    SelectPersistentVolumeClaim {
        namespace: String,
        name: String,
    },
    DeletePersistentVolumeClaim {
        namespace: String,
        name: String,
    },

    // storage_storageclasses
    RefreshStorageClasses,
    SelectStorageClass {
        name: String,
    },
    DeleteStorageClass {
        name: String,
    },

    // access_roles
    RefreshRoles,
    SelectRole {
        namespace: String,
        name: String,
    },
    DeleteRole {
        namespace: String,
        name: String,
    },

    // access_rolebindings
    RefreshRoleBindings,
    SelectRoleBinding {
        namespace: String,
        name: String,
    },
    DeleteRoleBinding {
        namespace: String,
        name: String,
    },

    // access_clusterroles
    RefreshClusterRoles,
    SelectClusterRole {
        name: String,
    },
    DeleteClusterRole {
        name: String,
    },

    // access_clusterrolebindings
    RefreshClusterRoleBindings,
    SelectClusterRoleBinding {
        name: String,
    },
    DeleteClusterRoleBinding {
        name: String,
    },

    // config_configmaps
    RefreshConfigMaps,
    SelectConfigMap {
        namespace: String,
        name: String,
    },
    DeleteConfigMap {
        namespace: String,
        name: String,
    },

    // config_secrets
    RefreshSecrets,
    SelectSecret {
        namespace: String,
        name: String,
    },
    DeleteSecret {
        namespace: String,
        name: String,
    },

    // config_resourcequotas
    RefreshResourceQuotas,
    SelectResourceQuota {
        namespace: String,
        name: String,
    },
    DeleteResourceQuota {
        namespace: String,
        name: String,
    },

    // config_limitranges
    RefreshLimitRanges,
    SelectLimitRange {
        namespace: String,
        name: String,
    },
    DeleteLimitRange {
        namespace: String,
        name: String,
    },

    // config_horizontalpodautoscalers
    RefreshHorizontalPodAutoscalers,
    SelectHorizontalPodAutoscaler {
        namespace: String,
        name: String,
    },
    DeleteHorizontalPodAutoscaler {
        namespace: String,
        name: String,
    },

    // config_poddisruptionbudgets
    RefreshPodDisruptionBudgets,
    SelectPodDisruptionBudget {
        namespace: String,
        name: String,
    },
    DeletePodDisruptionBudget {
        namespace: String,
        name: String,
    },

    // config_priorityclasses
    RefreshPriorityClasses,
    SelectPriorityClass {
        name: String,
    },
    DeletePriorityClass {
        name: String,
    },

    // config_runtimeclasses
    RefreshRuntimeClasses,
    SelectRuntimeClass {
        name: String,
    },
    DeleteRuntimeClass {
        name: String,
    },

    // config_leases
    RefreshLeases,
    SelectLease {
        namespace: String,
        name: String,
    },
    DeleteLease {
        namespace: String,
        name: String,
    },

    // config_mutatingwebhooks
    RefreshMutatingWebhookConfigurations,
    SelectMutatingWebhookConfiguration {
        name: String,
    },
    DeleteMutatingWebhookConfiguration {
        name: String,
    },

    // config_validatingwebhooks
    RefreshValidatingWebhookConfigurations,
    SelectValidatingWebhookConfiguration {
        name: String,
    },
    DeleteValidatingWebhookConfiguration {
        name: String,
    },

    // UI preferences (~/.fathom/config.json); answered with `SystemEvent::UiConfigLoaded`.
    LoadUiConfig,
    SaveUiConfig {
        ui: UiConfig,
        expanded_groups: Vec<String>,
    },
    SetMcpAutoStart {
        enabled: bool,
    },
}

pub type BackendCmd = BackendCmdV1;

/// A command as it travels from a shell to the backend thread: the command plus the
/// bridge-assigned [`RequestId`]. Every envelope the backend emits while handling it
/// (including the terminal `BackendEventV1::CommandFinished`) carries
/// `Correlation::Request(id)`; with `id: None` they carry `Correlation::None`.
///
/// Not part of the wire protocol: the bridge returns the id from `send_cmd`.
#[derive(Debug, Clone)]
pub struct CmdRequest {
    pub id: Option<RequestId>,
    pub cmd: BackendCmd,
}

impl CmdRequest {
    pub fn new(id: RequestId, cmd: BackendCmd) -> Self {
        Self { id: Some(id), cmd }
    }
}

/// Uncorrelated (the legacy egui app, the MCP-only shell).
impl From<BackendCmd> for CmdRequest {
    fn from(cmd: BackendCmd) -> Self {
        Self { id: None, cmd }
    }
}
