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
    CordonNode {
        name: String,
    },
    UncordonNode {
        name: String,
    },
    DrainNode {
        name: String,
        ignore_daemonsets: bool,
        force_delete_empty_dir: bool,
    },
    CancelDrainNode {
        name: String,
        request_id: String,
    },
    TaintNode {
        name: String,
        op: TaintOp,
        key: String,
        value: Option<String>,
        effect: String,
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
    StartPodExec {
        namespace: String,
        name: String,
        container: Option<String>,
    },
    SendPodExecInput {
        data: String,
    },
    ResizePodExec {
        cols: u16,
        rows: u16,
    },
    StopPodExec,

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

    // Resource YAML edit + apply (any kind; identity fields validated pre-cluster-write).
    ApplyResourceYaml {
        api_version: String,
        kind: String,
        namespace: Option<String>,
        name: String,
        yaml: String,
        expected_resource_version: String,
    },

    // Custom Resource Definition browsing (generic, read-only)
    RefreshCrds,
    SelectCrdKind {
        group: String,
        kind: String,
        version: String,
    },
    RefreshCrdInstances {
        group: String,
        kind: String,
        version: String,
        namespace: Option<String>,
    },
    SelectCrdInstance {
        group: String,
        kind: String,
        version: String,
        namespace: Option<String>,
        name: String,
    },

    // Applications (synthesized, read-only)
    RefreshApplications,
    SelectApplication {
        namespace: String,
        instance: String,
    },

    // Generic related-resources resolver (read-only, any kind)
    RefreshRelatedResources {
        section: SectionKey,
        kind: String,
        namespace: Option<String>,
        name: String,
    },

    // Helm releases (read-only, reads release-storage Secret/ConfigMap objects directly)
    RefreshHelmReleases,
    SelectHelmRelease {
        namespace: String,
        name: String,
    },

    // Flux single-resource lookup (read-only; distinct from RefreshFlux's dashboard aggregate)
    SelectFluxResource {
        kind: String,
        namespace: String,
        name: String,
    },
}

/// Direction of a `TaintNode` mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum TaintOp {
    Add,
    Remove,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(cmd: BackendCmdV1) {
        let json = serde_json::to_string(&cmd).unwrap();
        let back: BackendCmdV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(format!("{back:?}"), format!("{cmd:?}"));
    }

    #[test]
    fn start_pod_exec_roundtrip() {
        roundtrip(BackendCmdV1::StartPodExec {
            namespace: "default".to_owned(),
            name: "web-0".to_owned(),
            container: Some("app".to_owned()),
        });
        roundtrip(BackendCmdV1::StartPodExec {
            namespace: "default".to_owned(),
            name: "web-0".to_owned(),
            container: None,
        });
    }

    #[test]
    fn send_pod_exec_input_roundtrip() {
        roundtrip(BackendCmdV1::SendPodExecInput {
            data: "bHM=".to_owned(),
        });
    }

    #[test]
    fn resize_pod_exec_roundtrip() {
        roundtrip(BackendCmdV1::ResizePodExec { cols: 80, rows: 24 });
    }

    #[test]
    fn stop_pod_exec_roundtrip() {
        roundtrip(BackendCmdV1::StopPodExec);
    }

    #[test]
    fn apply_resource_yaml_roundtrip() {
        roundtrip(BackendCmdV1::ApplyResourceYaml {
            api_version: "apps/v1".to_owned(),
            kind: "Deployment".to_owned(),
            namespace: Some("default".to_owned()),
            name: "web".to_owned(),
            yaml: "spec:\n  replicas: 3\n".to_owned(),
            expected_resource_version: "12345".to_owned(),
        });
        roundtrip(BackendCmdV1::ApplyResourceYaml {
            api_version: "v1".to_owned(),
            kind: "PersistentVolume".to_owned(),
            namespace: None,
            name: "pv-1".to_owned(),
            yaml: "spec: {}\n".to_owned(),
            expected_resource_version: "1".to_owned(),
        });
    }

    #[test]
    fn cordon_uncordon_node_roundtrip() {
        roundtrip(BackendCmdV1::CordonNode {
            name: "node-1".to_owned(),
        });
        roundtrip(BackendCmdV1::UncordonNode {
            name: "node-1".to_owned(),
        });
    }

    #[test]
    fn drain_node_roundtrip() {
        roundtrip(BackendCmdV1::DrainNode {
            name: "node-1".to_owned(),
            ignore_daemonsets: true,
            force_delete_empty_dir: false,
        });
    }

    #[test]
    fn cancel_drain_node_roundtrip() {
        roundtrip(BackendCmdV1::CancelDrainNode {
            name: "node-1".to_owned(),
            request_id: "req-1".to_owned(),
        });
    }

    #[test]
    fn taint_node_roundtrip() {
        roundtrip(BackendCmdV1::TaintNode {
            name: "node-1".to_owned(),
            op: TaintOp::Add,
            key: "dedicated".to_owned(),
            value: Some("gpu".to_owned()),
            effect: "NoSchedule".to_owned(),
        });
        roundtrip(BackendCmdV1::TaintNode {
            name: "node-1".to_owned(),
            op: TaintOp::Remove,
            key: "dedicated".to_owned(),
            value: None,
            effect: "NoSchedule".to_owned(),
        });
    }

    #[test]
    fn crd_commands_roundtrip() {
        roundtrip(BackendCmdV1::RefreshCrds);
        roundtrip(BackendCmdV1::SelectCrdKind {
            group: "source.toolkit.fluxcd.io".to_owned(),
            kind: "GitRepository".to_owned(),
            version: "v1".to_owned(),
        });
        roundtrip(BackendCmdV1::RefreshCrdInstances {
            group: "source.toolkit.fluxcd.io".to_owned(),
            kind: "GitRepository".to_owned(),
            version: "v1".to_owned(),
            namespace: Some("flux-system".to_owned()),
        });
        roundtrip(BackendCmdV1::SelectCrdInstance {
            group: "source.toolkit.fluxcd.io".to_owned(),
            kind: "GitRepository".to_owned(),
            version: "v1".to_owned(),
            namespace: Some("flux-system".to_owned()),
            name: "podinfo".to_owned(),
        });
    }

    #[test]
    fn application_commands_roundtrip() {
        roundtrip(BackendCmdV1::RefreshApplications);
        roundtrip(BackendCmdV1::SelectApplication {
            namespace: "default".to_owned(),
            instance: "redis".to_owned(),
        });
    }

    #[test]
    fn helm_release_commands_roundtrip() {
        roundtrip(BackendCmdV1::RefreshHelmReleases);
        roundtrip(BackendCmdV1::SelectHelmRelease {
            namespace: "cache".to_owned(),
            name: "redis".to_owned(),
        });
    }

    #[test]
    fn select_flux_resource_roundtrip() {
        roundtrip(BackendCmdV1::SelectFluxResource {
            kind: "HelmRelease".to_owned(),
            namespace: "flux-system".to_owned(),
            name: "redis".to_owned(),
        });
    }

    #[test]
    fn refresh_related_resources_roundtrip() {
        roundtrip(BackendCmdV1::RefreshRelatedResources {
            section: crate::protocol::section_keys::nodes(),
            kind: "Node".to_owned(),
            namespace: None,
            name: "node-1".to_owned(),
        });
        roundtrip(BackendCmdV1::RefreshRelatedResources {
            section: crate::protocol::section_keys::workloads_pods(),
            kind: "Pod".to_owned(),
            namespace: Some("default".to_owned()),
            name: "web-0".to_owned(),
        });
    }
}
