# Setting Up Prometheus for Fathom

This guide shows how to set up Prometheus in your Kubernetes cluster to work with Fathom.

## Prerequisites

- Kubernetes 1.20+
- `kubectl` configured
- Helm 3+ (recommended)

## Option 1: Prometheus Operator (Recommended)

### Install Prometheus Stack via Helm

```bash
# Add Prometheus Helm repo
helm repo add prometheus-community https://prometheus-community.github.io/helm-charts
helm repo update

# Install kube-prometheus-stack
helm install prometheus prometheus-community/kube-prometheus-stack \
  --namespace monitoring --create-namespace \
  --values values.yaml
```

### Minimal values.yaml

```yaml
prometheus:
  prometheusSpec:
    retention: 7d
    storageSpec:
      volumeClaimTemplate:
        spec:
          storageClassName: default
          accessModes: ["ReadWriteOnce"]
          resources:
            requests:
              storage: 50Gi

grafana:
  enabled: true
  adminPassword: "your-password"

alertmanager:
  enabled: true
```

### Verify Installation

```bash
# Check if Prometheus pods are running
kubectl get pods -n monitoring

# Access Prometheus UI (port-forward)
kubectl port-forward -n monitoring svc/prometheus-operated 9090:9090
# Visit: http://localhost:9090

# List available metrics
curl http://localhost:9090/api/v1/labels
```

## Option 2: Manual Prometheus Setup

### 1. Create Monitoring Namespace

```bash
kubectl create namespace monitoring
```

### 2. Create ConfigMap for Prometheus Config

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: prometheus-config
  namespace: monitoring
data:
  prometheus.yml: |
    global:
      scrape_interval: 15s
      evaluation_interval: 15s
    scrape_configs:
      - job_name: 'kubernetes-apiservers'
        kubernetes_sd_configs:
          - role: endpoints
        scheme: https
        tls_config:
          ca_file: /var/run/secrets/kubernetes.io/serviceaccount/ca.crt
        bearer_token_file: /var/run/secrets/kubernetes.io/serviceaccount/token
        relabel_configs:
          - source_labels: [__meta_kubernetes_namespace, __meta_kubernetes_service_name]
            action: keep
            regex: default;kubernetes
      
      - job_name: 'kubernetes-nodes'
        kubernetes_sd_configs:
          - role: node
        scheme: https
        tls_config:
          ca_file: /var/run/secrets/kubernetes.io/serviceaccount/ca.crt
        bearer_token_file: /var/run/secrets/kubernetes.io/serviceaccount/token
        relabel_configs:
          - action: labelmap
            regex: __meta_kubernetes_node_label_(.+)
      
      - job_name: 'kubernetes-pods'
        kubernetes_sd_configs:
          - role: pod
        relabel_configs:
          - source_labels: [__meta_kubernetes_pod_annotation_prometheus_io_scrape]
            action: keep
            regex: 'true'
          - source_labels: [__meta_kubernetes_pod_annotation_prometheus_io_path]
            action: replace
            target_label: __metrics_path__
            regex: (.+)
          - source_labels: [__address__, __meta_kubernetes_pod_annotation_prometheus_io_port]
            action: replace
            regex: ([^:]+)(?::\d+)?;(\d+)
            replacement: $1:$2
            target_label: __address__
          - action: labelmap
            regex: __meta_kubernetes_pod_label_(.+)
          - source_labels: [__meta_kubernetes_namespace]
            action: replace
            target_label: kubernetes_namespace
          - source_labels: [__meta_kubernetes_pod_name]
            action: replace
            target_label: kubernetes_pod_name
```

### 3. Create Prometheus Deployment

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: prometheus
  namespace: monitoring
spec:
  replicas: 1
  selector:
    matchLabels:
      app: prometheus
  template:
    metadata:
      labels:
        app: prometheus
    spec:
      serviceAccountName: prometheus
      containers:
      - name: prometheus
        image: prom/prometheus:v2.40.0
        args:
          - '--config.file=/etc/prometheus/prometheus.yml'
          - '--storage.tsdb.path=/prometheus'
          - '--storage.tsdb.retention.time=7d'
        ports:
        - containerPort: 9090
          name: web
        volumeMounts:
        - name: config-volume
          mountPath: /etc/prometheus
        - name: storage-volume
          mountPath: /prometheus
        resources:
          requests:
            cpu: 500m
            memory: 512Mi
          limits:
            cpu: 2000m
            memory: 2Gi
      volumes:
      - name: config-volume
        configMap:
          name: prometheus-config
      - name: storage-volume
        emptyDir: {}
---
apiVersion: v1
kind: Service
metadata:
  name: prometheus
  namespace: monitoring
spec:
  selector:
    app: prometheus
  ports:
  - port: 9090
    targetPort: web
  type: ClusterIP
---
apiVersion: v1
kind: ServiceAccount
metadata:
  name: prometheus
  namespace: monitoring
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: prometheus
rules:
- apiGroups: [""]
  resources:
  - nodes
  - nodes/metrics
  - services
  - endpoints
  - pods
  verbs: ["get", "list", "watch"]
- apiGroups: ["extensions"]
  resources:
  - ingresses
  verbs: ["get", "list", "watch"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: prometheus
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: prometheus
subjects:
- kind: ServiceAccount
  name: prometheus
  namespace: monitoring
```

### 4. Deploy

```bash
kubectl apply -f prometheus-config.yaml
kubectl apply -f prometheus-deployment.yaml
```

## Connecting Fathom to Prometheus

### 1. Find the Prometheus Service

```bash
kubectl get svc -n monitoring
# Output should show prometheus-operated or similar
```

### 2. Configure in Fathom

Via UI settings or config file:

```json
{
  "metrics": {
    "providers": [
      {
        "name": "production-prometheus",
        "type": "prometheus",
        "namespace": "monitoring",
        "service": "prometheus-operated",
        "port": 9090
      }
    ]
  }
}
```

### 3. Port Forward

Fathom will automatically port-forward to Prometheus:

```bash
kubectl port-forward -n monitoring svc/prometheus-operated 9090:9090
```

Or manually:

```bash
kubectl port-forward -n monitoring svc/prometheus-operated 9090:9090 &
# Fathom connects to http://localhost:9090
```

## Verifying Metrics Collection

### Check if metrics are being scraped

```bash
# Port-forward to Prometheus
kubectl port-forward -n monitoring svc/prometheus-operated 9090:9090 &

# Query the targets endpoint
curl http://localhost:9090/api/v1/targets | jq '.data.activeTargets | length'
```

### Check available metrics

```bash
# Get all metric names
curl http://localhost:9090/api/v1/labels | jq '.data | .[0:10]'

# Query a specific metric
curl 'http://localhost:9090/api/v1/query?query=container_cpu_usage_seconds_total'
```

## Common Issues

### Prometheus not scraping kubelet metrics

**Error:** `No metrics available for pods`

**Solution:**
```bash
# Add kubelet scrape config
kubectl edit configmap prometheus-config -n monitoring

# Add to scrape_configs:
- job_name: 'kubelet'
  scheme: https
  tls_config:
    ca_file: /var/run/secrets/kubernetes.io/serviceaccount/ca.crt
  bearer_token_file: /var/run/secrets/kubernetes.io/serviceaccount/token
  kubernetes_sd_configs:
  - role: node
  relabel_configs:
  - action: labelmap
    regex: __meta_kubernetes_node_label_(.+)
```

### Port-forward connection refused

**Error:** `Connection refused when connecting to Prometheus`

**Solution:**
1. Verify Prometheus pod is running: `kubectl get pods -n monitoring`
2. Check service exists: `kubectl get svc -n monitoring | grep prometheus`
3. Restart port-forward: `kubectl port-forward -n monitoring svc/prometheus-operated 9090:9090`

### Metrics stale or missing

**Error:** `No recent data points`

**Solution:**
- Check scrape_interval in ConfigMap (default 15s)
- Verify targets are up: `http://localhost:9090/targets`
- Check retention time: `http://localhost:9090/api/v1/status/runtimeinfo`

## Next Steps

1. **Configure Service Discovery** - Set up ServiceMonitor CRDs if using Prometheus Operator
2. **Enable Alertmanager** - Add alerting rules for important metrics
3. **Set Up Grafana** - Create dashboards for visualization
4. **Configure RBAC** - Restrict Prometheus access appropriately
5. **Add Recording Rules** - Pre-aggregate complex queries for performance
