# Research: Topology Graph View

## R1: Canvas + force-layout library choice

**Decision**: Add `d3-force` (and its small peer `d3-quadtree`, pulled in transitively) as the
only new npm dependency. Render with a plain `<canvas>` element and hand-rolled draw calls; no
scene-graph or WebGL library.

**Rationale**: `ui/package.json` has no existing graph-layout or canvas-scene library (checked:
no `d3`, `cytoscape`, `reactflow`, `sigma`, `pixi`, `elkjs`, `dagre`). `d3-force` is a
headless physics simulation (nodes/links in, `{x, y}` out) with zero DOM/SVG coupling, so it
composes cleanly with a hand-rolled canvas renderer instead of fighting an opinionated
SVG-per-node library like `react-flow`/`cytoscape` — both of which the spec's own Assumptions
section flags as too slow at pod scale. It's ~24 KB min+gzip, has no runtime dependencies beyond
other `d3-*` micro-packages, and is a mature, widely-used building block (not a full framework),
which fits the constitution's "dependencies SHOULD be stable releases" guidance.

**Alternatives considered**:
- `react-flow`/`cytoscape.js`: full SVG/DOM-per-node graph frameworks. Rejected — exactly what
  the spec's Assumptions section says is too slow at pod scale (thousands of nodes), and each
  pulls in much more than we need (their own pan/zoom, own node renderer, own layout engine) when
  we already need a custom canvas renderer for LOD culling.
- `sigma.js`/`pixi.js` (WebGL): rejected — spec.md Assumptions explicitly says WebGL is not
  required for v1's target scale (thousands, not hundreds of thousands, of nodes); adds a much
  heavier dependency and shader-debugging surface for no scale benefit at this size.
- Hand-rolled layout (no library): rejected — force-directed layout (repulsion + link springs +
  centering) has enough subtlety (tick convergence, collision avoidance) that reimplementing it
  is more risk than a well-tested ~24 KB headless module.

## R2: Graph computation location (confirming spec.md's Assumption)

**Decision**: Confirmed — compute entirely client-side in a new pure `ui/src/topology/` module,
from data already in the Zustand store: list rows (`s.lists[...]` for Namespace, Pod, Service,
Node, Deployment/StatefulSet/DaemonSet, Application, HelmRelease sections) plus
`RelatedResourceRef`s already fetched per-object via the existing `RefreshRelatedResources`
command (`s.details[detailKey(ctx, 'RelatedResources', ns, name)]`).

**Rationale**: `src/backend/related.rs` is already the one relationship resolver (Principle II,
"one contract"). A new backend "topology snapshot" aggregate event would either duplicate that
resolver's logic or wrap it in a second protocol shape — both against the constitution's "no
second, independently-maintained relationship model" framing (FR-002) and "one contract" stance.
Because list rows are already streamed continuously for open list panes, and
`RefreshRelatedResources` is already dispatched by the Inspector for any object the operator has
looked at, the graph reuses exactly that data with no new network path (FR-008, SC-006). The one
consequence (documented as already-accepted scope in spec.md): the graph can only show detail for
resources whose data has actually been fetched into the store — namespace-level summary data
(from the Namespaces list, always cheap) is fetched eagerly when the topology pane mounts;
workload/pod detail for a given namespace is fetched on-demand only once the operator zooms into
it (mirroring how the Inspector already lazily fetches related resources on demand), not
speculatively for the whole cluster up front.

**Alternatives considered**: A new `RefreshTopologySnapshot` backend command returning a full
graph — rejected per the above; also would need its own polling/refresh cadence, duplicating the
live-update mechanism the list/detail stream already provides for free.

## R3: Level-of-detail / zoom-level model

**Decision**: "Zoom" is modeled as a continuous camera scale (pan/zoom transform on the canvas,
driven by wheel/pinch/drag) plus a small set of **per-namespace/per-workload focus flags** stored
in the topology pane's own viewport state (not global): `namespaceFocus: Set<namespace>` and
`workloadFocus: Set<"ns/kind/name">`. Crossing a camera-scale threshold while a namespace's
screen-space bounding box exceeds a size threshold adds that namespace to `namespaceFocus`
(revealing its workload nodes); crossing a further threshold on one workload adds it to
`workloadFocus` (revealing its pod nodes). Zooming back out only shrinks the camera scale; focus
sets are recomputed from the current scale/viewport every frame (not persisted), so "zoom back out
then back in" naturally recomputes detail from live data (Acceptance Scenario US2.3) rather than
caching stale expansion state.

**Rationale**: This keeps "zoom" as one continuous gesture (FR-003) while giving the renderer
discrete tiers to decide what to draw and what data to fetch, without a separate
route/page/view-mode per tier. It directly implements the spec's own "Zoom Level... scoped per
namespace/workload currently in or near the viewport... not a single global mode" Key Entity.

**Alternatives considered**: A single global `zoomLevel: 'namespace' | 'workload' | 'pod'` enum —
rejected, spec.md's own Key Entities section rules this out explicitly (different regions of a
large graph may be at different detail levels simultaneously, per US2 Acceptance Scenario 1's
"namespaces the operator did not zoom into remain collapsed").

## R4: Viewport-state persistence across inspector round-trips (FR-007)

**Decision**: Add an optional `topology?: { scale: number; x: number; y: number }` field to
`PaneState` (alongside the existing `scrollTop`), updated via the existing `updatePane` action on
pan/zoom, exactly mirroring how list panes already persist `scrollTop`/`sort`/`search` across
re-layout. No new store slice, no new persistence mechanism.

**Rationale**: The pane already survives unmount/remount (PaneHost's own doc comment: "so the host
can unmount on re-layout and come back intact"); clicking a node calls the existing
`openInspector(ref, topologyPaneId)`, which does not remove/reset the topology pane — it only
changes which pane occupies the inspector slot/stack entry. The topology pane's own React
component reads its last `scale/x/y` from `PaneState` on mount (same pattern `PaneHost`'s
`useLayoutEffect` uses for `scrollTop`), so no explicit "return to graph" plumbing is needed beyond
normal pane-stack navigation.

**Alternatives considered**: A separate global "last topology viewport" singleton — rejected,
inconsistent with the existing per-pane state model and would not support multiple topology panes
open at once (e.g. wide/ultra widths with more than one tool slot).

## R5: Node/edge de-duplication (FR-013)

**Decision**: Build edges into a `Map<string, TopologyEdge>` keyed by
`` `${sourceKey}|${targetKey}` `` (undirected pair, keys sorted) before rendering; the first
relation seen for a pair wins (relations are additive metadata on the edge, not separate edges) —
concretely, if both `Application --owns--> Deployment` and (redundantly) `Deployment
--owned-by--> Application` appear from different resolver calls, they collapse to one edge object
carrying both relation labels rather than two overlapping lines.

**Rationale**: Directly implements FR-013 and the spec's "redundant edge" Edge Case. A `Map` keyed
by sorted node-id pair is the simplest correct de-dup structure and is trivially unit-testable
with fixture `RelatedResourceRef[]` input (Principle III: "UI logic that decides what the operator
sees... MUST have unit tests").

## R6: Status/health color derivation

**Decision**: Reuse existing status-derivation logic already used by list views (pod phase colors,
workload ready/replica comparison) rather than inventing a parallel color scheme — a thin
`ui/src/topology/status.ts` wraps whatever the existing `resources/` descriptors already expose
for pod phase and workload readiness (checked: `resources/format.ts` and per-kind descriptors
already classify pod phase for the Pods list's row styling), so graph node colors and list-view
row colors never drift apart.

**Rationale**: Constitution Principle I ("show what's happening") and avoiding a second,
independently-maintained notion of "healthy" alongside the one the list views already use.
