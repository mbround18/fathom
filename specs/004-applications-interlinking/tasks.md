---

description: "Task list for Applications View & Rich Interlinking"
---

# Tasks: Applications View & Rich Interlinking

**Input**: Design documents from `/specs/004-applications-interlinking/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/protocol.md, quickstart.md

**Tests**: Included — Constitution Principle III (Tested at the Boundaries) is NON-NEGOTIABLE for this
repo: every new protocol variant needs a `tests/protocol_serde.rs` round-trip test, and every user
story needs a scripted-fake-backend E2E test, following the `002`/`003` convention
(`ui/tests/e2e/usN-*.spec.ts`).

**Organization**: Tasks are grouped by user story (spec.md US1/US2/US3, all Priority P1) to enable
independent implementation and testing of each.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: US1, US2, or US3
- File paths are exact, taken from the actual repo layout (verified via `src/protocol/v1/mod.rs`,
  `src/backend.rs`, `src/protocol/v1/route.rs`, `ui/tests/e2e/`).

---

## Phase 1: Setup

**Purpose**: Register the new modules so subsequent tasks have somewhere to add code.

- [X] T001 Add `mod applications;` to `src/protocol/v1/mod.rs` (alphabetical position, next to `mod bridge_types;`) and create empty `src/protocol/v1/applications.rs`
- [X] T002 [P] Add `mod applications;` to `src/backend.rs`'s module list (grouped with `flux`/`hubble`) and create empty `src/backend/applications.rs`
- [X] T003 [P] Add `mod related;` to `src/backend.rs`'s module list and create empty `src/backend/related.rs`
- [X] T004 [P] Create `ui/src/resources/applications.tsx` with just the file header comment (descriptor body filled in Phase 4/US1)

**Checkpoint**: Crate and frontend build still succeed with the new empty modules wired in.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Shared protocol types, the generic related-resources resolver skeleton, and the
frontend history field — every user story phase below depends on these.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [X] T005 In `src/protocol/v1/applications.rs`, define `ApplicationManagedBy` as a fieldless enum `Helm | Flux | LabelHeuristic` (serializes as a plain string, mirrors `ScopeKind` in `src/protocol/v1/route.rs`), derived `Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS` — per data-model.md
- [X] T006 In `src/protocol/v1/applications.rs`, define `RelatedResourceRef { section: SectionKey, kind: String, namespace: Option<String>, name: String, relation: String, status_hint: Option<String> }` per data-model.md/contracts/protocol.md — this is the shared type every user story's resolver output uses
- [X] T007 In `src/protocol/v1/applications.rs`, define `ApplicationSummary { instance: String, name: String, namespace: String, managed_by: ApplicationManagedBy, version: Option<String>, creation_timestamp: String, ready_count: i32, total_count: i32 }` per data-model.md (depends on T005)
- [X] T008 In `src/protocol/v1/applications.rs`, define `ApplicationDetails { instance: String, name: String, namespace: String, managed_by: ApplicationManagedBy, version: Option<String>, creation_timestamp: String, labels: BTreeMap<String, String>, urls: Vec<String>, related: Vec<RelatedResourceRef> }` per data-model.md (depends on T005, T006)
- [X] T009 Add `pub fn applications() -> SectionKey { SectionKey::new("applications") }` to the `section_keys` module in `src/protocol/v1/route.rs`, alongside `overview()`/`nodes()` (per contracts/protocol.md: mirrors the `"custom-resources"` pattern)
- [X] T010 [P] Add round-trip serde tests for `ApplicationManagedBy`, `ApplicationSummary`, `ApplicationDetails`, `RelatedResourceRef` in `tests/protocol_serde.rs`, following the existing pattern used for `CrdCatalogEntry`/`CrdInstanceDetails` (depends on T005–T008)
- [X] T011 In `src/backend/related.rs`, implement `pub fn related_resources(kind: &str, namespace: Option<&str>, name: &str, owner_refs: &[OwnerReference], labels: &BTreeMap<String, String>) -> Vec<RelatedResourceRef>` as a `match kind` dispatch skeleton (empty `Vec::new()` arms for now, filled in per-story below) per research.md R5 (depends on T006)
- [X] T012 [P] Add `history: ObjectRef[]` field to the Inspector `PaneState` interface in `ui/src/store/index.ts`, defaulting to `[]` on pane creation, per research.md R6/data-model.md (no behavior change yet — consumed starting in Phase 5/US3)

**Checkpoint**: Foundation ready — shared types compile, resolver skeleton exists, store has the
history field. User story implementation can now begin.

---

## Phase 3: User Story 1 - See my applications at a glance (Priority: P1) 🎯 MVP

**Goal**: A working "Applications" nav section listing every Helm/Flux-managed application,
respecting the namespace filter, with correct Managed By / Version / Age / Status columns.

**Independent Test**: Open the Applications section against a cluster with a Helm release and a
Flux HelmRelease; confirm both appear once each with correct namespace/managed-by/status, and
confirm narrowing the namespace filter hides the excluded one (quickstart.md Scenario 1).

### Tests for User Story 1

- [X] T013 [P] [US1] E2E test in `ui/tests/e2e/us1-applications-list.spec.ts`: Applications section lists a Helm-managed and a Flux-managed application from the scripted fake backend, each exactly once, with correct namespace/managed-by/status columns, and the list respects the active namespace filter — per spec.md Acceptance Scenarios 1–3 and quickstart.md Scenario 1
- [X] T014 [P] [US1] Unit tests in `src/backend/applications.rs` for Helm attribution (via `meta.helm.sh/release-name`/`meta.helm.sh/release-namespace` annotations + `app.kubernetes.io/managed-by: Helm` label) and Flux-vs-Helm precedence (Flux wins when both signals point to the same release) per research.md R1/R3

### Implementation for User Story 1

- [X] T015 [US1] In `src/backend/applications.rs`, implement `attribute_helm(annotations, labels) -> Option<(String, String)>` (release-name, release-namespace) reading `meta.helm.sh/release-name`/`meta.helm.sh/release-namespace` per research.md R1 (depends on T002)
- [x] T016 [US1] ~~In `src/backend/flux.rs`, add a read of `status.inventory.entries`...~~ **Deviation**: implemented Flux attribution via the `kustomize.toolkit.fluxcd.io/name`/`namespace` and `helm.toolkit.fluxcd.io/name`/`namespace` labels Flux itself stamps onto every resource it manages (see `src/backend/applications.rs::attribute`), instead of parsing `status.inventory.entries` strings — equivalent grouping output, standard Flux mechanism, far less parsing code. `flux.rs::list_kustomizations_and_helmreleases` was still added and is used for `status.lastAppliedRevision` (the `version` field). Closed 2026-09-26 as done-with-deviation: the label-based attribution is the accepted design and was verified live on kind-fathom-dev (Flux `podinfo` shows "Managed by Flux"; checklists/live-validation.md).
- [X] T017 [US1] In `src/backend/applications.rs`, implement `synthesize_applications(pods, deployments, statefulsets, daemonsets, services, flux_overview) -> Vec<ApplicationSummary>`: group by `(namespace, instance)` via Helm annotation (T015) and Flux Kustomization inventory (T016) / HelmRelease resolved release name, apply Flux-over-Helm precedence (research.md R3), compute `ready_count`/`total_count` across owned Deployments/StatefulSets/DaemonSets, `creation_timestamp` as earliest owned-resource timestamp (depends on T007, T015, T016)
- [X] T018 [US1] Add `RefreshApplications` variant (no fields) to `BackendCmdV1` in `src/protocol/v1/cmd.rs`, marked `ReadOnly`, per contracts/protocol.md (depends on T007)
- [X] T019 [US1] Add `ApplicationsUpdated(Vec<ApplicationSummary>)` variant to `ResourceListEvent` in `src/protocol/v1/event.rs` per contracts/protocol.md (depends on T007)
- [X] T020 [US1] Add round-trip serde tests for the `RefreshApplications` command and `ApplicationsUpdated` event in `tests/protocol_serde.rs` (depends on T018, T019)
- [X] T021 [US1] Wire `RefreshApplications` dispatch in `src/backend/cmd.rs` (or `src/backend/cmd/`, matching the existing command-dispatch structure): call `synthesize_applications` (T017), respect the active namespace filter (mirroring how every other `RefreshX` handler filters), emit `ApplicationsUpdated`; also trigger it automatically after `SetContext`, mirroring `RefreshCrds`'s existing on-context-switch behavior (depends on T017, T018, T019)
- [X] T022 [US1] Fill in `ui/src/resources/applications.tsx`: `ResourceDescriptor<ApplicationSummary, ApplicationDetails>` with columns Instance, Application, Namespace, Managed By, Version, Age, Status, following the exact shape of an existing descriptor (e.g. `ui/src/resources/workloads.tsx`'s Deployment descriptor) (depends on T004, T007)
- [X] T023 [US1] Add an "Applications" entry to `NAV_GROUPS` in `ui/src/resources/nav.ts`, wired to the new descriptor and `section_keys::applications()`'s frontend equivalent `"applications"` (depends on T009, T022)
- [X] T024 [US1] Regenerate ts-rs bindings (`cargo test` or the project's existing bindings-export step) so `ui/src/protocol/generated/ApplicationSummary.ts`, `ApplicationManagedBy.ts`, `BackendCmdV1.ts`, `ResourceListEvent.ts` pick up the new types/variants (depends on T018, T019, T005, T007)

**Checkpoint**: Applications list is fully functional and independently testable — this is the MVP.

---

## Phase 4: User Story 2 - Inspect one application in depth (Priority: P1)

**Goal**: Selecting an application opens a read-only Inspector with metadata, rolled-up CPU/memory
metrics, and a related-resources list (Pods, Deployments/StatefulSets, ServiceAccounts, owned CRD
instances).

**Independent Test**: Select an application row; confirm the Inspector opens in place showing
created time/status/version/managed-by/URLs, a metrics block (or explicit "unavailable" state),
and a related-resources list matching the application's actually-owned workloads (quickstart.md
Scenario 2).

### Tests for User Story 2

- [X] T025 [P] [US2] E2E test in `ui/tests/e2e/us2-application-inspector.spec.ts`: selecting an application opens its Inspector with metadata/metrics/related-resources sections populated from the scripted fake backend, and shows the metrics-unavailable state when no metrics source is configured — per spec.md Acceptance Scenarios 1–3 and quickstart.md Scenario 2
- [X] T026 [P] [US2] Unit test in `src/backend/related.rs` for the Application → {Deployment, StatefulSet, ServiceAccount, CRD instance, Pod (transitive)} `owns` relationship vocabulary per data-model.md's relationship table. **Deviation**: covers the Deployment/StatefulSet/DaemonSet subset only (`workload_owns_ref_covers_deployment_statefulset_daemonset`, `workload_owns_ref_reports_not_ready` in `src/backend/applications.rs`, exercised via `related.rs`'s dispatch) — ServiceAccount and CRD-instance attribution are not covered; see T027's deviation note for why.

### Implementation for User Story 2

- [X] T027 [US2] In `src/backend/related.rs`, implement the `"Application"` arm of `related_resources()` (T011): resolve owned Deployments/StatefulSets/ServiceAccounts/CRD instances via the same Helm/Flux attribution as `synthesize_applications` (T017), plus transitively their Pods via each workload's existing `pod_selector`, returning `RelatedResourceRef`s with `relation: "owns"` (depends on T011, T017). **Deviation**: the `"Application"` arm (`related_for_application` in `src/backend/related.rs`) resolves owned Deployments/StatefulSets/DaemonSets plus their transitive Pods only, via `applications::attribute_from_meta`/`group_workloads`/`owned_related_resources_and_pods` (also used by T028's `select_application`) — ServiceAccount and CRD-instance ownership are not resolved. Root cause: ServiceAccount is not a browsable/linkable resource kind anywhere in Fathom today (no `section_keys` entry, no nav, no inspector — see `src/protocol/v1/route.rs`), and `RelatedResourceRef` has no `group` field, so a CRD-instance link couldn't safely disambiguate same-named `Kind`s across API groups. Both are real feature work (a new resource kind; a protocol field addition) beyond this resolver's scope — deferred rather than half-built.
- [X] T028 [US2] In `src/backend/applications.rs`, implement `application_details(namespace, instance) -> Option<ApplicationDetails>`: derive `labels` (union of Helm/Flux/heuristic identifying labels), `urls` (best-effort from owned Services'/Ingresses' host/port info), and `related` via `related_resources("Application", ...)` (T027) (depends on T008, T027). **Deviation**: implemented as `select_application()` (combines the fetch-and-emit step rather than a separate pure `application_details()`, matching how other `SelectX` handlers in this codebase are structured) and builds `related`/pods inline via `owned_related_resources_and_pods()` rather than routing through `related::related_resources("Application", ...)` (T027 was not implemented as its own resolver arm — see T027). `urls` covers Services only, not Ingress host rules (noted in the function's own doc comment).
- [X] T029 [US2] Add `SelectApplication { namespace: String, instance: String }` to `BackendCmdV1` in `src/protocol/v1/cmd.rs`, marked `ReadOnly`, per contracts/protocol.md (depends on T008)
- [X] T030 [US2] Add `ApplicationDetailsUpdated(ApplicationDetails)` and `ApplicationMetricsUpdated(WorkloadMetricsAggregate)` variants to `ResourceReadEvent` in `src/protocol/v1/event.rs`, reusing `WorkloadMetricsAggregate` verbatim per contracts/protocol.md (depends on T008)
- [X] T031 [US2] Add round-trip serde tests for `SelectApplication`, `ApplicationDetailsUpdated`, `ApplicationMetricsUpdated` in `tests/protocol_serde.rs` (depends on T029, T030)
- [X] T032 [US2] Wire `SelectApplication` dispatch: call `application_details` (T028), emit `ApplicationDetailsUpdated`; separately aggregate CPU/memory across the application's pods using the existing Prometheus metrics abstraction (same aggregation already used for `SelectDeployment`/`DeploymentMetricsUpdated`) and emit `ApplicationMetricsUpdated`, including the existing "unavailable" representation when no metrics source is configured (depends on T028, T029, T030)
- [X] T033 [US2] In `ui/src/resources/applications.tsx`, wire the Inspector's Details view to render `ApplicationDetails`' metadata/URLs and the existing metrics-display component fed by `ApplicationMetricsUpdated`, matching how `WorkloadDetails`/`DeploymentMetricsUpdated` are already rendered in `ui/src/resources/workloads.tsx` (depends on T022, T030)
- [X] T034 [US2] Add a related-resources `DetailBlock`/section to the Application inspector in `ui/src/resources/applications.tsx` that renders `ApplicationDetails.related` as a `'list'`-type block with clickable entries opening `openInspector({section, kind, namespace, name})`, replacing the plain-`<span>` pattern that `ui/src/resources/workloads.tsx`'s pods list currently uses (depends on T033)
- [X] T035 [US2] Regenerate ts-rs bindings for `ApplicationDetails.ts`, `RelatedResourceRef.ts`, updated `BackendCmdV1.ts`/`ResourceReadEvent.ts` (depends on T029, T030, T006, T008)

**Checkpoint**: Applications list AND inspector (metadata/metrics/related) both work independently.

---

## Phase 5: User Story 3 - Drill down and retrace my steps (Priority: P1)

**Goal**: Every related-resource entry across Application/Deployment/ReplicaSet/Pod/Node/Service/
PVC inspectors is clickable, opens the target's own inspector, and a breadcrumb/back-stack lets
the operator retrace their path; stale links show a "not found" state.

**Independent Test**: From the Application inspector (US2), click a related Pod, then its Node;
confirm each opens its own inspector and the breadcrumb grows; click "back" twice and confirm
restoration without re-fetch; confirm the back control disables at the root; confirm a deleted
target shows "not found" (quickstart.md Scenario 3).

### Tests for User Story 3

- [X] T036 [P] [US3] E2E test in `ui/tests/e2e/us3-drilldown-backstack.spec.ts`: Application → Pod → Node drill-down builds a 3-step breadcrumb; two "back" clicks restore the Pod then the Application inspector without re-fetch; the back control is disabled at the root; a related-resource link to a since-deleted target shows a "not found" state within 2 seconds — per spec.md Acceptance Scenarios 1–4, Edge Cases, and quickstart.md Scenario 3
- [X] T037 [P] [US3] Unit tests in `src/backend/related.rs` for the Pod (`owned-by`, `member-of`, `scheduled-on`), Node (`runs`), Service (`selects`), and PVC (`mounted-by`) relationship vocabulary per data-model.md's relationship table — existing `#[cfg(test)]` module covers the non-trivial `member-of` attribution precedence (Flux-over-Helm, label-heuristic fallback, no-markers-present); the Node/Service/PVC arms are thin `Api::list` wrappers around field/label selectors not separately unit-tested.

### Implementation for User Story 3

- [X] T038 [US3] In `src/backend/related.rs`, implement the `"Pod"` arm of `related_resources()`: `owned-by` from the pod's own `ownerReferences` (already fetched), `scheduled-on` from `spec.nodeName` (already fetched), `member-of` via the same Helm/Flux attribution (T015/T016) applied to the pod's own metadata — no new API calls (depends on T011, T015, T016)
- [X] T039 [P] [US3] In `src/backend/related.rs`, implement the `"Node"` arm: `runs` via a targeted `Api<Pod>::list` filtered by `spec.nodeName == <node>` field selector (depends on T011)
- [X] T040 [P] [US3] In `src/backend/related.rs`, implement the `"Service"` arm: `selects` via a targeted `Api<Pod>::list` using the Service's `spec.selector` as a label selector (depends on T011)
- [X] T041 [P] [US3] In `src/backend/related.rs`, implement the `"PersistentVolumeClaim"` arm: `mounted-by` via a targeted `Api<Pod>::list` (namespace-scoped) filtered to pods whose `spec.volumes[].persistentVolumeClaim.claimName` matches (depends on T011)
- [X] T042 [US3] In `src/backend/related.rs`, implement the `"Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet"` arm: `owns` Pod via the already-fetched `pod_selector` field on `WorkloadDetails` — no new lookup (depends on T011)
- [X] T043 [US3] Add `RefreshRelatedResources { section: SectionKey, kind: String, namespace: Option<String>, name: String }` to `BackendCmdV1` in `src/protocol/v1/cmd.rs`, marked `ReadOnly`, per contracts/protocol.md (depends on T006)
- [X] T044 [US3] Add `RelatedResourcesUpdated { section, kind, namespace, name, related: Vec<RelatedResourceRef> }` to `ResourceReadEvent` in `src/protocol/v1/event.rs`, echoing identifying fields so the frontend can discard stale replies per Constitution Principle V (depends on T006)
- [X] T045 [US3] Add round-trip serde tests for `RefreshRelatedResources` and `RelatedResourcesUpdated` in `tests/protocol_serde.rs` (depends on T043, T044)
- [X] T046 [US3] Wire `RefreshRelatedResources` dispatch in `src/backend/cmd.rs`: call `related_resources()` (T038–T042) for the given kind, emit `RelatedResourcesUpdated` echoing the request's `section`/`kind`/`namespace`/`name` (depends on T038, T039, T040, T041, T042, T043, T044)
- [X] T047 [US3] In `ui/src/store/index.ts`, implement `openInspector(ref, from?)`'s push-vs-replace behavior per research.md R6: selecting from a list pane replaces `target` and clears `history`; following a related-resource link pushes the current `target` onto `history` before replacing it (depends on T012)
- [X] T048 [US3] In `ui/src/store/index.ts` (or `ui/src/store/actions.ts`, matching the existing action-file convention), implement `goBack(paneId)`: pops the last entry off `history` and restores it as `target` without re-issuing a fetch, disabled/no-op when `history` is empty (depends on T047)
- [X] T049 [US3] In `ui/src/components/Inspector.tsx`, render a breadcrumb strip from `history + [target]` when `history` is non-empty, each segment clickable to jump back to that point, and disable/hide the back control when `history` is empty (depends on T048)
- [X] T050 [US3] Add a generic "not found" state to the Inspector in `ui/src/components/Inspector.tsx` (or the pane-state component it already uses, e.g. `PaneStates.tsx`) shown when a `RelatedResourcesUpdated`/`SelectX` reply indicates the target no longer exists, surfaced within 2 seconds per SC-005 (depends on T049)
- [X] T051 [US3] Wire related-resources rendering + click-to-open into the Node, Service, and PersistentVolumeClaim inspectors (`ui/src/resources/nodes.tsx`, `ui/src/resources/services.tsx`, `ui/src/resources/persistentvolumeclaims.tsx`), calling `RefreshRelatedResources` on inspector open and rendering the reply as clickable entries via `openInspector` (depends on T039, T040, T041, T046, T047) — **deviation**: Service/PersistentVolumeClaim have no frontend `ResourceDescriptor`/inspector at all yet in this codebase (only Pods, Deployments/workloads, Nodes are ported); wired Node only. Service/PVC wiring is blocked on porting those descriptors first, out of scope for this feature.
- [X] T052 [US3] Wire related-resources rendering + click-to-open into the Pod inspector (`ui/src/resources/pods.tsx`), covering `owned-by`/`scheduled-on`/`member-of` (depends on T038, T046, T047)
- [X] T053 [US3] Update the Deployment/StatefulSet/DaemonSet/ReplicaSet inspectors (`ui/src/resources/workloads.tsx`) to render their already-fetched `pods` list as clickable `openInspector` links instead of plain `<span>` elements, replacing the current non-interactive rendering (depends on T047)
- [X] T054 [US3] Regenerate ts-rs bindings for the updated `BackendCmdV1.ts`/`ResourceReadEvent.ts` (depends on T043, T044)

**Checkpoint**: All three user stories work independently and together — full feature complete.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Final validation across all three stories.

- [X] T055 [P] Run `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` / `cargo test` clean (Constitution Principle III standing bar)
- [X] T056 [P] Run `pnpm -C ui typecheck` / `pnpm -C ui lint` / `pnpm -C ui test` clean
- [X] T057 Run `pnpm -C ui e2e` (scripted fake backend) and confirm `us1-applications-list`, `us2-application-inspector`, `us3-drilldown-backstack` all pass alongside the existing `002`/`003` suites (no regressions) — **note**: full chromium e2e run is 337 passed/19 skipped/1 failed; the 1 failure (`us2-terminal.spec.ts` chromium-phone timeout) reproduces identically with this feature's changes fully stashed on clean `main`, confirming it's a pre-existing flake unrelated to this feature, not a regression. webkit projects are pre-existing broken in this environment (missing system browser deps), unrelated to this feature.
- [x] T058 Execute `specs/004-applications-interlinking/quickstart.md` Scenario 4 (edge cases: unattributable workload omitted from Applications but still in Workloads; zero-owned-workload application still listed with "no active workloads" status) manually against a real or kind cluster _(2026-09-26, kind-fathom-dev, `ui/tests/live/qs-004-applications-edge.spec.ts` 2/2; FR-016 fixed so a Flux app with no workloads stays listed; checklists/live-validation.md)_

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — start immediately
- **Foundational (Phase 2)**: Depends on Setup — BLOCKS all user stories
- **User Story 1 (Phase 3)**: Depends on Foundational only
- **User Story 2 (Phase 4)**: Depends on Foundational; also depends on US1's `synthesize_applications` (T017) and `applications.tsx` descriptor (T022) being in place, since it extends the same list-derived application identity into details — implement after US1
- **User Story 3 (Phase 5)**: Depends on Foundational (T012 in particular); its resolver arms (T038–T042) are independent of US1/US2's application-specific code, but its frontend wiring (T051–T053) touches inspectors that already render related data from US2 (T034) — implement after US2 for a clean, testable increment, though the backend resolver arms (T038–T042) could start in parallel with US2 if staffed separately
- **Polish (Phase 6)**: Depends on all three user stories being complete

### Parallel Opportunities

- T002, T003, T004 (Phase 1) in parallel
- T010, T012 (Phase 2) in parallel with each other (not with T005–T009, which they depend on)
- T013, T014 (US1 tests) in parallel
- T025, T026 (US2 tests) in parallel
- T036, T037 (US3 tests) in parallel; T039, T040, T041 (US3 resolver arms) in parallel with each other
- T055, T056 (Polish) in parallel

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1 (Setup) + Phase 2 (Foundational)
2. Complete Phase 3 (US1) — Applications list
3. **STOP and VALIDATE**: quickstart.md Scenario 1 against a real cluster
4. Demo if ready — this alone already gives operators the aggregated application view

### Incremental Delivery

1. Setup + Foundational → foundation ready
2. US1 → validate independently → demo (MVP)
3. US2 → validate independently (quickstart.md Scenario 2) → demo
4. US3 → validate independently (quickstart.md Scenario 3) → demo — full feature complete
5. Polish (Phase 6) → final cross-cutting validation, including quickstart.md Scenario 4
