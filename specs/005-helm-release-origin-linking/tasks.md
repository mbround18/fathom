---

description: "Task list for 005-helm-release-origin-linking"
---

# Tasks: Helm Release View & Origin-Context Linking

**Input**: Design documents from `/specs/005-helm-release-origin-linking/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/protocol.md, quickstart.md

**Tests**: Included — Constitution Principle III (Tested at the Boundaries) requires round-trip
protocol tests and unit tests for decision logic; quickstart.md specifies E2E per user story.

**Organization**: Tasks are grouped by user story (US1/US2/US3, priorities from spec.md) so each
can be implemented and tested independently after the Foundational phase.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies on incomplete tasks)
- **[Story]**: US1, US2, US3 — maps to spec.md's priority-ordered user stories
- Every task names an exact file path

---

## Phase 1: Setup

- [X] T001 Add `flate2 = "1"` to `[dependencies]` in `Cargo.toml` (research.md R2)
- [X] T002 [P] Create `src/protocol/v1/helm.rs` module stub and register it in `src/protocol/v1/mod.rs`
- [X] T003 [P] Create `src/backend/helm.rs` module stub and register it in `src/backend/mod.rs`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Protocol types, section keys, and command/event scaffolding every user story depends on.

**⚠️ CRITICAL**: No user story task may begin until this phase is complete.

- [X] T004 Define `HelmReleaseStatus` enum (`Deployed, Failed, PendingInstall, PendingUpgrade, PendingRollback, Superseded, Uninstalled, Uninstalling, Unknown`) in `src/protocol/v1/helm.rs`
- [X] T005 [P] Define `HelmReleaseSummary` struct (`name, namespace, chart, chart_version, app_version, revision, status, updated`) in `src/protocol/v1/helm.rs` (data-model.md)
- [X] T006 [P] Define `HelmRevisionSummary` struct (`revision, status, description, updated, app_version, chart_version`) in `src/protocol/v1/helm.rs` (data-model.md)
- [X] T007 [P] Define `GitOpsController` enum (`Flux` variant only for v1) and `GitOpsOriginRef` struct (`controller, section, kind, namespace, name, ready, message`) in `src/protocol/v1/helm.rs` (data-model.md — additive enum for future `Argo`)
- [X] T008 Define `HelmReleaseDetails` struct (`name, namespace, chart, chart_version, app_version, revision, status, updated, description, values_user_supplied, values_effective, manifest, revisions, origin, related, degraded`) in `src/protocol/v1/helm.rs` — depends on T004-T007
- [X] T009 Add `section_keys::helm_releases()` → `SectionKey::new("helm/releases")` and `section_keys::flux_resource()` → `SectionKey::new("integrations/flux-resource")` in `src/protocol/v1/route.rs`
- [X] T010 Add `BackendCmdV1::RefreshHelmReleases`, `SelectHelmRelease { namespace, name }`, `SelectFluxResource { kind, namespace, name }` variants in `src/protocol/v1/cmd.rs`
- [X] T011 Add `ResourceListEvent::HelmReleasesUpdated(Vec<HelmReleaseSummary>)` and `ResourceReadEvent::HelmReleaseDetailsUpdated(HelmReleaseDetails)`, `FluxResourceDetailsUpdated(FluxResourceSummary)`, `FluxResourceNotFound { kind, namespace, name }` in `src/protocol/v1/event.rs`
- [X] T012 [P] Add round-trip serde tests `helm_release_commands_roundtrip`, `select_flux_resource_roundtrip`, `helm_release_events_roundtrip`, `flux_resource_events_roundtrip` in `tests/protocol_serde.rs` (contracts/protocol.md)
- [X] T013 Implement `decode_release_payload(base64_gzip_json: &str) -> anyhow::Result<ReleasePayload>` (base64 → gzip → JSON) in `src/backend/helm.rs` (research.md R1)
- [X] T014 [P] Unit tests for `decode_release_payload` against a fixture-encoded blob (valid + corrupted cases, no live cluster) in `src/backend/helm.rs`

**Checkpoint**: Foundation ready — user story implementation can now begin.

---

## Phase 3: User Story 1 - Browse installed Helm releases (Priority: P1) 🎯 MVP

**Goal**: A "Helm Releases" nav section listing every installed release with chart/version/revision/status/last-updated, respecting the namespace filter.

**Independent Test**: Scripted-fake-backend fixture with releases across two namespaces including one `failed`; confirm all rows render correctly and narrowing the namespace filter hides out-of-scope rows (spec.md US1 Acceptance Scenarios 1-3).

### Implementation for User Story 1

- [X] T015 [US1] Implement `list_releases(client, namespace: Option<&str>) -> anyhow::Result<Vec<HelmReleaseSummary>>` in `src/backend/helm.rs`: list `Secret`/`ConfigMap` objects typed/labeled `helm.sh/release.v1` via `Api::all`/`Api::namespaced` (research.md R1, R8), decode via `decode_release_payload`, degrade gracefully per FR-014 using storage-object labels when decode fails
- [X] T016 [US1] Wire `RefreshHelmReleases` dispatch in `src/backend/cmd/` → `helm::list_releases` → `ResourceListEvent::HelmReleasesUpdated`, routed via `Route::namespaced(section_keys::helm_releases(), ctx, ns)` (never `Route::global` — per the 004 postmortem bug)
- [X] T017 [P] [US1] Create `ui/src/resources/helm.tsx` with list columns (Name, Namespace, Chart, Chart Version, App Version, Revision, Status, Last Updated) and `refresh: () => ({ type: 'RefreshHelmReleases' })`, mirroring `ui/src/resources/applications.tsx`'s descriptor shape
- [X] T018 [US1] Add `"Helm Releases"` entry to `NAV_GROUPS` in `ui/src/resources/nav.ts` pointing at `section_keys::helm_releases()`
- [X] T019 [US1] Register the `helm` descriptor in `ALL_DESCRIPTORS` in `ui/src/resources/index.ts`
- [X] T020 [P] [US1] Unit tests for `list_releases` namespace-scoping and `HelmReleaseStatus` mapping in `src/backend/helm.rs`
- [X] T021 [P] [US1] Playwright E2E "browse Helm releases" (list renders, namespace filter narrows) in `ui/tests/e2e/` against the scripted fake backend

**Checkpoint**: User Story 1 fully functional and independently testable.

---

## Phase 4: User Story 2 - Trace a broken release to its GitOps origin (Priority: P1)

**Goal**: A clickable origin link from a Flux-managed release to its owning Flux `HelmRelease`/`Kustomization`, opening that controller's live Ready/Failed/Reconciling/Suspended state in two clicks.

**Independent Test**: Fixture release with Flux ownership labels vs. one without; confirm origin link presence/absence, click-through to live Ready state with verbatim message, breadcrumb back-navigation, and "not found" handling for a deleted origin (spec.md US2 Acceptance Scenarios 1-5, FR-013).

### Implementation for User Story 2

- [X] T022 [US2] Implement `get_release_details(client, namespace, name) -> anyhow::Result<HelmReleaseDetails>` core in `src/backend/helm.rs`: resolve the latest revision's storage object, populate identity/status fields, detect Flux ownership via `helm.toolkit.fluxcd.io/name`/`namespace` labels (mirrors `applications.rs::attribute_from_meta`) to build `origin: Option<GitOpsOriginRef>` — `values_*`/`manifest`/`revisions` left as placeholders for T032/T033
- [X] T023 [US2] Wire `SelectFluxResource` dispatch in `src/backend/cmd/`: re-run `flux::list_kustomizations_and_helmreleases` + `summarize()`, filter by `{kind, namespace, name}`, emit `ResourceReadEvent::FluxResourceDetailsUpdated` on a match else `FluxResourceNotFound` (research.md R3, FR-013)
- [X] T024 [US2] Wire `SelectHelmRelease` dispatch in `src/backend/cmd/` → `helm::get_release_details` → `ResourceReadEvent::HelmReleaseDetailsUpdated`
- [X] T025 [US2] Add `"HelmRelease" => related_for_helm_release(...)` match arm to `related_resources()` in `src/backend/related.rs`; implement the `"reconciled-by"` relation pointing at `section_keys::flux_resource()` with `kind` = `"HelmRelease"`/`"Kustomization"` (data-model.md)
- [X] T026 [P] [US2] Create `ui/src/resources/flux-resource.tsx`: minimal `ResourceDescriptor<FluxResourceSummary, FluxResourceSummary>` for `section_keys::flux_resource()` — one Details `fields` block (Ready state, message, revision, suspended, last_transition), `read()` issuing `SelectFluxResource`
- [X] T027 [US2] Register the `flux-resource` descriptor in `ALL_DESCRIPTORS` in `ui/src/resources/index.ts` (not added to `nav.ts` — reachable only via origin link, research.md R4)
- [X] T028 [US2] Extend `ui/src/resources/helm.tsx`'s `read()` to also issue `RefreshRelatedResources { kind: "HelmRelease", ... }`; add a Related detail section reusing `relatedResourcesBlock` from `ui/src/resources/related.tsx` unchanged (renders the `reconciled-by` link alongside owned-resource links)
- [X] T029 [P] [US2] Unit tests for `related_for_helm_release`'s `"reconciled-by"` resolution — Flux-owned case (link present, correct kind/section) and bare-Helm case (no link, FR-011) — in `src/backend/related.rs`
- [X] T030 [P] [US2] Unit test for `SelectFluxResource` "not found" handling (origin deleted after Helm release creation, FR-013) in `src/backend/helm.rs` or `src/backend/cmd/`
- [X] T031 [P] [US2] Playwright E2E "trace release to GitOps origin" (link present/absent, click-through shows Failed/Ready state + message, breadcrumb back, not-found state) in `ui/tests/e2e/`

**Checkpoint**: User Stories 1 and 2 both functional independently.

---

## Phase 5: User Story 3 - Inspect a release's full detail (Priority: P2)

**Goal**: Effective vs. user-supplied values, the applied manifest, revision history, and owned resources — all in the release's inspector.

**Independent Test**: Fixture release with a values override and multiple revisions; confirm values distinction, manifest content, ordered revision history, and owned-resource drill-down (spec.md US3 Acceptance Scenarios 1-4).

### Implementation for User Story 3

- [X] T032 [US3] Extend `get_release_details()` in `src/backend/helm.rs` to decode and populate `values_user_supplied` (release's own `config`), `values_effective` (chart defaults merged with `config`), and `manifest` (verbatim) as YAML text for the current revision
- [X] T033 [US3] Implement revision-history listing in `src/backend/helm.rs`: list every release-storage object sharing `{name, namespace}`, shallow-decode each (status/description/timestamp only, not full values/manifest) into `HelmRevisionSummary`, sorted descending by revision (data-model.md)
- [X] T034 [US3] Implement the `"owns"` relation in `related_for_helm_release` (`src/backend/related.rs`): resolve owned workloads via Helm tracking labels (`app.kubernetes.io/managed-by=Helm`, `meta.helm.sh/release-name`, `meta.helm.sh/release-namespace`), reusing `applications.rs::list_owned_workloads`/`owned_related_resources_and_pods` (research.md R6)
- [X] T035 [P] [US3] Extend `ui/src/resources/helm.tsx`'s `detailSections`: Values (user-supplied + effective, each via the existing YAML viewer from 003, read-only), Manifest (same viewer), Revisions (list block per `HelmRevisionSummary`)
- [X] T036 [P] [US3] Unit tests for values-merge/manifest decode correctness and revision-history ordering in `src/backend/helm.rs`
- [X] T037 [P] [US3] Unit test for `"owns"` owned-workload resolution via Helm tracking labels in `src/backend/related.rs`
- [X] T038 [P] [US3] Playwright E2E "inspect full release detail" (values distinction, manifest, revision history, owned-resource click-through) in `ui/tests/e2e/`

**Checkpoint**: All three user stories independently functional.

---

## Phase 6: Polish & Cross-Cutting Concerns

- [X] T039 [P] Verify FR-014 degraded-release path end-to-end: corrupted-payload fixture still appears in list and inspector with explicit "unavailable" indicators, no crash
- [X] T040 [P] Verify zero-owned-resources edge case shows an explicit "no resources found" state, not an error (spec.md Edge Cases)
- [ ] T041 (DEFERRED — no live cluster available in this environment; all other verification done via unit/E2E tests against scripted fixtures) Run `quickstart.md` validation end-to-end (all three user-story scenarios, live-cluster spot check)
- [X] T042 [P] `cargo clippy --all-targets -- -D warnings` clean
- [X] T043 [P] `pnpm -C ui typecheck` and `pnpm -C ui lint` clean
- [X] T044 Update `docs/MVP.md` §5a/§5b status markers to reflect the shipped Helm view and origin-linking

---

## Dependencies & Execution Order

- **Setup (Phase 1)** → no dependencies, run first.
- **Foundational (Phase 2)** → depends on Setup; BLOCKS all user stories (shared protocol types, section keys, decode primitive).
- **US1 (Phase 3)** → depends on Foundational only; no dependency on US2/US3.
- **US2 (Phase 4)** → depends on Foundational; T022/T024 build on the same `helm.rs`/`cmd` files T015/T016 touch, so within a single contributor's sequence do US1 before US2, but US2 does not require US1's UI pieces to be *complete*, only the shared files to exist.
- **US3 (Phase 5)** → depends on Foundational; extends `get_release_details` (T022) and `related_for_helm_release` (T025) from US2, so US2's backend scaffolding (T022, T025) must land first even though US3 is independently testable once it has.
- **Polish (Phase 6)** → depends on all three user stories being complete.

### Parallel Opportunities

- T002, T003 in parallel (different files).
- T005, T006, T007 in parallel within Phase 2 (same file `helm.rs` but additive, non-overlapping struct definitions — sequence if working solo, parallelize only across contributors).
- T012, T014 in parallel (test files, no shared state).
- Within each story, frontend descriptor tasks (T017, T026, T035) can run parallel to backend tasks in the same phase since they touch different files, once the protocol types they depend on (Phase 2) exist.
- T020, T021 / T029, T030, T031 / T036, T037, T038 (test tasks within a story) can all run in parallel once their story's implementation tasks land.

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Phase 1 (Setup) → Phase 2 (Foundational) → Phase 3 (US1).
2. **STOP and VALIDATE**: run T021's E2E + manual quickstart.md US1 check.
3. This alone ships a browsable Helm release list — a real increment even before origin-linking exists.

### Incremental Delivery

1. Setup + Foundational → Foundation ready.
2. US1 → validate independently → demoable list view.
3. US2 → validate independently → the operator's explicit highest-value ask (origin-context linking) ships.
4. US3 → validate independently → full diagnostic detail completes the spec.
5. Polish → quickstart.md full pass, lint/clippy clean, docs updated.
