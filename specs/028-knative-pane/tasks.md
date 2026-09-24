---
description: "Task list for the Knative pane feature"
---

# Tasks: Knative Pane

**Input**: Design documents from `specs/028-knative-pane/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/protocol.md, quickstart.md

**Tests**: Included — constitution Principle III (Tested at the Boundaries) is NON-NEGOTIABLE for this repo: protocol roundtrip tests, backend unit tests for the pure classification functions, frontend unit tests, and one focused Playwright e2e spec are all required, not optional.

## Format: `[ID] [P?] [Story] Description`

## Phase 1: Setup

- [x] T001 Create `src/protocol/v1/knative.rs` with `KnativeReadyState`, `KnativeScaleReason`, `KnativeRevisionSummary`, `KnativeRouteTrafficTarget`, `KnativeRouteSummary`, `KnativeServiceSummary`, `KnativeOverview` structs/enums per `specs/028-knative-pane/data-model.md`, each `derive(Debug, Clone, Serialize, Deserialize, TS)` (and `Default` on `KnativeOverview`), following the exact style of `src/protocol/v1/flux.rs`.
- [x] T002 Register the new module in `src/protocol/v1/mod.rs` (mirror how `flux` is registered) and re-export `KnativeOverview`, `KnativeRevisionSummary`, `KnativeRouteSummary`, `KnativeServiceSummary`, `KnativeScaleReason`, `KnativeReadyState`, `KnativeRouteTrafficTarget` at the `v1` level the same way Flux types are re-exported.

## Phase 2: Foundational (blocking prerequisites)

**Purpose**: Backend discovery module, protocol command/event wiring, and poll loop — required by every user story.

- [x] T003 Add `RefreshKnative` variant to `BackendCmdV1` in `src/protocol/v1/cmd.rs` (no payload, mirrors `RefreshFlux`), plus a `refresh_knative_roundtrip` test in that file's existing `#[cfg(test)] mod tests` per `specs/028-knative-pane/contracts/protocol.md`.
- [x] T004 Add `KnativeOverviewUpdated(KnativeOverview)` variant to `SystemEvent` in `src/protocol/v1/event.rs`, importing `KnativeOverview` at the top of the file alongside the existing `FluxOverview` import, plus a roundtrip test using `KnativeOverview::default()` in that file's existing test module.
- [x] T005 Create `src/backend/knative.rs`: CRD name constants for `services.serving.knative.dev`, `revisions.serving.knative.dev`, `routes.serving.knative.dev`, `configurations.serving.knative.dev`; reuse the `find_served`/dynamic-object `list_instances` pattern from `src/backend/flux.rs` (either by copying the small helpers locally or extracting them to a shared `crds.rs` helper if that keeps both modules simplest — prefer local copies if extraction would touch `flux.rs`'s public surface, to keep this feature additive-only).
- [x] T006 [P] In `src/backend/knative.rs`, implement `fn resolve_scale_reason(revision_status: &serde_json::Value) -> KnativeScaleReason` per `specs/028-knative-pane/research.md` R3: check `status.conditions[type=Active]` first (`status: False` + reason mentioning inactivity/no-traffic → `ScaledToZero { idle_since }` using that condition's `lastTransitionTime`; `status: Unknown` + an activating-type reason → `ScalingUp { pending_requests: None }`), then fall back to comparing `status.actualReplicas`/`status.desiredReplicas` (`AtDesiredScale` when equal), else `Unavailable`. Add unit tests in the same file covering: scaled-to-zero JSON fixture, scaling-up JSON fixture, at-desired-scale fixture (equal replica counts, no Active condition needed), and an unrecognized/missing-fields fixture asserting `Unavailable`.
- [x] T007 [P] In `src/backend/knative.rs`, implement `fn resolve_route_traffic(route_status: &serde_json::Value, known_revision_names: &std::collections::HashSet<String>) -> Vec<KnativeRouteTrafficTarget>` reading `status.traffic[]` (`revisionName`, `percent`, `tag`) per `data-model.md`'s `KnativeRouteTrafficTarget`, setting `revision_missing: true` when `revisionName` is not in `known_revision_names`, and never renormalizing percentages. Add unit tests: a 2-way split summing to 100, a single 100% target, and a target naming a Revision not present in `known_revision_names` (asserts `revision_missing: true`).
- [x] T008 In `src/backend/knative.rs`, implement `pub(super) async fn scan_knative(client: &Client) -> anyhow::Result<KnativeOverview>` following `scan_flux`'s structure exactly: list CRDs once, `find_served` each of the 4 kinds, list instances per served kind, build `KnativeServiceSummary`/`KnativeRevisionSummary`/`KnativeRouteSummary` via small `summarize_*` helpers (owning-service derived from the `serving.knative.dev/service` label per Knative convention, or owner references as fallback), call `resolve_scale_reason`/`resolve_route_traffic`, compute each Revision's `traffic_percent` by summing matching targets across all Routes, sort each list by `(namespace, name)`, and set `installed = ` true iff any of the 4 CRDs was found served. Add an `#[ignore]`d live-cluster test named `scan_knative_against_live_cluster` mirroring `scan_flux_against_live_cluster`.
- [x] T009 Register `knative` as a `mod` in `src/backend/mod.rs` (or wherever `mod flux;` lives) at the same visibility as `flux`.
- [x] T010 In `src/backend/sync.rs`, add `ensure_knative_poll` and `poll_knative_overview` mirroring `ensure_knative_poll`'s Flux counterparts exactly (20s interval, `state.watch_tasks.knative_poll_task`, calls `knative::scan_knative`, emits `SystemEvent::KnativeOverviewUpdated`); add the `knative_poll_task: Option<...>` field to the `watch_tasks` struct alongside `flux_poll_task`; wire `ensure_knative_poll` to be started the same place/lifecycle `ensure_flux_poll` is started (on connect).
- [x] T011 In `src/backend/cmd/integrations.rs`, add a `BackendCmd::RefreshKnative => refresh_knative(state, sender).await` arm mirroring the existing `RefreshFlux` arm, and implement `refresh_knative` mirroring `refresh_flux`'s implementation (calls `knative::scan_knative`, emits `KnativeOverviewUpdated`).
- [x] T012 Run `cargo build --workspace --all-targets` and fix any compile errors introduced by T001–T011 before starting user-story work (this phase must build cleanly since every story depends on it).

**Checkpoint**: Backend fully discovers and streams `KnativeOverview` via the protocol; no UI yet. This is independently verifiable with `cargo test -p fathom knative`.

---

## Phase 3: User Story 1 - See Knative workloads at a glance (P1)

**Goal**: A working inventory pane listing Services, Revisions, and Routes.

**Independent Test**: Fixture cluster/backend with Knative CRDs + one Service/Revision/Route → open the Knative pane → see all three listed with name/namespace/status.

- [x] T013 [P] [US1] Add `knative: { overview?: KnativeOverview }` to the store shape in `ui/src/store/index.ts`, initialized to `{}` in the store's initial state (mirror the existing `flux: {}` line), importing the generated `KnativeOverview` type once `ts-rs` regenerates it (run the project's existing protocol-generation step, e.g. `cargo test` or the dedicated bindings-export step used elsewhere in this repo, to produce `ui/src/protocol/generated/KnativeOverview.ts` and friends from T001).
- [x] T014 [US1] In `ui/src/store/handlers/system.ts`, add a `KnativeOverviewUpdated` case mirroring the existing `FluxOverviewUpdated` case, writing the payload into `store.knative.overview`.
- [x] T015 [US1] Add a `refreshKnative` store action in `ui/src/store/index.ts` mirroring `refreshFlux` (sends `{ type: 'RefreshKnative' }` via `send`).
- [ ] T016 [US1] Register the Knative pane's entry point alongside the Flux pane's registration (find wherever `FluxPanel`/`IntegrationSummary` panes are registered into the tool-pane switcher — likely a sections/panels registry near `ui/src/components/panels/` or `ui/src/sections/` — and add a `Knative` entry using a suitable Phosphor icon, e.g. `RocketIcon` or `CloudArrowUpIcon`).
- [ ] T017 [US1] Create `ui/src/components/panels/KnativePanel.tsx`: on mount, call `refreshKnative()` (mirror `FluxPanel`'s `useEffect`); if `overview` is undefined, render `MetricsUnavailable` with a "waiting for the Knative scan" detail; if `overview.installed === false`, render `MetricsUnavailable` with detail "Knative Serving CRDs were not found in this cluster." (FR-002, User Story 4); otherwise render three sections/lists (Services, Revisions, Routes) each showing name, namespace, and ready/status per FR-003/004/005, plus each Revision's actual/desired replica counts and its `scale_reason` rendered as human text (map `KnativeScaleReason` variants to strings: `AtDesiredScale` → "At desired scale", `ScaledToZero` → "Scaled to zero: no traffic" + idle-since if present, `ScalingUp` → "Scaling up" + pending-request count if present, `Unavailable` → "Reason unavailable (actual X / desired Y)").
- [ ] T018 [P] [US1] Add `ui/src/components/panels/KnativePanel.test.tsx` (Vitest + Testing Library, matching the project's existing panel test conventions) covering: undefined overview → waiting state; `installed: false` → not-installed empty state; populated overview → all three lists render with correct name/namespace/status text; a Revision with `ScaleReason.Unavailable` renders the generic fallback text without fabricating a reason.

**Checkpoint**: Opening the pane against fixture data shows a correct, honest inventory. Independently demoable.

---

## Phase 4: User Story 2 - Understand traffic distribution across Revisions (P1)

**Goal**: Traffic-split visualization built on the existing topology component.

**Independent Test**: Route with an 80/20 split across two Revisions renders a Route node with two labeled edges summing to 100%.

- [ ] T019 [US2] Add `label?: string` to `TopologyEdge` in `ui/src/topology/types.ts`, and add `'KnativeRevision' | 'KnativeRoute'` to `TopologyNodeKind` (per `specs/028-knative-pane/data-model.md`); this field must remain optional and unused by every existing edge so current topology rendering is unaffected.
- [ ] T020 [US2] Implement `buildKnativeTrafficGraph(overview: KnativeOverview, routeKey: { namespace: string; name: string }): TopologyGraph` in `ui/src/topology/build.ts` (or a new co-located `ui/src/topology/knative.ts` if that keeps `build.ts` focused): one `KnativeRoute` node for the selected Route, one `KnativeRevision` node per `traffic` target (tone `err`/`warn` when `revision_missing: true`, `ok` otherwise), one edge per target with `label: `${percent}%`` and `relations: ['traffic']`.
- [ ] T021 [P] [US2] Add `ui/src/topology/knative.test.ts` (or extend `build.ts`'s existing test file) asserting: a 2-way 80/20 split produces 2 edges labeled "80%"/"20%" whose percentages sum to 100; a single 100%-target Route produces exactly 1 edge labeled "100%"; a target with `revision_missing: true` still produces a node/edge (not silently dropped) and is tonally distinct.
- [ ] T022 [US2] In `ui/src/components/topology/TopologyCanvas.tsx`'s `draw()` edge-drawing loop, when `e.label` is set, draw the label text at the edge's screen-space midpoint (reuse the existing `style.getPropertyValue('--foreground')` text-color lookup already used for node labels) — additive change only, must not alter rendering for edges without a `label`.
- [ ] T023 [US2] In `KnativePanel.tsx` (from T017), add a traffic-split visualization section per Route: render a `<TopologyCanvas>`-equivalent small canvas seeded via a new lightweight wrapper (e.g. `KnativeTrafficGraph.tsx` in `ui/src/components/panels/`) that calls `buildKnativeTrafficGraph` + `layoutTopologyGraph` directly (bypassing the store-`lists`-driven `buildTopologyGraph` path used by the main cluster graph, since Knative data comes from `store.knative.overview`, not `store.lists`/`store.details` — per research.md R4) and reuses `TopologyCanvas.utils`'s pan/zoom/hit-test helpers for consistency.
- [ ] T024 [P] [US2] Add `ui/src/components/panels/KnativeTrafficGraph.test.tsx` asserting the rendered canvas/DOM exposes the correct percentage text for a fixture Route (query via `data-testid` or accessible text, matching how `TopologyCanvas` is tested elsewhere in this repo — check existing topology tests for the established assertion pattern, e.g. inspecting the built graph's node/edge data rather than pixel-reading the canvas).

**Checkpoint**: Traffic-split percentages are visually correct and independently demoable on top of US1.

---

## Phase 5: User Story 3 - Understand why replica count differs from desired (P2)

**Goal**: Depth on the per-Revision scaling explanation already surfaced minimally in US1.

**Independent Test**: A scaled-to-zero Revision's detail shows a specific "scaled to zero" reason; a scaling-up Revision shows a "scaling up" reason; an at-desired Revision shows a neutral state; an unrecognized-fields Revision shows "reason unavailable" with raw numbers.

- [ ] T025 [US3] Extend the Revision row/detail in `KnativePanel.tsx` (built minimally in T017) into a fuller detail view: show `actual_replicas`/`desired_replicas` numerically alongside the mapped `scale_reason` text, and when `scale_reason` is `ScaledToZero` with `idle_since` present, format and display that timestamp; when `ScalingUp` with `pending_requests` present, display the count.
- [ ] T026 [P] [US3] Add/extend `ui/src/components/panels/KnativePanel.test.tsx` (from T018) with the four acceptance-scenario cases from spec.md User Story 3: scaled-to-zero fixture shows the idle-duration text when present and still shows a sensible message when absent; scaling-up fixture shows pending-request count when present; at-desired fixture shows the neutral state (no fabricated discrepancy text); unavailable fixture shows raw actual/desired numbers plus "reason unavailable".

**Checkpoint**: Full diagnostic depth delivered; independently verifiable via the same panel test file.

---

## Phase 6: User Story 4 - Honest empty state when Knative isn't installed (P3)

**Goal**: Confirm the not-installed path (already implemented defensively in T017) is correct end-to-end, including pane discoverability.

**Independent Test**: Fixture backend with `installed: false` → opening/looking for the pane shows a clear "not detected" message and renders no lists/graph.

- [ ] T027 [US4] Verify (and adjust if needed) that the tool-pane registration from T016 always shows the Knative pane entry point (so it's discoverable) but that `KnativePanel.tsx` itself is the sole gate on `installed`, per FR-002 — i.e., the pane entry is not conditionally hidden based on detection state, matching how the Flux pane entry behaves, so "not installed" is an honest in-pane message rather than the feature disappearing.
- [ ] T028 [P] [US4] Add a `KnativePanel.test.tsx` case (if not already covered by T018) asserting that with `installed: false`, neither the Services/Revisions/Routes lists nor `KnativeTrafficGraph` render at all — only the not-installed message.

**Checkpoint**: All four user stories independently demoable.

---

## Phase 7: Polish & Cross-Cutting

- [ ] T029 [P] Add `ui/tests/e2e/knative-pane.spec.ts`: a focused Playwright spec against the project's scripted fake backend (per constitution Principle III) covering exactly: (a) open the Knative pane against a fixture `KnativeOverviewUpdated` with one Service/Revision/Route set and confirm the lists render; (b) confirm an 80/20 (or similar) traffic split renders both percentages correctly; (c) confirm a scaled-to-zero Revision fixture shows the correct "why" text; (d) confirm an `installed: false` fixture shows the "Knative not installed" empty state with no lists/graph rendered. Keep this spec tight — 4 scenarios, no more.
- [ ] T030 Run `cargo build --workspace --all-targets`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` (full workspace); fix any warnings/failures.
- [ ] T031 Run `pnpm -C ui typecheck`, `pnpm -C ui lint`, `pnpm -C ui test`, and the new e2e spec (`pnpm -C ui test:e2e -- knative-pane` or the repo's equivalent invocation); fix any failures.
- [ ] T032 Re-read `specs/028-knative-pane/spec.md` FR-001 through FR-014 and confirm each is met by the shipped code; update `specs/028-knative-pane/checklists/requirements.md` notes if any gap is found and fixed.

## Dependencies

- Phase 1 (Setup) blocks Phase 2 (Foundational).
- Phase 2 (Foundational) blocks all user stories (Phases 3–6) — no story can start until `scan_knative`/protocol/poll compile and pass `cargo test -p fathom knative`.
- Phase 3 (US1) blocks Phase 4 (US2) and Phase 5 (US3) only in the sense that `KnativePanel.tsx` is created in T017 and extended thereafter — US2/US3 are additive edits to files US1 creates, not independent files, so implement in order US1 → US2 → US3 → US4 even though each has its own independent acceptance test.
- Phase 6 (US4) only needs Phase 3's `KnativePanel.tsx` gating logic (already present) — mostly a verification phase.
- Phase 7 (Polish) runs last, after all stories.

## Parallel Execution Notes

- T006 and T007 (pure classification functions in `knative.rs`) are independently parallelizable — different functions, same new file, no shared state.
- T018, T021, and T024 (test files) can be written in parallel with each other once their respective implementation tasks land, since they touch different files.
- T013 and later frontend tasks cannot start until T001–T012 (protocol types + backend) compile, since the generated TS types are a build output of the Rust side.

## Implementation Strategy

**MVP = User Story 1** (Phases 1–3): CRD detection + a correct, honest, read-only inventory list is already a shippable, demoable increment consistent with the Flux-panel precedent. User Story 2 (traffic-split visualization) is the feature's headline differentiator and should follow immediately after. User Story 3 deepens diagnostics on the same panel. User Story 4 is largely already satisfied by US1's own gating and mostly needs verification + a test.
