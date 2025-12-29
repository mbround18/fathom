use super::SectionKey;

#[derive(Debug, Clone)]
pub enum BackendCmdV1 {
    // Global / shell-owned
    Initialize,
    RefreshContexts,
    SetContext {
        context: String,
    },
    RefreshNamespaces,
    SetNamespaceFilter {
        namespaces: Vec<String>,
    },

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

    // Keeps the original Day-1 “proof” button around.
    SimulateSlowCall,

    // Future hook: allow UI to hint active section for prefetching (optional).
    SetActiveSection {
        section: SectionKey,
    },
}

pub type BackendCmd = BackendCmdV1;
