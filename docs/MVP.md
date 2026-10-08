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
| Ephemeral debug containers on a running pod (add-only, confirm-gated; logs via existing viewer) | ✅ | 025-ephemeral-debug-containers |
| YAML edit + diff + apply (any resource) | ✅ | 003-yaml-node-crd-parity |
| Node actions (cordon/uncordon/drain/taint) | ✅ | 003-yaml-node-crd-parity |
| Custom Resources — generic CRD discovery/browsing | ✅ | 003-yaml-node-crd-parity |
| GitOps (Flux) integration panel | ✅ | promo-brochure-v2 work |
| KEDA autoscaler pane (ScaledObjects/ScaledJobs, triggers, scale reasoning) | ✅ | 029-keda-autoscaler-pane |
| Hubble/Cilium traffic panel | ✅ | promo-brochure-v2 work |
| **Applications view** | ✅ | 004-applications-interlinking |
| **Helm** — read (list/inspector/origin-link) | ✅ | 005-helm-release-origin-linking |
| **Helm** — mutate (rollback/upgrade/uninstall) | ✅ | 006-helm-mutate |
| **Rich interlinking / drill-down** (chart → pod, resource → resource) | ✅ | 004-applications-interlinking, extended in 005 |
| **Topology graph view** (zoomable namespace→pod block-and-line diagram) | ✅ | 007-topology-graph-view |
| **Cluster linter** (read-only Popeye-style findings: limits, probes, deprecated APIs, unused ConfigMaps/Secrets, naked pods, RBAC wildcards, missing PDBs) | ✅ | 017-cluster-linter |
| Pod lifecycle hooks / probes / init-container detail | ✅ | 008-pod-lifecycle-diagnostics |
| Port-forward rich UI (backend already shipped, no frontend panel yet) | ✅ | 009-port-forward-ui |
| Container image layer inspection | ✅ | see §9a |
| Access Control (RBAC/OAuth/OIDC, role-scoped UI) | ✅ | 011-rbac-oidc-administration (cluster-admin RBAC browse/create/edit/delete + namespace-access lookup + OIDC issuer config generator; see §14 Tier C) plus 033-rauthy-oidc-teams-users (OIDC sign-in, per-context bearer auth, capability-scoped UI, IdP directory; live cluster validation partial, see §14 Tier B) |
| In-cluster context daemon / multi-cloud federation (AKS/GKE/EKS) | 🚧 | P1 (multi-context sidebar + live status) and P2 (034 simultaneous federation, live cross-cluster overview) shipped — see §14 Tier D; 036 in-cluster daemon shipped (aggregated API, per-user filtering, live topology, metrics ring, warm pod/event snapshots with snapshot-then-delta client: first pod list 810 ms vs 1,311 ms direct at 5k pods; render peaks 156–179 MiB, inside the 250 MiB budget); cloud-auth plugins live-cloud verification still open |
| Agent SDK ops (AI-assist troubleshooting, BYOK/local) | ✅ | 013-agent-sdk-ops |
| Teams & organizations (self-hosted multi-user) | 🚧 | 014-teams-organizations — shared non-secret connection profiles (Story 1) and local §10 Cluster Settings + org-level sync (expanded Story 2) shipped; per-user RBAC-scoped views (Story 3) delivered by 033-rauthy-oidc-teams-users (automated tests only; live multi-user validation not run) — see §14 Tier F |
| Hosted server mode (in-cluster web UI, OIDC sign-in, per-user RBAC) | 🚧 | 031-hosted-server-mode — `fathom-server` + Helm chart; pass-through and impersonation access models; per-user backends with isolated state; attributed, append-only audit log (own entries, or all for `audit.viewerGroups`); security review done (`specs/031-hosted-server-mode/security-review.md`); operator guide `docs/HOSTED.md`; live Chromium login/RBAC/revocation suite passes on Rauthy and Keycloak (2026-09-26); remaining: 25-user soak (written, not run), Helm upgrade/uninstall check (written, not run), WebKit pass, distroless-image verification. Namespace-only users (2026-09-26): namespaced lists go through `ns_scope::list_scoped` (cluster-wide, else per reachable namespace when `list namespaces` is denied, skipping 403s); a 403 becomes `AccessDenied` and the pane shows "Not permitted" instead of a toast; pollers log a denial once and back off; the picker falls back to the context namespace and accepts a typed one. Guard: `ui/tests/live/restricted-walk.spec.ts` with `k8s/samples/restricted-user.yaml`. The remaining `Api::all` sites are cluster-scoped kinds, CRD discovery, SelfSubject reviews, node relations, and the fleet/federation/MCP paths; they report "Not permitted" rather than failing |
| SDK login (GitHub/Claude/OpenAI) + local Ollama inference | ✅ | 038-sdk-login-ollama (GitHub device flow, Claude/OpenAI keys in the OS keyring, Ollama detection and model picker, offline mode; provider calls verified against mocks only, no live credentials) |
| Node Shell (on-demand ephemeral debug pod, distinct from pod exec) | 🚧 | `037-node-shell` — privileged debug pod pinned to a node with confirm, SSAR pre-check, guaranteed cleanup and orphan sweep; shipped incl. settings and a11y e2e; validated on kind (Linux only); remaining: T030 Tier 1 platform runs; brochure shots captured |
| Command palette | ✅ | 016-command-palette |
| Proactive RBAC-aware mutation UI (disable/hide actions the identity can't perform) | ✅ | 018-rbac-aware-mutation-ui, extended per-user in 033 |
| Cluster version badge | ✅ | 019-cluster-version-badge |
| Security & Policy panes — Gatekeeper, Trivy Operator vulnerabilities | ✅ | 020-gatekeeper-policy-pane, 021-trivy-vulnerability-pane |
| Velero backup pane | ✅ | 022-velero-backup-pane |
| cert-manager Certificates pane | ✅ | 023-cert-manager-certificates-pane |
| Pod exec session gate (confirmation + RBAC pre-check) | ✅ | 024-pod-exec-terminal |
| Local mutation audit-log viewer | ✅ | 026-mutation-audit-log-viewer |
| Fleet rollup dashboard (per-context health cards) | ✅ | 027-fleet-rollup-dashboard |
| Knative pane | ✅ | 028-knative-pane |
| In-app local kind cluster provisioning | ✅ | 030-kind-cluster-provisioning |
| Hosted server mode (in-cluster web app with OIDC login) | 🚧 | 031-hosted-server-mode — server, OIDC login, per-user tenancy, audit sink, security review and operator guide shipped; live suite passes on Rauthy and Keycloak; soak, upgrade/uninstall and WebKit runs still open (see Status Summary) |
| Managed-cloud auth plugins (EKS/GKE/AKS exec credentials) | 🚧 | 035-cloud-auth-plugins — implemented; live EKS/GKE/AKS and Windows/macOS manual checks pending |
| Observability settings persistence (Prometheus/Grafana, secrets in OS keyring) | ✅ | 039-observability-settings |
| Manifest export (copy/download YAML per resource, bundles, clean-for-re-apply, manifest before every apply, kubeconfig export) | ✅ | 040-manifest-export (live re-apply dry run on kind; ServiceAccount-kubeconfig minting and namespace bundles deferred to the backlog) |
| Observability auto-connect (Prometheus via API server proxy, Open Grafana via Fathom port-forward, Topology tool sheet renders) | ✅ | 041-observability-autoconnect (live on kind-fathom-dev) |
| Nav completeness: every nav section and tool loads real data or states why not | ✅ | nav walk 2026-09-26: `ui/tests/e2e/nav-walk.spec.ts` (mock, every section and tool) and `ui/tests/live/nav-walk.spec.ts` (10/10 on kind-fathom-dev); fixed Overview node count, false DANGLING role refs, Logs tool pod picker, Argo/Metrics/Grafana probes, honest integration empty states; post-connect discovery moved out of the backend lock (no more "Loading" behind Initialize); Gatekeeper ScanFailed state. Dev addons: Argo CD, Grafana, optional Cilium/Hubble cluster (`k8s/cluster.sh up-cilium`) |
| Setup and navigation: full-screen setup when no context is current (pick, connect, sign in, create a local cluster), Settings menu in the header, navigator grouped for newcomers (Cluster, Integrations with greyed not-installed entries, Velero hidden until detected), Topology as a Cluster section and an inspector tab, a calm dock (Logs, Events, Port forwards, and Cluster history while the daemon runs), the daemon switch labelled and explained, the role wizard reachable from Access | ✅ | 043-setup-and-navigation (feedback 2026-09-27 #4, #5, #9–#17, #38, #54) |
| Role and ClusterRole create/edit (rules editor, discovery-fed pickers, presets, YAML review, confirm-gated apply, escalation errors surfaced) | ✅ | 042-role-create (pre-flight SSAR escalation hints and the aggregation-label editor are deferred to the backlog) |
| Cascading wide-screen layout (navigator + section list pinned, slots 3+ fill only as the operator drills in, the section's most relevant item auto-opens) | ✅ | 044-cascading-layout (wide/ultra/ultra-plus only; phone, tablet, laptop and desktop keep today's behaviour) |
| Guided OIDC setup (replaces the flat issuer form for add and edit: configuration, read-only cluster checks, an offer to create the bindings through 042's forms, and API server authentication config emitted and applied for clusters Fathom provisioned) | ✅ | 045-guided-oidc-setup — 79/79 tasks done; gate green, restricted walk 13/13, promo 40/40 |
| Fleet model (clusters as named, labelled, groupable members; groups are label selectors; definitions shareable through 014's store; rollups state their own coverage) | ✅ | 046-fleet-model — shipped 2026-10-08. Metadata lives in `~/.fathom/fleet.json`; hosted mode allows fleet reads and edits to the `home` member only (FR-043 table in `server/src/filter.rs`). Promo scene `wide-fleet-model` captured live. Open: T045 idle-CPU figure (see Remaining, item 6) |
| Cross-cluster search (one query over a fleet group: label selector, CRD presence, name substring, container image; streamed per-cluster outcomes with explicit coverage) | ⬜ | 047-cross-cluster-search — spec only; depends on 046 |
| Docker management (the containers, Compose projects, images, volumes and networks on this machine; read-only until a project is adopted; typed confirmation on destructive verbs; usage sampler and log follow stop when the scene closes) | ✅ | 048-docker-management — shipped 2026-10-08. Every container-runtime command is denied in hosted mode (`HostedDenial::ContainerRuntime`). Promo scene `wide-containers` captured live against the host daemon. Open: T079 idle-CPU figure (see Remaining, item 6) |

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

#### Wide screens cascade *(`044-cascading-layout`)*

From 1920 px up, the extra columns are a drill-down trail rather than a set of prefilled panels.
Slot 1 is the sections navigator and slot 2 the selected section's list; both stay put. Everything
to their right is a cascade column, filled left to right and empty until there is something to put
in it — an empty one says "Follow a link to open the next level here" rather than reading as a
broken layout.

* Clicking a section opens its **most relevant item** in the first cascade column: the first row
  whose status is an error, else the first warning, else the first row — in the list's current
  order, so it is always a row the operator can see. An empty section auto-opens nothing.
* Each follow (a related object, a link, a row in an earlier column) lands one column further
  right. Selecting a row in a column that is not the last drops every column to its right, and a
  new section click returns the row to navigator + list + the auto-opened item.
* With the chain deeper than the window is wide, the oldest steps scroll off the left of the
  cascade while the navigator and the list stay pinned; narrowing and widening the window
  re-reveals them with their selections, search text and scroll positions intact.
* **Below 1920 px nothing changes**: phone, tablet, laptop and desktop keep exactly the behaviour
  they had, and the inspector waits for the operator's click. Saved layout profiles from earlier
  releases still load; their slots from the third on resolve to cascade columns, keeping the widths
  the operator dragged.

---

## 3. Navigation & Information Architecture

### Global Navigation Tree

#### Cluster Root

* Overview ✅
* Applications ✅ *(`004-applications-interlinking`, see §5)*
* Nodes ✅
* Workloads ✅
* Config ✅
* Network ✅
* Storage ✅
* Namespaces ✅
* Helm ✅ read path *(see §5a)*
* Access Control ✅ *(`011-rbac-oidc-administration`, `033-rauthy-oidc-teams-users` — see §14)*
* Custom Resources ✅ *(generic, `003-yaml-node-crd-parity`)*
* Events ✅
* Topology ✅ *(whole-cluster section, `043-setup-and-navigation`)*
* Linter ✅
* Cluster history ✅ *(the optional daemon's record of what changed while Fathom was closed; formerly "While you were away")*
* Integrations ✅ *(Argo, Flux, Knative, KEDA Autoscalers, Velero once detected, Certificates, Gatekeeper, Hubble, Vulnerabilities, Metrics, Grafana; not-installed ones stay listed, greyed out)*
* Settings ✅ *(also the header's Settings menu, left of the sign-in chip: Cluster settings, Sign-in, Team, Connection profiles, App settings, Local clusters, Audit log)*

With no current context, Fathom opens a full-screen setup instead of the panes: pick a context,
connect (Retry and Details on failure), sign in if the cluster needs it, or create a local kind
cluster. Settings pages open from setup as a detour with "Back to setup".

The bottom dock holds only tools that follow the current selection: Logs, Events, Port forwards,
and Cluster history while the daemon is available. Topology and the integrations moved to the
navigator; the navigator's Tools list keeps Logs.

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

*Status: ✅ Shipped — `004-applications-interlinking` (Flux/Helm/label attribution, inspector, drill-down via the related-resources resolver in §5b).*

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

*Status: ✅ Shipped — read path (`005-helm-release-origin-linking`): list, inspector, and GitOps origin-context linking to Flux. Mutate (`006-helm-mutate`): rollback, upgrade, uninstall, all diff-previewed and confirmation-gated, with GitOps-ownership warnings.*

Fathom now reads Helm-managed resources directly from Helm's own release storage, not just indirectly via Flux inventories or ownership labels.

### Read (shipped)

* List installed Helm releases (name, namespace, chart, chart version, app version, revision, status, last updated) — via Helm's own release storage (Secrets/ConfigMaps in the release namespace, `helm.sh/release.v1` type), not a shelled-out `helm` binary
* Release inspector: values (effective + user-supplied), rendered manifest, revision history, resources owned by the release (feeds §5b drill-down)
* GitOps origin-context linking: a Flux-owned release's inspector shows a click-through link to its owning `HelmRelease`/`Kustomization`, surfacing that controller's live Ready/Failed/Reconciling/Suspended state and message — the operator's originally-requested "trace to origin" workflow

### Mutate (shipped, `006-helm-mutate`)

* Rollback to a prior revision — pick from the release's revision history, diff-preview (current manifest vs. target revision's stored manifest) before applying
* Upgrade — edit the release's effective values via the existing YAML editor, diff-preview (rendered `helm upgrade --dry-run` manifest vs. current) before applying
* Uninstall, confirmation-gated — no typed-name input required (single-target action)
* All three actions surface a GitOps-ownership warning (linking to the owning Flux `HelmRelease`/`Kustomization`) when the release is Flux-managed, before the operator confirms

**Explicit non-goal for v1**: chart install/search from a repo, or a values-editor wizard — this is read/rollback parity first, not a package-manager UI.

---

## 5b. Rich Interlinking / Drill-down

*Status: ✅ Shipped — generic related-resources resolver (`src/backend/related.rs`) and Inspector breadcrumb/back-stack landed in 004-applications-interlinking, extended with the `"reconciled-by"` relation in 005-helm-release-origin-linking. Covers Node, Service, PVC, Pod, Deployment/StatefulSet/DaemonSet/ReplicaSet, Application, and Helm release relationships. Foundation for §5c is now in place.*

**Problem today**: every resource kind is its own list + inspector island. Going from "this Application" or "this Helm release" to "the specific Pod that's crash-looping" means manually noticing an owner reference or label and re-navigating by hand.

### Requirement

* Every resource inspector that has a natural parent/child relationship (Application → Deployment → ReplicaSet → Pod → Container; Helm release → its owned resources; Node → the Pods scheduled on it; Service → the Pods it selects; PVC → the Pod(s) mounting it) surfaces those relationships as **clickable links**, not just text.
* Clicking a related-resource link opens that resource's inspector directly (same no-route-change pattern as today), preserving a **breadcrumb/back-stack** so the operator can retrace the drill-down path.
* Relationship resolution reuses data already fetched for the current resource's `ownerReferences`/label selectors where possible — no new backend round-trip if the data is already in hand; otherwise a targeted lookup (not a full list-and-filter).

### Backend implications

* A small generic "related resources" resolver, keyed by kind + ownerReferences/selector, that existing resource-read handlers can call rather than each kind hand-rolling its own relationship lookups.

---

## 5c. Topology Graph View

*Status: ✅ Shipped (`007-topology-graph-view`) — canvas-rendered, force-directed graph with
continuous zoom (namespace → workload → pod tiers), click-to-inspect reusing the existing
Inspector pane/back-stack, and live health-state recoloring. **v1 scope reduction**: node kinds
are Namespace/Application/HelmRelease/Node/Deployment/StatefulSet/DaemonSet/Pod — Service and
generic CRD instances are deferred to a later iteration (see
`ui/src/topology/types.ts`'s `TopologyNodeKind` doc comment), a deviation from this section's
original "Namespace, Application/Helm-release, Workload, Pod, Service, Node, or CRD instances"
wording.*

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

*Status: ✅ Done — `008-pod-lifecycle-diagnostics` shipped the items below as a new "Lifecycle" Pod Inspector section (`ui/src/components/PodLifecyclePanels.tsx`, `PodDetails.containers_lifecycle`).*

* **Init containers** — listed distinctly from app containers, with status/exit-code and a "view logs" affordance that switches to the Logs tab and starts a stream scoped to that container.
* **Container lifecycle hooks** — `postStart`/`preStop` as configured on the container spec, correlated with `FailedPostStartHook`/`FailedPreStopHook` Events to flag a hook that actually failed.
* **Probes** — `livenessProbe`/`readinessProbe`/`startupProbe` configuration (type, mechanism/target, thresholds, period) per container, correlated with the container's current probe-failure state (`ContainerStatus.ready`/Events) rather than just the aggregate `Ready` condition.
* **Restart history per container** — restart count, last termination reason, exit code, and last-terminated timestamp (`ContainerStatus.lastState`), including the `restartPolicy: Never` edge case where a container that never restarted still surfaces its termination from `state`.
* **Events timeline** — the pod's own `Event` objects rendered as a chronological, live-updating timeline inside the inspector, sourced from the same cluster-events list the Events tab uses (no duplicate fetch).

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

## 9a. Container Image Inspection

*Status: ✅ Shipped (`010-container-image-inspection`) — lives off the Pod inspector's "Image" tab, not its own nav section.*

A "download and inspect layers" affordance on a container's image reference, without pulling the whole image: fetch the OCI manifest + config from the image's registry (using the same `imagePullSecrets` the cluster already has, when the registry is private) and show layer sizes, the image's build history (`Cmd`/commands per layer, from the OCI config's `history` field), total size, and base-image digest — the kind of thing `docker history`/`dive` give you today, surfaced in-app instead of requiring a shell.

### Requirements

* Backend fetches only the manifest + config blobs (OCI distribution spec `GET /v2/<name>/manifests/<ref>` and the config blob it references) — never pulls layer content itself; this stays fast and small regardless of image size.
* Reuses a registry-client crate that already speaks the OCI distribution spec (e.g. `oci-client`/`oci-distribution`) rather than hand-rolling registry HTTP + auth — same "check Cargo.toml, use a public crate" discipline this repo already applies to `kube`, `tonic`, etc.
* Auth: try anonymous first, fall back to the pod's resolved `imagePullSecrets` (already visible to the backend via the pod spec) — no new credential UI.
* Container inspector gains an "Image" tab/section per container: registry, repo, tag/digest, total size, per-layer size + history command, base image (best-effort, from the last shared layer digest against well-known base images — best-effort only, not guaranteed).

### Explicit non-goals for v1

* Vulnerability/CVE scanning — a real feature, but a distinct one (needs a vuln database, not just registry metadata); track separately if/when prioritized.
* Pulling/caching full image content locally.
* Private registries requiring interactive/browser-based auth (device-code OIDC registries) — service-account/secret-based auth only for v1.

---

## 10. Configuration & Settings

*Status: ✅ Done — cluster settings/preferences in `001-tauri-shadcn-frontend`, Node Shell in `037-node-shell`, port-forward UI in `009-port-forward-ui`, observability settings persistence in `039-observability-settings`.*

### Cluster Settings

*043: Settings is a navigator group and a header menu. Cluster settings holds the per-context
settings below plus the in-cluster daemon switch with its explanation; app-wide preferences
moved to App settings, and kind provisioning to Local clusters.*

* Metrics source
* Prometheus path
* HTTPS toggle
* Path prefix
* Hidden metrics
* **Vulnerabilities** *(✅ shipped, `021-trivy-vulnerability-pane`)* — read-only surfacing of Aqua Trivy Operator `VulnerabilityReport`s: a Pod Inspector "Vulnerabilities" tab (severity counts + top findings, honest not-installed / no-report-yet / forbidden states) and an opt-in cluster-wide tool pane ranking workloads worst-first. Fathom never triggers or configures scans.
* **Available Agents** *(✅ shipped, `015-mcp-agent-exposure`)* — the stdio MCP server's on/off toggle (off by default), its live running/not-running state, the connection command an external MCP client should launch, a read-only Anthropic/Ollama auth summary reused from `AuthSetup.tsx`, and the "Launch example chat" affordance with honest not-detected/server-off states. See §14 Tier A.
* **Audit Log** *(✅ shipped, `026-mutation-audit-log-viewer`)* — Team → Audit Log: a local, read-only, newest-first history of the mutating actions Fathom completed (action, target, context, timestamp, success/failed with reason), bounded to the last 500 in `~/.fathom/audit-log.json`, with client-side filters (action, outcome, context, time range, name search). It is local-only and is not Kubernetes' own audit log.

### Node Shell

*Status: ✅ Shipped (`037-node-shell`; remaining: T030 Tier 1 platform runs; brochure shots captured).* An on-demand ephemeral debug pod scheduled onto a specific node (distinct from `002`'s in-pod `kubectl exec`-equivalent terminal, which attaches to an existing container).

* Configurable image
* Optional image pull secret
* On-demand ephemeral pod

### Port Forward Interface

*Status: ✅ Shipped (`009-port-forward-ui`) — one-click "Forward" action from Pod and Service inspectors; a live forwards list (desktop nav `network/portforwards` and the phone/tablet `Forwards` stack tab) showing local port, target namespace/pod (and originating service, when launched from one), target/remote port, and live tunnel status, with stop/restart controls per row; local port picker with debounced conflict detection (`CheckPortAvailability`/`PortAvailability`) that blocks confirming on an unresolved conflict; copy-address action (`localhost:<port>`, disabled until the tunnel is `Active`). Cluster-context-scoped teardown was already true at the backend level (`PortForwardManager`) — the UI renders that state as-is.*

* One-click "Forward" action from a Service or Pod inspector (mirrors the existing per-resource action affordances), not just a standalone form requiring the operator to already know the target port.
* A live list of active forwards (this cluster context) showing local port, target namespace/pod-or-service/port, and live tunnel status (`TunnelStatus`), with stop/restart controls per row.
* Local port picker with conflict detection (don't silently fail if the port's taken — surface it before the operator tries to connect).
* Copy-address button (`localhost:<port>`) for pasting into a browser/`psql`/etc.
* Forwards are cluster-context-scoped and torn down on context switch/app exit (already true at the backend level per `PortForwardManager`'s existing lifecycle — the UI just needs to reflect it, not reimplement it).

### Preferences

* Default time range
* Namespace persistence
* UI density (compact / normal)

---

## 11. Integration Expectations

*Status: ✅ Required integrations done — Kubernetes API, Prometheus, CRDs, GitOps (Flux), Helm metadata (§5a), logs (`002`); Grafana settings persisted (`039`). Optional Alertmanager and tracing integrations are not started and are post-MVP.*

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

* RBAC-aware UI ✅ *(`018-rbac-aware-mutation-ui`, per-user in `033` — see §14)*
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

*Status: ✅ Shipped — port-forwarding, the MCP server (`015`), the fleet rollup (`027`), SDK login and Ollama inference (`038`).*

* Port-forwarding — ✅ already shipped (`src/backend/portforward.rs`)
* **✅ MCP server (`015-mcp-agent-exposure`) — local stdio, off by default, RBAC-scoped.** Exposes the cluster context Fathom is already connected to to an external MCP client (Claude Code, Claude Desktop, any MCP-speaking agent) over **stdio on the local machine** — `rmcp` 0.12's `serve_server_with_ct` over `rmcp::transport::io::stdio()`, no network listener and no HTTP surface. This *replaced* the earlier prototype rather than extending it: the `axum` server on port 18288 and its ungated `restart_deployment` tool were deleted outright (`src/mcp_server.rs`, `src/app/views/mcp_integration.rs`, three superseded `docs/mcp/*.md`), so there is no second MCP surface left to reason about.
  * **Off by default, and off means nothing exists** (FR-002). `McpState::start` is reachable from exactly two places — the `SetMcpServerEnabled { enabled: true }` handler and the startup gate — and no context switch, status read or app launch can spawn a server the operator has not enabled. Turning the toggle off cancels the `CancellationToken` and awaits the task, so no listener survives the command that disabled it (FR-013).
  * **Five read tools** (`get_resource`, `list_resources`, `get_pod_logs`, `get_events`, `describe_pod`) and **three mutation tools** (`scale_workload`, `apply_resource_yaml`, `delete_resource`). Every call is scoped through `rbac_access::resolve_namespace_access` first: a denial names the identity, verb, resource and namespace and reads nothing from the cluster, rather than returning a generic error (FR-005). Resolution failure denies — fail-closed, never fail-open.
  * **Mutation tools are new callers of the existing guarded write path, not a new one.** Each takes a mandatory (non-`Option`) `expected_resource_version` in its JSON schema, which the backend re-fetches and re-validates against the live object before delegating to the exact `apply::apply_resource`/scale/delete functions the UI's own confirmation flow calls. A successful agent-initiated mutation emits the same `BackendEvent` a UI-initiated one does, so the operator's own window reflects it live (Principle I applies to agent-initiated actions exactly as to operator-initiated ones).
  * **Settings → Cluster Settings → "Available Agents"** (§10) holds the whole surface: the on/off toggle, a running/not-running indicator that reads the manager rather than inferring health from `enabled`, the copyable connection command, and a read-only reuse of the Anthropic/Ollama auth summary `AuthSetup.tsx` already populates (no duplicate credential detection). The preference persists in `~/.fathom/config.json`'s `auto_start.mcp_stdio` (renamed from `mcp_http`, with a `serde` alias so older files still load) and is honoured at startup, so an enabled server comes back up across a restart.
  * **The connection command is the real entry point**: an external client launches `fathom --only-mcp`, which runs the backend headless with the process's own stdin/stdout *as* the MCP channel, connects to the kubeconfig's current context, serves its client, and exits when the client disconnects — no orphaned process. With the toggle off the same command refuses on stderr with a stated reason instead of hanging. `tracing` writes to stderr so a log line can never corrupt the JSON-RPC framing.
  * **"Launch example chat"** spawns a real local `claude` CLI subprocess pre-wired to this Fathom's MCP server via ephemeral `--mcp-config`/`--strict-mcp-config` (nothing is written into the operator's own Claude Code config). Detection is a fresh `PATH` scan on every call with no cache; when there is no `claude`, or the server is off, the panel says so and shows setup instructions. There is no simulated or mocked chat window in any state (FR-009/FR-011).
* **✅ Fleet Rollup Dashboard (`027-fleet-rollup-dashboard`)** — read-only per-context health cards (nodes ready/total, unhealthy pods, version, Flux status) for every kubeconfig context at once, opened from the top bar. Backend pollers (`src/backend/fleet.rs`, 30s interval, 10s timeout) exist only while the view is open and never touch the active context.
* SDK-based login (GitHub / Claude / OpenAI SDKs) — ✅ (038; GitHub device flow, Claude/OpenAI keys in the OS keyring; verified against mocks, no live credentials)
* Local LLM inference via Ollama for the MCP/AI-assist surface — ✅ (038; detection, model picker and offline mode; verified against a mock server)

### Tier B — Enterprise auth / access
*Status: 🚧 Implemented in 033-rauthy-oidc-teams-users; live end-to-end validation against a real cluster is only partly done (see the verification notes below).*

* OAuth/OIDC-based cluster login — ✅ implemented (033: PKCE loopback + device-code sign-in, silent refresh, per-context opt-in, `Authorization: Bearer` layer with no kubeconfig fallback after an OIDC failure). Unit/integration-tested against a mock provider (`tests/oidc_*.rs`); the live kind check against Rauthy (quickstart section 2, task T023) has **not** been run.
* LDAP integration — ✅ by design with zero Fathom LDAP code: LDAP users sign in through the IdP (Rauthy via Dex). `scripts/check-no-ldap.sh` enforces no LDAP crate in CI. The lab manifests deploy and OpenLDAP/Dex are verified live, but Rauthy upstream-provider registration and the group reconciler are **not** verified live (blocked on the bootstrap-only `ldap-lab` API key; request bodies are unprobed).
* Role assignment UI — ✅ (033 US4 on top of 011: IdP directory picker, preview with `oidc:`-prefixed subject, orphaned-subject badge).
* RBAC-scoped UI (hiding/disabling actions a role can't perform, not just erroring after the fact — ties to constitution Principle VI) — ✅ (033 US5: lazy per-namespace `SelfSubjectRulesReview` with `SelfSubjectAccessReview` fallback, fail closed, 60 s cache). Verified by unit and Playwright tests; not verified live with distinct teammates on a cluster.

### Tier C — Cluster-admin RBAC / OIDC-OAuth administration

*Status: ✅ Shipped (011-rbac-oidc-administration). This is a UI/workflow layer over Kubernetes' own OIDC-auth and RBAC primitives, not a parallel auth system — the API server already supports OIDC token auth and `Role`/`ClusterRole`/`RoleBinding` objects; Fathom's job is to make configuring and auditing them tractable through a GUI instead of hand-written YAML, gated behind the operator already having cluster-admin (or the specific RBAC-edit permissions) themselves.*

* A cluster-admin-only settings surface for wiring up the API server's OIDC issuer/audience/claim-mapping (where the operator manages the API server config directly, e.g. via kubeadm/managed-cluster provider settings) — or, where the platform doesn't expose that, a guided "here's what to add to your cluster's OIDC config" doc-generation flow rather than a live edit (many managed clusters don't let a client mutate API-server flags).
* A role-assignment UI: browse existing `Role`/`ClusterRole`s, create/edit `RoleBinding`/`ClusterRoleBinding`s mapping OIDC groups/claims (or service accounts) to roles, namespace-scoped or cluster-scoped — reusing the existing YAML-apply diff-preview/confirmation pattern (Principle I) rather than a bespoke mutation path.
* A namespace-assignment view: for a given identity (user/group), which namespaces they can act in and with what verbs — computed from `RoleBinding`/`ClusterRoleBinding` + `Role`/`ClusterRole` resolution, read-only first.
* Ties directly into Tier B's "RBAC-scoped UI" item — once role resolution exists for the signed-in identity, the same data disables/hides actions the identity can't perform.

#### Guided OIDC setup *(`045-guided-oidc-setup`)*

The Tier C bullet above asked for "a guided 'here's what to add to your cluster's OIDC config'
doc-generation flow rather than a live edit". 045 is that flow, and it replaces the flat issuer
form rather than sitting beside it: the six `Input` fields are gone, and the one way to add or
review an issuer is the guided path.

Six steps, each of which can be left and returned to without losing the draft:

* **Issuer** — discovery URL, client id, and whether the issuer is one Fathom seeded (hosted) or
  one the operator is adding. A seeded issuer is reviewable but not editable or deletable.
* **Discovery** — fetches `.well-known/openid-configuration` and shows what came back, so a typo
  in the URL fails here rather than three steps later.
* **Test sign-in** — a real PKCE sign-in against the draft config before anything is persisted.
  Its refresh token is held in memory only; the draft lives under a session key that belongs to
  the protocol, not to the UI.
* **Claims and role slots** — the claims sample from the test sign-in, mapped onto Fathom's three
  role slots. Group names are carried raw; the `oidc:` prefix is applied at preview time by the
  existing `render_subject_name`, and the step warns when the draft's prefix and the cluster's
  disagree.
* **Bindings** — hands a chosen group to the existing role-assignment form, reusing 011's
  diff-preview/confirm path (Principle I) rather than a bespoke mutation.
* **API server config** — offers to write the authentication config, but only for a control plane
  Fathom provisioned itself. Everywhere else, including hosted, the same step shows the exact
  configuration to apply by hand, copyable, with a read of what the cluster currently has beside
  it. Nothing is hidden: the write is previewed and confirmed like any other mutation.

Hosted skips the write and reaches the path through Settings › Sign-in rather than the identity
chip, which stays hidden there. The protocol surface is additive and stays v1.

*Status: 🚧 Implemented; live validation against the dev cluster (quickstart) and the restricted
suite's audit-log check for the API server write are pending.*


### Tier D — In-cluster context daemon & multi-cloud federation

*Status: 🚧 Mostly shipped — P1 (012), P2 (034) and P4 (036) shipped; P3 (035 cloud-auth plugins) implemented with live EKS/GKE/AKS verification pending.*

* **✅ P1 (012-incluster-daemon-federation): always-visible multi-context sidebar with live per-context connection status.** Every kubeconfig context is listed at once (not just the active one), with a live `connected` / `connecting` / `error` / `not-connected` indicator — the active context's status is driven by its real connect attempt, every other context gets a lightweight, bounded (`apiserver_version()` + 5s timeout) background reachability probe triggered on `Initialize`/`RefreshContexts`/`SetContext`, never a periodic poll. An unreachable context's error is inspectable without switching to it. Switching contexts still uses the existing single-active-client model and reconnect path unchanged. Delivered via an additive `SystemEvent::ContextStatusUpdated`; no new command, no new dependency.
* **✅ P2 (034-simultaneous-federation): true simultaneous federation** — an opt-in, persisted set (`~/.fathom/federation.json`, names only, soft cap 8) of contexts each held live with its own persistent client and bounded watches (Pods, Nodes, Deployments, Warning Events) reduced to counters, plus a cross-cluster aggregate overview in the Fleet dialog. A context is either polled (027) or live, never both; federated contexts also render on the 027 fleet cards with a Live marker. Read-only; the active-context model is unchanged. Added `FederateContext`/`UnfederateContext`/`RequestFederationSnapshot` and `FederationStateUpdated`/`FederationRollupUpdated` (additive, protocol v1). Automated tests only; the idle-CPU soak (SC-002) and reachability-recovery (SC-004) checks against local kind have not been run.
* **🟡 P3 (035-cloud-auth-plugins implemented; live EKS/GKE/AKS and 3-OS manual checks pending): native credential/context plumbing for managed-cloud auth plugins** — `aws-iam-authenticator`/`aws eks get-token` for EKS, `gke-gcloud-auth-plugin` for GKE, `kubelogin` for AKS — shelling out to (or vendoring) each provider's existing official plugin rather than reimplementing its OIDC/IAM token exchange.
* **✅ P4 (036-incluster-daemon): optional lightweight in-cluster daemon** (reusing the existing Rust backend/protocol crates rather than a new stack) that can push richer or lower-latency context than polling the API server from outside the cluster — e.g. controller-level events, long-lived watches that survive the desktop client disconnecting/reconnecting. Strictly optional — Fathom already works with zero in-cluster footprint today (direct `kube-rs` against the API server), and that must stay true; the daemon is an enhancement, not a requirement.

### Tier E — Agent SDK ops (AI-assist troubleshooting)

*Status: ✅ Shipped — `013-agent-sdk-ops` (in-app AI-assist troubleshooting grounded in streamed store data, BYOK Anthropic key or local Ollama, proposals go through the normal confirmation gate), auth extended by `038-sdk-login-ollama`.*

* An in-app "what's wrong with this pod/cluster" assistant built on the Claude Agent SDK, grounded in data already streamed to the store (events, conditions, logs, the Pod-lifecycle detail from §7's gap) rather than a separate scraping/ingestion path — the assistant reads the same protocol events the UI renders.
* Auth modes chosen per the operator, not hardcoded: bring-your-own API key (Anthropic key entered locally, self-hosted-first, matches Fathom's no-cloud-dependency default) or local inference via Ollama (✅ `038`, Tier A) for a fully offline mode.
* Built against the Agent SDK's public interface (not its internals) specifically so a future hosted/cloud-ops mode is a different auth/config path later, not a rewrite — "cloud-ops ready" without requiring a cloud dependency today.
* Explicit non-goal for v1: autonomous/unattended mutation — any action the assistant proposes still goes through the same diff-preview + confirmation gate as a human-initiated one (Principle I applies to agent-initiated actions exactly as it does to operator-initiated ones).

### Tier F — Teams & organizations (self-hosted)

*Status: 🚧 Mostly shipped — 014-teams-organizations (shared profiles, org-level §10 sync) and 033-rauthy-oidc-teams-users (per-user RBAC-scoped views, IdP directory, Teams view). Live multi-teammate validation on a cluster is not done.*

* ✅ Shared cluster-connection profiles within a team (kubeconfig/context sharing, not shared credentials) via a self-hosted shared config store the operator points at — the shared record carries no credential field, and a heuristic scan refuses any publish that looks like it contains one (`TeamPublishRejected { kind: 'suspectedSecret' }`), surfaced in the UI rather than silently dropped.
* ✅ Per-user RBAC-scoped views once signed in via OIDC — controls reflect what that identity can do, per-user, not per-install (capability cache is keyed by context + identity `sub` + namespace). 011 is in `main`; delivered by 033 US3/US5. Automated tests only; the two-teammates-on-one-cluster check in quickstart sections 2-3 has not been run live.
* ✅ Org-level settings/preferences sync (metrics source, hidden-metrics config, etc. from §10) shared across a team rather than re-configured per machine — publish-as-org-default with optimistic-concurrency conflict detection, and an accept/dismiss banner when a teammate publishes a newer default for a context you're on.
* Explicit non-goal: a hosted multi-tenant SaaS control plane — this is about self-hosted teams who already share real clusters, not standing up a new hosted backend.

**Design constraint across Tiers C–F**: prefer existing public crates, cloud SDKs/CLIs, and Kubernetes-native primitives (OIDC, RBAC objects, kubeconfig contexts, each cloud's official auth plugin, the OCI distribution spec, the Agent SDK's public surface) over inventing new protocols — the same discipline already applied when adding `tonic`/`oci-client`-style dependencies for Flux/Helm/Hubble: check what's already a standard, well-supported crate or API before building a bespoke one.

---

## Remaining to MVP Release

All ten originally suggested slices (Applications, interlinking, Helm read and mutate, topology, pod lifecycle, port-forward UI, image inspection, RBAC/OIDC administration, daemon/federation, Agent SDK ops) have shipped. Feature scope for a desktop MVP is complete; what remains is release engineering and validation:

1. **CI** — automatic runs are disabled (manual dispatch only, 2026-09-26) because the Rust `Test` job was running out of runner disk and the Chromium/WebKit E2E job was failing. Re-enable once both are fixed and the suite is sharded to a reasonable wall-clock. 2026-10-08: the local gate (`scripts/ci.sh`) was run for the 046/048 batch: `check` passed (581 s); `test` failed once on the hosted-denial table not knowing the 048 commands (fixed in e94216c, `cargo test -p fathom-server filter::` green); the `ui` job was stopped on request before its summary, with one e2e failing on `chromium-ultra-plus` only (`a11y-containers.spec.ts:38`, three attempts in the container, passes in isolation on the host) and four WebKit single-attempt failures pending retry. A full green run is still owed before CI is re-enabled.
2. **Release pipeline**: MVP ships **Linux only, unsigned** (decided 2026-09-26). `release.yml` builds deb, rpm and AppImage for x86_64 and aarch64 on a `vX.Y.Z` tag; no release has been tagged yet (version `0.1.0`). A local `pnpm -C ui tauri build` produced all three x86_64 bundles, and the release binary launched against kind (2026-09-26). 2026-10-08: `make build-linux` rebuilt the deb from the 046/048 tree (`target/release/bundle/deb/Fathom_0.1.0_amd64.deb`) for hands-on use; still untagged. macOS and Windows bundles, code signing and notarization come after the MVP; their matrix entries are commented out in `release.yml`.
3. **Platform validation**: the MVP target is Linux (Ubuntu 26.04). The x86_64 packages were validated on 2026-09-26 (`001` T126, `specs/001-tauri-shadcn-frontend/checklists/release-validation.md`):
   - The deb on Ubuntu 26.04, the rpm on Fedora 44, and the AppImage on both install and resolve their dependencies, and start with a rendered window. Desktop entries and icons are correct.
   - First run with no, empty, malformed or context-less kubeconfig, an unreachable server, a rejected token, or a cluster deleted mid-session each shows an explained state with next steps (retry, pick a context, create a local kind cluster).

   Fixed along the way:
   - an empty `Categories`
   - missing 256/512 px icons
   - a panic with no CA bundle
   - "Connected" reported for dead or unauthorized clusters
   - a blank AppImage window (EGL abort caused by the bundled `libwayland-client`; `scripts/appimage-host-libs.sh` removes it)

   Open:
   - `release.yml` now repacks the AppImage with `scripts/appimage-host-libs.sh` and replaces the release asset (2026-09-26), but that step has not run on a real tag yet.
   - aarch64 packages are untested.
   - Windows 11 and macOS 26 (`001` T126 and the Windows/macOS parts of `035` and `037`) come after the MVP, alongside cross-platform builds.
4. **Hosted server mode (`031`)** — optional for a desktop MVP; the open items are listed in the Status Summary row.
5. **E2E stability**: mostly addressed on 2026-09-26. The node-actions request-id race and WebKit clipboard reads are fixed. `perf-10k` now runs in serial `chromium-perf`/`webkit-perf` projects after the functional ones (it passes: chromium 8–36 ms, webkit 18–67 ms). WebKit gets one retry, and only when its WPE engine crashes (a known upstream compositor segfault, roughly 1 in 60–150 runs). Still to do before CI can go green: prove two consecutive full-suite green runs. 2026-10-08: see item 1 for the one ultra-plus-only a11y failure.
6. **Idle CPU (046 T045, 048 T079)**: `scripts/measure-idle-cpu.sh` against the 2026-10-08 release binary, launched fresh on Overview against `kind-fathom-dev` (metrics-server, Prometheus, daemon and sandbox installed), measured 5.3% of one core over 60 s after a 90 s settle; a per-process breakdown put ~3.9% in the Rust process and ~1.1% in WebKit. The box was under load from other projects (load average 14–25), so the figure is not comparable to the 0.092% from 2026-09-26 (Pods view, quiet machine). Re-measure on a quiet box; if it holds, profile the Rust process for a poll that runs while nothing is open.

### Live validation, 2026-09-26 (local kind)

- **Passed live:** 002 T034 (after fixing terminal keystroke order, orphaned shells, initial PTY size and a log stop/start race), 004 T058, 021 T045, 026 T035, 039 T023, 033 T023, 030. 034 SC-004 fixed; SC-002 measured at 0.092% idle CPU.
- **Partial (the rest is covered by unit tests only):** 002 T031 (promo: Hubble and daemon scenes need those installed), 006 T036, 026 T036, 039 T024.
- **Fixed along the way:** every `helm` call is now pinned to Fathom's selected context (it previously followed the kubeconfig's current context); Helm rollback now changes the cluster; Trivy reports for Deployment pods are found.
- **031:** see the Status Summary row. **036:** T033–T036 and T052 done; T037 partial (render peaks over budget).
- **035 / 037:** validated on Linux only.
