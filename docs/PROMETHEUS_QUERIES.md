# Prometheus Queries Reference

This document lists all standard PromQL queries used by Fathom for monitoring.

## Pod Metrics

### CPU Usage (percentage, 5-minute average)
```promql
rate(container_cpu_usage_seconds_total{pod="{pod}",namespace="{namespace}"}[5m]) * 100
```
**Units:** Percentage (0-100)
**Interpretation:** CPU utilization as a percentage

### Memory Usage (bytes)
```promql
container_memory_usage_bytes{pod="{pod}",namespace="{namespace}"}
```
**Units:** Bytes
**Interpretation:** Current memory consumption

### Memory Limit (bytes)
```promql
container_spec_memory_limit_bytes{pod="{pod}",namespace="{namespace}"}
```
**Units:** Bytes
**Interpretation:** Maximum memory allowed (cgroup limit)

### Memory Usage Percentage
Calculated as: `usage_bytes / limit_bytes * 100`

### Network Bytes Received (5-minute increase)
```promql
increase(container_network_receive_bytes_total{pod="{pod}",namespace="{namespace}"}[5m])
```
**Units:** Bytes
**Interpretation:** Data received in the last 5 minutes

### Network Bytes Transmitted (5-minute increase)
```promql
increase(container_network_transmit_bytes_total{pod="{pod}",namespace="{namespace}"}[5m])
```
**Units:** Bytes
**Interpretation:** Data sent in the last 5 minutes

## Node Metrics

### CPU Usage (percentage, 5-minute average)
```promql
(1 - avg(rate(node_cpu_seconds_total{mode="idle",instance="{node}"}[5m]))) * 100
```
**Units:** Percentage (0-100)
**Interpretation:** CPU utilization across all cores

### Memory Usage (bytes)
```promql
node_memory_MemTotal_bytes{instance="{node}"} - node_memory_MemAvailable_bytes{instance="{node}"}
```
**Units:** Bytes
**Interpretation:** Used memory (MemTotal - MemAvailable)

### Memory Total (bytes)
```promql
node_memory_MemTotal_bytes{instance="{node}"}
```
**Units:** Bytes
**Interpretation:** Total system memory

### Disk Available (bytes)
```promql
node_filesystem_avail_bytes{instance="{node}",fstype!="tmpfs"}
```
**Units:** Bytes
**Interpretation:** Disk space available

### Disk Total (bytes)
```promql
node_filesystem_size_bytes{instance="{node}",fstype!="tmpfs"}
```
**Units:** Bytes
**Interpretation:** Total disk space

## Cluster-Wide Metrics

### Pod Count by Namespace
```promql
count(kube_pod_info) by (namespace)
```
**Interpretation:** Number of pods per namespace

### Node Status
```promql
kube_node_status_condition{condition="Ready",status="true"}
```
**Interpretation:** Healthy nodes (1 = healthy, 0 = unhealthy)

### Container Restart Count
```promql
kube_pod_container_status_restarts_total{pod="{pod}",namespace="{namespace}"}
```
**Interpretation:** Number of times container has restarted

## Substitution Variables

When using these queries, replace:
- `{pod}` - Pod name
- `{namespace}` - Kubernetes namespace
- `{node}` - Node instance label (usually hostname or IP:port)

Example:
```promql
container_cpu_usage_seconds_total{pod="nginx-12345",namespace="default"}
```

## Common Issues

### Metrics Not Available
- **Cause:** Prometheus not scraping container/node metrics
- **Solution:** Verify prometheus ServiceMonitor or scrape config
- **Alternative:** Check if kubelet metrics endpoint is accessible

### No Data Points
- **Cause:** Container/pod doesn't exist yet or is terminating
- **Solution:** Wait for pod to stabilize, retry query
- **Error Handling:** Show "N/A" in UI, don't fail the entire details view

### High Cardinality
- **Cause:** Querying too many series at once
- **Solution:** Always filter by namespace and pod/node name
- **Performance:** Use metric relabeling to reduce cardinality

## Advanced Queries

### Container Memory Pressure
```promql
container_memory_working_set_bytes / container_spec_memory_limit_bytes > 0.9
```

### CPU Throttling
```promql
increase(container_cpu_cfs_throttled_seconds_total[5m]) > 0
```

### Network Errors
```promql
increase(container_network_receive_errs_total[5m]) > 0
or
increase(container_network_transmit_errs_total[5m]) > 0
```

## Query Performance Tips

1. **Always filter by namespace:** Dramatically reduces query time
2. **Use specific time ranges:** Avoid querying unbounded time periods
3. **Cache results:** Store metrics for 30-60 seconds to reduce API calls
4. **Use rate() for counters:** Never query raw counter values directly
5. **Aggregate at scrape time:** Use Prometheus recording rules for complex aggregations

## References

- [Prometheus Query Language Docs](https://prometheus.io/docs/prometheus/latest/querying/basics/)
- [Kubernetes Metrics](https://kubernetes.io/docs/tasks/debug-application-cluster/resource-metrics-pipeline/)
- [cAdvisor Metrics](https://github.com/google/cadvisor/blob/master/docs/storage/prometheus.md)
