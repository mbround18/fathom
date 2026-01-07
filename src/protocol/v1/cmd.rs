use super::SectionKey;

#[derive(Debug, Clone)]
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
    RefreshPortForwards,

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
    // MCP Server integration
    StartMcpServer,
    StopMcpServer,
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
}

pub type BackendCmd = BackendCmdV1;
