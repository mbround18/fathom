# Research: Knative Pane

## R1: CRD detection pattern

**Decision**: Mirror `src/backend/flux.rs`'s `find_served`/`list_instances` dynamic-object
discovery exactly, targeting `services.serving.knative.dev`, `revisions.serving.knative.dev`,
`routes.serving.knative.dev`, `configurations.serving.knative.dev`. `installed` is true iff at
least one of these CRDs is found served (matches FR-001's "checking for the CRD group" and the
edge case of partial registration — detection is per-kind, `installed` is an OR).

**Rationale**: Same underlying mechanism (list `CustomResourceDefinition`, find a served version,
read instances as `DynamicObject`) works for any optional CRD-based integration without pinning
an API version; the project already has two examples of this pattern (Flux, Longhorn per the
comment in `flux.rs`).

**Alternatives considered**: Compile-time typed CRD structs (`kube::CustomResource` derive) —
rejected, same reason the project rejected it for Flux: version skew across Knative releases,
and the project's own comment in `flux.rs` explicitly says dynamic discovery is used "so this
works across Flux versions without pinning an API version."

## R2: Traffic-split source of truth

**Decision**: Read `status.traffic` on the Route object (list of `{revisionName |
configurationName, percent, latestRevision, tag}`), not `spec.traffic`, since status reflects
what is actually being served (spec can be pending reconciliation).

**Rationale**: Knative's Route status always resolves `configurationName`/`latestRevision`
targets down to a concrete `revisionName` in `status.traffic`, so the visualization never needs
to itself resolve "latest ready revision" — it just reads the already-resolved status list. This
also naturally produces the "stale target" edge case (FR-007): if `status.traffic[].revisionName`
names a Revision that no longer exists in the cluster's Revision list, the resolver flags it
instead of dropping it.

**Alternatives considered**: Reading `spec.traffic` and resolving Configuration → latest-ready
Revision ourselves — rejected as duplicate logic that Knative's own controller already does more
reliably; would also drift from what's actually serving traffic during a rollout.

## R3: Autoscaler "why" derivation

**Decision**: Derive the explanation from three sources, in priority order, all read from the
Revision (and, if present, its status conditions) rather than requiring a separate
`PodAutoscaler` CRD read:
1. `status.conditions[type=Active]` — `status: False, reason: NoTraffic` (or `reason:
   "Inactive"/message` mentioning idle) → "scaled to zero: no traffic in the last N (unknown)
   period" using `lastTransitionTime` on that condition as the point it went inactive if present.
2. `status.conditions[type=Active]` — `status: Unknown, reason: "ActivatingRevision"` — or a
   pending-request signal if the cluster's Knative Pod Autoscaler exposes it — → "scaling up:
   activating".
3. `status.actualReplicas` vs `status.desiredReplicas` (Knative's Revision status carries these
   directly in modern versions) — if actual == desired → "at desired scale"; else if we could not
   classify a specific reason from (1)/(2) → generic "reason unavailable" with the raw numbers.

**Rationale**: Spec Assumptions already say the exact pending-request count is "when available" —
the `Active` condition's reason/message is the portable signal across Knative versions; a
dedicated `podautoscalers.autoscaling.internal.knative.dev` read is optional best-effort (attempt
it, but never block or error the pane if that CRD isn't served/readable, since it's an internal
resource not always RBAC-exposed to read-only viewers).

**Alternatives considered**: Requiring the `PodAutoscaler` object for every explanation — rejected
because it's an internal-group CRD that may not be watchable under a typical read-only
ClusterRole, and the spec's FR-010 explicitly requires a graceful "reason unavailable" fallback
rather than a hard dependency.

## R4: Extending the topology component vs. a new renderer

**Decision**: Extend `ui/src/topology/types.ts` (`TopologyEdge` gets an optional `label?: string`
and an optional `weight?: number` for potential future styling; `TopologyNodeKind` gets
`'KnativeRevision' | 'KnativeRoute'`), extend `ui/src/topology/build.ts` with a
`buildKnativeTrafficGraph` builder that produces a small `TopologyGraph` (Route node → N Revision
nodes) fed from the `KnativeOverview` store slice (not from `lists`/`details` like the main
cluster graph, since Knative data arrives via its own poll, not the generic resource-list
machinery), and extend `TopologyCanvas.tsx`'s `draw()` to render `edge.label` at the edge midpoint
when set. The Knative pane renders a `<TopologyCanvas>`-shaped small canvas seeded with this
graph, reusing `layoutTopologyGraph`/pan-zoom/hit-test utilities as-is.

**Rationale**: The task explicitly requires reusing spec 007's component instead of building a new
graph renderer. The existing `TopologyGraph`/`TopologyNode`/`TopologyEdge` shapes and the
canvas-based force layout are generic enough to host a second, independently-fed small graph;
the only genuinely missing capability is an edge label, which is a minimal, additive draw-loop
change (a few lines in the existing `for (const e of laidOut.edges)` loop) rather than a
rewrite.

**Alternatives considered**: A bespoke SVG/HTML traffic-split bar or Sankey-style widget —
rejected per explicit instruction to reuse the topology component; would also duplicate pan/zoom/
node-click-to-inspect behavior the existing component already provides for free.

## R5: Polling vs. watch

**Decision**: Mirror Flux exactly — a 20s background poll (`ensure_knative_poll` /
`poll_knative_overview`, keyed off `state.watch_tasks`) plus an immediate `RefreshKnative` sent
when the pane opens (same as `FluxPanel`'s `useEffect(() => refreshFlux(), [refreshFlux])`),
rather than a live `kube-rs` watch stream. FR-012 ("update without a manual full-app refresh") is
satisfied by the poll interval, consistent with how Flux already satisfies the same requirement
for its own panel.

**Rationale**: Consistency with the existing, already-approved integration pattern; a poll is
simpler than a multiplexed watch across 4 CRD kinds with unknown/varying API versions, and Flux's
own poll-not-watch choice for a structurally identical problem (multiple optional CRDs) is direct
precedent already accepted into this codebase.

**Alternatives considered**: A `kube-rs` `watcher()` stream per Knative kind — rejected as
inconsistent with the Flux precedent this feature is explicitly told to mirror, and unnecessary
complexity for a read-only diagnostic pane.
