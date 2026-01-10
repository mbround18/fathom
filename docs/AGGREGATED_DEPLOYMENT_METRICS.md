# Aggregated Deployment Metrics with Per-Container Visualization

## Overview

Deployments now display aggregated resource metrics across all replicas with per-container breakdowns. This provides critical visibility for SRE operations - seeing realtime CPU, memory usage and limits across all containers in a deployment at a glance.

## Features

### Aggregated Metrics Display

When viewing a deployment, you'll see a comprehensive metrics panel showing:

- **Total CPU Usage**: Aggregated across all replicas and containers (with color-coded thresholds)
- **Total Memory Usage**: Aggregated bytes used vs limits (with percentage calculations)
- **Replicas Count**: How many replicas the deployment has
- **Resource Requests**: CPU/Memory requested across all containers
- **Resource Limits**: CPU/Memory limits configured across all containers

### Per-Container Breakdown

Below the aggregates, each container is shown individually:

```
📦 Per-Container Breakdown

┌─ container-1 ────────────────────┐
│ CPU:  45.2%                      │
│ Mem:  256.5 MB / 512.0 MB        │
└──────────────────────────────────┘

┌─ container-2 ────────────────────┐
│ CPU:  12.8%                      │
│ Mem:  128.3 MB / 256.0 MB        │
└──────────────────────────────────┘

┌─ container-3 ────────────────────┐
│ CPU:  67.9%                      │
│ Mem:  384.7 MB / 1024.0 MB       │
└──────────────────────────────────┘
```

### Color Coding (SRE-Friendly)

Metrics use standard threshold-based colors:

- **🔴 RED** (>80%): Critical - immediate attention needed
- **🟡 YELLOW** (>50%): Warning - trending high, monitor closely
- **🟢 GREEN** (≤50%): Healthy - normal operation

This helps spot resource contention immediately without reading exact numbers.

## UI Layout

```
┌─────────────────────────────────────────────────┐
│ Deployment: my-app / backend                    │
│ [Delete] [Restart] [Scale]  [Close]             │
├─────────────────────────────────────────────────┤
│ namespace / deployment-name                     │
│                                                 │
│ 📊 Replicas                                    │
│ Desired: 3  Ready: 3  Updated: 3  Available: 3 │
│                                                 │
│ 📊 Resource Metrics (Aggregated)               │
│ Replicas: 3                                     │
│ Total CPU: 45.2% (Red: critical)               │
│   Requested: 1500m                              │
│   Limited: 3000m                                │
│ Total Memory: 769.5 MB / 1792.0 MB (42%)       │
│   Requested: 1024 MB                            │
│                                                 │
│ 📦 Per-Container Breakdown                     │
│                                                 │
│ ┌─ web-server ──────────────────────┐          │
│ │ CPU:  45.2%                        │          │
│ │ Mem:  256.5 MB / 512.0 MB          │          │
│ └────────────────────────────────────┘          │
│                                                 │
│ ┌─ cache ────────────────────────────┐          │
│ │ CPU:  12.1%                        │          │
│ │ Mem:  256.0 MB / 768.0 MB          │          │
│ └────────────────────────────────────┘          │
│                                                 │
│ ┌─ metrics-exporter ─────────────────┐          │
│ │ CPU:  8.1%                         │          │
│ │ Mem:  257.0 MB / 512.0 MB          │          │
│ └────────────────────────────────────┘          │
│                                                 │
│ Last updated: 2025-01-01T12:00:15Z              │
│                                                 │
│ ℹ️ Metadata                                     │
│ ...                                             │
└─────────────────────────────────────────────────┘
```

## Data Structure

### WorkloadMetricsAggregate

```rust
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
```

### ContainerMetricsAggregate

```rust
pub struct ContainerMetricsAggregate {
    pub name: String,
    pub cpu_usage_percent: Option<f64>,
    pub memory_usage_bytes: Option<u64>,
    pub memory_limit_bytes: Option<u64>,
    pub memory_request_bytes: Option<u64>,
    pub cpu_request_millicores: Option<u64>,
    pub cpu_limit_millicores: Option<u64>,
}
```

## Implementation Details

### Event System Integration

New events added to `ResourceReadEvent`:

```rust
DeploymentMetricsUpdated(WorkloadMetricsAggregate),
DaemonSetMetricsUpdated(WorkloadMetricsAggregate),
StatefulSetMetricsUpdated(WorkloadMetricsAggregate),
```

This allows each workload type to emit aggregated metrics independently.

### UI Component (`render_workload_metrics`)

The rendering function:
1. Displays aggregate statistics at the top
2. Shows color-coded CPU/Memory bars
3. Lists each container with its metrics
4. Includes timestamp for last update

### State Management

`WorkloadViewState` extended with:
```rust
pub metrics: Option<WorkloadMetricsAggregate>,
```

Cleared when workload is deselected.

## Use Cases for SREs

### 1. Spot Resource Contention
See immediately if a deployment is over-utilizing CPU or memory across all replicas.

### 2. Container-Level Analysis
Find which specific container is consuming excessive resources.

```
If Total CPU is RED:
→ Check per-container breakdown
→ Identify problematic container
→ Correlate with application logs
→ Decide: optimize code, increase limits, or scale
```

### 3. Capacity Planning
Monitor trends:
- Is memory usage trending upward?
- Do we need to increase replica limits?
- Are we approaching node capacity?

### 4. Incident Investigation
During an outage:
- Pull up deployment metrics
- See resource usage at time of incident
- Correlate with application errors
- Determine root cause (resource exhaustion vs code issue)

## Next Steps (Future Enhancement)

### Time-Series Graphs
Add historical metrics display:
```
CPU Usage (last hour)
|        ╱╲
|      ╱  ╲
|    ╱      ╲
|  ╱          ╲
|╱────────────────
  0:00  6:00  12:00
```

### Per-Pod Breakdown
Show metrics for each individual pod (not just container aggregates):
```
Pod 1 (backend-abc123)
  - cpu-intensive: 45%
  - cache: 10%
  
Pod 2 (backend-def456)
  - cpu-intensive: 42%
  - cache: 11%
```

### Custom Thresholds
Allow users to configure alert thresholds per deployment.

### Metric Alerts
Automatic notifications when:
- CPU > 85% for 5 minutes
- Memory > 90% of limits
- Container restarts spike

## Related Features

- **Pod Metrics**: Individual pod metrics with Prometheus integration
- **Configurable Polling**: Change refresh interval (1-300 seconds)
- **Port-Forward Integration**: Automatic Prometheus service discovery
- **Color Coding**: Consistent red/yellow/green thresholds across all views

## Performance Notes

- Metrics are queried per deployment on-demand
- Aggregation happens in the backend (no UI calculations)
- Updates respect the configured polling interval (see CONFIGURABLE_METRICS_INTERVAL.md)
- Minimal network overhead: one aggregated query per deployment per interval
