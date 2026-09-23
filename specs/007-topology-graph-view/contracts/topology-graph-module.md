# Contract: `ui/src/topology/` module + `topology` `ToolKind`

This feature adds no new backend command/event to `src/protocol/`. Its only "contract" surface is
(a) one additive `ToolKind` enum variant shared between Rust and generated TS bindings, and (b) the
pure TS module boundary below, which other frontend code (the new `TopologyPanel`/`TopologyCanvas`
components, and tests) consumes.

## (a) Protocol addition: `ToolKind::Topology`

`src/protocol/v1/ui_config.rs`: add `Topology` to `pub enum ToolKind`. This is a purely additive
enum variant — existing serialized `LayoutProfile`/`DockConfig`/`SlotConfig` values remain valid
(no protocol version bump per Principle II, since no existing client/server pairing can produce or
depend on the absence of this variant in a breaking way). Regenerate TS bindings
(`ui/src/protocol/generated/ToolKind.ts`) via the project's existing bindings-generation step, per
Principle II ("Any other client... MUST use types generated from it").

**Round-trip test**: existing protocol serde round-trip test coverage for `ToolKind`/`SlotConfig`/
`DockConfig` already exercises the full enum via `#[test]` in `src/protocol/v1` (or wherever the
existing round-trip harness lives) — extend it to assert `Topology` round-trips, per Principle III.

## (b) `ui/src/topology/build.ts`

```ts
export function buildTopologyGraph(
  store: Pick<StoreState, 'lists' | 'details' | 'session'>,
  viewport: TopologyViewport,
): TopologyGraph
```

- **Pure function.** No side effects, no dispatch. Callers (the `TopologyCanvas` component) are
  responsible for issuing `RefreshRelatedResources`/`refreshSection` when this function reports a
  node as `loading`.
- **Deterministic** given the same `store` slice and `viewport`: same node/edge set, same order.
  (Positions are NOT part of this contract — `layout.ts` owns those.)
- **Never throws** on missing/partial data — a namespace with no fetched workload list yet simply
  contributes zero workload nodes (with `loading: true` on request, see data-model.md step 3), not
  an error.

## (b) `ui/src/topology/layout.ts`

```ts
export function layoutTopologyGraph(graph: TopologyGraph, prior?: Map<string, {x: number; y: number}>): TopologyGraph
```

- Wraps `d3-force`: runs a bounded number of simulation ticks synchronously (no async/rAF inside
  this function — the caller drives ticking on its own render loop so it can stop when idle,
  Principle V) and returns the same graph with `x`/`y` populated on every node.
- Node positions from a `prior` map seed the simulation (so re-focusing a namespace doesn't jump
  wildly) but are never required — first render works with no prior.

## (b) `ui/src/topology/status.ts`

```ts
export function nodeTone(kind: TopologyNode['kind'], row: ListRow | undefined): Tone
```

Thin wrapper over `resources/search.ts`'s existing `statusTone` (pod phase, workload
ready-vs-desired) — see research.md R6. No new status vocabulary.

## Consumers

- `ui/src/components/topology/TopologyCanvas.tsx`: calls `buildTopologyGraph` on every relevant
  store change (via a memoized selector), then `layoutTopologyGraph`, then draws to canvas; owns
  pan/zoom input handling and click hit-testing (`TopologyCanvas.utils.ts`); on node click, calls
  the existing `store.openInspector(node.ref, topologyPaneId)` — no new inspector.
- `ui/src/components/panels/TopologyPanel.tsx`: the tool-pane body (mirrors `FluxPanel.tsx`),
  wraps `TopologyCanvas` with the pane's disconnected/loading states (reusing
  `components/shell/PaneStates.tsx`'s existing `Disconnected`/`Loading` per FR-017).
