# Metrics & Monitoring Integration

## Architecture Overview

Fathom now supports real-time metrics monitoring from Prometheus instances. The system is designed to:

1. **Discover** Prometheus services in the cluster (via Service lookups)
2. **Port Forward** to Prometheus instances for secure access
3. **Query** metrics using PromQL expressions
4. **Display** metrics in the UI alongside resource details

## Components

### Protocol Types (`src/protocol/v1/metrics.rs`)

- `MetricsProvider` - Configuration for connecting to monitoring systems
- `MetricsProviderType` - Enum supporting Prometheus, Thanos, etc. (Prometheus for now)
- `PrometheusMetrics` - Aggregated metrics for pods/nodes
- `CpuMetrics`, `MemoryMetrics`, `NetworkMetrics` - Component metric types
- Standard PromQL queries for common metrics

### Backend Module (`src/backend/metrics_provider.rs`)

- `PrometheusClient` - HTTP client for Prometheus API
- `MetricsProviderTrait` - Extensible trait for multiple providers
- `query_pod_metrics()` - Helper to fetch and aggregate metrics for a pod

## Usage Example

### Finding a Prometheus Service

```yaml
apiVersion: v1
kind: Service
metadata:
  name: prometheus-operated
  namespace: monitoring
spec:
  ports:
  - name: http-web
    port: 9090
    targetPort: http-web
  selector:
    app.kubernetes.io/name: prometheus
  clusterIP: None
  type: ClusterIP
```

### Connecting to Prometheus

1. User selects a Prometheus provider in UI
2. Backend initiates port-forward: `localhost:9090 → prometheus-operated:9090 (monitoring)`
3. Client connects to `http://localhost:9090/api/v1/`

### Querying Metrics

```rust
let client = PrometheusClient::new("http://localhost:9090".to_string());

// Query CPU usage for a pod
let metrics = query_pod_metrics(&client, "default", "my-pod").await?;

println!("CPU: {}%", metrics.cpu.usage_percent.unwrap_or(0.0));
println!("Memory: {} bytes", metrics.memory.usage_bytes.unwrap_or(0));
```

## PromQL Queries

### Pod Metrics

- **CPU Usage (%)**: `rate(container_cpu_usage_seconds_total[5m]) * 100`
- **Memory (bytes)**: `container_memory_usage_bytes`
- **Memory Limit**: `container_spec_memory_limit_bytes`
- **Network RX**: `increase(container_network_receive_bytes_total[5m])`
- **Network TX**: `increase(container_network_transmit_bytes_total[5m])`

### Node Metrics

- **CPU Usage (%)**: `(1 - avg(rate(node_cpu_seconds_total{mode="idle"}[5m]))) * 100`
- **Memory Usage**: `node_memory_MemTotal_bytes - node_memory_MemAvailable_bytes`
- **Memory Total**: `node_memory_MemTotal_bytes`

## Prometheus API Endpoints Used

- `/api/v1/query` - Instant queries (single point in time)
- `/api/v1/query_range` - Range queries (time series over period)
- `/api/v1/metadata` - Metric metadata discovery

## Next Steps

1. **Backend Handlers**: Add BackendCmd variants for metrics queries
2. **Port Forwarding**: Integrate with existing portforward module
3. **UI Display**: Show metrics in pod/node details panels with charts
4. **Multiple Providers**: Extend to support Thanos, Grafana Cloud, etc.
5. **Alert Integration**: Display active alerts from Prometheus

## Configuration

Store Prometheus endpoints in:
- `integrations.prometheus_url` - HTTP endpoint
- `integrations.prometheus_namespace` - K8s namespace
- `integrations.prometheus_service` - Service name
- `integrations.prometheus_port` - Port number

## Error Handling

- Network timeouts: Best-effort display of "metrics unavailable"
- Query failures: Log warning, show "-" in UI
- Service not found: Gracefully skip metrics section
- RBAC failures: Handle 403 responses appropriately

## Future Enhancements

- **Charts**: egui_plot integration for time series visualization
- **Alerts**: Display active Prometheus alerts
- **Custom Queries**: UI for custom PromQL expressions
- **Multi-cluster**: Query metrics across multiple Prometheus instances
- **Caching**: Cache metrics with TTL to reduce API calls
- **Real-time**: WebSocket streaming for live updates
