Below is a **developer-oriented feature criteria and specification set** derived from the provided screenshots and UI flows. This is written as if it were the internal product spec for building a **custom Kubernetes dashboard** for a power user (you), with placeholders where concrete implementations will vary.

The goal is **parity with serious tools (Rancher / Lens / OpenShift Console)** while remaining opinionated, fast, and composable.

Status legend: ✅ Done · 🚧 Partial · ⬜ Not started

---

# Custom Kubernetes Dashboard – Feature Criteria & Specification

## Status Summary

| Area | Status | Shipped in |
|---|---|---|
| Cluster Overview dashboard | ✅ | 001-tauri-shadcn-frontend |
| Navigation tree (Nodes/Workloads/Config/Network/Storage/Namespaces/Events) | ✅ | 001-tauri-shadcn-frontend |
| Metrics & Observability (Prometheus) | ✅ | 001-tauri-shadcn-frontend |
| Pod logs streaming | ✅ | 002-pod-logs-terminal |
| Interactive pod exec / terminal | ✅ | 002-pod-logs-terminal |
| YAML edit + diff + apply (any resource) | ✅ | 003-yaml-node-crd-parity |
| Node actions (cordon/uncordon/drain/taint) | ✅ | 003-yaml-node-crd-parity |
| Custom Resources — generic CRD discovery/browsing | ✅ | 003-yaml-node-crd-parity |
| GitOps (Flux) integration panel | ✅ | promo-brochure-v2 work |
| Hubble/Cilium traffic panel | ✅ | promo-brochure-v2 work |
| **Applications view** | ⬜ | planned next |
| **Helm** — read (list/inspector/origin-link) | ✅ | 005-helm-release-origin-linking |
| **Helm** — mutate (install/upgrade/rollback) | ⬜ | planned next |
| **Rich interlinking / drill-down** (chart → pod, resource → resource) | ⬜ | planned next |
| **Topology graph view** (zoomable namespace→pod block-and-line diagram) | ⬜ | planned, later |
| Access Control (RBAC/OAuth/LDAP, role-scoped UI) | ⬜ | future — enterprise-auth tier |
| SDK login (GitHub/Claude/OpenAI) + local Ollama inference | ⬜ | future — live/SDK-auth tier |
| Node Shell (on-demand ephemeral debug pod, distinct from pod exec) | ⬜ | not yet planned |

---

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

*Status: ✅ Done — shipped in `001-tauri-shadcn-frontend` (neon-noir theme, three-pane shell).*

---

## 3. Navigation & Information Architecture

### Global Navigation Tree

#### Cluster Root

* Overview ✅
* Applications ⬜ *(see §5)*
* Nodes ✅
* Workloads ✅
* Config ✅
* Network ✅
* Storage ✅
* Namespaces ✅
* Helm ✅ read path *(see §5a)*
* Access Control ⬜ *(future tier — see §14)*
* Custom Resources ✅ *(generic, `003-yaml-node-crd-parity`)*
* Events ✅
* Settings ✅

### Design Rules

* Expandable / collapsible sections
* Persistent selection highlight
* Searchable lists within each section
* Namespace selector is **global** and filter-based

---

## 4. Cluster Overview (Dashboard)

*Status: ✅ Done — `001-tauri-shadcn-frontend`.*

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

*Status: ⬜ Not started — next planned feature.*

An "application" is a logical grouping of workloads under one Helm release, Flux/Argo GitOps unit, or operator-managed bundle — not a native Kubernetes kind, so this view is synthesized from ownership/label metadata already visible to the backend (`app.kubernetes.io/*` labels, Helm release secrets, Flux `Kustomization`/`HelmRelease` inventories already read by `src/backend/flux.rs`).

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

### Drill-down (new requirement)

This view is the primary entry point for **§5b rich interlinking**: selecting an application must let the operator drill straight down its ownership chain — Application → Deployment/StatefulSet → ReplicaSet → Pod → container — without manually re-navigating the resource tree or re-searching by name/label. See §5b for the shared mechanism this depends on.

---

## 5a. Helm

*Status: ✅ Read path shipped (`005-helm-release-origin-linking`) — list, inspector, and GitOps origin-context linking to Flux. Mutate (upgrade/rollback/uninstall) still not started.*

Fathom now reads Helm-managed resources directly from Helm's own release storage, not just indirectly via Flux inventories or ownership labels.

### Read (shipped)

* List installed Helm releases (name, namespace, chart, chart version, app version, revision, status, last updated) — via Helm's own release storage (Secrets/ConfigMaps in the release namespace, `helm.sh/release.v1` type), not a shelled-out `helm` binary
* Release inspector: values (effective + user-supplied), rendered manifest, revision history, resources owned by the release (feeds §5b drill-down)
* GitOps origin-context linking: a Flux-owned release's inspector shows a click-through link to its owning `HelmRelease`/`Kustomization`, surfacing that controller's live Ready/Failed/Reconciling/Suspended state and message — the operator's originally-requested "trace to origin" workflow

### Mutate (later, explicit non-goal for the first pass — mirrors constitution Principle I)

* Upgrade / rollback to a prior revision, with a diff-preview step analogous to `003`'s YAML-apply diff
* Uninstall, confirmation-gated

**Explicit non-goal for v1**: chart install/search from a repo, or a values-editor wizard — this is read/rollback parity first, not a package-manager UI.

---

## 5b. Rich Interlinking / Drill-down

*Status: ⬜ Not started — cross-cutting requirement, needed by §5 and §5a, and the foundation for §5c.*

**Problem today**: every resource kind is its own list + inspector island. Going from "this Application" or "this Helm release" to "the specific Pod that's crash-looping" means manually noticing an owner reference or label and re-navigating by hand.

### Requirement

* Every resource inspector that has a natural parent/child relationship (Application → Deployment → ReplicaSet → Pod → Container; Helm release → its owned resources; Node → the Pods scheduled on it; Service → the Pods it selects; PVC → the Pod(s) mounting it) surfaces those relationships as **clickable links**, not just text.
* Clicking a related-resource link opens that resource's inspector directly (same no-route-change pattern as today), preserving a **breadcrumb/back-stack** so the operator can retrace the drill-down path.
* Relationship resolution reuses data already fetched for the current resource's `ownerReferences`/label selectors where possible — no new backend round-trip if the data is already in hand; otherwise a targeted lookup (not a full list-and-filter).

### Backend implications

* A small generic "related resources" resolver, keyed by kind + ownerReferences/selector, that existing resource-read handlers can call rather than each kind hand-rolling its own relationship lookups.

---

## 5c. Topology Graph View

*Status: ⬜ Not started — later, depends on §5b's relationship data being modeled once, generically.*

A block-and-line topology diagram of the cluster, letting the operator zoom continuously between **namespace-level** and **pod-level** detail rather than only drilling through a list/inspector chain.

### Requirements

* Nodes in the graph are Kubernetes objects (Namespace, Application/Helm-release, Workload, Pod, Service, Node) or CRD instances; edges are the same ownership/selector/traffic relationships modeled in §5b (plus, later, real traffic edges from the Hubble integration already shipped).
* **Zoom levels**, not separate pages: zoomed out shows namespace-to-namespace and namespace-to-service traffic/ownership; zooming in progressively reveals workloads, then individual pods, without a full view reload.
* Clicking any node opens that resource's existing inspector (reuses §5b, doesn't duplicate it).
* Live-updating: node/edge state (health, replica counts) reflects the same streamed data the list views use — no separate polling path.
* Layout: automatic (force-directed or hierarchical) — no manual diagram authoring by the operator.

### Explicit non-goals for v1

* Editing the cluster by manipulating the graph (drag-to-reparent, etc.) — this is a **read/navigate** surface, consistent with Principle I; any mutation still goes through the existing YAML-apply/action-confirmation flows.
* Custom/manual layout persistence.

### Open questions (to resolve at spec time)

* Rendering approach: canvas/WebGL graph library vs. SVG — pod-level zoom on a large cluster (thousands of pods) needs to stay interactive, which likely rules out a naive SVG-per-node approach.
* How much of the graph is computed client-side (from data already streamed to the store) vs. requires a new backend "topology snapshot" event.

---

## 6. Nodes View

*Status: ✅ Done — base view in `001-tauri-shadcn-frontend`; actions (cordon/uncordon/drain/taint) added in `003-yaml-node-crd-parity`.*

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

*Status: ✅ Done — `001-tauri-shadcn-frontend`; YAML edit/apply added in `003-yaml-node-crd-parity`; logs/exec added in `002-pod-logs-terminal`.*

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

*Status: ✅ Done — `001-tauri-shadcn-frontend`.*

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

* Clear "data source" labeling
* Visible loading / unavailable states
* No silent failures

---

## 9. Custom Resources (CRDs)

*Status: ✅ Done — generic discovery/browsing shipped in `003-yaml-node-crd-parity`; YAML edit/apply of any CR shipped in the same feature.*

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

*Status: 🚧 Partial — cluster settings/preferences shipped in `001-tauri-shadcn-frontend`; Node Shell not built.*

### Cluster Settings

* Metrics source
* Prometheus path
* HTTPS toggle
* Path prefix
* Hidden metrics

### Node Shell

*Status: ⬜ Not started.* An on-demand ephemeral debug pod scheduled onto a specific node (distinct from `002`'s in-pod `kubectl exec`-equivalent terminal, which attaches to an existing container).

* Configurable image
* Optional image pull secret
* On-demand ephemeral pod

### Preferences

* Default time range
* Namespace persistence
* UI density (compact / normal)

---

## 11. Integration Expectations

*Status: 🚧 Partial — Kubernetes API, Prometheus, CRDs, GitOps (Flux), and Helm metadata (see §5a) integrations done; logs done (`002`).*

### Required Integrations

* Kubernetes API (watch + list) ✅
* Prometheus ✅
* CRDs ✅
* Helm metadata ✅ *(see §5a)*
* GitOps annotations (Flux / Argo) — Flux ✅, Argo not yet evaluated

### Optional Integrations

* Alertmanager ⬜
* Tracing (future) ⬜
* Logs ✅ *(`002-pod-logs-terminal`)*
* Hubble/Cilium traffic ✅ *(promo-brochure-v2 work)*

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

* RBAC-aware UI *(⬜ not started — see §14)*
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

## 14. Future Tiers (beyond this MVP doc's original scope)

Carried over from roadmap discussion, in priority order:

### Tier A — Live/SDK-auth features
* Port-forwarding — ✅ already shipped (`src/backend/portforward.rs`)
* MCP server — ✅ already shipped (`rmcp`/`axum`)
* SDK-based login (GitHub / Claude / OpenAI SDKs) — ⬜
* Local LLM inference via Ollama for the MCP/AI-assist surface — ⬜

### Tier B — Enterprise auth / access
* OAuth-based cluster login — ⬜
* LDAP integration — ⬜
* Role assignment UI — ⬜
* RBAC-scoped UI (hiding/disabling actions a role can't perform, not just erroring after the fact — ties to constitution Principle VI) — ⬜

---

## Suggested Next Slices (in order)

1. **Applications view (§5)**, read-only first — aggregates data already available (labels, Flux inventories); no new backend integration required beyond what `003` and the Flux panel already stream.
2. **Rich interlinking (§5b)** — build this *as part of* the Applications view rather than after it, since Applications is the first place drill-down is needed; the generic relationship resolver then also benefits every existing inspector (Node → Pods, Service → Pods, etc.) for free.
3. **Helm (§5a)**, read-only (list + inspector + revision history) — new backend surface (release Secret/ConfigMap parsing), feeds Applications' "Managed By: Helm" rows with real data instead of a label guess.
4. **Helm mutate** (upgrade/rollback/uninstall) — once read path is solid, follows the same diff-preview + confirmation pattern as `003`'s YAML apply.
5. **Topology graph (§5c)** — once §5b's relationship model exists and is exercised by real UI, extend it into a zoomable graph rather than modeling relationships twice.
