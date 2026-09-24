---

description: "Task list template for feature implementation"
---

# Tasks: Ephemeral Debug Containers

**Input**: Design documents from `/specs/025-ephemeral-debug-containers/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/add-ephemeral-container.md, quickstart.md

**Tests**: Test tasks are included — the constitution (Principle III, NON-NEGOTIABLE) requires
boundary tests for every user story and protocol variant, and the implementation brief explicitly
requires a Playwright e2e scenario.

**Organization**: Tasks are grouped by user story to enable independent implementation and
testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (US1, US2, US3)

## Path Conventions

Single project (existing Fathom layout): Rust backend in `src/`, React/TS UI in `ui/src/`,
Playwright e2e in `ui/tests/e2e/`.

---

## Phase 1: Setup

**Purpose**: No new project scaffolding needed — this feature extends existing modules only.

- [x] T001 Confirm `kube` crate version in `Cargo.toml` exposes an ephemeral-containers
      subresource patch method (or confirm the JSON/strategic merge patch fallback path), and
      note the exact call shape to use in `src/backend/pods.rs` (or the module that owns pod
      cluster access — verify the exact file via `grep -rn "Api::<Pod>" src/backend/`).

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Protocol-level plumbing that every user story's UI/backend work depends on.

**⚠️ CRITICAL**: Must complete before any User Story phase begins.

- [x] T002 Add `BackendCmdV1::AddEphemeralContainer { namespace: String, pod: String,
      target_container: Option<String>, image: String, ephemeral_container_name: Option<String> }`
      variant to `src/protocol/v1/cmd.rs`, per contracts/add-ephemeral-container.md. Follow the
      existing field-shape conventions used by `DeletePod`/`StartPodExec` in the same file.
- [x] T003 Add a `roundtrip(BackendCmdV1::AddEphemeralContainer { .. })` serialization test in
      `src/protocol/v1/cmd.rs`'s existing test module, mirroring the `StartPodExec` roundtrip
      tests already present there.
- [x] T004 Classify `AddEphemeralContainer` in `src/protocol/v1/mutation.rs`'s
      `mutation_target()` match as `MutationTarget::Namespaced { namespaces: vec![namespace] }`,
      in the same arm group as `DeletePod`/`ScaleDeployment` (around the "Namespaced cluster
      writes" comment block). The match is exhaustive with no wildcard arm — the compiler will
      force this addition.
- [x] T005 [P] Add a unit test in `src/protocol/v1/mutation.rs` asserting
      `AddEphemeralContainer { namespace: "ns".into(), .. }.mutation_target()` returns
      `MutationTarget::Namespaced { namespaces: vec!["ns".to_string()] }` and that
      `.writes_cluster()` is `true`.
- [x] T006 Add `is_ephemeral: bool` field to `ContainerLifecycle` in `src/protocol/v1/pods.rs`
      (per data-model.md); default `false` for all existing (non-ephemeral) construction sites —
      grep for `ContainerLifecycle {` constructors in `src/backend/` and update each to set
      `is_ephemeral: false` explicitly (no `Default` derive currently exists on this struct, per
      the file — verify and add explicit fields at each call site rather than relying on
      `..Default::default()`).
- [x] T007 Run `cargo build --workspace --all-targets` after T002–T006 to confirm the exhaustive
      `mutation.rs` match and all `ContainerLifecycle` construction sites compile before starting
      user-story work.

**Checkpoint**: Protocol compiles; `AddEphemeralContainer` exists and is classified;
`ContainerLifecycle` can represent ephemeral entries. No behavior yet.

---

## Phase 3: User Story 1 - Attach a debug container to a running pod (Priority: P1) 🎯 MVP

**Goal**: Operator can add an ephemeral debug container to a running pod through a confirm-gated
action, and see it appear in the container list once added.

**Independent Test**: Open a running pod's detail view, launch "Add debug container," enter an
image + target container, confirm through the guarded dialog, observe the new ephemeral container
appear in the pod's container list with a live status.

### Backend for User Story 1

- [x] T008 [US1] Implement the Kubernetes API call in `src/backend/pods.rs` (or the module
      identified in T001): given namespace/pod/target_container/image/optional name, patch the
      pod's `ephemeralcontainers` subresource to append one `EphemeralContainer` entry (name
      auto-generated as `debugger-<short-random>` when not supplied, per research.md). Return a
      typed error (not a panic) on API failure so it can flow through `CommandFinished`.
- [x] T009 [US1] Dispatch `BackendCmdV1::AddEphemeralContainer` from
      `src/backend/cmd/workloads.rs` to the T008 function, following the existing dispatch
      pattern used for `DeletePod`/`StartPodExec` in the same file, and report the result via the
      existing `CommandFinished` event path (no new event variant, per contracts/).
- [x] T010 [US1] Populate ephemeral entries in `PodDetails.containers_lifecycle` from
      `status.ephemeralContainerStatuses[]` wherever `containers_lifecycle` is currently built
      (locate via `grep -rn "containers_lifecycle" src/backend/`), setting `is_ephemeral: true`,
      `is_init: false`, and mapping `state`/`ready`/`restart_count` the same way regular
      container statuses are already mapped; `probes`/`hooks` are empty vectors for these
      entries.
- [x] T011 [US1] Add a backend unit/integration test (using the project's existing kube-rs
      test/fixture pattern — check `src/backend/pods.rs` or its test module for existing fixture
      helpers) asserting: (a) the ephemeral-containers patch call is constructed with the correct
      `targetContainerName`/`image`/`name`; (b) a duplicate-name API error surfaces as a typed
      error rather than panicking.

### Frontend for User Story 1

- [x] T012 [US1] Add `addEphemeralContainerAction()` in `ui/src/resources/pods.tsx`, alongside
      the existing `deleteAction<PodSummary>()` (~line 227), producing an `ActionDescriptor` with
      `destructive: true` and fields for `image` (required, non-empty) and `target_container`
      (required unless the pod has exactly one container, in which case auto-select it — per
      data-model.md validation rules).
- [x] T013 [US1] Wire the new action's confirm copy in `ui/src/components/ActionDialogHost.tsx`:
      extend `detailFor()` (or add a sibling branch) to render the target pod name, chosen target
      container, and image, plus the fixed warning sentence: "Ephemeral containers cannot be
      removed once added and will persist for the life of the pod." Reuse the existing
      `ConfirmDialog.tsx` component as-is — no new dialog component.
- [x] T014 [US1] Add the "Add debug container" entry point (button/menu item) to the pod detail
      view where other pod actions (e.g. delete) are already exposed — locate via
      `grep -rn "deleteAction" ui/src/` to find the pod actions toolbar/menu.
- [x] T015 [US1] Ensure the pod container list component (the one rendering
      `containers_lifecycle`) displays `is_ephemeral: true` entries using its existing
      status/state rendering, with no special-casing required beyond confirming the field is
      simply included — verify via `grep -rn "containers_lifecycle" ui/src/`.
- [x] T016 [US1] [P] Add a Playwright e2e spec at `ui/tests/e2e/ephemeral-debug-container.spec.ts`
      against the project's existing scripted fake-backend harness (see an existing spec under
      `ui/tests/e2e/` for the harness pattern), covering exactly this single scenario: load a
      fixture pod with one container → open "Add debug container" → fill image → submit → assert
      the confirm dialog shows the "cannot be removed" warning text → confirm → assert the fake
      backend received `AddEphemeralContainer` with expected fields → script a `CommandFinished`
      success plus an updated pod detail containing the new `is_ephemeral: true` entry → assert
      the new container appears in the pod's container list in the UI.

**Checkpoint**: User Story 1 is independently functional and testable end-to-end (backend patch
call, protocol wiring, confirm-gated UI action, container list reflecting the result, one e2e
test).

---

## Phase 4: User Story 2 - View debug container output before exec support exists (Priority: P2)

**Goal**: Operator can view an added ephemeral container's logs via the existing log viewer, and
sees a clear status (not an error) if it hasn't started yet.

**Independent Test**: Add an ephemeral container to a pod, select it in the container list, open
its logs via the existing viewer, confirm log output streams the same as for a regular container.

- [x] T017 [US2] Verify `PodLogsTab.tsx` and the `StartPodLogs`/`StopPodLogs` command path require
      no code changes to accept an ephemeral container's name (per research.md decision) — do
      this by exercising the flow manually per quickstart.md step 5, and fix any place that
      filters container-picker options to `is_ephemeral: false` only (there should be none, but
      confirm via `grep -rn "is_ephemeral\|is_init" ui/src/` once T015 lands).
- [x] T018 [US2] If the container-picker feeding `PodLogsTab.tsx` explicitly enumerates container
      names from a source other than `containers_lifecycle` (e.g. `PodDetails.containers: Vec<String>`,
      which per pods.rs does NOT currently include ephemeral containers), extend that enumeration
      to include ephemeral container names too, in whichever UI file builds the log-viewer's
      container selector — locate via `grep -rn "container:" ui/src/components/panels/PodLogsTab.tsx`.
- [x] T019 [US2] Confirm the UI shows the ephemeral container's current status (e.g. "waiting:
      pulling image") instead of a log-fetch error when logs are requested before the container
      starts running — reuse whatever existing waiting-state messaging the regular container
      log/status UI already has (check `PodLifecyclePanels.tsx`); add it only if genuinely
      missing for the ephemeral case.

**Checkpoint**: User Story 2 works standalone on top of User Story 1's mutation, using only the
pre-existing log-viewing mechanism.

---

## Phase 5: User Story 3 - Understand irreversibility before committing (Priority: P3)

**Goal**: The confirm dialog reliably shows target pod, target container, image, and the
irreversibility warning before the confirm control is enabled.

**Independent Test**: Open "Add debug container," verify the summary and warning are both visible
before confirm can be pressed.

- [x] T020 [US3] Ensure the confirm control in the T013 dialog wiring is disabled until `image` is
      non-empty and `target_container` is resolved (set or auto-selected), per spec FR-013 — this
      should already follow from T012/T013's field validation; add an explicit UI test/assertion
      if not already covered by T016's e2e spec.
- [x] T021 [US3] [P] Add a focused UI unit test (Vitest, alongside existing component tests under
      `ui/src/components/` or `ui/src/resources/`) asserting the action descriptor's `detail`
      text contains the pod name, target container name, image name, and the exact irreversibility
      sentence, without needing a full Playwright run for this narrower check.

**Checkpoint**: All three user stories deliver an MVP-complete, independently verifiable feature.

---

## Phase 6: Polish & Cross-Cutting Concerns

- [x] T022 [P] Run `cargo clippy --all-targets -- -D warnings` and fix any new lint findings from
      T002–T011.
- [x] T023 [P] Run `pnpm -C ui typecheck` and `pnpm -C ui lint` and fix any new findings from
      T012–T021.
- [x] T024 Update `docs/FEATURE_BACKLOG.md` (if it tracks this feature) or any MVP/status doc the
      project maintains (see how spec 015 recorded its shipped status, e.g.
      `docs/MVP.md`/similar) to note ephemeral debug containers as shipped, matching the pattern
      used for prior specs.
- [x] T025 Full verification pass: `cargo build --workspace --all-targets`,
      `cargo clippy --all-targets -- -D warnings`, `cargo test`, `pnpm -C ui typecheck`,
      `pnpm -C ui lint`, `pnpm -C ui test`, and the new e2e spec
      (`ui/tests/e2e/ephemeral-debug-container.spec.ts`) all green.

---

## Dependencies

- **Phase 1 (Setup)** → **Phase 2 (Foundational)**: T001 informs T008's exact API call shape but
  does not block T002–T007.
- **Phase 2 (Foundational)** blocks all of Phase 3 (US1): the command variant, its classification,
  and the `ContainerLifecycle` field must exist before backend/UI wiring.
- **Phase 3 (US1)** blocks Phase 4 (US2) and Phase 5 (US3): both need an ephemeral container to
  exist and be visible in the container list before their scenarios are meaningful.
- **Phase 4 (US2)** and **Phase 5 (US3)** are independent of each other and can proceed in
  parallel once Phase 3 is done.
- **Phase 6 (Polish)** runs last, after all user stories.

## Parallel Execution Examples

- Within Phase 2: T005 ([P], mutation.rs test) can run in parallel with T006 (pods.rs field) once
  T004 lands, since they touch different files.
- Within Phase 3: T012–T014 (UI action wiring) can proceed in parallel with T008–T010 (backend
  patch call), since they meet only at the protocol boundary defined in Phase 2; T016 (e2e) should
  come last, after both sides are wired.
- Within Phase 6: T022 and T023 are independent (Rust vs. UI toolchains) and can run in parallel.

## Implementation Strategy

**MVP = User Story 1 (Phase 1–3)**: delivers the entire core value — adding a debug container
through the guarded confirm flow and seeing it appear in the container list. User Story 2 (log
viewing) and User Story 3 (irreversibility-summary hardening) are incremental, additive, and can
ship in the same PR or be deferred without blocking MVP value, per their independent-test
criteria in spec.md.
