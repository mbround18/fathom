# Tasks: Topology Graph View

Feature dir: `specs/007-topology-graph-view/` | Branch: `007-topology-graph-view`

## Phase 0: Setup

- [X] T001 Add `d3-force` dependency to `ui/package.json`, `pnpm -C ui install`
- [X] T002 [P] Add `Topology` variant to `pub enum ToolKind` in `src/protocol/v1/ui_config.rs`;
      extend its round-trip test in the same module
- [X] T003 [P] Regenerate/hand-sync `ui/src/protocol/generated/ToolKind.ts` to include `"topology"`

## Phase 1: Foundational (blocks all user stories)

- [X] T010 Create `ui/src/topology/types.ts` (`TopologyNode`, `TopologyEdge`, `TopologyGraph`,
      `TopologyViewport`) per data-model.md
- [X] T011 Create `ui/src/topology/status.ts` (`nodeTone`) wrapping `resources/search.ts`'s
      `statusTone` per research.md R6
- [X] T012 Create `ui/src/topology/build.ts` (`buildTopologyGraph`) per data-model.md steps 1-8,
      including the de-dup pass (research.md R5)
- [X] T013 [P] Unit tests `ui/tests/unit/topology/build.test.ts`: namespace-only graph, empty
      namespace still a node (FR-004), de-dup of `owns`/`owned-by` pair (FR-013), loading flag
      when related-resources not yet fetched (FR-015)
- [X] T014 Create `ui/src/topology/layout.ts` (`layoutTopologyGraph`, d3-force wrapper) per
      contracts/topology-graph-module.md
- [X] T015 [P] Unit test `ui/tests/unit/topology/layout.test.ts`: all nodes get finite x/y; prior
      positions seed the simulation
- [X] T016 Add `topology?: { scale: number; x: number; y: number }` to `PaneState` in
      `ui/src/store/index.ts`; ensure `newPane`/`assignPanes` leave it undefined by default
      (research.md R4)
- [X] T017 Register `'topology'` tool: `TOOL_KINDS` set and `TOOL_LABEL['topology']` in
      `ui/src/layout/defaults.ts` / `ui/src/components/shell/labels.ts`; add to
      `DOCK.tabs` default (laptop+) and to phone/tablet `StackTab` lists
      (`ui/src/components/shell/PhoneStack.tsx`, `TabletTabs.tsx`, `stack-tabs.ts`) so the pane is
      reachable at every width class (FR-016)

**Checkpoint**: pure graph-building/layout logic is unit-tested; pane plumbing compiles; no UI yet.

## Phase 2: User Story 1 - See the cluster shape at a glance (P1)

- [X] T020 [US1] Create `ui/src/components/topology/TopologyCanvas.tsx`: canvas element, draws
      namespace-tier nodes/edges from `buildTopologyGraph`+`layoutTopologyGraph`, no pan/zoom/LOD
      yet (namespace tier only for this story)
- [X] T021 [US1] Create `ui/src/components/panels/TopologyPanel.tsx` (mirrors `FluxPanel.tsx`):
      disconnected/no-kubeconfig states via existing `PaneStates`, else `<TopologyCanvas/>`
- [X] T022 [US1] Wire `pane.tool === 'topology'` in `ui/src/components/shell/PaneHost.tsx`'s
      `PaneBody` switch to render `<TopologyPanel/>`
- [X] T023 [US1] On `TopologyPanel` mount, dispatch the Namespaces/Applications/Services/HelmRelease
      list `refreshSection` calls needed for the overview tier (idempotent, matches existing
      list-pane mount behavior)
- [X] T024 [US1] Live redraw: subscribe to the relevant store slices so a new namespace row
      triggers a rebuild without reload (`requestAnimationFrame`-gated, no polling loop —
      Principle V)
- [X] T025 [P] [US1] `ui/tests/e2e/us1-topology-overview.spec.ts`: 5 namespaces / 3 with
      Application+Service fixture → asserts 5 namespace nodes rendered (`data-testid="topology-node"`
      count via canvas-adjacent a11y list, see T026) and edges only for the 3; scripted
      namespace-created event → new node appears without navigation

**Checkpoint**: overview alone is a usable, testable MVP.

## Phase 3: User Story 2 - Zoom from namespace down to pod (P1)

- [X] T030 [US2] Add pan/zoom input handling to `TopologyCanvas.tsx` (wheel, drag, pinch via
      Pointer Events) updating `scale/x/y`, persisted to `PaneState.topology` via `updatePane`
      (research.md R4), `data-testid="topology-canvas"`
- [X] T031 [US2] Implement `TopologyViewport` focus-set derivation in
      `ui/src/components/topology/TopologyCanvas.utils.ts` (`deriveFocus(viewport, nodes) ->
      {namespaceFocus, workloadFocus}` per research.md R3), unit-tested in
      `ui/tests/unit/topology/viewport.test.ts`
- [X] T032 [US2] Extend `buildTopologyGraph` call sites to pass current focus sets; on entering
      workload/pod tier for a namespace/workload not yet fetched, dispatch
      `RefreshRelatedResources`/`refreshSection` and render `loading: true` node (FR-015)
- [X] T033 [US2] Large-pod-count handling (FR-012, US2 Acceptance Scenario 4): when a focused
      workload's pod count exceeds a threshold, render pods with lighter per-node draw (no
      per-pod hit-test hover, batched draw calls) but keep individual click targets distinguishable
- [X] T034 [P] [US2] `ui/tests/e2e/us2-topology-zoom.spec.ts`: zoom into one namespace ->
      workload nodes appear, others stay collapsed; zoom into one workload -> pod nodes with phase
      appear; zoom out -> overview restored, same pane; 500-pod fixture stays responsive to a
      subsequent click

**Checkpoint**: continuous zoom works end-to-end; namespace+workload+pod tiers all render.

## Phase 4: User Story 3 - Jump from a graph node into its inspector (P2)

- [X] T040 [US3] Add click hit-testing to `TopologyCanvas.utils.ts` (`hitTest(point, nodes) ->
      TopologyNode | undefined`), unit-tested
- [X] T041 [US3] Wire node click in `TopologyCanvas.tsx` to `store.openInspector(node.ref,
      topologyPaneId)` (no new inspector code path)
- [X] T042 [US3] Confirm/verify (no code change expected) that `PaneState.topology` viewport
      persists across the inspector round-trip per research.md R4; add regression assertion in
      the e2e spec rather than new production code if it already works
- [X] T043 [P] [US3] `ui/tests/e2e/us3-topology-inspector.spec.ts`: click a graph node -> its
      inspector opens (Node kind, since Service is out of scope for v1 — see
      `ui/src/topology/types.ts`'s `TopologyNodeKind` comment); Escape/back-navigate -> graph
      shows prior zoom/pan, verified for chromium-desktop/tablet/phone (fixed a real
      `openInspector` slot-routing bug found along the way: tablet's tool tabs are mutually
      exclusive with its Resources-tab list|inspector row, so the always-present slotted inspector
      pane is no longer preferred when the click didn't originate from that visible row)

**Checkpoint**: graph nodes are real, reusable entry points into the existing inspector.

## Phase 5: User Story 4 - Live health state (P3)

- [X] T050 [US4] Confirm `nodeTone`/`detail` (ready-count, phase) recompute on every store-driven
      rebuild already wired in T024/T032 (likely no new code — add regression test only)
- [X] T051 [P] [US4] `ui/tests/e2e/us4-topology-live-update.spec.ts`: Deployment at 3/3, scripted
      pod-failure event -> pod node color/status and Deployment ready-count label update in place,
      with no new command sent by the graph beyond what the equivalent list/detail views already
      send (assert via fake backend's received-commands log)

**Checkpoint**: all 4 user stories independently verified end-to-end.

## Phase 6: Polish & cross-cutting

- [X] T060 [P] Idle-CPU sanity: verify `TopologyCanvas`'s redraw is `requestAnimationFrame`-gated
      by an actual "did anything change" check (no continuous loop) — code review + note in
      `TopologyCanvas.tsx` doc comment; SC-006 is inherently unautomatable in CI so this is a
      documented manual check in quickstart.md, not a new test
- [X] T061 [P] Phone-width pass: verify pan/zoom/tap-to-inspect works at phone width
      (`data-testid="topology-canvas"` under `PhoneStack`); add to `us1-topology-overview.spec.ts`
      as a phone-viewport variant if the existing e2e project config parametrizes viewport
      (covered: every `usN-topology-*.spec.ts` file already runs its full assertion set under the
      `chromium-phone`/`chromium-tablet` projects, including click-to-inspect via `PhoneStack`)
- [X] T062 Run `cargo build && cargo clippy --all-targets -- -D warnings && cargo test`
- [X] T063 Run `pnpm -C ui typecheck && pnpm -C ui lint && pnpm -C ui test && pnpm -C ui e2e`
- [ ] T064 Update `docs/MVP.md` Status Summary row for Topology Graph View to shipped, matching
      006-helm-mutate's row format
