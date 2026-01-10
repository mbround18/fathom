/// Protocol types for metrics queries and responses
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A metrics provider configuration (for discovering and connecting to monitoring systems)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsProvider {
    pub name: String,
    pub provider_type: MetricsProviderType,
    pub url: String,
    pub namespace: Option<String>,
    pub service: Option<String>,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MetricsProviderType {
    Prometheus,
    // Future: Thanos, Grafana Cloud, etc.
}

/// An instant metric value (single point in time)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricValue {
    pub timestamp: u64,
    pub value: String, // Prometheus returns as string to preserve precision
}

/// A single metric sample with labels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSample {
    pub metric: BTreeMap<String, String>,
    pub value: MetricValue,
}

/// Range metric data (time series over a period)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RangeMetricSample {
    pub metric: BTreeMap<String, String>,
    pub values: Vec<MetricValue>,
}

/// Response from an instant query
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstantQueryResponse {
    pub result_type: String,
    pub result: Vec<MetricSample>,
}

/// Response from a range query
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RangeQueryResponse {
    pub result_type: String,
    pub result: Vec<RangeMetricSample>,
}

/// CPU metrics for a resource (pod, node, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuMetrics {
    pub usage_percent: Option<f64>,
    pub cores: Option<f64>,
}

/// Memory metrics for a resource
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryMetrics {
    pub usage_bytes: Option<u64>,
    pub usage_percent: Option<f64>,
    pub limit_bytes: Option<u64>,
}

/// Network I/O metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkMetrics {
    pub bytes_in: Option<u64>,
    pub bytes_out: Option<u64>,
    pub packets_in: Option<u64>,
    pub packets_out: Option<u64>,
}

/// Complete metrics summary for a pod (from Prometheus)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrometheusMetrics {
    pub namespace: String,
    pub name: String,
    pub cpu: CpuMetrics,
    pub memory: MemoryMetrics,
    pub network: NetworkMetrics,
    pub timestamp: u64,
}

/// Complete metrics summary for a node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMetricsSummary {
    pub name: String,
    pub cpu: CpuMetrics,
    pub memory: MemoryMetrics,
    pub disk: Option<DiskMetrics>,
    pub timestamp: u64,
}

/// Disk I/O metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskMetrics {
    pub read_bytes: Option<u64>,
    pub write_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
}

/// Container-level aggregated metrics (for workload aggregation)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerMetricsAggregate {
    pub name: String,
    pub cpu_usage_percent: Option<f64>,
    pub memory_usage_bytes: Option<u64>,
    pub memory_limit_bytes: Option<u64>,
    pub memory_request_bytes: Option<u64>,
    pub cpu_request_millicores: Option<u64>,
    pub cpu_limit_millicores: Option<u64>,
}

/// Aggregated metrics across all containers in a workload (deployment, statefulset, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadMetricsAggregate {
    pub namespace: String,
    pub name: String,
    pub workload_kind: String,
    pub replica_count: u32,
    // Aggregated totals
    pub total_cpu_usage_percent: Option<f64>,
    pub total_memory_usage_bytes: Option<u64>,
    pub total_memory_limit_bytes: Option<u64>,
    pub total_memory_request_bytes: Option<u64>,
    pub total_cpu_request_millicores: Option<u64>,
    pub total_cpu_limit_millicores: Option<u64>,
    // Per-container breakdown
    pub containers: Vec<ContainerMetricsAggregate>,
    pub timestamp: u64,
}

/// Standard PromQL queries for common metrics
pub mod queries {
    /// CPU usage as percentage (Prometheus: container_cpu_usage_seconds_total rate)
    pub const POD_CPU_USAGE_PERCENT: &str = "rate(container_cpu_usage_seconds_total{pod=\"{pod}\",namespace=\"{namespace}\"}[5m]) * 100";

    /// Memory usage in bytes (Prometheus: container_memory_usage_bytes)
    pub const POD_MEMORY_USAGE_BYTES: &str =
        "container_memory_usage_bytes{pod=\"{pod}\",namespace=\"{namespace}\"}";

    /// Memory usage as percentage (Prometheus: container_memory_limit_bytes)
    pub const POD_MEMORY_LIMIT_BYTES: &str =
        "container_spec_memory_limit_bytes{pod=\"{pod}\",namespace=\"{namespace}\"}";

    /// Network bytes received
    pub const POD_NETWORK_RX_BYTES: &str = "increase(container_network_receive_bytes_total{pod=\"{pod}\",namespace=\"{namespace}\"}[5m])";

    /// Network bytes sent
    pub const POD_NETWORK_TX_BYTES: &str = "increase(container_network_transmit_bytes_total{pod=\"{pod}\",namespace=\"{namespace}\"}[5m])";

    /// Node CPU usage percentage
    pub const NODE_CPU_USAGE_PERCENT: &str =
        "(1 - avg(rate(node_cpu_seconds_total{mode=\"idle\",instance=\"{node}\"}[5m]))) * 100";

    /// Node memory usage bytes
    pub const NODE_MEMORY_USAGE_BYTES: &str = "node_memory_MemTotal_bytes{instance=\"{node}\"} - node_memory_MemAvailable_bytes{instance=\"{node}\"}";

    /// Node memory limit (total) bytes
    pub const NODE_MEMORY_TOTAL_BYTES: &str = "node_memory_MemTotal_bytes{instance=\"{node}\"}";
}
