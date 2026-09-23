use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PodSummary {
    pub name: String,
    pub namespace: String,
    pub phase: String,
    pub node: String,
    pub ready: u32,
    pub total: u32,
    pub restarts: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PodOwner {
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ContainerResourceSpec {
    pub container: String,
    pub cpu_request_millicores: Option<f64>,
    pub cpu_limit_millicores: Option<f64>,
    pub memory_request_bytes: Option<f64>,
    pub memory_limit_bytes: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PodDetails {
    pub namespace: String,
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub containers: Vec<String>,
    pub ports: Vec<ContainerPort>,
    pub resource_specs: Vec<ContainerResourceSpec>,

    /// Best-effort controller owner (from metadata.ownerReferences).
    pub owner: Option<PodOwner>,
    /// Best-effort resolved owner (e.g. ReplicaSet -> Deployment).
    pub resolved_owner: Option<PodOwner>,

    /// Names of PersistentVolumeClaims referenced by this Pod (best-effort)
    pub persistent_volume_claims: Vec<String>,

    pub raw_json: String,

    /// 008-pod-lifecycle-diagnostics: init + app container lifecycle detail (status, restarts,
    /// probes, hooks). Additive; does not replace `containers` above.
    pub containers_lifecycle: Vec<ContainerLifecycle>,
}

/// 008-pod-lifecycle-diagnostics: current or last-observed state of a container.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ContainerState {
    /// `"waiting"` | `"running"` | `"terminated"`.
    pub phase: String,
    pub reason: Option<String>,
    pub exit_code: Option<i32>,
    /// RFC3339.
    pub started_at: Option<String>,
    /// RFC3339, terminated only.
    pub finished_at: Option<String>,
}

/// 008-pod-lifecycle-diagnostics: a `livenessProbe`/`readinessProbe`/`startupProbe` config.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ProbeSpec {
    /// `"liveness"` | `"readiness"` | `"startup"`.
    pub probe_type: String,
    /// `"httpGet"` | `"tcpSocket"` | `"exec"` | `"grpc"`.
    pub mechanism: String,
    /// Human-readable target, e.g. `GET /healthz:8080`, `tcp:8080`, `exec: cat /tmp/ready`.
    pub target: String,
    pub period_seconds: i32,
    pub failure_threshold: i32,
    pub success_threshold: i32,
}

/// 008-pod-lifecycle-diagnostics: a `postStart`/`preStop` lifecycle hook config.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LifecycleHookSpec {
    /// `"postStart"` | `"preStop"`.
    pub hook_type: String,
    /// `"exec"` | `"httpGet"` | `"sleep"`.
    pub mechanism: String,
    pub target: String,
}

/// 008-pod-lifecycle-diagnostics: one entry per container in `spec.initContainers` (is_init =
/// true) and `spec.containers` (is_init = false). Probe/hook *failure* correlation against the
/// Pod's Events is computed in the frontend (already has the Events via `RefreshEvents`), not
/// stored here — this is a pure snapshot of spec + status.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ContainerLifecycle {
    pub name: String,
    pub is_init: bool,
    pub image: String,
    pub ready: bool,
    pub state: ContainerState,
    pub restart_count: i32,
    pub last_state: Option<ContainerState>,
    pub probes: Vec<ProbeSpec>,
    pub hooks: Vec<LifecycleHookSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PodMetricsSummary {
    pub namespace: String,
    pub name: String,
    pub containers: Vec<ContainerMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ContainerMetrics {
    pub name: String,
    pub cpu: String,
    pub memory: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct MetricsSample {
    pub timestamp: String, // ISO8601
    pub cpu_millicores: f64,
    pub memory_bytes: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ContainerMetricsHistory {
    pub name: String,
    pub samples: Vec<MetricsSample>,
    pub cpu_request_millicores: Option<f64>,
    pub cpu_limit_millicores: Option<f64>,
    pub memory_request_bytes: Option<f64>,
    pub memory_limit_bytes: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PodMetricsHistory {
    pub namespace: String,
    pub name: String,
    pub containers: Vec<ContainerMetricsHistory>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ContainerPort {
    pub container: String,
    pub name: Option<String>,
    pub port: u16,
    pub protocol: String,
}
