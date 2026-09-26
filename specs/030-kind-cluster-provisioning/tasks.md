# Tasks: In-App Local Kind Cluster Provisioning

**Input**: Design documents from `/specs/030-kind-cluster-provisioning/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/kind-cluster-provisioning.md, quickstart.md

**Tests**: Included per the constitution's non-negotiable testing principle (III) and the
spec's explicit call for protocol round-trip tests, unit tests on fixtures, and Playwright e2e.

**Organization**: Tasks are grouped by user story (US1-US4 from spec.md), preceded by Setup and
Foundational phases, followed by Polish.

## Format: `[ID] [P?] [Story] Description`

## Phase 1: Setup

- [x] T001 Add a `kind` fixture kubeconfig file with 2+ unrelated pre-existing contexts at
      `src/backend/fixtures/kubeconfig_multi_context.yaml` (or alongside existing kube.rs test
      fixtures if a fixtures dir already exists), for the non-destructiveness merge test in T010.
- [x] T002 [P] Confirm `tokio::process::Command` and `serde_yaml` (or the crate already used for
      YAML in `src/backend/helm.rs`) are available as dependencies in `Cargo.toml`; add only if
      genuinely missing (do not add a new YAML crate if one is already a dependency).

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Protocol types and backend module shared by every user story below.

**⚠️ CRITICAL**: No user story task may begin until this phase is complete.

- [x] T003 Add `KindClusterProfileId` (`"bare" | "custom_yaml"`), `KindPrerequisites`,
      `KindClusterSummary`, `KindClusterOperationKind` payload types (per data-model.md) to a new
      `src/protocol/v1/kind_cluster.rs`, derived `Serialize, Deserialize, TS` matching the
      existing convention in `src/protocol/v1/bridge_types.rs`. Re-export from
      `src/protocol/v1/mod.rs`.
- [x] T004 Add the five new `BackendCmdV1` variants (`RefreshKindPrerequisites`,
      `RefreshKindClusters`, `CreateKindCluster { request_id, name, profile, custom_yaml }`,
      `CancelCreateKindCluster { request_id }`, `DeleteKindCluster { request_id, name }`) to
      `src/protocol/v1/cmd.rs`, per contracts/kind-cluster-provisioning.md.
- [x] T005 Add the four new `StreamEvent` variants (`KindPrerequisitesUpdated`,
      `KindClustersUpdated`, `KindProvisionProgress { request_id, stage }`,
      `KindClusterActionFinished { request_id, ok, error }`) to `src/protocol/v1/event.rs`, per
      contracts/kind-cluster-provisioning.md.
- [x] T006 Classify all 5 new commands in `src/protocol/v1/mutation.rs`:
      `RefreshKindPrerequisites`/`RefreshKindClusters` → `MutationTarget::ReadOnly`;
      `CreateKindCluster`/`DeleteKindCluster`/`CancelCreateKindCluster` → `MutationTarget::Local`.
      Since both match arms in mutation.rs are exhaustive with no wildcard, this MUST compile
      cleanly with the new arms added (no `_ =>` allowed).
- [x] T007 [P] Create `src/backend/kind.rs` module skeleton: `pub async fn detect_prerequisites()
      -> KindPrerequisites`, `pub async fn list_clusters() -> Result<Vec<KindClusterSummary>,
      String>`, `pub async fn validate_cluster_name(name: &str) -> Result<(), String>` (rules
      from data-model.md: 1-63 chars, lowercase alphanumeric + `-`, must start/end alphanumeric),
      stubbed create/delete functions to be filled in by US3/US4 tasks. Register the module in
      `src/backend/mod.rs`.
- [x] T008 [P] Wire `KindPrerequisites`/`KindClusterSummary` TS bindings generation into whatever
      existing `ts-rs` export mechanism the project uses (check how `src/protocol/v1/event.rs`
      types currently reach `ui/src/protocol/generated/`) so the new types appear there after a
      build.

**Checkpoint**: Protocol compiles (`cargo build`), new types exist backend- and frontend-side.

---

## Phase 3: User Story 1 - See whether my machine is ready (Priority: P1) 🎯 MVP

**Goal**: Panel accurately reports kind + container runtime readiness, with actionable guidance
when not ready.

**Independent Test**: Toggle presence/absence of `kind` on `PATH` and container runtime running
state; confirm the panel reflects each of the 4 states in spec.md US1 acceptance scenarios.

### Tests for User Story 1

- [x] T009 [P] [US1] Serde round-trip test for `KindPrerequisites`,
      `BackendCmdV1::RefreshKindPrerequisites`, and `StreamEvent::KindPrerequisitesUpdated` in
      `src/protocol/v1/kind_cluster.rs` (or a `#[cfg(test)]` module alongside), per Principle III.
- [x] T010 [US1] Rust unit tests in `src/backend/kind.rs` for `detect_prerequisites()`: mock/stub
      the `kind version` and `docker info`/`podman info` subprocess calls (via a small
      injectable command-runner trait, matching whatever seam `src/backend/helm.rs` already uses
      for testing subprocess-dependent code, if any) covering: kind absent, kind present +
      runtime absent, kind present + runtime present-but-not-running, fully ready.
- [x] T011 [P] [US1] Playwright e2e scenario "prerequisite-detection empty state" in
      `ui/tests/e2e/kind-cluster-provisioning.spec.ts`: scripted fake backend emits
      `KindPrerequisitesUpdated` with `kind_installed: false`; assert the panel shows
      "kind is not installed" guidance text and the create-cluster action is disabled.

### Implementation for User Story 1

- [x] T012 [US1] Implement `detect_prerequisites()` in `src/backend/kind.rs`: `kind version` via
      `tokio::process::Command::new("kind").arg("version")`; `docker info` via
      `tokio::process::Command::new("docker").arg("info")`, falling back to
      `podman info` if docker's binary is not found on `PATH` (per research.md R2). Derive
      `ready = kind_installed && runtime_installed && runtime_running`.
- [x] T013 [US1] Add the `RefreshKindPrerequisites` command handler in
      `src/backend/cmd/cluster.rs` (or a new `src/backend/cmd/kind_cluster.rs` handler module if
      `cluster.rs` is getting large — match existing file-splitting convention), calling
      `kind::detect_prerequisites()` and emitting `StreamEvent::KindPrerequisitesUpdated`.
- [x] T014 [P] [US1] Create `ui/src/components/panels/KindClusterPanel.tsx` with a
      prerequisites banner section: ready state (green, "Create cluster" enabled), kind-missing
      state (guidance + install link, e.g. https://kind.sigs.k8s.io/docs/user/quick-start/#installation),
      runtime-missing vs. runtime-not-running states (distinct copy per FR-002), and a manual
      refresh control that dispatches `RefreshKindPrerequisites`.
- [x] T015 [US1] Extend `ui/src/store/handlers/stream.ts` to reduce
      `KindPrerequisitesUpdated` into new `session`/local state (e.g.
      `state.kindPrerequisites`), following the existing reducer-per-event-variant pattern
      (see the `DrainProgress` case at line ~108 for the shape to mirror).
- [x] T016 [US1] Mount `KindClusterPanel` in the app's Settings/cluster-management surface
      (locate the existing Settings navigation entry point used by spec 015's "Available Agents"
      panel or similar, in `ui/src/components/`, and add a sibling entry for this panel).
- [x] T017 [US1] Dispatch `RefreshKindPrerequisites` on panel mount.

**Checkpoint**: US1 fully functional and independently testable — panel accurately shows
readiness state in all 4 acceptance-scenario combinations.

---

## Phase 4: User Story 2 - See existing local clusters (Priority: P1)

**Goal**: Panel lists existing kind clusters with an accurate empty state and manual refresh.

**Independent Test**: Seed zero/one/multiple kind clusters (via scripted fake backend for e2e,
via subprocess mocking for Rust unit tests); confirm list and empty state match spec.md US2.

### Tests for User Story 2

- [x] T018 [P] [US2] Serde round-trip test for `KindClusterSummary` and
      `BackendCmdV1::RefreshKindClusters` / `StreamEvent::KindClustersUpdated`.
- [x] T019 [US2] Rust unit test for `list_clusters()` parsing `kind get clusters` output
      (newline-separated cluster names — mock the subprocess output) into `Vec<KindClusterSummary>`,
      including the zero-clusters case (kind's own "No kind clusters found." stderr message
      must map to an empty `Vec`, not an error).
- [x] T020 [P] [US2] Playwright e2e scenario extension: fake backend emits
      `KindClustersUpdated` with zero, then multiple, cluster summaries; assert empty-state copy
      and list rendering respectively.

### Implementation for User Story 2

- [x] T021 [US2] Implement `list_clusters()` in `src/backend/kind.rs`:
      `tokio::process::Command::new("kind").arg("get").arg("clusters")`, parse stdout lines,
      cross-reference each name against the current context list (via the existing
      `load_and_send_contexts`-adjacent read path) to populate `context_name` (`kind-<name>`
      convention) per data-model.md.
- [x] T022 [US2] Add the `RefreshKindClusters` command handler, emitting
      `StreamEvent::KindClustersUpdated`.
- [x] T023 [P] [US2] Extend `KindClusterPanel.tsx` with the cluster list section: empty state
      ("No local clusters yet — create one") and a table/list of `KindClusterSummary` rows
      (name, status, profile), each cross-referencing `session.activeContext` from spec 012's
      state to show a "currently selected" badge when `context_name` matches, per data-model.md's
      "Relationship to Spec 012" section.
- [x] T024 [US2] Extend `stream.ts` reducer for `KindClustersUpdated`.
- [x] T025 [US2] Add a manual "Refresh" button dispatching `RefreshKindClusters`.

**Checkpoint**: US1 + US2 both independently functional; panel shows accurate live inventory.

---

## Phase 5: User Story 3 - Create a new local cluster (Priority: P1)

**Goal**: Full create flow: name+profile form → validated inline → confirm summary → live
progress → success with new cluster selectable in the context switcher.

**Independent Test**: Enter a valid name, confirm, observe progress events, confirm new cluster
appears in both the panel's list and spec 012's `FederationContextSwitcher`.

### Tests for User Story 3

- [x] T026 [P] [US3] Serde round-trip tests for `CreateKindCluster`, `CancelCreateKindCluster`,
      `KindProvisionProgress`, `KindClusterActionFinished`.
- [x] T027 [US3] Rust unit tests for `validate_cluster_name()`: valid names, empty string,
      >63 chars, uppercase/invalid characters, leading/trailing `-`, and collision against a
      supplied existing-names list — covering every case in data-model.md's Validation Rules.
- [x] T028 [US3] Rust fixture test: kubeconfig-merge non-destructiveness — using the fixture
      from T001 (2+ unrelated contexts), simulate a `kind`-style write adding one new
      context/cluster/user triple, and assert the unrelated entries are byte-for-byte unchanged
      afterward (Constitution VI gate named in plan.md's Constitution Check).
- [x] T029 [US3] Rust unit test: concurrent-operation rejection — starting a second
      `CreateKindCluster`/`DeleteKindCluster` for a `cluster_name` already tracked in
      `KindClusterOperation` state must return an immediate error, never queue (FR-016).
- [x] T030 [P] [US3] Playwright e2e scenario "create-cluster confirm-and-progress flow" in
      `ui/tests/e2e/kind-cluster-provisioning.spec.ts`: fill in a valid name, assert the
      `ConfirmDialog` shows name/profile/node-count summary before any command is sent, confirm,
      assert incremental `KindProvisionProgress` stage updates render, then assert
      `KindClusterActionFinished { ok: true }` results in the cluster appearing in both the
      panel's list and the context switcher (`FederationContextSwitcher`).
- [x] T031 [P] [US3] Playwright e2e scenario: invalid/colliding name is rejected inline before
      the confirm dialog is reachable (FR-006/FR-007/SC-006).

### Implementation for User Story 3

- [x] T032 [US3] Implement `create_cluster()` in `src/backend/kind.rs`: re-validate name
      server-side, check for an existing tracked operation on that name (reject if present, per
      T029), spawn `tokio::process::Command::new("kind").arg("create").arg("cluster")
      .arg("--name").arg(&name)` with `.arg("--config").arg("-")` + piped stdin when
      `profile == "custom_yaml"` (feeding `custom_yaml`), else a minimal generated single-node
      kind-config; set `KIND_EXPERIMENTAL_PROVIDER=podman` env var when Docker was not detected
      but Podman was (research.md R2).
- [x] T033 [US3] Stream `kind`'s stderr lines as `StreamEvent::KindProvisionProgress
      { request_id, stage }` as they arrive (tokio `BufReader`/`lines()` over the child's stderr
      pipe), mirroring the line-streaming approach already used for pod logs
      (`StartPodLogs`/`PodLogsChunk` handling) if one exists — check
      `src/backend/pods.rs` for the established async line-streaming idiom before writing a new
      one.
- [x] T034 [US3] On process exit, re-run the existing `load_and_send_contexts(...)` path (spec
      012, `src/backend/kube.rs`) so the new context is picked up, re-run `list_clusters()`,
      emit `StreamEvent::KindClustersUpdated` then `StreamEvent::KindClusterActionFinished
      { request_id, ok, error }`. On non-zero exit, `error` MUST contain `kind`'s stderr
      verbatim (Principle I: backend errors reach the operator verbatim).
- [x] T035 [US3] Add `CreateKindCluster` and `CancelCreateKindCluster` command handlers: track
      the spawned child process in a `state.kind_actions: HashMap<String, ChildHandle>` registry
      (mirroring `state.node_actions` for `CancelDrainNode`), and implement cancellation by
      killing the child process and emitting a `KindClusterActionFinished { ok: false, error:
      Some("cancelled") }`.
- [x] T036 [P] [US3] Add the create-cluster form to `KindClusterPanel.tsx`: name input with
      inline DNS-1123-label validation (mirrored client-side copy of the backend rule) and
      collision check against the current `KindClusterSummary` list, profile selector
      (`bare` default, `custom_yaml` advanced toggle per research.md R7 rendering a plain
      textarea backed by `ui/src/resources/yaml.ts`'s parse helper for basic YAML validation).
- [x] T037 [US3] Wire the create form's submit into a `ConfirmDialog` (reusing the shared
      component per `ui/src/components/ConfirmDialog.tsx`) showing name/profile/node-count
      before dispatching `CreateKindCluster`.
- [x] T038 [US3] Add a `DrainProgressView`-style progress component (new, e.g.
      `KindProvisionProgressView` in `KindClusterPanel.tsx` or its own file) rendering the
      latest `stage` string per `request_id`, keyed exactly like `NodeActionsPanel.tsx`'s
      drain-progress state.
- [x] T039 [US3] Extend `stream.ts` reducer for `KindProvisionProgress` and
      `KindClusterActionFinished` (create case).

**Checkpoint**: US1-US3 all independently functional; the core "hyper powerful, all in app"
create flow works end-to-end against the fake backend.

---

## Phase 6: User Story 4 - Delete a local cluster (Priority: P2)

**Goal**: Destructive delete flow with confirmation, graceful fallback if the active context is
the one deleted.

**Independent Test**: Delete a seeded/created cluster through the app; confirm the destructive
prompt blocks until confirmed, and the cluster disappears from both the panel and the switcher.

### Tests for User Story 4

- [x] T040 [P] [US4] Serde round-trip test for `DeleteKindCluster` and the delete-path use of
      `KindClusterActionFinished`.
- [x] T041 [US4] Rust unit test: deleting the currently-active context's cluster triggers the
      same "active context no longer exists" fallback path spec 012 already has for
      `RefreshContexts` (verify this exists per research.md R4; if it does not, this task
      additionally implements the minimal fallback — pick another available context or clear
      the active context, matching FR-014).
- [x] T042 [P] [US4] Playwright e2e scenario "delete-cluster confirm flow" in
      `ui/tests/e2e/kind-cluster-provisioning.spec.ts`: trigger delete on a listed cluster,
      assert the destructive `ConfirmDialog` names the exact cluster and blocks the action until
      confirmed, confirm, and assert the cluster disappears from both the panel's list and the
      context switcher.

### Implementation for User Story 4

- [x] T043 [US4] Implement `delete_cluster()` in `src/backend/kind.rs`:
      `tokio::process::Command::new("kind").arg("delete").arg("cluster").arg("--name").arg(&name)`,
      applying the same concurrent-operation guard as create (FR-016).
- [x] T044 [US4] Add the `DeleteKindCluster` command handler: on process exit, re-run
      `load_and_send_contexts(...)` and `list_clusters()`, emit `KindClustersUpdated` then
      `KindClusterActionFinished`; if the deleted cluster's `context_name` was
      `session.activeContext`, ensure the fallback from T041 fires.
- [x] T045 [P] [US4] Add delete affordance to `KindClusterPanel.tsx`'s cluster list rows,
      wired through the shared `ConfirmDialog` in destructive mode (`destructive: true`,
      naming the exact cluster), matching `HelmReleaseActionsPanel.tsx`'s usage.
- [x] T046 [US4] Extend `stream.ts` reducer for the delete-path terminal event, removing the
      deleted cluster from local list state immediately upon `ok: true` (no optimistic removal
      before that per Principle I).

**Checkpoint**: All 4 user stories independently functional and testable.

---

## Phase 7: Polish & Cross-Cutting Concerns

- [x] T047 [P] Run `pnpm -C ui typecheck` and `pnpm -C ui lint`; fix any violations introduced
      by `KindClusterPanel.tsx` and store changes.
- [x] T048 [P] Run `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`; fix any
      violations introduced by `src/backend/kind.rs` and `src/protocol/v1/kind_cluster.rs`.
- [x] T049 Run `cargo test` and `pnpm -C ui test:e2e` (or the project's Playwright invocation)
      for the full new suite; confirm all tasks' tests pass.
- [x] T050 Execute quickstart.md's manual validation scenarios end-to-end against a real local
      `kind` + Docker/Podman install, at least once, to confirm the automated tests match real
      tool behavior (kind's actual progress-line wording, actual `kind get clusters` output
      format). (Done 2026-09-26 against real kind v0.27.0 + Docker via the headless real backend;
      all 5 scenarios passed; the 9.4s silent control-plane start led to an elapsed counter in the
      progress view. UI dialogs covered by the fake-backend e2e; Podman not tried. See
      checklists/live-validation.md.)
- [x] T051 [P] Update `docs/FEATURE_BACKLOG.md` if needed to note that local kind provisioning
      (narrower than the previously-declined general cluster lifecycle scope) has shipped, per
      the task's context about reversing part of an earlier scoping call.

---

## Dependencies & Execution Order

### Phase Dependencies

- Setup (Phase 1) → Foundational (Phase 2) → User Stories (Phases 3-6) → Polish (Phase 7).
- Phase 2 (protocol types + `kind.rs` skeleton) blocks every user story phase.
- US1 and US2 (Phase 3, 4) are independent of each other and can proceed in parallel once Phase
  2 completes.
- US3 (Phase 5, create) depends on US1's prerequisite-check plumbing being in place (it
  re-validates prerequisites at create time) and benefits from US2's list/name-collision data,
  but its own tasks are self-contained enough to be developed in parallel with US2 if the
  collision check is stubbed initially.
- US4 (Phase 6, delete) depends on US2's list existing (to have something to delete) and reuses
  the `KindClusterActionFinished`/context-refresh plumbing US3 establishes — implement after
  US3 for the smoothest path, though its own tasks touch different code paths (`delete_cluster`
  vs `create_cluster`) and could be parallelized by a second developer.

### Suggested MVP Scope

Phases 1-4 (Setup, Foundational, US1, US2) deliver a read-only "see what's on my machine"
MVP. Phase 5 (US3, create) is the feature's core value and should ship in the same release as
US1/US2 given the task's "hyper powerful, all in app" framing — but if a smaller increment is
needed, US1+US2 alone are independently shippable and testable per spec.md.

## Parallel Example: Foundational Phase

```bash
Task: "Create src/protocol/v1/kind_cluster.rs payload types (T003)"
Task: "Create src/backend/kind.rs module skeleton (T007)"
Task: "Wire ts-rs binding generation for new types (T008)"
```

## Notes

- Every mutating command (`CreateKindCluster`, `DeleteKindCluster`) MUST be sent only after a
  `ConfirmDialog` confirmation on the frontend — no task above implements a "confirm-less" path.
- All `kind`/`docker`/`podman` invocation MUST use `tokio::process::Command::new(bin).arg(...)`
  argument-array form — no task above should introduce a `sh -c` or formatted shell string.
- Cluster name validation MUST be enforced backend-side regardless of frontend state (T027,
  T032) — the frontend copy (T036) is UX-only, not the security boundary.
