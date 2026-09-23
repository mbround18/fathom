# Feature Specification: Topology Graph View

**Feature Branch**: `007-topology-graph-view`

**Created**: 2026-09-22

**Status**: Draft

**Input**: User description: "Build the Topology Graph View (docs/MVP.md §5c) on top of the generic related-resources model shipped in §5b (004-applications-interlinking, extended in 005-helm-release-origin-linking): a block-and-line diagram of the cluster that lets the operator zoom continuously between namespace-level and pod-level detail, instead of only drilling through the existing list/inspector chain. Nodes in the graph are Kubernetes objects (Namespace, Application/Helm-release, Workload, Pod, Service, Node) or CRD instances; edges are the same ownership/selector relationships modeled in §5b. Zoom levels, not separate pages. Clicking any node opens that resource's existing inspector. Live-updating from the same streamed store data the list views use. Automatic layout only. Non-goals: graph-driven mutation, custom layout persistence, Hubble traffic edges, full CRD-instance support beyond what the related-resources resolver already handles."

## User Scenarios & Testing *(mandatory)*

<!--
  IMPORTANT: User stories should be PRIORITIZED as user journeys ordered by importance.
  Each user story/journey must be INDEPENDENTLY TESTABLE - meaning if you implement just ONE of them,
  you should still have a viable MVP (Minimum Viable Product) that delivers value.
-->

### User Story 1 - See the cluster shape at a glance (Priority: P1)

An operator who has just connected to an unfamiliar cluster (or is checking in on a familiar one after time away) opens the Topology Graph View and sees, without drilling through any list, a namespace-level map of the cluster: namespaces as nodes, with edges showing which namespaces contain which Applications/Helm releases and which Services front which namespaces' workloads.

**Why this priority**: This is the view's entire reason to exist — the MVP.md problem statement is that every resource kind is its own list/inspector island, and an operator can't currently see cluster shape without mentally stitching together several list views. A namespace-level overview delivers that value standing alone, even before any zoom/inspector integration is built.

**Independent Test**: Can be fully tested by opening the Topology Graph View against a cluster/fake-backend fixture with several namespaces, some containing Applications/Helm releases and Services, and asserting the rendered graph shows one node per namespace with correctly-drawn ownership/selector edges to the Applications/Services within it — no zooming or click-through required.

**Acceptance Scenarios**:

1. **Given** a cluster with 5 namespaces, 3 of which contain at least one Application or Helm release, **When** the operator opens the Topology Graph View, **Then** the initial (zoomed-out) view shows 5 namespace nodes, with edges only to the 3 namespaces that actually contain an Application/Helm release.
2. **Given** the zoomed-out view is showing, **When** a new namespace is created in the connected cluster while the view remains open, **Then** the new namespace's node appears in the graph without the operator reloading or reopening the view.
3. **Given** the zoomed-out view is showing, **When** the operator has not interacted with the graph for a while, **Then** the view consumes no meaningful CPU beyond re-rendering the specific nodes/edges whose underlying data actually changed (no continuous redraw loop), consistent with the app's idle-CPU expectations elsewhere.

---

### User Story 2 - Zoom from namespace down to pod without losing place (Priority: P1)

An operator investigating a namespace-level warning (e.g. a Service with no healthy backends) zooms continuously into that namespace, watching workloads and then individual pods progressively appear, without the view reloading or the operator having to reopen a separate page/route for each level of detail.

**Why this priority**: Continuous zoom (vs. separate pages per level) is the specific interaction MVP.md calls out as the point of a topology view over the existing list/inspector chain — it's what makes this P1 alongside User Story 1, not a follow-on enhancement.

**Independent Test**: Can be fully tested by opening the graph at the zoomed-out level, driving a zoom gesture/control toward one namespace, and asserting that workload nodes appear once the zoom threshold is crossed, then pod nodes appear once zoomed further into a single workload, with the same graph instance (no navigation/route event) throughout.

**Acceptance Scenarios**:

1. **Given** the zoomed-out namespace view, **When** the operator zooms into a single namespace, **Then** that namespace's workload nodes (Deployments/StatefulSets/DaemonSets) progressively appear, and namespaces the operator did not zoom into remain collapsed to their namespace-level summary node.
2. **Given** a namespace zoomed in to workload level, **When** the operator zooms further into a single workload, **Then** that workload's individual pod nodes appear, each showing its current phase.
3. **Given** the operator is zoomed to pod level inside one namespace, **When** the operator zooms back out, **Then** the view returns to the namespace-level overview without a full reload, and previously-expanded state is not required to persist (re-zooming in recomputes the expanded detail from current live data).
4. **Given** a namespace with a very large pod count (e.g. 500+ pods for one workload), **When** the operator zooms to pod level for that workload, **Then** individual pod nodes remain distinguishable and the view stays responsive to further zoom/pan input (per SC-002).

---

### User Story 3 - Jump from a graph node into its full inspector (Priority: P2)

An operator who has spotted a suspicious-looking node while exploring the graph (e.g. a pod rendered in an unhealthy color) clicks it and lands in the same Details/YAML/Events inspector already used everywhere else in the app, then uses the existing back-stack to return to the graph.

**Why this priority**: This is what makes the graph a navigation surface rather than a dead-end picture — MVP.md is explicit that node clicks must reuse the existing inspector rather than duplicate it. It's sequenced after Stories 1-2 because it depends on the graph already rendering real nodes at both zoom levels, but the graph still delivers standalone "see cluster shape" value without it.

**Independent Test**: Can be fully tested by rendering the graph down to pod level, clicking a pod node, and asserting the existing Inspector pane opens with that pod as its target (same pane/back-stack mechanism used by every other related-resource link in the app), then asserting a back action returns to the graph in its prior zoom/pan state.

**Acceptance Scenarios**:

1. **Given** the graph is zoomed to show a specific Service node, **When** the operator clicks that node, **Then** the Service's existing inspector opens (Details/YAML/Events), identical to opening it from the Services list.
2. **Given** the operator opened a pod's inspector by clicking its graph node, **When** the operator uses the inspector's back navigation, **Then** they return to the topology graph at the same zoom level and pan position they left it at, not to the zoomed-out default.
3. **Given** the operator is inside a resource's inspector reached via the graph, **When** that inspector shows further related-resource links (as it already does today per §5b), **Then** clicking one of those links continues to behave exactly as it does when the inspector was opened from a list — the graph does not alter existing drill-down behavior.

---

### User Story 4 - Trust that graph health state is live, not stale (Priority: P3)

An operator leaves the Topology Graph View open on a secondary monitor while working elsewhere, glancing at it periodically to catch a workload's replica count dropping or a pod flipping to a failed phase, the same way they would trust a live list view.

**Why this priority**: Live-updating is a stated requirement in MVP.md, but it is additive polish over Stories 1-3 (which already require the graph to read from live store data at least at initial render) — this story is specifically about *sustained* correctness over time without any separate polling path, which is lower risk to get wrong in a way that blocks the other stories from shipping.

**Independent Test**: Can be fully tested by rendering the graph with a workload at N/N ready replicas, then driving a simulated pod failure through the fake backend, and asserting the corresponding pod node's color/status and the workload's replica-count label update in place, without the operator refreshing or reopening the view, and without any new network/poll request being issued solely to produce that update.

**Acceptance Scenarios**:

1. **Given** the graph is showing a Deployment at 3/3 ready replicas, **When** one of its pods transitions to a failed phase (as already reflected in the Pods list's live data), **Then** the graph's workload node updates its ready-count label and the corresponding pod node's status color, within the same latency the existing list views already achieve for the same underlying update.
2. **Given** the graph view is open, **When** the operator opens no other view and takes no action, **Then** no new backend command is issued by the graph beyond what would already be issued by the equivalent list/detail views for the resources currently expanded in the graph.

---

### Edge Cases

- What happens when a namespace has zero Applications, Services, or workloads (an effectively-empty namespace, e.g. `kube-node-lease`)? It MUST still render as a namespace node with no outgoing edges, not be silently omitted — omission would misrepresent cluster shape, which is the view's whole purpose.
- What happens when the operator zooms into a workload whose pods are still being resolved (e.g. the related-resources lookup for that workload hasn't returned yet)? The workload node MUST show a loading affordance at the pod level rather than rendering zero pods as if the workload truly had none.
- What happens when a resource shown in the graph is deleted from the cluster while the operator is looking at it? Its node MUST be removed (or marked gone, consistent with how the existing Inspector's `NoLongerExists` state is presented elsewhere) rather than left stale/unclickable.
- What happens for a resource kind the related-resources resolver does not yet cover relationships for (e.g. a CRD instance, or `ServiceAccount`, per `related.rs`'s own documented gaps)? It MUST simply not appear as a graph node/edge in v1, consistent with the CRD non-goal below — this is a known, documented scope limit, not a rendering error to fail loudly on.
- What happens when the operator is on a narrow (phone-width) viewport? The graph MUST remain usable (pan/zoom/tap-to-inspect) rather than being hidden or degraded to a non-functional placeholder, consistent with the project's mobile-first density principle — though the amount of simultaneously-visible detail may reasonably be lower than on a wide viewport.
- What happens when the connected cluster has no live connection (backend disconnected)? The graph MUST show the same disconnected-state affordance already used by other live views, not a frozen or blank canvas presented as current.
- What happens when two different relationship sources would draw a redundant edge between the same pair of nodes (e.g. a pod is both `owned-by` its ReplicaSet and reachable transitively via `Application owns Deployment`)? The graph MUST de-duplicate to a single edge per node pair rather than drawing overlapping/duplicate lines.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST provide a Topology Graph View that renders Kubernetes objects — Namespace, Application/Helm release, Workload (Deployment/StatefulSet/DaemonSet/ReplicaSet), Pod, Service, and Node — as graph nodes.
- **FR-002**: System MUST draw graph edges using the same ownership/selector relationship vocabulary already produced by the existing related-resources resolver (`src/backend/related.rs`) — no second, independently-maintained relationship model may be introduced for the graph.
- **FR-003**: System MUST support continuous zoom between a namespace-level overview and pod-level detail within a single graph view — distinct levels of detail MUST NOT be implemented as separate pages/routes that reload the view.
- **FR-004**: At the zoomed-out level, System MUST show namespace-to-namespace and namespace-to-Service/Application ownership relationships; namespaces with no such relationships MUST still render as nodes.
- **FR-005**: As the operator zooms into a namespace, System MUST progressively reveal that namespace's workload nodes, and, on zooming further into a workload, that workload's individual pod nodes — without requiring a full view reload at either transition.
- **FR-006**: System MUST let the operator click any graph node to open that resource's existing inspector (the same Details/YAML/Events pane and pane/back-stack mechanism used by every other entry point into the inspector), and MUST NOT introduce a second, graph-specific inspector.
- **FR-007**: System MUST return the operator to the topology graph, at the zoom level and pan position they left it at, when they navigate back from an inspector opened via a graph node click.
- **FR-008**: Graph node state that reflects live cluster data (health/status color, replica/ready counts, pod phase) MUST be derived from the same streamed store data already populating the app's existing list views, and MUST update in place as that underlying data changes, without the graph view issuing a separate polling loop for data already available in the store.
- **FR-009**: System MUST lay out graph nodes automatically (force-directed or hierarchical); the operator MUST NOT be required or able to manually author node positions to get a usable layout.
- **FR-010**: System MUST NOT let the operator mutate cluster state by manipulating the graph (e.g. drag-to-reparent, delete-by-removing-a-node); any mutating action reachable from a graph-opened inspector MUST go through the app's existing confirmation-gated mutation flows unchanged.
- **FR-011**: System MUST NOT persist a manually-arranged/custom graph layout across sessions in v1; layout is recomputed automatically each time the graph is rendered.
- **FR-012**: System MUST remain interactive (pan, zoom, and node click-through continue to respond) at a scale of at least several thousand pods in a single cluster, per SC-002, by rendering full per-node detail only for the namespace(s)/workload(s) currently in or near the viewport and collapsing out-of-view detail to aggregate counts.
- **FR-013**: System MUST de-duplicate edges so that at most one edge is drawn between any two given nodes, even when more than one relationship path in the related-resources data would otherwise produce a redundant edge between the same pair.
- **FR-014**: System MUST remove (or mark as gone, consistent with the existing Inspector's not-found state) a node for a resource that is deleted from the cluster while shown in the graph, rather than leaving a stale, non-representative node in place.
- **FR-015**: System MUST show a loading affordance for a node's not-yet-resolved child detail (e.g. a workload's pods before its related-resources lookup returns) rather than rendering as if no children exist.
- **FR-016**: System MUST remain usable — pannable, zoomable, and tappable through to a node's inspector — at phone-width viewports, consistent with the app's mobile-first density expectations, though the volume of simultaneously-rendered detail MAY be lower than at wider viewports.
- **FR-017**: System MUST show the same disconnected/backend-unavailable affordance already used elsewhere in the app when the cluster connection is lost while the graph is open, rather than presenting a frozen graph as current.
- **FR-018**: System MUST NOT render nodes or edges for resource kinds and relationships the related-resources resolver does not currently cover (e.g. CRD instances, `ServiceAccount`) — these are silently absent in v1, not an error state, consistent with `related.rs`'s own documented scope limits.
- **FR-019**: System MUST NOT render real network-traffic edges (e.g. from the Hubble/Cilium integration) in v1 — only ownership/selector relationships already modeled in §5b.

### Key Entities

- **Topology Node**: A graph-rendered representation of one Kubernetes object already browsable elsewhere in the app (Namespace, Application/Helm release, Workload, Pod, Service, or Node) — carries enough identity (kind, namespace, name) to resolve to the same `ObjectRef` the existing Inspector pane already accepts, plus a live-derived status/health indicator and, for workloads, a ready/replica count.
- **Topology Edge**: A directed relationship between two Topology Nodes, sourced one-to-one from the existing `RelatedResourceRef` relation vocabulary (`owns`, `selects`, `runs`, `scheduled-on`, `member-of`, `mounted-by`, `reconciled-by`, etc.) — the graph introduces no new relation kinds beyond what §5b already models.
- **Zoom Level**: The graph's current level of detail (namespace overview, workload detail, or pod detail), scoped per namespace/workload currently in or near the viewport — not a single global mode, since different regions of a large graph may be at different effective detail levels simultaneously.
- **Viewport State**: The operator's current pan position and zoom level within the graph, preserved across a round trip to an inspector opened by clicking a node, so returning via back-navigation restores the same view rather than resetting to the default overview.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator unfamiliar with a cluster can identify, from the initial zoomed-out graph alone (no clicks), which namespaces contain running workloads within 10 seconds of the view opening.
- **SC-002**: With a cluster containing at least 2,000 pods across all namespaces, panning and zooming the graph and clicking a node to open its inspector each remain responsive to operator input — no interaction feels delayed — consistent with the app's existing responsiveness standard for large, continuously-updating lists.
- **SC-003**: An operator can go from the zoomed-out namespace overview to a specific unhealthy pod's full inspector (Details/YAML/Events) in three or fewer direct interactions (zoom-in, zoom-in, click), without leaving the graph view in between.
- **SC-004**: 100% of graph node clicks open the same inspector the equivalent list-view row would open for that resource — zero graph-specific alternate inspector surfaces exist.
- **SC-005**: A live cluster state change already visible in an existing list view (e.g. a pod's phase change) becomes visible in the graph's corresponding node within the same order of latency as it becomes visible in that list view — an operator watching both side by side does not perceive the graph as lagging behind.
- **SC-006**: An idle, open Topology Graph View (no operator interaction, no underlying cluster state change) does not measurably increase the app's idle CPU usage beyond the baseline already established for an idle list view of comparable node count.
- **SC-007**: An operator can return from an inspector opened via the graph back to the graph and find it at the same zoom/pan position they left, on 100% of back-navigations.

## Assumptions

- **Rendering approach**: The graph renders on an HTML canvas (not SVG-per-node), with level-of-detail culling so that only the namespace(s)/workload(s) currently in or near the viewport render full pod-level detail; out-of-view regions collapse to aggregate summary nodes. This is chosen over a naive SVG-per-node approach specifically because MVP.md flags SVG-per-node as likely too slow at pod scale, and canvas rendering with viewport-based culling is the standard technique for keeping thousands of interactive graph elements responsive. WebGL is not required for v1's target scale (thousands, not hundreds of thousands, of nodes).
- **Graph computation location**: The graph is computed client-side from data already streamed into the frontend store for existing list/detail views, plus `RelatedResourceRef` edges already fetched through the existing related-resources dispatch (`RefreshRelatedResources`) — no new backend "topology snapshot" aggregate event is introduced for v1. This keeps relationship modeling in the one place it already lives (`src/backend/related.rs`) and avoids the backend growing a second, graph-specific view of the same data, consistent with Principle II's "one contract" stance. If a future iteration finds it needs edges for resources not present in any currently-open list (e.g. building the full graph before any list has been viewed), that is out of scope for v1 and would require its own versioned protocol addition, not a workaround here.
- **CRD instance nodes**: Full CRD-instance support (nodes/edges for arbitrary custom resources) is out of scope for v1, matching a documented, pre-existing limitation of the related-resources resolver itself (`RelatedResourceRef` carries no `group` field, so a CRD `Kind` name cannot be disambiguated across API groups today). Extending the resolver to carry a `group` field and support CRD relationships is future work, tracked as a dependency for a future iteration of this view, not solved here.
- **Traffic edges**: Real network-traffic edges from the already-shipped Hubble/Cilium integration are explicitly out of scope for v1, per MVP.md — noted as a natural future extension point (the same edge-drawing mechanism would apply once real flow data is available) but not built now.
- **Namespace scope**: The graph reflects the same cluster/context scope the rest of the app is currently connected to; it does not introduce a separate multi-cluster or multi-context aggregation capability.
- **Existing inspector, unchanged**: No changes to the Inspector pane's own behavior, tabs, or mutation flows are required by this feature — the graph is purely a new entry point into it.
