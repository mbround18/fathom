# Implementation Plan: Topology Graph View

**Branch**: `007-topology-graph-view` | **Date**: 2026-09-22 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/007-topology-graph-view/spec.md`

## Summary

A new "Topology" tool pane that renders a canvas-based, zoomable block-and-line diagram of the
cluster: namespaces at the zoomed-out level, expanding to workloads and then pods as the operator
zooms into a namespace/workload. Nodes and edges are derived entirely client-side from data
already in the Zustand store (list rows already fetched for existing list panes, plus
`RelatedResourceRef`s already fetched via the existing `RefreshRelatedResources` command) — no new
backend command or event is introduced. Clicking a node opens the existing Inspector pane via the
existing `openInspector` action; the graph pane's own zoom/pan state lives in `PaneState`
(mirroring how `scrollTop` already survives re-layout) so returning from the inspector restores
the same view.

## Technical Context

**Language/Version**: Rust 2024 edition (no backend changes required for v1); TypeScript 5 strict
(frontend, all new code)

**Primary Dependencies**: React 19, Zustand 5, existing `@/store`, `@/resources`,
`@/protocol/commands`; **new**: `d3-force` (force-directed layout physics only — no DOM/SVG
coupling, so it composes with our own canvas renderer) — see research.md for the "why this, not
X" tradeoff since `ui/package.json` has no existing graph-layout/canvas library.

**Storage**: N/A (no persistence; layout recomputed each render per FR-011)

**Testing**: `cargo test` (n/a new coverage — no backend logic changes, existing `related.rs`
tests stay green), Vitest (unit tests for the pure graph-building/layout-selection module),
Playwright (`ui/tests/e2e/us{1..4}-topology-*.spec.ts` against the fake backend)

**Target Platform**: Desktop (Tauri v2, Tier 1 platforms), mobile-width web layout per Principle IV

**Project Type**: Desktop app (existing Tauri + React monorepo; this feature is frontend-only)

**Performance Goals**: Interactive pan/zoom/click at 2,000+ pods (SC-002); idle CPU budget
unaffected when the pane isn't visible/mounted (Principle V, SC-006)

**Constraints**: Canvas rendering with viewport/LOD culling (per spec.md Assumptions); no new
backend aggregate event; no new relationship model; automatic layout only (FR-009, FR-011)

**Scale/Scope**: One new tool pane, one new pure data-transform module, no protocol changes

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design (see below).*

- **I. Explicit Mutation**: PASS. The graph is read-only; FR-010 explicitly forbids any
  graph-driven mutation. No new mutating command is introduced.
- **II. One Contract, Backend Owns the Cluster**: PASS. No new protocol command/event. The graph
  reads only from data the backend already streamed for existing list/detail panes and the
  existing `RefreshRelatedResources` command/event pair. The frontend does not talk to the cluster
  directly; it derives a client-side *view* over already-fetched data, which is explicitly
  sanctioned by spec.md's "Graph computation location" assumption.
- **III. Tested at the Boundaries**: PASS (planned). No new protocol variants, so no new
  round-trip serde tests are needed. New UI logic that decides what's shown (graph node/edge
  derivation, LOD/zoom-level selection) gets unit tests (this is exactly the "layout selection"
  category the constitution calls out). Every user story gets a Playwright e2e spec against the
  fake backend.
- **IV. Density Scales With Width**: PASS (planned). The topology pane is a tool pane like
  `metrics`/`flux`/`hubble`, participating in the same responsive slot/dock/stack layout; FR-016
  requires it stay usable at phone width.
- **V. Fast and Quiet**: PASS (planned), with a specific design commitment: canvas rendering (not
  SVG-per-node), viewport-based LOD culling so only in/near-view namespaces render pod-level
  detail, `requestAnimationFrame`-gated redraw only on actual store changes or user
  pan/zoom/click input (no continuous redraw loop) to hold the idle-CPU budget (SC-006).
- **VI. User Data Is Sacred**: PASS (N/A). No `~/.fathom/` state, no persisted layout (FR-011).

No violations; Complexity Tracking table is empty.

## Project Structure

### Documentation (this feature)

```text
specs/007-topology-graph-view/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── topology-graph-module.md
└── tasks.md
```

### Source Code (repository root)

```text
ui/src/
├── components/
│   ├── panels/
│   │   └── TopologyPanel.tsx           # new: tool-pane body, mirrors FluxPanel.tsx/HubblePanel.tsx
│   └── shell/PaneHost.tsx              # edit: dispatch pane.tool === 'topology'
├── topology/                            # new module: pure, unit-testable, no React/canvas coupling
│   ├── build.ts                         # store state -> TopologyGraph (nodes/edges), dedup (FR-013)
│   ├── layout.ts                        # d3-force wrapper: positions per zoom level
│   ├── types.ts                         # TopologyNode, TopologyEdge, ZoomFocus
│   └── status.ts                        # node health/status-color derivation
├── components/topology/
│   ├── TopologyCanvas.tsx               # canvas element: render loop, pan/zoom/click hit-testing
│   └── TopologyCanvas.utils.ts          # hit-testing, viewport math (unit-testable)
└── store/index.ts                       # edit: PaneState gains optional `topology` viewport field
                                          # edit: newPane/claimPane recognize 'topology' ToolKind

ui/tests/unit/topology/                  # Vitest: build.ts, layout focus transitions, status colors
ui/tests/e2e/
├── us1-topology-overview.spec.ts
├── us2-topology-zoom.spec.ts
├── us3-topology-inspector.spec.ts
└── us4-topology-live-update.spec.ts

docs/MVP.md                              # status row update after implementation
```

**Structure Decision**: Frontend-only feature inside the existing `ui/` React app. No new Rust
modules; `src/backend/related.rs` and the protocol are read, not modified. New pure-TS graph logic
lives outside `components/` (like other derivation modules) so it can be unit-tested without
mounting React or canvas.

## Complexity Tracking

*No entries — no constitutional violations.*
