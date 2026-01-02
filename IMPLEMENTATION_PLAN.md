# Fathom Implementation Plan

A comprehensive checklist of features and code cleanup tasks to complete the Fathom Kubernetes dashboard.

## 🏗️ Core Features to Implement

### Workloads - Pods
- [x] **Pod Details Display** - Fully render pod details panel with metadata, status, containers
- [ ] **Pod Logs Streaming** - Real-time log streaming from pod containers with pause/resume
- [x] **Pod Metrics** - Fetch and display CPU/memory metrics in details panel
- [x] **Pod Deletion** - Implement delete pod with confirmation modal
- [ ] **Container Selection** - Allow switching between pod containers for log viewing

### Workloads - Deployments
- [x] **Deployment Details Panel** - Show replicas, strategy, selector, conditions
- [ ] **Deployment Scaling** - Scale replica count with confirmation modal
- [x] **Deployment Restart** - Trigger rolling restart of deployment
- [ ] **Deployment Deletion** - Delete deployment with confirmation
- [ ] **Deployment Metrics** - Display CPU/memory across pods

### Workloads - DaemonSets
- [ ] **DaemonSet Details** - Display daemon set configuration and status
- [ ] **DaemonSet Restart** - Trigger DaemonSet rolling restart
- [ ] **DaemonSet Deletion** - Delete DaemonSet with confirmation
- [ ] **DaemonSet Metrics** - Show pod metrics across all nodes

### Workloads - StatefulSets
- [ ] **StatefulSet Details** - Show replicas, service name, persistent volumes
- [ ] **StatefulSet Scaling** - Scale replica count
- [ ] **StatefulSet Restart** - Restart StatefulSet with proper pod ordering
- [ ] **StatefulSet Deletion** - Delete with confirmation
- [ ] **StatefulSet Metrics** - Display metrics

### Workloads - ReplicaSets
- [ ] **ReplicaSet Details** - Show owner (Deployment/manual), selector, conditions
- [ ] **ReplicaSet Scaling** - Manual replica scaling
- [ ] **ReplicaSet Restart** - Trigger restart
- [ ] **ReplicaSet Deletion** - Delete with confirmation

### Workloads - ReplicationControllers
- [ ] **ReplicationController Details** - Legacy RC support
- [ ] **ReplicationController Scaling** - Scale replicas
- [ ] **ReplicationController Restart** - Restart pods
- [ ] **ReplicationController Deletion** - Delete with confirmation

### Workloads - Jobs & CronJobs
- [ ] **Job Details** - Show status, parallelism, completion info, conditions
- [ ] **Job Deletion** - Delete job with confirmation
- [ ] **CronJob Details** - Show schedule, last/next run, suspension status
- [ ] **CronJob Suspension** - Toggle suspension of CronJob
- [ ] **CronJob Manual Trigger** - Trigger immediate run of CronJob
- [ ] **CronJob Deletion** - Delete with confirmation

### Network - Port Forwards
- [ ] **Port Forward Creation** - Create local→remote port forward with UI form
- [ ] **Port Forward Listing** - List all active port forwards
- [ ] **Port Forward Termination** - Stop active port forward
- [ ] **Port Forward Persistence** - Save/restore across sessions
- [ ] **Port Forward Status Display** - Show connection status (connecting/active/error)

### Integrations - Metrics (Prometheus)
- [ ] **Metrics API Detection** - Probe and notify if metrics-server missing
- [ ] **Prometheus Configuration** - Save Prometheus namespace/service/port
- [ ] **Prometheus Auto Port-Forward** - Automatically setup port forward to Prometheus
- [ ] **Prometheus Query API** - Query time-series metrics for workloads
- [ ] **Metrics Visualization** - Render CPU/memory charts in details panels
- [ ] **Prometheus Connection Test** - Test connectivity to Prometheus endpoint
- [ ] **Custom Prometheus Queries** - Allow arbitrary PromQL queries via UI

### Integrations - Grafana
- [ ] **Grafana Configuration** - Save base URL and API key
- [ ] **Grafana Dashboards** - Link to Grafana dashboards from resource details
- [ ] **Grafana Connection Test** - Verify API connectivity
- [ ] **Grafana Dashboard URLs** - Generate links for workloads/pods

### Integrations - MCP (Model Context Protocol)
- [ ] **MCP Server Startup** - Start/stop MCP server from UI (✅ Done)
- [ ] **MCP Auto-Start** - Load auto-start config and launch on init (✅ Done)
- [ ] **MCP Tool Registration** - Register all available tools
- [ ] **MCP Status Queries** - get_status tool (MCP)
- [ ] **MCP Logs Queries** - get_logs tool (MCP)
- [ ] **MCP State Queries** - get_app_state tool (MCP)
- [ ] **MCP Health Check** - health_check tool (MCP)

### Search & Navigation
- [ ] **Pod Search** - Implement elastic/fuzzy search for pods
- [ ] **Resource Quick Navigation** - Search all resources by name
- [ ] **Namespace Filtering** - Filter views by selected namespaces
- [ ] **Label-Based Filtering** - Filter by resource labels
- [ ] **Owner Reference Navigation** - Click pod→ReplicaSet→Deployment chain

### Context & Configuration
- [ ] **Multi-Cluster Support** - Switch between kubeconfig contexts
- [ ] **RBAC Graceful Degradation** - Handle 403 Forbidden gracefully
- [ ] **Kubeconfig Reload** - Hot reload kubeconfig without restart
- [ ] **Context Switching** - Clean state reset when switching contexts
- [ ] **In-Cluster Detection** - Auto-detect and use in-cluster config

---

## 🧹 Code Cleanup & Clippy Fixes

### High Priority (Unused/Dead Code)
- [ ] **Shell Module** - `Shell` struct never constructed, decide if removing or implementing
- [ ] **Modal System** - `ModalState` variants (`Confirm`, `ScaleWorkload`, `CreatePortForward`) unused - wire up to UI
- [ ] **Search Functions** - `pod_matches_search`, `normalize_for_search`, etc. - implement fuzzy search or remove
- [ ] **ViewRegistry** - Registry system never used - either wire up or remove
- [ ] **WorkloadViewState** - Workload tracking structure - verify if needed

### Clippy Warnings to Fix
#### Refactoring
- [ ] **single_match** (14 occurrences) - Replace `match` with `if let` where applicable
- [ ] **collapsible_if** (12 occurrences) - Combine nested if statements using `&&`
- [ ] **while_let_loop** - Change `loop { let cmd = match receiver.recv()` to `while let Ok(cmd) = receiver.recv()`
- [ ] **module_inception** - Remove `pub mod shell` from `shell/mod.rs` (rename file or restructure)
- [ ] **ptr_arg** (6 occurrences) - Change `&Vec<T>` parameters to `&[T]` slices
- [ ] **clone_on_copy** - Remove `.clone()` calls on `Copy` types (e.g., `WorkloadKind`)
- [ ] **map_clone** - Use `.cloned()` instead of `.map(|x| x.clone())`
- [ ] **bind_instead_of_map** - Change `.and_then(|x| Some(y))` to `.map(|x| y)`

#### Code Quality
- [ ] **format_in_format_args** - Simplify nested `format!` calls in `search.rs`
- [ ] **collapsible_else_if** - Combine `} else { if` blocks in `mcp_integration.rs`
- [ ] **or_insert_with** - Change `.or_insert_with(Vec::new)` to `.or_default()`
- [ ] **large_enum_variant** - Box large `ResourceReadEvent` in `BackendEventV1` enum
- [ ] **enum_variant_names** - Remove redundant `Updated` suffix from event variants

#### Derivable Implementations
- [ ] **AutoStartConfig Default** - Add `#[derive(Default)]` instead of manual impl
- [ ] **Correlation Default** - Add `#[derive(Default)]` to `Correlation::None`

---

## 📊 State & Data Structure Improvements

- [ ] **Pod Metrics History** - Track metrics over time, not just current
- [ ] **Workload Annotations** - Store fathom/interactions audit log in annotations
- [ ] **Resource Caching** - Implement smarter caching strategy with TTL
- [ ] **Watch Stream Management** - Properly manage Kubernetes watch streams lifecycle
- [ ] **Error Recovery** - Implement retry logic for transient API failures

---

## 🖥️ UI/UX Polish

- [ ] **Loading States** - Show "Loading..." for all async operations
- [ ] **Error Display** - Toast notifications for actionable errors
- [ ] **Confirmation Modals** - Wire up delete/restart confirmation flows
- [ ] **Details Panel Tabs** - Organize pod details into tabs (info/logs/metrics/events)
- [ ] **Responsive Layout** - Better spacing and layout for various screen sizes
- [ ] **Keyboard Shortcuts** - Add vim keys or common shortcuts for navigation
- [ ] **Status Bar** - Implement connection status, cluster name, active namespace count

---

## 🔌 Backend Async Task Management

- [ ] **Pod Watcher** - Background watch of pods with smart reconciliation
- [ ] **Namespace Sync** - Periodic sync of available namespaces
- [ ] **Metrics Polling** - Background polling of Prometheus for selected resources
- [ ] **Log Streaming** - Efficient streaming with backpressure handling
- [ ] **Port Forward Tunnels** - Manage socat/kubectl port-forward processes
- [ ] **Task Lifecycle** - Proper cleanup of all background tasks on disconnect/context switch

---

## 🧪 Testing & Documentation

- [ ] **Unit Tests** - Add tests for utility functions (search, filtering, formatting)
- [ ] **Integration Tests** - Mock Kubernetes API for backend handlers
- [ ] **UI Snapshot Tests** - Egui rendering validation
- [ ] **RBAC Edge Cases** - Test with restricted permissions
- [ ] **Documentation** - Architecture docs, API reference, user guide
- [ ] **Example Clusters** - Kind/minikube example setup guides

---

## 🔐 Security & Data Handling

- [ ] **Kubeconfig Security** - Never log token values or certificates
- [ ] **Annotation Data** - Implement ring buffer for interaction logs (max 50 items)
- [ ] **RBAC Graceful Handling** - No panics on 403 Forbidden responses
- [ ] **Secret Masking** - Hide sensitive values in logs (API keys, tokens)
- [ ] **Input Validation** - Sanitize all user input before Kubernetes API calls

---

## 🚀 Performance Optimizations

- [ ] **Bounded Buffers** - Ring buffers for logs, metrics, status messages
- [ ] **Efficient Watching** - Use Kubernetes watch API instead of polling
- [ ] **Memory Management** - Proper cleanup of large data structures
- [ ] **Lazy Loading** - Load details only when user selects resource
- [ ] **Batch Operations** - Group API calls where possible
- [ ] **Smart Caching** - Cache resource lists with generation-based invalidation

---

## 📋 Legend

- ✅ Done
- 🚧 In Progress  
- ⬜ Not Started

---

## Priority Tiers

### Tier 1 (Core functionality)
1. Pod details & logs
2. Deployment scaling/restart/delete
3. Modal confirmation flows
4. Port forwards basic

### Tier 2 (Enhanced UX)
1. All workload types (DaemonSets, StatefulSets, etc.)
2. Metrics integration
3. Search & filtering
4. Error handling polish

### Tier 3 (Advanced)
1. Custom Prometheus queries
2. Grafana integration
3. MCP tools
4. Multi-cluster support

---

Generated from clippy output and codebase analysis.
Last updated: 2026-01-02
