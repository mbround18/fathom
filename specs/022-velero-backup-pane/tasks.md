---
description: "Task list template for feature implementation"
---

# Tasks: Velero Backup Pane

**Input**: Design documents from `/specs/022-velero-backup-pane/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/velero-protocol.md, quickstart.md

**Tests**: Explicitly requested by the feature owner — a focused Playwright e2e spec covering the
completed/failed-Backup rendering case and the "Velero not installed" empty state — plus backend
unit/round-trip tests per Constitution Principle III (NON-NEGOTIABLE). Both are included below.

**Organization**: Tasks are grouped by user story (spec.md) to enable independent implementation
and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (US1, US2, US3, US4)

## Path Conventions

Single project: Rust backend at `src/`, Tauri/React frontend at `ui/src/`, Playwright specs at
`ui/tests/e2e/`, per plan.md's Project Structure.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Nothing new to initialize — this feature reuses the existing Rust workspace and
`ui/` toolchain as-is (no new dependency, no new project). This phase only confirms the
integration points this feature will touch.

- [x] T001 Confirm `src/backend/flux.rs`, `src/backend/sync.rs`, `src/backend/state.rs`,
      `src/backend/cmd/integrations.rs`, `src/protocol/v1/{flux,event,cmd,mutation,route}.rs`,
      `ui/src/components/panels/FluxPanel.tsx`, `ui/src/components/panels/IntegrationSummary.tsx`,
      `ui/src/store/index.ts`, `ui/src/store/handlers/system.ts`, and
      `ui/src/components/shell/PaneHost.tsx` are unmodified from the versions read during
      planning (`git status` clean on these paths) before starting, so diffs stay attributable to
      this feature.

**Checkpoint**: No code changes in this phase.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: The CRD-discovery/scan backend, the protocol types, and the plumbing to get a
`VeleroOverview` from cluster to frontend store. Every user story needs this to render anything at
all (including the "not installed" state in US4), so it is genuinely foundational, not
story-specific.

**⚠️ CRITICAL**: No user story task can be verified end-to-end until this phase is complete.

- [x] T002 [P] Add `VeleroPhase` enum (`Completed`, `Failed`, `InProgress`, `Unknown` — data-model.md
      §VeleroPhase) plus `VeleroBackupSummary`, `VeleroScheduleSummary`, `VeleroRestoreSummary`,
      `VeleroStorageLocationSummary`, and `VeleroOverview` structs (fields exactly as listed in
      `specs/022-velero-backup-pane/data-model.md`) in new file `src/protocol/v1/velero.rs`,
      deriving `Debug, Clone, Serialize, Deserialize, TS` per the pattern in
      `src/protocol/v1/flux.rs`.
- [x] T003 [P] Wire `src/protocol/v1/velero.rs` into the protocol module tree (`src/protocol/v1/mod.rs`
      and `src/protocol/mod.rs` re-exports) so `VeleroOverview`, `VeleroPhase`, etc. are importable
      the same way `FluxOverview` is from `crate::protocol`.
- [x] T004 [P] Add `section_keys::integrations_velero()` returning `SectionKey::new("integrations/velero")`
      in `src/protocol/v1/route.rs`, next to `integrations_flux()`.
- [x] T005 Add `SystemEvent::VeleroOverviewUpdated(VeleroOverview)` variant in
      `src/protocol/v1/event.rs`, next to `FluxOverviewUpdated`, plus a serialization round-trip
      test in the same file's test module (mirror `crd_instances_updated_roundtrip`/the Flux
      equivalent).
- [x] T006 Add `BackendCmdV1::RefreshVelero` unit variant in `src/protocol/v1/cmd.rs`, next to
      `RefreshFlux`, plus a round-trip test in that file's test module (mirror the `RefreshFlux`
      roundtrip test, or add one if none exists — pattern from `SelectCrdKind`'s roundtrip test at
      `src/protocol/v1/cmd.rs:943`).
- [x] T007 Classify `BackendCmdV1::RefreshVelero` as non-mutating, routed to
      `section_keys::integrations_velero()`, in `src/protocol/v1/mutation.rs` (next to the
      `RefreshFlux`/`RefreshHubble`/`SelectCrdKind` arms at both match sites, e.g. lines ~142-144
      and ~494-496), plus a unit test asserting `RefreshVelero` is classified non-mutating
      (mirror any existing `RefreshFlux` non-mutation test, or add the pattern if Flux has none).
- [x] T008 Create `src/backend/velero.rs`: `find_served`/`list_instances`-equivalent CRD discovery
      for `backups.velero.io`, `schedules.velero.io`, `restores.velero.io`,
      `backupstoragelocations.velero.io` (mirror `src/backend/flux.rs`'s `find_served`/
      `list_instances`, reusing `kube::core::DynamicObject` — no new dependency per plan.md
      Constraints). Per research.md R5, `list_instances`-equivalent for this feature MUST return a
      `Result` distinguishing a Forbidden/Unauthorized API error from success (do not swallow to
      an empty `Vec` the way Flux's helper does), so `scan_velero` can set `permission_denied`.
- [x] T009 In `src/backend/velero.rs`, implement `summarize_backup`, `summarize_schedule`,
      `summarize_restore`, `summarize_storage_location` functions building the structs from T002 out
      of a `DynamicObject`'s `spec`/`status`/`metadata`, applying the field mappings and phase
      normalization exactly as specified in `specs/022-velero-backup-pane/data-model.md` and
      research.md R2–R4 (Schedule last-backup-age derivation via the
      `velero.io/schedule-name` label per research.md R3).
- [x] T010 In `src/backend/velero.rs`, implement `pub(super) async fn scan_velero(client: &Client)
      -> anyhow::Result<VeleroOverview>` that discovers the four CRDs, sets `installed = false`
      with empty lists when none are found (FR-003), sets `permission_denied = true` when CRDs are
      found but at least one kind's listing was Forbidden/Unauthorized (FR-012, research.md R5),
      lists+summarizes+sorts each resource kind (namespace, name — mirror `scan_flux`'s sort
      calls), and returns the aggregate `VeleroOverview`.
- [x] T011 [P] Add unit tests in `src/backend/velero.rs`'s `#[cfg(test)]` module for: phase
      normalization (`Completed`/`Failed`/`PartiallyFailed`→Failed/`FailedValidation`→Failed/
      `InProgress`/`New`→InProgress/unrecognized→`Unknown`), Schedule last-backup-age derivation
      (matching Backup found vs. none found → `None`), and BackupStorageLocation health mapping
      (`Available`/`Unavailable`/absent phase). Add an `#[ignore]`d live-cluster smoke test
      `scan_velero_against_live_cluster` mirroring `flux.rs`'s `scan_flux_against_live_cluster`.
- [x] T012 Add `velero_poll_task: Option<tokio::task::JoinHandle<()>>` field to the watch-tasks
      struct in `src/backend/state.rs` (next to `flux_poll_task`), and abort it in that struct's
      `abort_all`/equivalent cleanup method (mirror lines around `src/backend/state.rs:82` and `:91`).
- [x] T013 In `src/backend/sync.rs`, add `pub(super) async fn ensure_velero_poll(state: &mut
      BackendState, sender: &Sender<BackendEventEnvelope>)` and `async fn poll_velero_overview(...)`
      mirroring `ensure_flux_poll`/`poll_flux_overview` exactly (20s `tokio::time::interval`,
      `flux::scan_flux` → `velero::scan_velero`, `SystemEvent::FluxOverviewUpdated` →
      `SystemEvent::VeleroOverviewUpdated`, `section_keys::integrations_flux()` →
      `section_keys::integrations_velero()`, same error path via `events::notify_error`).
- [x] T014 Call `sync::ensure_velero_poll(state, sender).await;` from `src/backend/cmd/cluster.rs`
      at the same call site as the existing `sync::ensure_flux_poll(state, sender).await;`
      (`src/backend/cmd/cluster.rs:313`).
- [x] T015 In `src/backend/cmd/integrations.rs`, add `async fn refresh_velero(state: &BackendState,
      sender: &Sender<BackendEventEnvelope>)` mirroring `refresh_flux` (calls `velero::scan_velero`,
      emits `SystemEvent::VeleroOverviewUpdated` via `emit_cluster_system` on success, emits
      `events::notify_error("Failed to scan cluster for Velero resources: {e}")` on failure, emits
      `events::notify_error("Not connected to a cluster.")` if no client), and add
      `BackendCmd::RefreshVelero => refresh_velero(state, sender).await,` to that file's dispatch
      match (next to the `RefreshFlux` arm).
- [x] T016 [P] Register a `"Velero"` section (`section_keys::integrations_velero()`,
      `ScopeKind::Namespaced`, same group as Flux) in `src/sections/mod.rs`'s integrations-section
      builder, next to the `integrations_flux()` entry.
- [x] T017 Run `pnpm -C ui run generate:bindings` (or the project's equivalent ts-rs binding
      generation script — check `Makefile.toml`/`package.json` for the exact command) to produce
      `ui/src/protocol/generated/VeleroPhase.ts`, `VeleroBackupSummary.ts`,
      `VeleroScheduleSummary.ts`, `VeleroRestoreSummary.ts`, `VeleroStorageLocationSummary.ts`,
      `VeleroOverview.ts`, and the updated `SystemEvent.ts`/`BackendCmd.ts` (or equivalent
      generated union files) reflecting T002/T005/T006.
- [x] T018 Add a `velero: { overview: VeleroOverview | null }` slice and `refreshVelero: () =>
      get().send({ type: 'RefreshVelero' })` action to `ui/src/store/index.ts`, mirroring the
      existing `flux`/`refreshFlux` slice and action exactly (see lines around `589`, `1020`).
- [x] T019 Add a `VeleroOverviewUpdated` case to `ui/src/store/handlers/system.ts` that sets
      `velero.overview` to the event payload, mirroring the existing `FluxOverviewUpdated` case in
      that same file.

**Checkpoint**: A cluster with Velero installed now produces a `VeleroOverview` that reaches the
frontend store; a cluster without it produces `installed: false`. Nothing renders yet — that's
each user story's job below.

---

## Phase 3: User Story 1 - See backup health at a glance (Priority: P1) 🎯 MVP

**Goal**: An operator opens the Velero pane and sees every Backup with a correctly categorized
status (completed/failed/in-progress) and a human-readable age.

**Independent Test**: Point Fathom at a cluster with one completed and one failed Backup CR
(CRDs need not be a full Velero install — just the CRDs + sample CRs, per quickstart.md), open the
pane, confirm both render with correct status and age.

### Tests for User Story 1

- [x] T020 [P] [US1] Create fixture data (fake cluster state / mock backend responses, following
      whatever fixture mechanism the existing Playwright suite uses — check
      `ui/tests/e2e/` for the pattern other panels use, e.g. a scripted fake backend per
      Constitution Principle III) representing one completed Backup and one failed Backup, in
      `ui/tests/e2e/fixtures/velero.ts` (or the project's existing fixtures location/naming
      convention if different).
- [x] T021 [US1] Write `ui/tests/e2e/velero-pane.spec.ts` Scenario 1: open the Velero pane against
      the T020 fixture cluster, assert the completed Backup renders with a "completed"/success
      tone and the failed Backup renders with a "failed"/error tone, and both show a correct
      relative age (depends on T020; this is the first of the two hyper-focused scenarios the
      feature owner asked for).

### Implementation for User Story 1

- [x] T022 [P] [US1] Create `ui/src/components/panels/VeleroPanel.tsx`: read
      `store.velero.overview`, call `refreshVelero()` in a `useEffect` on mount (mirror
      `FluxPanel.tsx` exactly), render via `IntegrationSummary` with tiles summarizing backup
      counts by phase and rows listing each Backup (name, namespace, phase-derived tone, age)
      using a `VELERO_PHASE_TONE: Record<VeleroPhase, SummaryRow['tone']>` map (`Completed` → 'ok',
      `Failed` → 'err', `InProgress` → 'warn', `Unknown` → 'muted').
- [x] T023 [US1] In `VeleroPanel.tsx`, compute each Backup row's human-readable age from
      `completion_timestamp` (falling back to `start_timestamp` when still in progress, per
      data-model.md), using whatever relative-time utility other panels already use (search
      `ui/src/lib` / `ui/src/utils` for an existing "age"/"relative time" helper before adding a
      new one — Constitution Principle II implies reuse, not a parallel utility).
- [x] T024 [US1] Route `pane.tool === 'velero'` to `<VeleroPanel />` in
      `ui/src/components/shell/PaneHost.tsx`, next to the `pane.tool === 'flux'` case, and add
      whatever pane-tool-id registration/enum entry is needed elsewhere in that file/its types for
      `'velero'` to be a valid tool id.
- [x] T025 [US1] Add a Vitest unit test for the `VELERO_PHASE_TONE` mapping and Backup row
      construction logic in `ui/src/components/panels/VeleroPanel.test.tsx` (or colocated per the
      project's existing convention — check for `FluxPanel.test.tsx`; if none exists, place this
      test next to the component per prevailing convention).

**Checkpoint**: User Story 1 is fully functional and independently testable — a completed and a
failed Backup render correctly in the pane, verified by both Playwright and Vitest.

---

## Phase 4: User Story 4 - Understand when Velero is not installed (Priority: P1)

**Goal**: On a cluster without Velero, the pane shows an honest "not installed" state — no
errors, no blank screen.

**Independent Test**: Point Fathom at a cluster without `velero.io` CRDs, open the pane, confirm
the explicit not-installed message and no error notifications.

### Tests for User Story 4

- [x] T026 [P] [US4] Write `ui/tests/e2e/velero-pane.spec.ts` Scenario 2: open the Velero pane
      against a fixture cluster with no `velero.io` CRDs (or `installed: false` in the mocked
      backend response), assert the "Velero is not installed" empty state renders and no error
      toast/notification appears (this is the second of the two hyper-focused scenarios the
      feature owner asked for — same spec file as T021, so the suite has exactly two scenarios as
      directed).

### Implementation for User Story 4

- [x] T027 [US4] In `VeleroPanel.tsx` (from T022), add the `!overview` "waiting for scan" state and
      the `!overview.installed` "Velero is not installed on this cluster" state, using
      `MetricsUnavailable` exactly as `FluxPanel.tsx` does for its equivalent states.
- [x] T028 [US4] In `VeleroPanel.tsx`, add a distinct state for `overview.installed &&
      overview.permission_denied` (FR-012) — e.g. "Velero is installed, but this cluster
      connection doesn't have permission to read its backup resources" via `MetricsUnavailable` or
      an equivalent existing empty-state component — rendered before the normal tiles/rows so a
      denied identity never sees a misleadingly empty list.

**Checkpoint**: User Stories 1 and 4 (both P1) are both independently functional — this is the
suggested MVP cutoff.

---

## Phase 5: User Story 2 - Confirm scheduled backups are current (Priority: P2)

**Goal**: An operator sees each Schedule's last-backup age, with an explicit "no backups yet" when
a schedule has never produced one.

**Independent Test**: Point Fathom at a cluster with a Schedule whose most recent matching Backup
completed 2 hours ago; confirm the pane shows ~2h. Add a second Schedule with no matching Backup;
confirm "no backups yet".

### Implementation for User Story 2

- [x] T029 [US2] Extend `VeleroPanel.tsx` to render a Schedules section (tile: schedule count;
      rows: name, namespace, cron cadence, last-backup age or "no backups yet" per
      `last_backup_completion: Option<String>` — data-model.md), consistent with the existing
      tile/row visual language from `IntegrationSummary`.
- [x] T030 [P] [US2] Add a Vitest unit test in `ui/src/components/panels/VeleroPanel.test.tsx`
      (from T025) covering: a Schedule with a `last_backup_completion` timestamp renders a
      relative age, and a Schedule with `last_backup_completion: null` renders "no backups yet"
      exactly (not a blank cell — FR-005's explicit requirement).

**Checkpoint**: User Story 2 is independently functional on top of the P1 stories.

---

## Phase 6: User Story 3 - Check restore history and storage location health (Priority: P3)

**Goal**: An operator sees Restore history and per-BackupStorageLocation health.

**Independent Test**: Point Fathom at a cluster with a completed Restore, a failed Restore, an
`Available` BackupStorageLocation, and an `Unavailable` one with a message; confirm all four
render distinctly.

### Implementation for User Story 3

- [x] T031 [P] [US3] Extend `VeleroPanel.tsx` to render a Restores section (tile: restore count;
      rows: name, namespace, source `backup_name`, phase-derived tone via the same
      `VELERO_PHASE_TONE` map from T022, age).
- [x] T032 [P] [US3] Extend `VeleroPanel.tsx` to render a Storage Locations section (rows: name,
      health tone from `available: bool` — 'ok' when true else 'err', and `message` shown as the
      row's secondary line when unhealthy, per data-model.md's `VeleroStorageLocationSummary`).
- [x] T033 [US3] Add a Vitest unit test in `VeleroPanel.test.tsx` covering: a Restore's phase→tone
      mapping, and a Storage Location with `available: false` and a non-empty `message` rendering
      that message as the row's secondary text.

**Checkpoint**: All four user stories are independently functional.

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Satisfy FR-011 (Inspector linking) across all resource kinds, and close out the
quality gates the feature owner asked for.

- [x] T034 Add click-through from each `VeleroPanel` row (Backup, Schedule, Restore,
      BackupStorageLocation) to the existing generic CRD Inspector, using the same
      `SelectCrdKind`/`SelectCrdInstance`/`RefreshCrdInstances` commands and
      `section_keys::custom_resources()` route already used by the CRD browser (research.md R6),
      rather than a new Inspector code path — satisfies FR-011.
- [x] T035 [P] Run `cargo fmt --check` and fix any formatting drift introduced by this feature's
      Rust files.
- [x] T036 Run `cargo clippy --all-targets -- -D warnings` and fix any lint findings in
      `src/backend/velero.rs`, `src/protocol/v1/velero.rs`, and every modified file.
- [x] T037 Run `cargo build --workspace --all-targets` and fix any build errors (including the
      `egui-ui`-featured build, since it's the default feature set — confirm this feature adds no
      egui-side code and the legacy UI still compiles unchanged, per plan.md's Constitution
      Check).
- [x] T038 Run `cargo test` (full workspace) and fix any failures, including the new round-trip
      tests (T005, T006, T007) and unit tests (T011).
- [x] T039 Run `pnpm -C ui typecheck` and fix any type errors from the new generated bindings
      (T017) or `VeleroPanel.tsx`.
- [x] T040 Run `pnpm -C ui lint` and fix any findings in the new/modified frontend files.
- [x] T041 Run `pnpm -C ui test` (Vitest) and fix any failures, including T025/T030/T033.
- [x] T042 Run the new `ui/tests/e2e/velero-pane.spec.ts` (both scenarios, T021 and T026) via
      Playwright and fix any failures.
- [x] T043 Walk through `specs/022-velero-backup-pane/quickstart.md` manually (or confirm each of
      its numbered scenarios is covered by an automated test above) and note any gap.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies.
- **Foundational (Phase 2)**: Depends on Phase 1. BLOCKS every user story — nothing renders
  without T002-T019.
- **User Story 1 (Phase 3, P1)**: Depends on Phase 2 only.
- **User Story 4 (Phase 4, P1)**: Depends on Phase 2 only; shares `VeleroPanel.tsx` with US1 (T022)
  but its own states (T027/T028) are additive, not blocking on US1's rows being finished.
- **User Story 2 (Phase 5, P2)**: Depends on Phase 2; extends the same `VeleroPanel.tsx` as US1/US4
  so is easiest to land after Phase 3/4 land, though its backend data (schedules) was already
  produced in Phase 2.
- **User Story 3 (Phase 6, P3)**: Same shape as US2 — depends on Phase 2, easiest after Phase 3-5.
- **Polish (Phase 7)**: Depends on all desired user stories being complete.

### Parallel Opportunities

- T002, T003, T004 (protocol scaffolding) can run in parallel — different files.
- T011 (backend unit tests) can be written in parallel with T012-T019 once T008-T010 land.
- T016 (section registration) is independent of the rest of Phase 2 and can run in parallel.
- Within Phase 3, T020 (fixtures) and T022 (panel component skeleton) can start in parallel; T021
  depends on T020, T023/T024/T025 depend on T022.
- Phases 5 and 6 (US2, US3) can be implemented in parallel by different people once Phase 2 and
  the `VeleroPanel.tsx` skeleton (T022) exist, since they touch different sections of the same
  file but describe non-overlapping row types — coordinate merges carefully since it is the same
  file.

## Implementation Strategy

### MVP First

Suggested MVP scope: **User Story 1 + User Story 4** (both P1) — a working pane that shows Backup
health and degrades honestly when Velero isn't installed. Complete Phase 1 → Phase 2 → Phase 3 →
Phase 4, validate against quickstart.md scenarios 1-2, then decide whether to continue to
Schedules (US2) and Restores/Storage (US3) in the same pass or a follow-up.

### Full Delivery (this spec's scope)

Given the feature owner asked for the complete pane (not a staged MVP) plus the two-scenario e2e
spec up front, the practical path is: Phase 1 → Phase 2 → Phase 3 → Phase 4 → Phase 5 → Phase 6 →
Phase 7, running the full quality-gate list in Phase 7 once before declaring the feature done.
