---
description: "Task list for Pod Lifecycle Diagnostics (008)"
---

# Tasks: Pod Lifecycle Diagnostics

**Input**: Design documents from `/specs/008-pod-lifecycle-diagnostics/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/pod-lifecycle.md, quickstart.md

**Tests**: Explicitly required by the user's instructions (Rust round-trip + unit tests, frontend
component/unit tests, Playwright e2e per user story). Included throughout.

## Format: `[ID] [P?] [Story] Description`

## Phase 1: Setup

- [ ] T001 Confirm no new dependency is needed: re-check `Cargo.toml` and `ui/package.json` against
  plan.md's Primary Dependencies (already done during planning — no `Cargo.toml`/`package.json`
  change required for this feature).

## Phase 2: Foundational (blocks all user stories)

**Purpose**: The new protocol types and backend assembly function that every user story's UI
section reads from.

- [ ] T002 Add `ContainerState`, `ProbeSpec`, `LifecycleHookSpec`, `ContainerLifecycle` structs to
  `src/protocol/v1/pods.rs` per data-model.md (all `#[derive(Debug, Clone, Serialize, Deserialize, TS)]`,
  matching the file's existing style).
- [ ] T003 Add `containers_lifecycle: Vec<ContainerLifecycle>` field to `PodDetails` in
  `src/protocol/v1/pods.rs`, appended after `raw_json` (additive, per data-model.md — no existing
  field renamed or removed).
- [ ] T004 Implement `pub(super) fn build_container_lifecycle(pod: &Pod) -> Vec<ContainerLifecycle>`
  in `src/backend/pods.rs`: iterate `pod.spec.init_containers` (is_init=true) then
  `pod.spec.containers` (is_init=false), matching each to `pod.status.init_container_statuses` /
  `pod.status.container_statuses` by name; extract `ContainerState` from `ContainerStatus.state`,
  `last_state` from `ContainerStatus.last_state` (`None` when the terminated variant is absent, per
  data-model.md's edge case note), `restart_count`, `ready`; extract `ProbeSpec`s from
  `container.liveness_probe`/`readiness_probe`/`startup_probe` (skip absent ones — FR-007's "no
  probes" is an empty `Vec`, not a placeholder entry); extract `LifecycleHookSpec`s from
  `container.lifecycle.post_start`/`pre_stop` (skip absent ones — FR-012's "none configured" is an
  empty `Vec`).
- [ ] T005 Wire `build_container_lifecycle(&pod)` into `fetch_pod_details` in `src/backend/pods.rs`,
  setting `PodDetails.containers_lifecycle` before constructing `details`.
- [ ] T006 [P] Add round-trip serde tests for `ContainerState`, `ProbeSpec`, `LifecycleHookSpec`,
  `ContainerLifecycle`, and the extended `PodDetails` (update `sample_pod_details()`) in
  `tests/protocol_serde.rs`, mirroring the file's existing `sample_*`/round-trip convention.
- [ ] T007 Regenerate TypeScript bindings: `cargo test --test export_bindings`, confirm
  `ui/src/protocol/generated/{ContainerState,ProbeSpec,LifecycleHookSpec,ContainerLifecycle,PodDetails}.ts`
  are created/updated and exported from `ui/src/protocol/generated/index.ts`.

**Checkpoint**: `cargo build && cargo test` green; `PodDetails['containers_lifecycle']` is a typed,
importable field in the frontend. No UI changes yet.

---

## Phase 3: User Story 1 — Init containers distinct from app containers (P1) 🎯 MVP

**Goal**: A separate "Init Containers" section in the Pod Inspector, each labeled and showing its
own status/exit code, with logs reachable the same way as app containers.

**Independent Test**: Pod with 2 init containers (one succeeds, one fails) + 1 app container never
starting → inspector shows init containers in a distinct, labeled section with per-container
status/exit code; failing one's logs reachable via the existing Logs tab.

### Tests for User Story 1

- [ ] T008 [P] [US1] Backend unit tests in `src/backend/pods.rs`'s `#[cfg(test)] mod tests`:
  `build_container_lifecycle` on a fixture `Pod` with 2 init containers (one
  `ContainerStateTerminated{exit_code:0,reason:"Completed"}`, one
  `ContainerStateTerminated{exit_code:1,reason:"Error"}`) and 1 app container `Waiting` — assert
  both init entries have `is_init: true`, correct exit codes/reasons, and the app container entry
  has `is_init: false`. Also test a Pod with zero `init_containers` → assert no entries have
  `is_init: true` (FR-004).
- [ ] T009 [P] [US1] Vitest test in `ui/src/resources/pods.test.tsx` (create if absent, or add to
  existing pods test file if one exists — check first): the new Lifecycle → Init Containers
  section renders init containers separate from app containers, shows exit code for a failed one,
  and shows an explicit empty/none state when `containers_lifecycle` has no `is_init: true` entries
  (FR-004).
- [ ] T010 [US1] Playwright e2e `ui/tests/e2e/us1-pod-lifecycle-init-containers.spec.ts`: open a
  pod fixture with a failing init container, assert `data-testid="pod-lifecycle-init-containers"`
  section is visible, shows the failing container's name + exit code, and that clicking its
  logs affordance sends the same `StartPodLogs` command (with that init container's name) as an
  app container's logs affordance does.

### Implementation for User Story 1

- [ ] T011 [US1] Add `ui/tests/fixtures/gen-pods.ts`: extend `podDetails()` (or add a new
  `podDetailsWithLifecycle()` builder used only by these tests, keeping `podDetails()`'s existing
  callers unaffected) to include a `containers_lifecycle` array with init + app containers for use
  across all five stories' fixtures.
- [ ] T012 [US1] Create `ui/src/components/PodLifecyclePanels.tsx` with an `InitContainersSection`
  component: renders each `ContainerLifecycle` where `is_init === true` with name, status badge
  (waiting/running/terminated), exit code when terminated, and a link/button that opens the Logs
  tab pre-selected to that container (reuse whatever mechanism `PodLogsTab` already uses to target
  a specific container — check `ui/src/components/panels/PodLogsTab.tsx` first). Root element
  `data-testid="pod-lifecycle-init-containers"`; explicit empty state
  (`data-testid="pod-lifecycle-init-containers-empty"`) when there are no `is_init` entries
  (FR-004).
- [ ] T013 [US1] Add a new `lifecycle` `DetailSection` to `podSections` in
  `ui/src/resources/pods.tsx`, placed after the existing `related` section (additive, FR-018),
  rendering `InitContainersSection` (plus US2–US5's sections, added incrementally below) via a
  `{ type: 'list', items: [...] }` block whose one item is a `<PodLifecyclePanels>`-family
  component (components may use hooks; `DetailBlock.items` is `ReactNode[]`, confirmed in
  `ui/src/resources/types.ts`).

**Checkpoint**: US1 fully functional and independently testable/demoable.

---

## Phase 4: User Story 2 — Probe configuration + current failure state (P1)

**Goal**: Per-container probe config (type/thresholds/period) plus a currently-failing indicator
distinct from the aggregate pod-level `Ready` condition.

**Independent Test**: Pod with a deliberately-failing `readinessProbe` → inspector shows that
container's probe config and flags it as failing, distinct from a healthy probe on another
container.

### Tests for User Story 2

- [ ] T014 [P] [US2] Backend unit test in `src/backend/pods.rs`: a container with
  `liveness_probe`+`readiness_probe`+`startup_probe` all configured → assert 3 `ProbeSpec` entries
  with correct `probe_type`/`mechanism`/`period_seconds`/thresholds; a container with none
  configured → assert empty `probes` Vec (FR-007).
- [ ] T015 [P] [US2] Vitest unit tests in `ui/src/resources/podLifecycle.test.ts` for the new
  `probeState()` function in `ui/src/resources/podLifecycle.ts`: readiness probe on a
  `ready: false`, `state.phase: "running"` container → `failing: true`; a `Liveness probe failed`
  `Unhealthy` event mentioning the container name → that container's liveness probe `failing:
  true`; no matching signal → `failing: false`; container with zero probes → empty result array
  (FR-007).
- [ ] T016 [US2] Playwright e2e `ui/tests/e2e/us2-pod-lifecycle-probes.spec.ts`: pod fixture with a
  failing readiness probe and a healthy liveness probe on another container → assert
  `data-testid="pod-lifecycle-probes"` shows both, the failing one visually/textually distinguished
  (e.g. `data-testid="pod-lifecycle-probe-failing"` present only on the failing entry), and a
  container with no probes shows `data-testid="pod-lifecycle-probes-empty"` (FR-007).

### Implementation for User Story 2

- [ ] T017 [US2] Create `ui/src/resources/podLifecycle.ts` exporting `probeState(container:
  ContainerLifecycle, events: KubernetesEvent[]): { probe: ProbeSpec; failing: boolean }[]` per
  research.md's matching rules: readiness `failing` = configured && `state.phase === 'running'` &&
  `!ready`, OR a matching `Unhealthy` event; liveness/startup `failing` = a matching `Unhealthy`
  event only (substring match on container name + `"Liveness probe failed"` /
  `"Startup probe failed"` in `message`).
- [ ] T018 [US2] Add a `ProbesSection` component to `ui/src/components/PodLifecyclePanels.tsx`:
  renders every container's probes (app + init) via `probeState`, each with type/mechanism/
  target/thresholds/period, failing ones flagged (`data-testid="pod-lifecycle-probe-failing"` on
  that entry); explicit `data-testid="pod-lifecycle-probes-empty"` state per container with zero
  probes (FR-007). Root `data-testid="pod-lifecycle-probes"`.
- [ ] T019 [US2] Wire `ProbesSection` into the `lifecycle` `DetailSection` in
  `ui/src/resources/pods.tsx` (after `InitContainersSection`), passing the Pod's events (obtained
  via the store's existing `eventsFor` selector — see T024 for how US5 exposes it; if sequenced
  before US5's wiring, call `eventsFor` directly here since it's already a store export, not new
  infrastructure).

**Checkpoint**: US1 + US2 both independently functional.

---

## Phase 5: User Story 3 — Per-container restart history (P2)

**Goal**: Per-container restart count, last termination reason, exit code, last-terminated
timestamp — distinct from the pod-level restart count already shown.

**Independent Test**: Pod with 2 containers, only one restarted → inspector attributes restart
count/reason/exit code/timestamp to the correct container only.

### Tests for User Story 3

- [ ] T020 [P] [US3] Backend unit test in `src/backend/pods.rs`: container with
  `restart_count: 3` and `last_state: Terminated{reason: "OOMKilled", exit_code: 137, ...}` →
  assert `ContainerLifecycle.last_state` matches; container with `restart_count: 0` → assert
  `last_state: None` and no stale data leaks in (FR-009). Also a `restartPolicy: Never` pod
  fixture where `last_state` is absent but `state` is `Terminated` → assert `state` (not
  `last_state`) still carries the termination info (spec edge case).
- [ ] T021 [P] [US3] Vitest test verifying `RestartHistorySection` renders zero restarts explicitly
  (no stale "last termination" block) for a never-restarted container, and correctly attributes a
  restart's reason/exit code/timestamp to only the restarted container in a 2-container fixture
  (FR-009, Acceptance Scenario 3).
- [ ] T022 [US3] Playwright e2e `ui/tests/e2e/us3-pod-lifecycle-restarts.spec.ts`: 2-container pod
  fixture, one with `restart_count: 3` + `OOMKilled` last_state, one with `restart_count: 0` →
  assert `data-testid="pod-lifecycle-restarts"` shows the crashed container's reason/exit
  code/timestamp and the healthy container's explicit zero state, not conflated.

### Implementation for User Story 3

- [ ] T023 [US3] Add `RestartHistorySection` to `ui/src/components/PodLifecyclePanels.tsx`:
  per-container restart count, and when `restart_count > 0`, last termination reason/exit
  code/timestamp from `last_state` (fallback to `state` when `last_state` is `None` but `state` is
  terminated, per the `restartPolicy: Never` edge case); explicit "no restarts" text when
  `restart_count === 0`. Root `data-testid="pod-lifecycle-restarts"`.
- [ ] T024 [US3] Wire `RestartHistorySection` into the `lifecycle` `DetailSection` in
  `ui/src/resources/pods.tsx`.

**Checkpoint**: US1–US3 independently functional.

---

## Phase 6: User Story 4 — Lifecycle hooks + failure correlation (P2)

**Goal**: Per-container `postStart`/`preStop` hook config, with a failure indication correlated
from the Pod's Events.

**Independent Test**: Container with a failing `postStart` hook → inspector shows the hook config
and a hook-failure indication sourced from a `FailedPostStartHook` Event.

### Tests for User Story 4

- [ ] T025 [P] [US4] Backend unit test in `src/backend/pods.rs`: container with
  `lifecycle.post_start` (exec) and `lifecycle.pre_stop` (httpGet) configured → assert 2
  `LifecycleHookSpec` entries with correct `hook_type`/`mechanism`/`target`; container with no
  `lifecycle` → assert empty `hooks` Vec (FR-012).
- [ ] T026 [P] [US4] Vitest unit tests in `ui/src/resources/podLifecycle.test.ts` for `hookState()`:
  a `FailedPostStartHook` event mentioning the container name → that container's postStart hook
  `failed: true`; no matching event → `failed: false`; hook ran without failing → config shown,
  `failed: false` (Acceptance Scenario 4).
- [ ] T027 [US4] Playwright e2e `ui/tests/e2e/us4-pod-lifecycle-hooks.spec.ts`: pod fixture with a
  container whose `postStart` hook failed (matching `FailedPostStartHook` event injected) →
  assert `data-testid="pod-lifecycle-hooks"` shows the hook config and
  `data-testid="pod-lifecycle-hook-failed"` on that entry; a container with no hooks shows
  `data-testid="pod-lifecycle-hooks-empty"` (FR-012).

### Implementation for User Story 4

- [ ] T028 [US4] Add `hookState(container: ContainerLifecycle, events: KubernetesEvent[]): {
  hook: LifecycleHookSpec; failed: boolean }[]` to `ui/src/resources/podLifecycle.ts` per
  research.md's substring-matching rule (`FailedPostStartHook`/`FailedPreStopHook` reason + container
  name in `message`).
- [ ] T029 [US4] Add `HooksSection` component to `ui/src/components/PodLifecyclePanels.tsx`:
  renders each container's hooks via `hookState`, failed ones flagged
  (`data-testid="pod-lifecycle-hook-failed"`); explicit `data-testid="pod-lifecycle-hooks-empty"`
  per container with zero hooks (FR-012). Root `data-testid="pod-lifecycle-hooks"`.
- [ ] T030 [US4] Wire `HooksSection` into the `lifecycle` `DetailSection` in
  `ui/src/resources/pods.tsx`.

**Checkpoint**: US1–US4 independently functional.

---

## Phase 7: User Story 5 — Pod Events timeline inside the inspector (P2)

**Goal**: The Pod's own Events as a chronological timeline inside the Details tab's Lifecycle
section (connective layer for US2/US4's correlation, and standalone value), reusing the same
underlying Event data as the cluster-wide Events list / existing generic Events tab.

**Independent Test**: Pod with several Events (Scheduled/Pulling/Pulled/Started) → all appear
inside the inspector's Lifecycle section in chronological order without navigating to the
cluster-wide Events view; updates live while open.

### Tests for User Story 5

- [ ] T031 [P] [US5] Vitest test: a `PodEventsTimeline` component fed a `KubernetesEvent[]`
  (unsorted) renders them in chronological order with reason/message/type/timestamp, and renders
  an explicit "no events" state for an empty array (FR-014).
- [ ] T032 [US5] Playwright e2e `ui/tests/e2e/us5-pod-lifecycle-events-timeline.spec.ts`: open a pod
  fixture, inject its `EventsUpdated` batch → assert `data-testid="pod-lifecycle-events-timeline"`
  shows all events chronologically without switching to the "Events" tab; then inject one more
  event via a follow-up `EventsUpdated` batch while the inspector stays open → assert the timeline
  updates to include it (FR-015); assert a pod with zero events shows
  `data-testid="pod-lifecycle-events-timeline-empty"` (FR-014); assert an event with an
  unrecognized `reason` (e.g. `SomeUnknownReason`) still appears with its raw reason/message
  (FR-019).

### Implementation for User Story 5

- [ ] T033 [US5] Add `PodEventsTimeline` component to `ui/src/components/PodLifecyclePanels.tsx`:
  accepts `events: KubernetesEvent[]`, sorts by `last_timestamp` ascending, renders each with
  reason/message/type/timestamp (mirroring `EventsFor`'s existing rendering style in
  `ui/src/components/Inspector.tsx` for visual consistency, but as its own component per plan.md's
  file layout); explicit `data-testid="pod-lifecycle-events-timeline-empty"` empty state (FR-014).
  Root `data-testid="pod-lifecycle-events-timeline"`.
- [ ] T034 [US5] Wire `PodEventsTimeline` into the `lifecycle` `DetailSection` in
  `ui/src/resources/pods.tsx`, sourcing events via the store's `eventsFor(s, 'Pod', namespace,
  name)` selector (`ui/src/store/index.ts`) inside a small wrapper component that calls
  `useStore` (since `DetailSection.render` itself is a pure function without hook access — see
  `ui/src/resources/types.ts`'s `DetailContext`) — same underlying data FR-016 requires, no new
  fetch.
- [ ] T035 [US5] Finish wiring `ProbesSection`/`HooksSection` (T019, T030) to source events from
  this same store-backed wrapper rather than a placeholder, so US2/US4's failure correlation and
  US5's timeline share one events read (FR-016).

**Checkpoint**: All 5 user stories independently functional and integrated into one "Lifecycle"
Details section.

---

## Phase 8: Polish & Cross-Cutting

- [ ] T036 [P] Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
  (full suite) — fix any issues.
- [ ] T037 [P] Run `pnpm -C ui typecheck`, `pnpm -C ui lint`, `pnpm -C ui test` (full suite) — fix
  any issues.
- [ ] T038 Run the full Playwright e2e suite (or at minimum the 5 new `*-pod-lifecycle-*` specs
  plus existing `us1-pod-logs.spec.ts`/`us1-inspector.spec.ts`/`us1-pods-list.spec.ts` to confirm no
  regression to the existing Pod Inspector).
- [ ] T039 Update `docs/MVP.md`'s Pod Inspector → Lifecycle row/note (§7) to reflect shipped status,
  matching 006-helm-mutate's row-update pattern (separate small commit).
- [ ] T040 Run quickstart.md's manual scenario checklist mentally against the implementation (or
  live-cluster if available) and confirm each of the 5 Independent Tests is satisfiable.

---

## Dependencies & Execution Order

- Phase 1 (Setup) → Phase 2 (Foundational) blocks all user stories.
- Phase 2 must fully complete (protocol types + backend assembly + bindings regenerated) before
  any Phase 3–7 UI task, since every UI section reads `PodDetails.containers_lifecycle`.
- User Stories 1–5 (Phases 3–7) are each independently testable once Phase 2 is done, but this plan
  sequences them in spec.md's priority order (P1, P1, P2, P2, P2) and has them converge on the same
  `lifecycle` `DetailSection` in `pods.tsx` and the same `PodLifecyclePanels.tsx` file, so within
  this plan they are implemented sequentially (US1 → US2 → US3 → US4 → US5) to avoid repeated
  merge conflicts in those two shared files, even though each story's *logic* has no dependency on
  another's.
- Phase 8 (Polish) depends on all desired stories being complete.

## Parallel Opportunities

- T006 (protocol_serde tests) can run parallel to T007 (bindings regen) once T002/T003 land.
- Within each user story, the `[P]`-marked backend unit test and frontend unit test tasks (e.g.
  T008/T009, T014/T015, T020/T021, T025/T026) touch different files/languages and can be done in
  parallel.
- T036/T037 (backend vs. frontend verification) are independent and parallelizable.

## Implementation Strategy

**MVP**: Phase 1 + Phase 2 + Phase 3 (US1) — init containers distinct from app containers is the
single highest-value, most standalone slice (spec.md ranks it P1 and it requires no events
correlation, unlike US2/US4).

**Incremental delivery**: US1 → US2 (both P1) → US3 → US4 → US5 (all P2), each independently
demoable per its Independent Test, converging into one "Lifecycle" section by the end of Phase 7.
