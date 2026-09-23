# Data Model: Topology Graph View

All types live in `ui/src/topology/types.ts` (frontend-only; no protocol/Rust changes). Built
purely from data already in `StoreState` — see research.md R2.

## TopologyNode

```ts
interface TopologyNode {
  id: string                 // `${kind}|${namespace ?? ''}|${name}` — stable, matches ObjectRef identity
  ref: ObjectRef              // passed to store.openInspector() unchanged on click
  kind: 'Namespace' | 'Application' | 'HelmRelease' | 'Deployment' | 'StatefulSet'
      | 'DaemonSet' | 'ReplicaSet' | 'Pod' | 'Service' | 'Node'
  label: string                // display name
  tone: 'ok' | 'warn' | 'err' | 'muted'   // from resources/search.ts statusTone (R6)
  detail?: string              // e.g. "3/3 ready" for workloads, phase for pods
  tier: 'namespace' | 'workload' | 'pod'  // which zoom tier this node belongs to
  parentId?: string             // namespace node id (workload/pod tier) or workload node id (pod tier)
  x?: number; y?: number        // layout-assigned position (mutated in place by d3-force each tick)
  loading?: boolean             // FR-015: true while this node's children are being resolved
}
```

## TopologyEdge

```ts
interface TopologyEdge {
  id: string          // sorted `${a.id}|${b.id}` — de-dup key (research.md R5)
  source: string       // TopologyNode.id
  target: string
  relations: string[]  // one or more RelatedResourceRef.relation values collapsed onto this edge
}
```

## TopologyGraph (build.ts output)

```ts
interface TopologyGraph {
  nodes: TopologyNode[]
  edges: TopologyEdge[]
}
```

Built by `buildTopologyGraph(store: StoreState, viewport: TopologyViewport): TopologyGraph`:

1. **Namespace tier** (always present): one `TopologyNode` per row in the Namespaces list
   (`s.lists[listKey(ctx, SECTION.namespaces, 'Cluster', null)]`), `tier: 'namespace'`. Edge Case
   "empty namespace" — every namespace row becomes a node even with zero edges (FR-004).
2. **Namespace → Application/Service edges**: for each namespace in `viewport.namespaceFocus`
   *or not* — the zoomed-out view only needs namespace-to-Application/Service edges (FR-004), so
   these are derived directly from the Applications and Services list rows' own `namespace` field
   (no related-resources fetch needed for this tier — namespace membership is already on the row).
3. **Workload tier**: for each namespace in `viewport.namespaceFocus`, one `TopologyNode` per
   Deployment/StatefulSet/DaemonSet row in that namespace (from the already-open or
   lazily-refreshed workload list sections), `tier: 'workload'`, `parentId` = the namespace node,
   `detail` = `"{ready}/{desired} ready"`. Application→workload edges come from that
   Application's already-fetched `RelatedResourceRef[]` (`owns`); if not yet fetched,
   `RefreshRelatedResources` is dispatched (side effect, in the React layer, not in `build.ts`
   itself — `build.ts` stays a pure function of current store state) and the node renders
   `loading: true` in the meantime (FR-015).
4. **Pod tier**: for each workload in `viewport.workloadFocus`, one `TopologyNode` per pod in that
   workload's `RelatedResourceRef[]` (`owns`, kind `Pod`) or, if already resolved via the
   Inspector's `WorkloadDetails.pods`, that cached list — `tier: 'pod'`, `parentId` = the workload
   node, `detail` = pod phase, `tone` from `statusTone(phase)`.
5. **Service edges**: Service rows in a focused namespace get a `selects` edge to each pod/workload
   their selector matches, from that Service's own `RelatedResourceRef[]` (fetched the same
   lazy way as step 3/4).
6. **Node (kube Node) tier**: kube `Node` rows never gate on zoom focus (there are typically tens,
   not thousands) — always rendered as their own small cluster of nodes, with `scheduled-on` edges
   to any pod nodes currently rendered (only pods actually on-screen at pod tier; not all pods
   cluster-wide, to keep this bounded).
7. **De-dup pass** (research.md R5): all edges from steps 2–6 are folded into a `Map` keyed by
   sorted node-id pair before being returned.
8. **Deleted-resource removal** (FR-014): a node whose backing list row is no longer present in
   the corresponding `ListState` on the next `buildTopologyGraph` call simply does not appear —
   the graph is always rebuilt fresh from current store state (no stale-node cache), so deletion
   is "free" (same mechanism as any list view already dropping a deleted row).

## TopologyViewport (per-pane, not global — research.md R3/R4)

```ts
interface TopologyViewport {
  scale: number
  x: number; y: number                  // canvas pan offset
  namespaceFocus: ReadonlySet<string>    // namespaces currently expanded to workload tier
  workloadFocus: ReadonlySet<string>     // "ns/kind/name" workloads currently expanded to pod tier
}
```

Persisted subset (`scale`, `x`, `y` only — focus sets are recomputed from viewport geometry every
render, per R3) lives on `PaneState.topology`.

## Relationship vocabulary used (subset of `RelatedResourceRef.relation`, unchanged from §5b)

`owns`, `selects`, `runs`, `scheduled-on`, `member-of`, `mounted-by` are all read; `owned-by`
(the reverse of `owns`, emitted by `related_for_pod`) is treated as the same logical edge as the
forward `owns` from the workload side and folded together by the de-dup pass (R5) — the graph
draws one edge per pair regardless of which direction produced it.

## State transitions

- **Node loading → resolved**: a workload/pod-tier node starts `loading: true` when its parent's
  `RelatedResourceRef[]` isn't yet in `s.details`; becomes `loading: false` with real children once
  the `RelatedResourcesUpdated` event lands (same event every Inspector already consumes).
- **Node present → absent**: on any store update where the backing list row disappears, the next
  `buildTopologyGraph` pass simply omits it (FR-014) — no explicit "removed" transition object is
  modeled, matching how list views already handle deletion.
- **Viewport tier transitions**: `scale` crossing `NAMESPACE_TO_WORKLOAD_THRESHOLD` for a
  namespace whose on-screen bounding box exceeds `MIN_FOCUS_PX` adds it to `namespaceFocus`;
  crossing back below removes it (workload/pod nodes for that namespace disappear, collapsing to
  the namespace summary node) — matches US2 Acceptance Scenario 3 (re-zooming recomputes from
  live data, no persisted expansion state).
