---
description: "Task list for KEDA Autoscaler Pane"
---

# Tasks: KEDA Autoscaler Pane

**Input**: Design documents from `/specs/029-keda-autoscaler-pane/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/protocol.md, quickstart.md

**Tests**: Included — the task explicitly requires a Playwright e2e spec and this repo's
constitution (III) mandates protocol round-trip tests and boundary tests for every behavior
change, so Rust unit/protocol tests and the e2e spec are part of the task list, not optional.

**Organization**: Tasks are grouped by user story (spec.md P1–P3) to enable independent
implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (US1, US2, US3, US4)

## Phase 1: Setup

- [ ] T001 Confirm `keda.sh` group name constants and default values to use in
  `src/backend/keda.rs`: CRD names `scaledobjects.keda.sh`, `scaledjobs.keda.sh`,
  `triggerauthentications.keda.sh`, `clustertriggerauthentications.keda.sh`; KEDA defaults
  `minReplicaCount: 0`, `maxReplicaCount: 100` (data-model.md `KedaResourceSummary`). No code
  yet — write these as doc-comment constants scaffolding in a new empty
  `src/backend/keda.rs` file with the module doc comment from research.md R1.
- [ ] T002 Register the new module in `src/backend/mod.rs` (`mod keda;`), matching how `flux`
  is registered, so it compiles as an empty stub.

**Checkpoint**: Project compiles with the new empty module registered.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Protocol types and commands/events all user stories depend on.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [ ] T003 [P] Add `src/protocol/v1/keda.rs` with `KedaOverview`, `KedaResourceSummary`,
  `KedaResourceDetails`, `KedaTrigger`, `TriggerAuthRef`, `ScaleReasoning`,
  `ScaleReasoningState` per data-model.md's field tables exactly (field names, types,
  `#[derive(Debug, Clone, Serialize, Deserialize, TS)]` matching `flux.rs`'s pattern). Export
  the module from `src/protocol/v1/mod.rs` and re-export the new types from
  `src/protocol/mod.rs` alongside `FluxOverview`'s existing re-export.
- [ ] T004 Add `BackendCmdV1::RefreshKeda`, `SelectScaledObject { namespace: String, name:
  String }`, `SelectScaledJob { namespace: String, name: String }` to `src/protocol/v1/cmd.rs`
  per contracts/protocol.md, placed near the existing `RefreshFlux`/`SelectFluxResource`
  variants; add serde round-trip tests for all three mirroring the existing
  `roundtrip(BackendCmdV1::SelectFluxResource {...})` test in the same file.
- [ ] T005 Add `SystemEvent::KedaOverviewUpdated(KedaOverview)`,
  `KedaResourceDetailsUpdated(KedaResourceDetails)`, `KedaResourceNotFound { kind: String,
  namespace: String, name: String }` to `src/protocol/v1/event.rs` per contracts/protocol.md,
  placed near `FluxOverviewUpdated`/`FluxResourceDetailsUpdated`/`FluxResourceNotFound`; add
  serde round-trip tests for all three mirroring the existing Flux event tests in the same
  file.
- [ ] T006 Add a `section_keys::autoscalers()` (or equivalent) route key in
  `src/protocol/v1/mod.rs`'s `section_keys` module, mirroring `section_keys::gitops()` (or
  whatever the Flux route key is named), for use by `Route::global(...)` in the new backend
  handlers.
- [ ] T007 Run `cargo build --workspace --all-targets` to confirm the protocol additions
  compile and the `ts-rs` bindings regenerate without conflicts (no code depends on the new
  types yet, so this is a pure scaffolding checkpoint).

**Checkpoint**: Protocol contract fully defined and tested; ready for backend logic and UI to
consume it in parallel per story.

---

## Phase 3: User Story 1 - See whether KEDA is scaling my workloads (Priority: P1) 🎯 MVP

**Goal**: List every `ScaledObject`/`ScaledJob` in the current namespace scope with target
workload, current replicas, and min/max bounds; honest empty states for "KEDA not installed" and
"no autoscalers configured".

**Independent Test**: Open the Autoscalers section against a fixture with one `ScaledObject`
scaled to 4/1–10 replicas; confirm it renders correctly and disappears when namespace-filtered
out.

### Tests for User Story 1

- [ ] T008 [P] [US1] Rust unit test in `src/backend/keda.rs` for CRD detection: given a fake
  `CustomResourceDefinition` list containing `scaledobjects.keda.sh` served, `installed` is
  `true`; given none of the four KEDA CRD names present, `installed` is `false` (mirrors
  `flux.rs::find_served` test conventions).
- [ ] T009 [P] [US1] Rust unit test in `src/backend/keda.rs` for `KedaResourceSummary`
  min/max defaulting: a `ScaledObject` spec with no `minReplicaCount`/`maxReplicaCount` field
  set produces `min_replicas: 0`, `max_replicas: 100` (data-model.md's documented KEDA
  defaults).
- [ ] T010 [P] [US1] Playwright e2e scaffold in `ui/tests/e2e/keda-autoscaler-pane.spec.ts`:
  navigate to "Autoscalers", assert `RefreshKeda` is sent (mirrors
  `us1-applications-list.spec.ts`'s `goToApplications` helper pattern), per quickstart.md step
  1–2. This is the first test in the file; later US tasks append to it.

### Implementation for User Story 1

- [ ] T011 [US1] Implement CRD discovery in `src/backend/keda.rs`: `find_served` helper
  (mirrors `flux.rs`) for the four `keda.sh` CRD names from T001; `installed` is `true` iff
  `scaledobjects.keda.sh` OR `scaledjobs.keda.sh` is served (research.md R1).
  Depends on: T001.
- [ ] T012 [US1] Implement `list_scaled_objects`/`list_scaled_jobs` in
  `src/backend/keda.rs`: list `ScaledObject`/`ScaledJob` instances as `DynamicObject` via
  `Api::<DynamicObject>::all_with`/`namespaced_with` scoped by the discovered
  `ApiResource`/`GroupVersionKind` (mirrors `flux.rs::list_instances`), respecting the current
  namespace filter the same way every other resource list command does.
  Depends on: T011.
- [ ] T013 [US1] Implement `summarize_scaled_object`/`summarize_scaled_job` in
  `src/backend/keda.rs`, producing `KedaResourceSummary` per data-model.md's field table:
  `target_kind`/`target_name` from `spec.scaleTargetRef` (default `target_kind: "Deployment"`
  for ScaledObject when unset, per KEDA's own default); `current_replicas` from
  `status.hpaReplicas`, falling back to the live target workload's replica count when absent;
  `min_replicas`/`max_replicas` per T009's defaulting rule; `paused` from the
  `autoscaling.keda.sh/paused` annotation; `ready` from `status.conditions`; `target_found` by
  attempting to fetch the target workload and recording whether it exists (FR-014).
  Depends on: T012.
- [ ] T014 [US1] Implement `scan_keda(client) -> KedaOverview` top-level entry point in
  `src/backend/keda.rs`, mirroring `flux.rs::scan_flux`, aggregating `installed` +
  `scaled_objects` + `scaled_jobs`.
  Depends on: T013.
- [ ] T015 [US1] Wire `BackendCmd::RefreshKeda` handler `refresh_keda` in
  `src/backend/cmd/integrations.rs` (mirrors `refresh_flux`), calling `scan_keda` and emitting
  `SystemEvent::KedaOverviewUpdated` via `Route::global(section_keys::autoscalers())` from T006.
  Depends on: T014, T006.
- [ ] T016 [US1] Add a 20s background poller `ensure_keda_poll`/`poll_keda_overview` in
  `src/backend/sync.rs`, mirroring `ensure_flux_poll`/`poll_flux_overview` exactly (same
  cadence, same emit path as T015).
  Depends on: T015.
- [ ] T017 [P] [US1] Add `keda: { overview?: KedaOverview }` slice + `refreshKeda()` action
  (sends `RefreshKeda`) in `ui/src/store/index.ts`, mirroring the existing `flux` slice +
  `refreshFlux()`.
  Depends on: T004.
- [ ] T018 [P] [US1] Handle `KedaOverviewUpdated` in `ui/src/store/handlers/system.ts`
  (updates the `keda` slice), mirroring the existing `FluxOverviewUpdated` handler.
  Depends on: T005, T017.
- [ ] T019 [US1] Create `ui/src/components/panels/KedaPanel.tsx`: list pane rendering
  `scaled_objects`/`scaled_jobs` rows (name, namespace, kind, target workload, current
  replicas, min/max bounds), respecting the active namespace filter; when `!overview.installed`
  render the "KEDA not detected" honest-empty-state (FR-003); when `installed` and both arrays
  are empty, render the distinct "no autoscalers configured" empty state (FR-004). Mirrors
  `FluxPanel.tsx`'s structure using `IntegrationSummary` or the plain resource-list pattern,
  whichever `FluxPanel.tsx` actually uses.
  Depends on: T018.
- [ ] T020 [US1] Register the new "Autoscalers" section in the app's navigation/section
  registry (wherever `FluxPanel`/`Applications` are registered, e.g.
  `ui/src/components/shell/*` or `ui/src/resources/*` section list), so it is reachable from
  the nav per FR-002.
  Depends on: T019.
- [ ] T021 [US1] Complete the Playwright e2e scaffold from T010 with the "KEDA not installed"
  scenario (emit `KedaOverviewUpdated` with `installed: false`, assert the empty-state text and
  zero rows) and the happy-path list scenario (emit one `ScaledObject` fixture per
  quickstart.md step 4, assert the row's namespace/current-replicas/min-max render).
  Depends on: T020, T010.

**Checkpoint**: User Story 1 fully functional and independently testable — the Autoscalers
list works end to end with both empty states.

---

## Phase 4: User Story 2 - Understand why a workload is scaled the way it is (Priority: P1)

**Goal**: Selecting a `ScaledObject`/`ScaledJob` opens an Inspector showing all triggers
(type + generic key/value metadata) and a plain-language min/max scale-reasoning explanation.

**Independent Test**: Select a fixture `ScaledObject` with one `prometheus` trigger at
mid-range replicas; confirm the Inspector shows trigger type, metadata, and the correct
reasoning message; repeat for at-floor and at-ceiling fixtures.

### Tests for User Story 2

- [ ] T022 [P] [US2] Rust unit tests in `src/backend/keda.rs` for `ScaleReasoning` derivation,
  one per rule in data-model.md's "Derivation rules" list: `TargetNotFound` (when
  `target_found == false`), `Paused` (when `paused == true`), `ScaledToZero` (when
  `min_replicas == 0 && current_replicas == 0`), `AtFloor` (when `current_replicas <=
  min_replicas`), `AtCeiling` (when `current_replicas >= max_replicas`), `MidRange`
  (otherwise) — verify rule precedence order matches data-model.md exactly (TargetNotFound
  checked before Paused, before ScaledToZero, before AtFloor/AtCeiling).
- [ ] T023 [P] [US2] Playwright e2e addition to `ui/tests/e2e/keda-autoscaler-pane.spec.ts`
  per quickstart.md step 5: select the "queue-worker" row, emit `KedaResourceDetailsUpdated`
  with a `prometheus` trigger (`threshold: "50"`, no `current_value` yet — US3 covers that) and
  `reasoning: { state: "MidRange", message: "..." }`; assert the Inspector shows the trigger
  type ("Prometheus"), its metadata, and the reasoning message text.

### Implementation for User Story 2

- [ ] T024 [US2] Implement trigger extraction in `src/backend/keda.rs`: parse
  `spec.triggers[]` from the `DynamicObject` into `Vec<KedaTrigger>` per data-model.md's field
  table — `trigger_type` from `type` verbatim, `metadata` as a `BTreeMap<String, String>` from
  `metadata` verbatim (generic, no per-type parsing beyond what T027 adds), `auth_ref` from
  `authenticationRef` when present (name + `cluster_scoped` based on whether it's
  `ClusterTriggerAuthentication` vs `TriggerAuthentication`, never resolving the referenced
  secret — constitution VI, research.md R6), `threshold` copied from
  `metadata["threshold"]`/`metadata["value"]` when present.
  Depends on: T012.
- [ ] T025 [US2] Implement `derive_scale_reasoning` in `src/backend/keda.rs` per
  data-model.md's six ordered derivation rules exactly, returning `ScaleReasoning { state,
  message }` with a human-readable `message` per rule (e.g. "At floor (1) — no trigger
  currently reports active demand.", "At ceiling (10) — demand may exceed what the configured
  maximum allows.", "Paused via the autoscaling.keda.sh/paused annotation.", "Scaled to zero —
  idle, no active demand.", "Target workload no longer exists.").
  Depends on: T013.
- [ ] T026 [US2] Assemble `KedaResourceDetails { summary, triggers, reasoning }` and wire
  `BackendCmd::SelectScaledObject`/`SelectScaledJob` handlers in
  `src/backend/cmd/integrations.rs` (mirrors `select_flux_resource`): fetch the single
  resource, call T024/T025, emit `KedaResourceDetailsUpdated`; emit `KedaResourceNotFound` when
  the named resource no longer exists.
  Depends on: T024, T025, T004, T005.
- [ ] T027 [P] [US2] Add `selectKedaResource(kind, namespace, name)` action to
  `ui/src/store/index.ts` (sends `SelectScaledObject` or `SelectScaledJob` based on `kind`) and
  handle `KedaResourceDetailsUpdated`/`KedaResourceNotFound` in
  `ui/src/store/handlers/system.ts`, mirroring the existing Flux single-resource select/handle
  pair.
  Depends on: T026.
- [ ] T028 [US2] Create `ui/src/resources/keda.tsx` Inspector: render each trigger's type,
  its metadata as a generic key/value table (FR-008, FR-020 — no per-type custom widgets),
  `auth_ref.name` when present (never any secret value), and `reasoning.message`.
  Depends on: T027.
- [ ] T029 [US2] Wire row-selection in `KedaPanel.tsx` to call `selectKedaResource` and open
  the `keda.tsx` Inspector in place (no navigation away from the list), matching the existing
  Applications/Flux inspector-in-place UX.
  Depends on: T028, T019.
- [ ] T030 [US2] Complete the e2e assertions from T023 with a second fixture at
  `current_replicas === min_replicas` asserting the "AtFloor" reasoning message text renders
  distinctly (quickstart.md step 6 — kept to one boundary case in e2e; the full rule matrix is
  covered by T022's Rust unit tests).
  Depends on: T029, T023.

**Checkpoint**: User Stories 1 and 2 both work independently — operators can see and
understand every autoscaler's current state without leaving Fathom.

---

## Phase 5: User Story 3 - See live trigger metric values when metrics are available (Priority: P2)

**Goal**: When Fathom's Prometheus integration is configured and reachable, a `prometheus`-type
trigger's Inspector shows its live current value next to its threshold; every other case falls
back gracefully with no error.

**Independent Test**: With the metrics stack configured, open a `prometheus`-trigger
`ScaledObject`'s Inspector and confirm the live value renders; with the metrics stack
unavailable (or a non-Prometheus trigger), confirm only trigger-type + threshold render, no
error.

### Tests for User Story 3

- [ ] T031 [P] [US3] Rust unit test in `src/backend/keda.rs` for the Prometheus-trigger query
  path: given a `prometheus` trigger with `metadata.serverAddress` and `metadata.query` set,
  and a mocked/injectable `MetricsProviderTrait` returning a value, `current_value` is
  populated; given the metrics stack unreachable (provider returns an error) or a non-`
  prometheus` trigger type, `current_value` is `None` and no error propagates out of trigger
  extraction (FR-012/FR-013 — a fetch failure must not affect already-built trigger data).
- [ ] T032 [P] [US3] Playwright e2e addition to `ui/tests/e2e/keda-autoscaler-pane.spec.ts`
  per quickstart.md step 5: extend the `prometheus` trigger fixture from T023 with
  `current_value: 73`, assert the Inspector shows "current: 73 / threshold: 50" (or equivalent
  live-value-next-to-threshold rendering); add a second assertion with `current_value:
  undefined`/absent confirming only the trigger type + threshold render, with no error text
  present.

### Implementation for User Story 3

- [ ] T033 [US3] Extend trigger extraction (T024) in `src/backend/keda.rs`: when
  `trigger_type == "prometheus"` and Fathom's metrics integration
  (`src/backend/integrations.rs`'s existing Prometheus-reachability check) reports Prometheus
  configured and reachable, call `PrometheusClient::query_instant` (from
  `src/backend/metrics_provider.rs`, unchanged) using the trigger's own
  `metadata["serverAddress"]` and `metadata["query"]`/`metadata["metricName"]` fields verbatim
  (research.md R3 — no new query-building heuristics); populate `current_value` from the
  result. On any error (unreachable, malformed response, missing fields) or any other trigger
  type, leave `current_value: None` and do not propagate the error to the overall
  `SelectScaledObject`/`SelectScaledJob` response (FR-012/FR-013).
  Depends on: T024.
- [ ] T034 [US3] Update `ui/src/resources/keda.tsx`'s trigger rendering (from T028) to show
  `current_value` alongside `threshold` when `current_value` is present, and to render nothing
  extra (just the existing type + metadata display) when it is absent — no loading spinner that
  can hang forever, no error banner for a missing/failed metric value (FR-012/FR-013).
  Depends on: T028, T033.
- [ ] T035 [US3] Complete the e2e assertions from T032.
  Depends on: T034, T032.

**Checkpoint**: User Stories 1–3 all work independently — the pane degrades gracefully when
metrics aren't available, and upgrades gracefully when they are.

---

## Phase 6: User Story 4 - Trace a scaled workload's pods to its scaling history (Priority: P3)

**Goal**: Bidirectional related-resource links between a `ScaledObject`/`ScaledJob` and its
target workload's pods, plus a reference from the pod-lifecycle timeline to the responsible
autoscaler.

**Independent Test**: Inspect a pod belonging to a workload targeted by a `ScaledObject`;
confirm a related-resource link to that `ScaledObject` appears; inspect the `ScaledObject`;
confirm its related resources include the target workload's running pods.

### Tests for User Story 4

- [ ] T036 [P] [US4] Rust unit test in `src/backend/related.rs` for the new `"ScaledObject"`/
  `"ScaledJob"` match arms: given a `ScaledObject` targeting a Deployment with 2 running pods,
  `related_resources` returns a `scales` ref to the Deployment and `owns` refs to both pods.
- [ ] T037 [P] [US4] Rust unit test in `src/backend/related.rs` for the extended `"Pod"` arm:
  given a pod owned by a Deployment that is the `scaleTargetRef` of a `ScaledObject` in the
  same namespace, `related_resources` for that pod includes a `scaledBy` ref to the
  `ScaledObject`.
- [ ] T038 [P] [US4] Playwright e2e addition to `ui/tests/e2e/keda-autoscaler-pane.spec.ts`:
  open the "queue-worker" `ScaledObject`'s related-resources section, assert its target
  workload's pod(s) appear as clickable related-resource links (reuses the existing generic
  related-resource renderer — no new UI component to test beyond data wiring).

### Implementation for User Story 4

- [ ] T039 [US4] Add `"ScaledObject" | "ScaledJob"` match arm to
  `src/backend/related.rs::related_resources`, resolving the target workload via
  `spec.scaleTargetRef` (reusing `keda.rs`'s target-resolution logic from T013) and, for the
  target workload, the currently running Pods via the same pod-selector logic
  `related_for_workload` already uses.
  Depends on: T013, T039 tests T036.
- [ ] T040 [US4] Extend the existing `"Pod"` arm (`related_for_pod`) in
  `src/backend/related.rs` to additionally look up any `ScaledObject`/`ScaledJob` in the pod's
  namespace whose `spec.scaleTargetRef` resolves to the pod's owning workload, adding a
  `scaledBy` `RelatedResourceRef` when found.
  Depends on: T039.
- [ ] T041 [US4] Add a `scaledBy`-relationship reference in the pod-lifecycle diagnostics view
  (`ui/src/resources/podLifecycle.ts`, spec 008): when the pod's related resources (already
  fetched generically) include a `scaledBy` entry, surface it as a link/label in the lifecycle
  timeline header, per research.md R5 (thin reference only, no new scale-history data model).
  Depends on: T040.
- [ ] T042 [US4] Complete the e2e assertion from T038, and add one assertion opening a pod
  belonging to "queue-worker"'s target workload and confirming the `scaledBy` related-resource
  link to "queue-worker" appears in the pod Inspector.
  Depends on: T041, T038.

**Checkpoint**: All four user stories independently functional; the KEDA pane is fully
integrated with Fathom's existing interlinking and pod-lifecycle features.

---

## Phase 7: Polish & Cross-Cutting Concerns

- [ ] T043 [P] Run `cargo clippy --all-targets -- -D warnings` and fix any lint findings
  introduced by `src/backend/keda.rs`, `src/backend/related.rs`, and the protocol additions.
- [ ] T044 [P] Run `pnpm -C ui lint` and `pnpm -C ui typecheck`, fixing any findings in the new
  `KedaPanel.tsx`/`keda.tsx`/store changes.
- [ ] T045 Verify the Autoscalers list remains scrollable/usable with 200+ fixture rows
  (FR-018/SC-... large-list requirement), matching the pattern already used by other large
  resource lists (virtualization or equivalent) — add a targeted check only if the shared list
  component doesn't already guarantee this generically.
- [ ] T046 Run the full quickstart.md validation sequence end to end (`cargo test`, `pnpm -C
  ui typecheck`, `pnpm -C ui lint`, `pnpm -C ui test`, the new e2e spec) and record results.
- [ ] T047 Update `docs/MVP.md`'s integration/feature table to mark the KEDA autoscaler pane as
  shipped, mirroring the existing Flux/Hubble rows' format.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies.
- **Foundational (Phase 2)**: Depends on Setup — BLOCKS all user stories (protocol types must
  exist before any backend/frontend code references them).
- **User Story 1 (Phase 3)**: Depends on Foundational. No dependency on other stories — this is
  the MVP.
- **User Story 2 (Phase 4)**: Depends on Foundational; reuses US1's `keda.rs` discovery/summary
  functions (T012/T013) and `KedaPanel.tsx` (T019/T029 wiring), so in practice follows US1, but
  is independently testable once US1's list/select plumbing exists.
- **User Story 3 (Phase 5)**: Depends on Foundational and US2's trigger extraction (T024) —
  extends it rather than duplicating it.
- **User Story 4 (Phase 6)**: Depends on Foundational and US1's target-resolution logic (T013);
  independent of US2/US3's trigger/metrics work.
- **Polish (Phase 7)**: Depends on all desired user stories being complete.

### Parallel Opportunities

- T003 (protocol types), and once done, T004/T005 (cmd/event variants) can proceed in parallel
  with each other after T003.
- Within US1: T008/T009/T010 (tests) in parallel; T017 (store slice) in parallel with backend
  tasks T011–T016 since it only depends on T004.
- Within US2/US3/US4: unit test tasks marked [P] run in parallel with each other before their
  corresponding implementation tasks.
- US4 (Phase 6) can be implemented in parallel with US2/US3 (Phase 4/5) by a different
  contributor once Phase 3 (US1) is complete, since US4 only depends on US1's target-resolution
  logic, not on trigger/reasoning/metrics work.

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1 (Setup) and Phase 2 (Foundational).
2. Complete Phase 3 (User Story 1) — the Autoscalers list with both empty states.
3. **STOP and VALIDATE**: run the T021 e2e scenarios independently.
4. This alone delivers the spec's SC-001, SC-003, and SC-006.

### Incremental Delivery

1. Setup + Foundational → protocol contract ready.
2. Add US1 → list + empty states → validate → demo (MVP).
3. Add US2 → trigger detail + scale reasoning → validate → demo.
4. Add US3 → optional live metric value → validate → demo.
5. Add US4 → pod/workload interlinking → validate → demo.
6. Polish (Phase 7) → full quickstart.md pass, docs update.
