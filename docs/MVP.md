Below is a **developer-oriented feature criteria and specification set** derived from the provided screenshots and UI flows. This is written as if it were the internal product spec for building a **custom Kubernetes dashboard** for a power user (you), with placeholders where concrete implementations will vary.

The goal is **parity with serious tools (Rancher / Lens / OpenShift Console)** while remaining opinionated, fast, and composable.

---

# Custom Kubernetes Dashboard – Feature Criteria & Specification

## 1. Product Intent & Philosophy

### Primary Goals

* Single-pane, operator-first Kubernetes observability and control
* Fast navigation across **cluster → application → workload → resource**
* Tight integration with Prometheus, CRDs, and GitOps tooling
* Explicit state visibility over abstraction

### Non-Goals

* Beginner onboarding
* Wizard-driven resource creation
* Opinionated cluster mutation without visibility

---

## 2. Global UI / UX Design Criteria

### Visual Style

* **Dark-first theme**, high contrast
* Low saturation, neutral grays with semantic color accents
* Minimal gradients, sharp edges
* Dense information layout (no excessive padding)

### Typography

* Sans-serif, monospace accents for identifiers
* Hierarchical sizing:

  * Page titles
  * Section headers
  * Metadata labels
* Fixed-width font for:

  * Resource names
  * Node IDs
  * Pod hashes
  * Metric legends

### Layout Principles

* **Left-hand navigation tree** (persistent)
* **Main content pane**
* **Right-hand inspector drawer** (contextual details)
* Zero full-page navigation reloads

---

## 3. Navigation & Information Architecture

### Global Navigation Tree

#### Cluster Root

* Overview
* Applications
* Nodes
* Workloads
* Config
* Network
* Storage
* Namespaces
* Helm
* Access Control
* Custom Resources
* Events
* Settings

### Design Rules

* Expandable / collapsible sections
* Persistent selection highlight
* Searchable lists within each section
* Namespace selector is **global** and filter-based

---

## 4. Cluster Overview (Dashboard)

### Required Panels

#### Resource Summary (Donut / Ring Charts)

* CPU

  * Usage
  * Requests
  * Limits
  * Allocatable
  * Capacity
* Memory

  * Same breakdown as CPU
* Pods

  * Current usage
  * Allocatable limit
  * Hard capacity

#### Time-Series Utilization

* CPU usage over time
* Memory usage over time
* Adjustable time range (e.g. 1h, 6h, 24h)

#### Warnings / Events Panel

* Aggregated warning count
* Recent critical events
* Linkable to affected objects

### Data Expectations

* All values derived from Prometheus
* Graceful degradation if metrics unavailable
* Clear labeling of **usage vs requested vs allocatable**

---

## 5. Applications View

### Application List

* Aggregated logical apps (Helm, GitOps, operators)
* Columns:

  * Instance
  * Application
  * Namespace
  * Managed By
  * Version
  * Age
  * Status

### Selection Behavior

* Selecting an application opens **right-hand inspector**
* No route change

### Application Inspector

#### Metrics

* CPU usage over time
* Memory usage (optional)
* Source label: Prometheus endpoint

#### Metadata

* Created timestamp
* Status
* Version
* Managed by (Helm / Flux / Operator)
* Internal URLs
* Exposed URLs

#### Related Resources

* Pods (with readiness, CPU, memory)
* Deployments / StatefulSets
* ServiceAccounts
* CRDs (if applicable)

---

## 6. Nodes View

### Node List

* Name
* CPU bar
* Memory bar
* Status indicator

### Node Inspector

#### Metrics

* CPU utilization over time
* Memory utilization over time

#### System Details

* OS
* Kernel version
* Container runtime
* Kubelet version

#### Networking

* Internal IP
* External IP
* Hostname

#### Capacity & Allocatable

* CPU
* Memory
* Ephemeral storage
* Pods

#### Conditions

* Ready / NotReady
* Taints
* Labels
* Annotations

---

## 7. Workloads

### Supported Kinds

* Pods
* Deployments
* DaemonSets
* StatefulSets
* ReplicaSets
* Jobs
* CronJobs

### List View

* Name
* Namespace
* Ready status
* Node
* CPU
* Memory
* Status

### Pod Inspector

#### Lifecycle

* Created
* Controlled by (Job / Deployment / etc)
* QoS class
* Priority class

#### Metrics

* Per-container CPU
* Per-container memory
* Time-range selector

#### Conditions

* PodScheduled
* ContainersReady
* Initialized
* Ready

#### Volumes

* HostPath
* PVCs
* Projected volumes

---

## 8. Metrics & Observability

### Metrics Source Configuration

* Automatic detection
* Manual override
* Prometheus Operator support
* Configurable service path

### Query Behavior

* Filter empty containers toggle
* PromQL abstraction layer
* Graceful fallback when metrics are missing

### UI Requirements

* Clear “data source” labeling
* Visible loading / unavailable states
* No silent failures

---

## 9. Custom Resources (CRDs)

### CRD Browser

* Grouped by API group
* Expandable definitions
* Resource instances list

### CRD Instance View

* YAML / JSON viewer
* Status vs Spec split
* Events attached to CR instance

### Supported Examples

* cert-manager
* istio
* flux
* external-secrets
* longhorn
* monitoring.coreos.com

---

## 10. Configuration & Settings

### Cluster Settings

* Metrics source
* Prometheus path
* HTTPS toggle
* Path prefix
* Hidden metrics

### Node Shell

* Configurable image
* Optional image pull secret
* On-demand ephemeral pod

### Preferences

* Default time range
* Namespace persistence
* UI density (compact / normal)

---

## 11. Integration Expectations

### Required Integrations

* Kubernetes API (watch + list)
* Prometheus
* CRDs
* Helm metadata
* GitOps annotations (Flux / Argo)

### Optional Integrations

* Alertmanager
* Tracing (future)
* Logs (future)

---

## 12. Non-Functional Requirements

### Performance

* Sub-200ms UI interactions
* Virtualized lists
* Aggressive caching of static metadata

### Reliability

* Partial failure tolerant
* Metrics optional, API required
* Explicit error surfacing

### Security

* RBAC-aware UI
* No privilege escalation
* Read-only by default

---

## 13. Developer Experience (Internal)

### Architecture Guidance

* Component-based UI
* Resource-type adapters
* Central metrics abstraction
* Typed API models

### Extensibility

* Pluggable resource renderers
* Feature flags
* CRD-driven UI sections

---

## 14. Success Criteria

* Operator can diagnose cluster state in <30 seconds
* All core Kubernetes objects discoverable within 2 clicks
* Metrics discrepancies are obvious, not hidden
* UI feels closer to an **engineering console** than a “dashboard”

