---

description: "Task list for Helm Mutate (Upgrade, Rollback, Uninstall) with GitOps-Ownership Warnings"
---

# Tasks: Helm Mutate (Upgrade, Rollback, Uninstall) with GitOps-Ownership Warnings

**Input**: Design documents from `/specs/006-helm-mutate/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/protocol.md, quickstart.md — all present.

**Tests**: Included. Constitution Principle III (Tested at the Boundaries, NON-NEGOTIABLE) requires protocol round-trip tests, unit tests for decision logic, and scripted-fake-backend E2E tests per user story — this is a hard project rule, not an optional add-on.

**Organization**: Tasks are grouped by user story (US1 rollback, US2 upgrade, US3 uninstall) per spec.md's priority order (P1 → P2 → P3), preceded by a Foundational phase for the shared protocol types every story depends on.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies on incomplete tasks)
- **[Story]**: US1 / US2 / US3

## Phase 1: Setup

- [ ] T001 Verify `helm` binary detection utility exists or add one: `helm_binary_available() -> bool` in `src/backend/helm.rs`, wrapping `which helm` / `helm version --short` (async, non-blocking) — used by upgrade/uninstall to fail fast with the FR-012 "clear, specific error" when absent, per research.md R4.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Protocol types and command routing every user story depends on. No user story task may start before this phase is complete.

- [ ] T002 [P] Add `HelmMutationAction` (`Rollback | Upgrade | Uninstall`) and `HelmMutationStage` (`Preview | Apply`) enums to `src/protocol/v1/helm.rs`, `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]`, `#[serde(rename_all = "snake_case")]`, per data-model.md.
- [ ] T003 [P] Add `HelmMutationPreviewReady`, `HelmMutationApplied`, `HelmMutationFailed` structs to `src/protocol/v1/helm.rs` exactly as specified in data-model.md (field names, types, and the `gitops_warning: Option<GitOpsOriginRef>` reuse of 005's type unmodified).
- [ ] T004 Add 5 new `BackendCmdV1` variants to `src/protocol/v1/cmd.rs` per contracts/protocol.md: `PreviewHelmRollback`, `ApplyHelmRollback`, `PreviewHelmUpgrade`, `ApplyHelmUpgrade`, `UninstallHelmRelease` (field shapes exactly as in the contract). Depends on T002.
- [ ] T005 Add 3 new `SystemEvent` variants to `src/protocol/v1/event.rs`: `HelmMutationPreviewReady(HelmMutationPreviewReady)`, `HelmMutationApplied(HelmMutationApplied)`, `HelmMutationFailed(HelmMutationFailed)`. Depends on T003.
- [ ] T006 Add `mutation_target()` match arms in `src/protocol/v1/mutation.rs` for all 5 new `BackendCmdV1` variants, each returning `MutationTarget::Namespaced { namespaces: vec![namespace.clone()] }` per research.md R1 — the exhaustive match has no wildcard, so the compiler forces this; do not add one. Depends on T004.
- [ ] T007 [P] Round-trip serde tests in `tests/protocol_serde.rs` for all 5 new `BackendCmdV1` variants and 3 new `SystemEvent` variants, following the existing `roundtrip(...)` pattern used for `ApplyResourceYaml`/`SelectHelmRelease` (cmd.rs:664-785 style). Depends on T004, T005.
- [ ] T008 Regenerate TypeScript bindings (`ts-rs` codegen, existing project script) so `ui/src/protocol/generated/` picks up the 5 commands + 3 events + new helm.rs types. Depends on T002–T005.
- [ ] T009 [P] Add a small frontend preview-state slice (mirrors `ui/src/store/editSession.ts`'s `'diffing' → 'applying'` shape per research.md R2, but as its own module since the domain differs) at `ui/src/store/helmMutation.ts`: `status: 'idle' | 'previewing' | 'preview_ready' | 'preview_failed' | 'applying' | 'applied' | 'apply_failed'`, holding the current `HelmMutationPreviewReady` payload when present.

**Checkpoint**: Protocol compiles, round-trips pass, bindings regenerated — user story implementation can now begin.

---

## Phase 3: User Story 1 - Roll back a release to a prior revision (Priority: P1) 🎯 MVP

**Goal**: An operator can pick a prior revision, see a diff-preview (current manifest vs. stored target manifest — no rendering needed per research.md R4), confirm, and have the rollback applied as a new revision.

**Independent Test**: Select a release with 2+ revisions, roll back to an earlier one, verify the diff-preview appears before any mutation, confirm, and verify the revision count increments with the target manifest now live — without touching upgrade or uninstall code.

### Tests for User Story 1

- [ ] T010 [P] [US1] Unit test in `src/backend/helm.rs` (or a new `#[cfg(test)]` module there): rollback diff-preview computation against fixture `HelmReleaseDetails`/`HelmRevisionSummary` data — asserts `current_manifest`/`target_manifest` populate correctly and no cluster call is made.
- [ ] T011 [P] [US1] Unit test: rollback against a target revision whose stored manifest fails to decode (corrupted payload edge case) — asserts `HelmMutationFailed { stage: Preview, .. }` with a clear message, no panic.
- [ ] T012 [P] [US1] Unit test: `ApplyHelmRollback` with a `base_revision` that no longer matches the release's live revision (stale-preview edge case) — asserts `HelmMutationFailed { stage: Apply, .. }`, no write attempted.
- [ ] T013 [P] [US1] Playwright E2E (scripted fake backend) in `ui/tests/e2e/` (path matching existing 005 E2E test file naming convention): drives the rollback flow end-to-end by injecting `HelmMutationPreviewReady`/`HelmMutationApplied` events and asserting `PreviewHelmRollback`/`ApplyHelmRollback` commands are emitted; includes both the GitOps-warning-present (Flux-owned fixture release) and GitOps-warning-absent (bare-release fixture) cases per FR-009/FR-010.

### Implementation for User Story 1

- [ ] T014 [US1] `async fn preview_helm_rollback(client, namespace, release, target_revision) -> Result<HelmMutationPreviewReady, HelmMutationFailed>` in `src/backend/helm.rs`: fetch the release's current `HelmReleaseDetails` (reuse existing 005 read path) and the target revision's stored manifest (reuse existing revision-fetch/decode logic), populate `gitops_warning` directly from `HelmReleaseDetails.origin` (research.md R5 — reuse `resolve_release_origin`/`origin_from_labels` unmodified, no new detection). Depends on T002, T003.
- [ ] T015 [US1] `async fn apply_helm_rollback(client, namespace, release, target_revision, base_revision) -> Result<HelmMutationApplied, HelmMutationFailed>` in `src/backend/helm.rs`: re-check the release's live revision against `base_revision` (stale-preview guard), then write a new release-storage revision entry pointing at the target's manifest, using the same release-storage Secret/ConfigMap read/write conventions 005 established for reads (standard Helm rollback semantics — creates a new revision, does not delete intervening ones). Depends on T014.
- [ ] T016 [US1] Wire `PreviewHelmRollback`/`ApplyHelmRollback` command handlers into the backend's command-dispatch module (extends the existing helm command handling alongside `RefreshHelmReleases`/`SelectHelmRelease`), emitting the corresponding `SystemEvent`. Depends on T014, T015, T006.
- [ ] T017 [P] [US1] Generalize 003's YAML-apply diff-view component (identify the concrete component file via `ui/src/components/panels/` or wherever 003 built it) to accept two arbitrary manifest strings (`current_manifest`/`target_manifest`) instead of live-vs-edited-buffer state, per research.md R2 and contracts/protocol.md's frontend-consumption contract.
- [ ] T018 [US1] Add a "Roll back" action on each revision row in `ui/src/resources/helm.tsx`'s Inspector (005's existing revision-history list), dispatching `PreviewHelmRollback` and routing the result through the T009 state slice into the T017 diff-view component. Depends on T009, T017, T008.
- [ ] T019 [US1] Add the GitOps-ownership warning banner to the rollback confirm step, reusing 005's origin-link rendering from `helm.tsx` (same link-out affordance) when `gitops_warning` is present, then reuse `ui/src/components/ConfirmDialog.tsx` unmodified (named-item confirm, no typed input — research.md R3) as the final confirm gate before dispatching `ApplyHelmRollback`. Depends on T018.

**Checkpoint**: Rollback is fully functional and independently testable — this is the MVP slice.

---

## Phase 4: User Story 2 - Upgrade a release to new values or a new chart version (Priority: P2)

**Goal**: An operator can edit values (existing YAML editor, no new form UI) and/or change chart version, preview the real rendered-manifest diff via `helm upgrade --dry-run`, and confirm to apply.

**Independent Test**: Open a release, choose Upgrade, edit values, preview, confirm, and observe a new revision with the edited values applied — without touching rollback or uninstall code.

### Tests for User Story 2

- [ ] T020 [P] [US2] Unit test: `helm_binary_available()` returning `false` short-circuits `preview_helm_upgrade`/`apply_helm_upgrade` with `HelmMutationFailed` and a clear "helm not found" message (FR-012), no subprocess spawned.
- [ ] T021 [P] [US2] Unit test (requires `helm` on the test-runner PATH, or mocked subprocess output per existing project convention for shelling-out tests if one exists — check `src/backend/` for prior art before adding a new mocking approach): `preview_helm_upgrade` with deliberately invalid values YAML surfaces `HelmMutationFailed { stage: Preview, .. }` with the `helm upgrade --dry-run` error surfaced verbatim (Constitution Principle I — errors reach the operator verbatim).
- [ ] T022 [P] [US2] Unit test: `ApplyHelmUpgrade` with a stale `base_revision` fails at `Apply` stage before invoking `helm upgrade`, mirroring T012's rollback case.
- [ ] T023 [P] [US2] Playwright E2E: drives the upgrade flow (values edit → preview → confirm) via injected events/asserted commands, including the invalid-values-error path and both GitOps-warning cases, mirroring T013's structure.

### Implementation for User Story 2

- [ ] T024 [US2] `async fn preview_helm_upgrade(namespace, release, values_yaml, chart_version, base_revision) -> Result<HelmMutationPreviewReady, HelmMutationFailed>` in `src/backend/helm.rs`: guard on `helm_binary_available()` (T001), then run `helm upgrade <release> <chart> -n <namespace> --dry-run -f -` (values piped via stdin) as an async subprocess, capture the rendered manifest from stdout, populate `gitops_warning` from the release's existing `origin` field (same reuse as T014). Depends on T001, T002, T003.
- [ ] T025 [US2] `async fn apply_helm_upgrade(namespace, release, values_yaml, chart_version, base_revision) -> Result<HelmMutationApplied, HelmMutationFailed>` in `src/backend/helm.rs`: re-check `base_revision` (stale-preview guard), then run `helm upgrade` for real (no `--dry-run`), parse the resulting revision number from `helm`'s output. Depends on T024.
- [ ] T026 [US2] Wire `PreviewHelmUpgrade`/`ApplyHelmUpgrade` command handlers into the backend's command-dispatch module, emitting the corresponding `SystemEvent`. Depends on T024, T025, T006.
- [ ] T027 [US2] Add an "Upgrade" action to `ui/src/resources/helm.tsx`'s Inspector that opens the existing raw YAML editor component (003's), pre-loaded with `HelmReleaseDetails.values_effective`, plus an optional chart-version field; on preview request, dispatches `PreviewHelmUpgrade` and routes through the T017 diff-view component. Depends on T009, T017, T008.
- [ ] T028 [US2] Reuse the T019 GitOps-warning-banner + `ConfirmDialog` confirm step for the upgrade apply gate, dispatching `ApplyHelmUpgrade`. Depends on T027, T019.

**Checkpoint**: Rollback (US1) and Upgrade (US2) both work independently.

---

## Phase 5: User Story 3 - Uninstall a release (Priority: P3)

**Goal**: An operator can uninstall a release through a confirmation-gated flow with no diff-preview step (nothing to preview).

**Independent Test**: Select Uninstall, confirm through the dialog, observe the release disappear from the list and its resources garbage-collected — without touching rollback/upgrade code.

### Tests for User Story 3

- [ ] T029 [P] [US3] Unit test: `uninstall_helm_release` with `helm_binary_available()` returning `false` fails clearly (FR-012), no subprocess spawned, no release-storage record removed.
- [ ] T030 [P] [US3] Unit test: `uninstall_helm_release` with a stale `base_revision` fails before invoking `helm uninstall`, mirroring T012/T022.
- [ ] T031 [P] [US3] Playwright E2E: drives the uninstall flow (confirm → applied), asserts the release is removed from the list, includes both GitOps-warning-present and -absent cases.

### Implementation for User Story 3

- [ ] T032 [US3] `async fn uninstall_helm_release(namespace, release, base_revision) -> Result<HelmMutationApplied, HelmMutationFailed>` in `src/backend/helm.rs`: guard on `helm_binary_available()` and `base_revision` staleness, then run `helm uninstall <release> -n <namespace>`. Depends on T001.
- [ ] T033 [US3] Wire `UninstallHelmRelease` command handler into the backend's command-dispatch module, emitting `HelmMutationApplied`/`HelmMutationFailed`. Depends on T032, T006.
- [ ] T034 [US3] Add an "Uninstall" action to `ui/src/resources/helm.tsx`'s Inspector: no preview step (per spec.md US3 rationale), goes straight from action click to the GitOps-warning-aware `ConfirmDialog` (reusing T019's banner logic directly, since there's no preview state to route through), dispatching `UninstallHelmRelease` on confirm. Depends on T009, T019, T008.
- [ ] T035 [US3] On `HelmMutationApplied` for an uninstall action, remove the release from the frontend's release-list state and close the Inspector if it was open on that release. Depends on T034.

**Checkpoint**: All three user stories (rollback, upgrade, uninstall) are independently functional.

---

## Phase 6: Polish & Cross-Cutting Concerns

- [ ] T036 [P] Run `quickstart.md`'s manual validation scenarios end-to-end against the dev sandbox (`podinfo` Flux-managed release for GitOps-warning cases, a bare `helm install` release for the no-warning cases), including the `helm`-binary-absent degraded path for upgrade/uninstall.
- [ ] T037 [P] `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` clean (Constitution Principle III CI gate).
- [ ] T038 [P] `pnpm -C ui lint && pnpm -C ui typecheck && pnpm -C ui test && pnpm -C ui e2e` clean.
- [ ] T039 Update `docs/MVP.md` §5a "Mutate" status from "planned next" to "✅ Shipped" (mirroring the doc-fix pattern already used for 004/005 in this repo), referencing `006-helm-mutate`.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies.
- **Foundational (Phase 2)**: Depends on Setup — BLOCKS all user stories (protocol types, command routing, bindings).
- **User Stories (Phase 3–5)**: All depend on Foundational completion. US1/US2/US3 are independently testable once Foundational is done and can proceed in parallel across agents/developers — they touch `src/backend/helm.rs` and `ui/src/resources/helm.tsx` in different functions/sections, but **file-level overlap in those two files means the actual write operations within them should be sequenced or carefully merged** (see Parallelization note below).
- **Polish (Phase 6)**: Depends on all three user stories being complete.

### Parallelization note (shared files across US1/US2/US3)

`src/backend/helm.rs` and `ui/src/resources/helm.tsx` are touched by all three user-story phases (T014–T019, T024–T028, T032–T035). Per the user's standing preference for parallel sub-agent fan-out on spec-kit implementations: run US1, US2, and US3 as three parallel agents once Phase 2 (Foundational) is complete and merged, each appending its own new functions/JSX blocks to the shared files rather than editing shared regions — then run a short sequential integration pass (build + `cargo clippy` + `pnpm typecheck`) to resolve any merge friction in the two shared files before Polish. Tests within each story (T010–T013, T020–T023, T029–T031) are file-isolated per story and fully parallelizable.

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Phase 1 (Setup) → Phase 2 (Foundational, blocking) → Phase 3 (US1 Rollback).
2. **STOP and VALIDATE**: run T010–T013, confirm rollback works end-to-end per quickstart.md Scenario 1.
3. Rollback alone is a shippable MVP increment per spec.md's own framing ("delivers standalone value even if upgrade/uninstall ship later").

### Incremental Delivery

1. Setup + Foundational → foundation ready.
2. US1 (Rollback) → validate independently → MVP.
3. US2 (Upgrade) → validate independently.
4. US3 (Uninstall) → validate independently.
5. Polish.

### Parallel Agent Strategy (per user's explicit instruction)

After Phase 2 completes and is verified (`cargo build`, `cargo test` green), fan out three parallel agents — one per user-story phase (US1/US2/US3) — each working its own task block above, followed by one short sequential merge-and-verify pass before Phase 6 Polish, per `feedback_parallelize_implementation.md`.
