---

description: "Task list for Resource Editing, Node Actions & Custom Resource Browsing"
---

# Tasks: Resource Editing, Node Actions & Custom Resource Browsing

**Input**: Design documents from `/specs/003-yaml-node-crd-parity/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/protocol.md, quickstart.md

**Tests**: Constitution Principle III (NON-NEGOTIABLE) requires protocol round-trip tests for every
new command/event variant, E2E tests per user story against the fake backend, and unit tests for
UI display-logic — so test tasks ARE included below, not optional.

**Organization**: Tasks are grouped by user story (US1 = YAML edit+apply, US2 = node actions,
US3 = CRD browsing) so each is independently implementable and testable, per plan.md's Project
Structure.

**Revision note**: `/speckit-analyze` flagged two coverage gaps against spec.md's edge cases —
C1 (FR-018's "CRD no longer available" state while viewing its instances) and C2 (apply failing
with "resource no longer exists" when the target is deleted mid-edit, distinct from the
`resourceVersion`-conflict path). Both are folded into this revision: two new test tasks (T015,
T049) and two implementation tasks with expanded scope (T020, T055/T056) below.

**Scope decision**: implementing all three user stories in this pass (not an MVP-only slice).
**Node action confirmation**: cordon/uncordon/drain/taint all use the existing single-click
`ConfirmDialog` pattern (same as delete/scale/restart) — no type-to-confirm. **YAML diff**: the
diff is live — `liveYaml` re-renders on every resource-watch update while the edit session is
open, with a "content changed, re-review" indicator when it shifts after the operator last
reviewed (research.md R1, data-model.md `liveYamlChangedSinceReview`).

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: US1, US2, or US3
- File paths are exact, per plan.md's Project Structure section

---

## Phase 1: Setup

- [x] T001 Add `serde_yaml = "0.9"` to root `Cargo.toml` `[dependencies]` (research.md R1)
- [x] T002 Add `diff` (jsdiff) to `ui/package.json` dependencies and run `pnpm -C ui install`
  (research.md R5)
- [x] T003 [P] Confirm `cargo build` and `pnpm -C ui typecheck` still pass after the two new
  dependencies are added, before any protocol changes

**Checkpoint**: dependencies present; nothing else depends on this beyond compiling.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Protocol plumbing every user story's backend/frontend work sits on top of. No user
story implementation should start until this phase compiles and its round-trip tests pass.

- [x] T004 Add `TaintOp` enum (`Add`, `Remove`) and the ten new `BackendCmdV1` variants
  (`ApplyResourceYaml`, `CordonNode`, `UncordonNode`, `DrainNode`, `CancelDrainNode`, `TaintNode`,
  `RefreshCrds`, `SelectCrdKind`, `RefreshCrdInstances`, `SelectCrdInstance`) in
  `src/protocol/v1/cmd.rs`, exact field shapes per contracts/protocol.md
- [x] T005 Add `StreamEvent::ResourceApplied`, `StreamEvent::DrainProgress`,
  `StreamEvent::NodeActionFinished` variants to `src/protocol/v1/event.rs`, exact shapes per
  contracts/protocol.md
- [x] T006 Add `SystemEvent::CrdCatalogUpdated` and `SystemEvent::CrdInstancesUpdated` (plus the
  `CrdCatalogEntry` struct per data-model.md) to `src/protocol/v1/event.rs`
- [x] T007 Add explicit `mutation_target()` and `section()` match arms in
  `src/protocol/v1/mutation.rs` for all ten new `BackendCmdV1` variants (T004) — `Namespaced`/
  `ClusterScoped` for the six mutating commands, `ReadOnly` for the four CRD-read commands,
  per the classification table in contracts/protocol.md; no wildcard arm (depends on: T004)
- [x] T008 [P] Add round-trip serialization tests for all ten new command variants and five new
  event variants in `tests/protocol_serde.rs`, extending its existing per-variant test table
  (depends on: T004, T005, T006)
- [x] T009 [P] Add unit tests in `src/protocol/v1/mutation.rs`'s existing test module for the new
  mutation-classification arms (mirroring `namespaced_sample`/`cluster_scoped_sample`/
  `read_only_sample` conventions) (depends on: T007)
- [x] T010 Regenerate ts-rs frontend bindings (`cargo test` runs the ts-rs export, or the
  project's existing regen command) so `ui/src/protocol/generated/BackendCmdV1.ts` and
  `StreamEvent.ts` pick up the new variants (depends on: T004, T005, T006)
- [x] T011 `cargo clippy --all-targets -- -D warnings` clean after T004–T007 (exhaustive-match
  compile guarantee holds) (depends on: T007)

**Checkpoint**: protocol compiles, round-trips, and is classified correctly. All three user
stories can now proceed in parallel.

---

## Phase 3: User Story 1 — Edit YAML + apply (Priority: P1) 🎯 MVP

**Goal**: Operator can open a resource's YAML, edit it, review a diff against the live object,
and apply the change, with identity-change and stale-version applies rejected before any cluster
write.

**Independent Test**: Edit a Deployment's replica count via the YAML tab, review the diff, apply,
confirm the cluster updates; separately confirm an identity-field edit, a stale-`resourceVersion`
apply, and an apply against a since-deleted resource are all rejected/errored distinctly and
pre-mutation where applicable (quickstart.md US1 steps 1–7).

### Tests for User Story 1

- [x] T012 [P] [US1] E2E test: open YAML tab, edit non-identity field, review diff, confirm apply
  succeeds, in `ui/tests/e2e/us1-apply-yaml.spec.ts` (fake backend fixture)
- [x] T013 [P] [US1] E2E test: identity-field edit (`metadata.name`) is rejected before any
  `ApplyResourceYaml` command reaches the backend or the fake backend reports the rejection, same
  file as T012
- [x] T014 [P] [US1] E2E test: stale-`resourceVersion` apply surfaces a specific conflict error
  and preserves the edit buffer for retry, same file as T012
- [x] T015 [P] [US1] E2E test (NEW — closes analyze finding C2): the target resource is deleted
  server-side after the YAML tab was opened but before apply; confirm apply fails with a clear,
  distinct "resource no longer exists" error (not conflated with the resourceVersion-conflict
  message from T014) and the edit buffer is preserved for the operator to abandon or re-target,
  same file as T012
- [x] T016 [P] [US1] E2E test: unsaved edit + navigate-away triggers a discard warning, same file
  as T012
- [x] T017 [P] [US1] Unit test: `ResourceEditSession` diff computation (`diffLines` wrapper),
  including that a `liveYaml` update while the session is open recomputes the diff and sets
  `liveYamlChangedSinceReview`, in `ui/tests/unit/yaml-diff.test.ts`
- [x] T018 [P] [US1] Backend unit test: YAML→object parsing rejects identity-field mismatches and
  malformed YAML with clear errors, in `src/backend/apply.rs`'s test module

### Implementation for User Story 1

- [x] T019 [US1] Create `src/backend/apply.rs`: `parse_and_validate_yaml()` (YAML text → validated
  `serde_json::Value`, checks `apiVersion`/`kind`/`namespace`/`name` match the command's own
  fields per FR-005) and `apply_resource()` (dispatches to typed `Api::<K>::replace()` for known
  kinds or `Api::<DynamicObject>::replace()` for CRD instances, using
  `expected_resource_version`) (depends on: T004, T007)
- [x] T020 [US1] Wire `BackendCmdV1::ApplyResourceYaml` handling into command dispatch (new or
  existing `src/backend/cmd/` module per plan.md), classifying the cluster's response into three
  distinct outcomes the frontend can render differently: success, 409 version-conflict, and
  404/"not found" (target deleted mid-edit, closes analyze finding C2) — emitting
  `StreamEvent::ResourceApplied` on success (depends on: T019)
- [x] T021 [P] [US1] Add `ResourceEditSession` state shape to `StoreData` in `ui/src/store/index.ts`
  per data-model.md, with `status` including both `'conflict'` and a distinct `'deleted'` variant
  (depends on: T010)
- [x] T022 [P] [US1] Handle `ResourceApplied` / apply-failure `CommandFinished` in
  `ui/src/store/handlers/stream.ts`, updating `ResourceEditSession.status` across
  `'applying' → 'applied' | 'conflict' | 'deleted' | 'error'` (depends on: T021)
- [x] T023 [US1] Create `ui/src/components/panels/YamlEditTab.tsx`: editable textarea seeded from
  the existing `jsonToYaml()` render, dirty-tracking, "Review changes" affordance (depends on:
  T021)
- [x] T024 [US1] Create `ui/src/components/panels/YamlDiffView.tsx`: renders a `diffLines()` result
  between `liveYaml` and `editedYaml` styled with the existing neon-noir tokens; re-renders when
  `liveYaml` updates from the resource-watch stream and shows a "content changed, re-review"
  indicator when it shifts after the operator last reviewed the diff (depends on: T002, T017)
- [x] T025 [US1] Wire `YamlEditTab`/`YamlDiffView` into `ui/src/components/Inspector.tsx`'s YAML
  tab, replacing the read-only-only render with edit affordance + diff review + confirm-to-apply
  routed through the existing `ConfirmDialog`/`ActionDialogHost`/`runAction` flow, rendering the
  `'conflict'` and `'deleted'` statuses with distinct, actionable messaging (depends on: T023,
  T024, T020, T022)
- [x] T026 [US1] Add the discard-on-navigate-away warning (dirty session + tab close/switch) to
  `Inspector.tsx` or the store's pane-switch handling (depends on: T025)

**Checkpoint**: User Story 1 fully functional and independently testable/deployable — this is the
MVP slice.

---

## Phase 4: User Story 2 — Node actions (Priority: P2)

**Goal**: Operator can cordon, uncordon, drain (with live progress and PDB visibility), and taint
nodes, all explicitly confirmed, none silently one-click.

**Independent Test**: Cordon → uncordon round trip; drain a node with a DaemonSet pod and
"ignore DaemonSets" checked, watch live progress to completion; attempt to drain a node with a
bare pod and confirm refusal; add/remove a taint (quickstart.md US2 steps 1–7).

### Tests for User Story 2

- [x] T027 [P] [US2] E2E test: cordon then uncordon a node, confirm both round trips and
  scheduling-state reflection, in `ui/tests/e2e/us2-node-actions.spec.ts`
- [x] T028 [P] [US2] E2E test: drain with `ignoreDaemonsets: true` completes, live progress events
  render incrementally, DaemonSet pod reported as ignored not evicted, same file as T027
- [x] T029 [P] [US2] E2E test: drain a node with a PDB-blocked pod shows it as "blocked" with the
  server's verbatim reason, not silently treated as evicted, same file as T027
- [x] T030 [P] [US2] E2E test: drain refuses up front for a node hosting a bare pod, same file as
  T027
- [x] T031 [P] [US2] E2E test: cancel a drain mid-way, confirm partial-progress reporting and
  clean stop, same file as T027
- [x] T032 [P] [US2] E2E test: add then remove a taint, both gated behind confirmation, same file
  as T027
- [x] T033 [P] [US2] Backend unit test: bare-pod pre-check refuses drain before any eviction call,
  in `src/backend/nodes.rs`'s test module
- [x] T034 [P] [US2] Backend unit test: cordon/uncordon/taint patch construction (correct
  `spec.unschedulable`/`spec.taints` merge-patch shapes), same test module as T033

### Implementation for User Story 2

- [x] T035 [US2] Create `src/backend/nodes.rs`, promoting `fetch_node_summaries`/`select_node`
  logic out of `src/backend/cmd/cluster.rs` (call-compatible move per research.md R2) (depends
  on: none beyond Phase 2)
- [x] T036 [US2] Add `cordon_node()`/`uncordon_node()` (`Patch::Merge` on `spec.unschedulable`) to
  `src/backend/nodes.rs` (depends on: T035)
- [x] T037 [US2] Add `taint_node()` (`Patch::Merge` on `spec.taints`, add/remove by key+effect) to
  `src/backend/nodes.rs` (depends on: T035)
- [x] T038 [US2] Add `drain_node()` background task to `src/backend/nodes.rs`: bare-pod pre-check,
  DaemonSet-pod filtering, per-pod `Api::<Pod>::evict()` calls, incremental
  `StreamEvent::DrainProgress` emission, `JoinHandle` tracked in `BackendState` (mirroring
  exec/log session task lifecycle) for `CancelDrainNode` to abort (depends on: T035)
- [x] T039 [US2] Wire `CordonNode`/`UncordonNode`/`DrainNode`/`CancelDrainNode`/`TaintNode`
  command dispatch in `src/backend/cmd/cluster.rs`, delegating to `src/backend/nodes.rs` (depends
  on: T036, T037, T038)
- [x] T040 [P] [US2] Add `NodeAction`/`DrainProgress` state shape to `StoreData` in
  `ui/src/store/index.ts` per data-model.md (depends on: T010)
- [x] T041 [P] [US2] Handle `DrainProgress`/`NodeActionFinished` events in
  `ui/src/store/handlers/stream.ts` (depends on: T040)
- [x] T042 [US2] Create `ui/src/components/panels/NodeActionsPanel.tsx`: cordon/uncordon/drain/
  taint controls, drain live-progress view, all routed through `ConfirmDialog`/`ActionDialogHost`/
  `runAction` (depends on: T039, T041)
- [x] T043 [US2] Wire `NodeActionsPanel` into the existing node details view (depends on: T042)

**Checkpoint**: User Story 2 fully functional and independently testable, on top of US1 or alone.

---

## Phase 5: User Story 3 — Browse Custom Resources (Priority: P3)

**Goal**: Operator can discover every installed CRD (not just Flux/Longhorn) and browse/inspect
its instances read-only through the existing generic resource-list/Inspector pattern.

**Independent Test**: Open "Custom Resources" nav, see every installed CRD listed, select one with
instances, inspect via Details/YAML/Events; confirm empty state on a zero-CRD cluster, per-CRD
permission-denied isolation, and a clear "no longer available" state if a viewed CRD is removed
(quickstart.md US3 steps 1–5).

### Tests for User Story 3

- [x] T044 [P] [US3] E2E test: CRD catalog lists all installed CRDs (fake backend fixture seeded
  with multiple CRD kinds, not just Flux), in `ui/tests/e2e/us3-crd-browse.spec.ts`
- [x] T045 [P] [US3] E2E test: select a CRD kind, list scoped instances correctly
  (namespaced vs. cluster-scoped), same file as T044
- [x] T046 [P] [US3] E2E test: open an instance, Details/YAML/Events tabs render via the existing
  generic Inspector, same file as T044
- [x] T047 [P] [US3] E2E test: zero-CRD cluster shows explicit empty state, not an error, same
  file as T044
- [x] T048 [P] [US3] E2E test: one CRD's instance listing returns 403 — that CRD alone shows
  permission-denied, every other CRD still lists normally, same file as T044
- [x] T049 [P] [US3] E2E test (NEW — closes analyze finding C1): while the operator is viewing a
  CRD's instances, a subsequent `CrdCatalogUpdated` refresh no longer lists that CRD (it was
  deleted from the cluster); confirm the instance view switches to a clear "this custom resource
  type is no longer available" state rather than a stale or silently-empty list, same file as
  T044
- [x] T050 [P] [US3] Backend unit test: CRD discovery correctly classifies `Namespaced`/`Cluster`
  scope and captures `permission_denied` per-entry without aborting the whole catalog scan, in
  `src/backend/crds.rs`'s test module

### Implementation for User Story 3

- [x] T051 [US3] Create `src/backend/crds.rs`: `discover_crds()` using `kube::discovery` against
  `apiextensions.k8s.io` to build `CrdCatalogEntry` list per data-model.md (depends on: Phase 2)
- [x] T052 [US3] Add `list_crd_instances()`/`get_crd_instance()` using
  `Api::<DynamicObject>::all()`/`namespaced()` scoped by each entry's discovered `Scope`, capturing
  403s per-CRD rather than failing the whole scan, in `src/backend/crds.rs` (depends on: T051)
- [x] T053 [US3] Wire `RefreshCrds`/`SelectCrdKind`/`RefreshCrdInstances`/`SelectCrdInstance`
  command dispatch (new `src/backend/cmd/` module or existing per plan.md), delegating to
  `src/backend/crds.rs`, emitting `CrdCatalogUpdated`/`CrdInstancesUpdated` and the existing
  generic resource-read event shape for instance selection (depends on: T052)
- [x] T054 [P] [US3] Add `CrdCatalog`/instances state shape to `StoreData` in
  `ui/src/store/index.ts` per data-model.md (depends on: T010)
- [x] T055 [P] [US3] Handle `CrdCatalogUpdated`/`CrdInstancesUpdated` in
  `ui/src/store/handlers/system.ts`, including detecting when the currently-selected CRD kind is
  no longer present in a fresh `CrdCatalogUpdated` payload and marking it `'unavailable'` (closes
  analyze finding C1) (depends on: T054)
- [x] T056 [US3] Create `ui/src/components/nav/CustomResourcesSection.tsx`: CRD catalog nav +
  generic instance list, reusing the existing resource-list component and Inspector for detail
  view, rendering three distinct non-error states — empty catalog, per-CRD permission-denied, and
  a viewed CRD becoming `'unavailable'` (depends on: T053, T055)
- [x] T057 [US3] Wire `CustomResourcesSection` into the app's top-level nav (depends on: T056)

**Checkpoint**: All three user stories independently functional.

---

## Phase 6: Polish & Cross-Cutting Concerns

- [x] T058 [P] Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test` clean across all new backend modules
- [x] T059 [P] Run `pnpm -C ui lint`, `pnpm -C ui typecheck`, `pnpm -C ui test` (Vitest) clean
- [x] T060 Run the full Playwright E2E suite (`ui/tests/e2e/`) including all new US1–US3
  scenarios, confirm green
- [x] T061 Execute quickstart.md end-to-end against a real or scripted cluster context, confirming
  SC-001 through SC-007 hold
- [x] T062 Verify GPG commit signing is active (`git config commit.gpgsign` / `user.signingkey`,
  global + local) before any commit, per standing repo rule

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: no dependencies, start immediately
- **Foundational (Phase 2)**: depends on Setup — BLOCKS all three user stories (protocol variants
  and their classification must exist and compile before any story wires a command through them)
- **User Stories (Phase 3, 4, 5)**: each depends only on Phase 2, not on each other — can proceed
  in parallel, or sequentially in priority order (US1 → US2 → US3)
- **Polish (Phase 6)**: depends on whichever user stories are in scope for the release being
  validated

### Within Each User Story

- Tests before implementation (write T012–T018 / T027–T034 / T044–T050 first per Principle III)
- Backend module creation before command-dispatch wiring before frontend store wiring before
  frontend components
- Story complete before moving to next priority (if sequencing rather than parallelizing)

### Parallel Opportunities

- T001–T003 (Setup) can run in parallel
- T008, T009 (Foundational tests) can run in parallel once T004–T007 land
- Once Phase 2 checkpoint is reached, US1/US2/US3 backend modules (T019, T035, T051) have zero
  cross-story file overlap and can be built fully in parallel by different contributors
- All `[P]`-marked test tasks within a story can run in parallel (different spec files/test
  modules)

## Implementation Strategy

### MVP First (User Story 1 only)

1. Phase 1 (Setup) → Phase 2 (Foundational) → Phase 3 (US1) → validate via quickstart.md US1
   section → this is a deployable MVP increment (YAML edit+apply alone already closes the
   single biggest P0 gap against Lens/Headlamp)

### Incremental Delivery

1. Setup + Foundational → protocol ready
2. US1 → validate independently → ship
3. US2 → validate independently → ship
4. US3 → validate independently → ship
5. Polish once all three are in

### Parallel Team Strategy

Once Phase 2's checkpoint is reached (protocol compiles, round-trips pass), US1/US2/US3 touch
disjoint backend modules (`apply.rs` / `nodes.rs` / `crds.rs`) and disjoint frontend component
files, so three contributors can take one story each without file conflicts.
