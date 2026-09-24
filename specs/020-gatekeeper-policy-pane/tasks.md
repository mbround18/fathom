---
description: "Task list for Gatekeeper Policy Pane"
---

# Tasks: Gatekeeper Policy Pane

**Input**: Design documents from `/specs/020-gatekeeper-policy-pane/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/protocol.md, quickstart.md

**Tests**: Included — Constitution Principle III (Tested at the Boundaries) is NON-NEGOTIABLE for
this repo: protocol round-trip tests, backend unit tests, frontend unit tests, and a Playwright
e2e spec are all in scope.

**Organization**: Tasks are grouped by user story (P1/P1/P2 from spec.md) so each story is an
independently testable increment, matching the Flux-panel precedent this feature mirrors.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies on incomplete tasks)
- **[Story]**: US1 (live constraint list), US2 (violation drill-down + Inspector link-out), US3
  (honest empty/permission states)

## Phase 1: Setup

- [x] T001 Confirm `kube-rs`/`k8s-openapi` versions already used by `src/backend/flux.rs` cover
      `CustomResourceDefinition` + `DynamicObject` listing (they do — no `Cargo.toml` change
      expected); note in a one-line comment at the top of the new `src/backend/gatekeeper.rs` file
      (created in T004) that it reuses the existing dependency set, no new crate added (FR-010).

## Phase 2: Foundational (blocking prerequisites for all user stories)

**Purpose**: Protocol types and route plumbing every user story's UI/backend code depends on.

- [x] T002 [P] Add `section_keys::integrations_gatekeeper() -> SectionKey` (value
      `"integrations/gatekeeper"`) to `src/protocol/v1/route.rs`, placed next to
      `integrations_flux()`/`integrations_hubble()` (contracts/protocol.md).
- [x] T003 [P] Create `src/protocol/v1/gatekeeper.rs` with `GatekeeperInstallState` (4 variants:
      `NotInstalled`, `InstalledNoConstraints`, `Active`, `PermissionDenied`),
      `GatekeeperViolation` (`section: SectionKey`, `kind: String`, `namespace: String`,
      `name: String`, `enforcement_action: String`, `message: String`),
      `GatekeeperConstraintSummary` (`kind: String`, `name: String`, `enforcement_action: String`,
      `total_violations: Option<u64>`, `violations: Vec<GatekeeperViolation>`,
      `violations_truncated: bool`, `template_missing: bool`), and `GatekeeperOverview`
      (`state: GatekeeperInstallState`, `error: String`, `constraints: Vec<GatekeeperConstraintSummary>`)
      — all deriving `Debug, Clone, Serialize, Deserialize, TS` per data-model.md, mirroring
      `src/protocol/v1/flux.rs`'s derive set and doc-comment style exactly.
- [x] T004 [P] Add serde round-trip tests in `src/protocol/v1/gatekeeper.rs` for
      `GatekeeperOverview`, `GatekeeperConstraintSummary`, `GatekeeperViolation`, and all four
      `GatekeeperInstallState` variants (contracts/protocol.md "Round-trip test coverage"),
      mirroring the round-trip test pattern already in `src/protocol/v1/flux.rs`.
- [x] T005 Re-export the new `gatekeeper` module's public types from `src/protocol/v1/mod.rs`
      (mirrors the existing `flux` module re-export).
- [x] T006 Add `BackendCmd::RefreshGatekeeper` unit variant to `src/protocol/v1/cmd.rs`, placed
      next to `RefreshFlux`/`RefreshHubble`, plus its serialization round-trip test (Constitution
      Principle III: "every protocol command ... variant MUST have a serialization round-trip
      test").
- [x] T007 Add `SystemEvent::GatekeeperOverviewUpdated(GatekeeperOverview)` variant to
      `src/protocol/v1/event.rs`, plus its serialization round-trip test.
- [x] T008 In `src/protocol/v1/mutation.rs`: add `C::RefreshGatekeeper` to the read-only command
      match arm alongside `C::RefreshFlux | C::RefreshHubble` (FR-009 — must never be classified
      as mutating), and add `C::RefreshGatekeeper => section_keys::integrations_gatekeeper()` to
      the route-classification match.
- [x] T009 Run the workspace's ts-rs codegen step (same command used after any protocol change;
      see `Makefile.toml`/CI config for the exact invocation) to generate
      `ui/src/protocol/generated/GatekeeperInstallState.ts`,
      `ui/src/protocol/generated/GatekeeperViolation.ts`,
      `ui/src/protocol/generated/GatekeeperConstraintSummary.ts`, and
      `ui/src/protocol/generated/GatekeeperOverview.ts`; commit the generated files (repo
      convention — generated bindings are checked in, per how `FluxOverview.ts` etc. already are).

**Checkpoint**: `cargo build --workspace` and `cargo test -p fathom` (protocol tests only) pass;
generated TS types exist. No user-story work should start before this phase is green.

## Phase 3: User Story 1 - See live policy compliance status at a glance (Priority: P1)

**Goal**: Operator opens the Gatekeeper pane and sees every active constraint with a live,
accurate violation count, including the "awaiting status" and "zero violations" edge cases.

**Independent Test**: Point Fathom at a cluster with Gatekeeper + a violating Constraint, open the
pane, confirm the list renders with accurate live-updating counts (per quickstart.md Scenario 1,
steps 1-3 and 8-9).

- [x] T010 [P] [US1] In new `src/backend/gatekeeper.rs`, implement CRD discovery: list
      `CustomResourceDefinition` cluster-wide (mirrors `flux.rs::find_served`/its `Api::all`
      call), find `constrainttemplates.templates.gatekeeper.sh`'s served version, and find every
      served CRD whose `spec.group == "constraints.gatekeeper.sh"` (research.md R1/R2). Return
      `GatekeeperInstallState::NotInstalled` when neither is found.
- [x] T011 [US1] In `src/backend/gatekeeper.rs`, implement `list_instances`-equivalent listing of
      every discovered constraint-kind CRD's instances as `DynamicObject`s (mirrors
      `flux.rs::list_instances` using `ApiResource::from_gvk` + `Api<DynamicObject>::all_with`),
      and on a `Forbidden`/403 API error while listing, return
      `GatekeeperInstallState::PermissionDenied` with the verbatim error text in
      `GatekeeperOverview.error` (FR-002, User Story 3 acceptance scenario 3 — implemented here
      since it's part of the same scan function, tested under US3 below).
- [x] T012 [US1] In `src/backend/gatekeeper.rs`, implement `summarize_constraint(obj: &DynamicObject, kind: &str) -> GatekeeperConstraintSummary`:
      read `spec.enforcementAction` (empty string if absent, data-model.md — do not default to
      "deny"); read `.status.totalViolations` as `Option<u64>`, falling back to
      `.status.violations.len()` when `totalViolations` is absent but `.status.violations` is
      present, and to `None` when `.status` itself is entirely absent (FR-004, the "awaiting
      status" edge case — `None` must never be coerced to `Some(0)`); read up to
      `.status.violations[]` entries into `Vec<GatekeeperViolation>` (message, namespace, kind,
      name, per-violation `enforcementAction` falling back to the constraint's own); set
      `violations_truncated = total_violations.is_some_and(|t| t > violations.len() as u64)`
      (FR-007, must be computed from live counts, never a hardcoded limit constant).
- [x] T013 [US1] In `src/backend/gatekeeper.rs`, implement `pub(super) async fn scan_gatekeeper(client: &Client) -> anyhow::Result<GatekeeperOverview>`
      wiring T010-T012 together: `NotInstalled` when no CRDs found; `InstalledNoConstraints` when
      CRDs found but zero constraint instances across all kinds; `Active` with the sorted (by
      kind, then name — mirrors `flux.rs`'s sort convention) `constraints` list otherwise;
      `PermissionDenied` with `error` populated on a listing 403.
- [x] T014 [P] [US1] Add unit tests in `src/backend/gatekeeper.rs` (`#[cfg(test)] mod tests`, not
      the live-cluster `#[ignore]` module) for `summarize_constraint`: awaiting-status (`.status`
      absent → `None`), zero-violations, truncation (`total_violations` > `violations.len()` →
      `violations_truncated: true`), and enforcement-action fallback, using hand-built
      `DynamicObject`/`serde_json::json!` fixtures (mirror any existing JSON-fixture unit test
      style already used for `flux.rs` or another dynamic-object backend module if one exists).
- [x] T015 [P] [US1] Add the `#[ignore]`d live-cluster test `scan_gatekeeper_against_live_cluster`
      in `src/backend/gatekeeper.rs`'s existing `live_cluster_tests`-style module, mirroring
      `flux.rs::scan_flux_against_live_cluster` (same `Client::try_default()` + rustls-provider
      setup), asserting `state == Active` and a nonempty `constraints` list.
- [x] T016 [US1] In `src/backend/sync.rs`: add `gatekeeper_poll_task` field to the watch-tasks
      struct (mirrors `flux_poll_task`), and implement `ensure_gatekeeper_poll` +
      `poll_gatekeeper_overview` (20s `tokio::time::interval`, mirrors
      `ensure_flux_poll`/`poll_flux_overview` exactly), emitting
      `BackendEvent::System(SystemEvent::GatekeeperOverviewUpdated(overview))` routed via
      `Route::cluster(section_keys::integrations_gatekeeper(), context.clone())` on success, and
      `events::notify_error` on a non-RBAC scan failure (contracts/protocol.md's error-path
      distinction from `PermissionDenied`).
- [x] T017 [US1] In `src/backend/cmd/integrations.rs`: add the `BackendCmd::RefreshGatekeeper =>
      refresh_gatekeeper(state, sender).await` dispatch arm and a `refresh_gatekeeper` function
      that calls `ensure_gatekeeper_poll` (idempotent start) and then immediately runs one
      `scan_gatekeeper` + event emit, decoupled from the 20s ticker (mirrors `refresh_flux`'s
      existing shape 1:1).
- [ ] T018 [US1] In `ui/src/store/index.ts`: add the `gatekeeper: { overview: GatekeeperOverview |
      null }` slice (initial `overview: null`), the `refreshGatekeeper()` action sending
      `BackendCmd::RefreshGatekeeper`, and the `SystemEvent::GatekeeperOverviewUpdated` case in the
      event reducer setting `gatekeeper.overview` to the payload verbatim — all three mirroring
      the existing `flux`/`refreshFlux`/`FluxOverviewUpdated` code paths.
- [ ] T019 [US1] Create `ui/src/components/panels/GatekeeperPanel.tsx`: on mount, call
      `refreshGatekeeper()` (mirrors `FluxPanel.tsx`'s `useEffect`); render
      `MetricsUnavailable` while `overview` is `null`; for `state === 'Active'`, render summary
      tiles via `IntegrationSummary` (e.g. total constraints, constraints with violations, total
      violation count) and a constraint list (name, kind, enforcement action, violation count —
      showing a distinct "awaiting status" label when `total_violations` is `null`, and "0
      violations"/compliant tone when it is `0`) — no click/drill-down wiring yet (that's US2).
- [ ] T020 [P] [US1] Add a unit test (Vitest) for the `gatekeeper` store slice in
      `ui/src/store/__tests__/` (or the existing store test location — match repo convention)
      covering: initial `overview: null`, `refreshGatekeeper()` sends the right command, and the
      event reducer updates `overview` on `GatekeeperOverviewUpdated`.
- [ ] T021 [P] [US1] Add a unit test (Vitest/RTL) for `GatekeeperPanel.tsx` covering: waiting
      state, `InstalledNoConstraints` empty state (distinct message), a constraint with
      `total_violations: null` rendering the "awaiting status" indicator (not "0 violations"), and
      a constraint with a real count rendering correctly.

**Checkpoint**: US1 is independently functional — opening the pane on a Gatekeeper cluster shows a
live, accurate constraint list; `cargo test`, `pnpm -C ui test` green for this scope.

## Phase 4: User Story 2 - Drill into a constraint to find and fix violating resources (Priority: P1)

**Goal**: Selecting a constraint reveals its violating resources with an Inspector link-out,
including the truncation-honesty and orphaned-template edge cases.

**Independent Test**: Select a constraint with known violations, confirm each violating resource's
detail renders, click one, confirm the Inspector opens on that exact object (quickstart.md
Scenario 1, steps 4-7).

- [x] T022 [US2] In `src/backend/gatekeeper.rs`, implement `fn section_for_kind(kind: &str) ->
      SectionKey` mapping common built-in kinds (`"Namespace"`, `"Pod"`, `"Deployment"`,
      `"StatefulSet"`, `"DaemonSet"`, `"ConfigMap"`, `"Secret"`, `"Service"`, `"Ingress"`, at
      minimum — extend as needed) to their existing `section_keys::*` functions, falling back to
      `section_keys::custom_resources()` for anything unmatched (research.md R4, mirrors
      `applications.rs::workload_owns_ref`'s match-based approach); use this when building each
      `GatekeeperViolation.section` in `summarize_constraint` (T012).
- [x] T023 [US2] In `src/backend/gatekeeper.rs`'s `summarize_constraint` (or a helper called from
      it), detect the orphaned-template edge case: if the constraint's owning `ConstraintTemplate`
      (matched by the constraint kind) was not found among the CRDs discovered in T010, set
      `template_missing: true`; otherwise `false`.
- [x] T024 [P] [US2] Extend the unit tests from T014 to cover `section_for_kind`'s known-kind
      mapping, its `custom_resources()` fallback for an unrecognized kind, and the
      `template_missing: true` case when a constraint's template CRD is absent from the fixture
      CRD list.
- [ ] T025 [US2] In `ui/src/components/panels/GatekeeperPanel.tsx`, add the drill-down: local
      `useState<string | null>` for the selected constraint (keyed by `kind`+`name`); clicking a
      constraint row selects it; when selected, render its `violations` list below (namespace,
      kind, name, message, enforcement action per entry); when `violations_truncated` is true,
      render "showing N of M violations" using `violations.length` and `total_violations` (FR-007
      — must not present the list as complete); when `template_missing` is true, show a small
      "references a missing ConstraintTemplate" flag on the constraint row.
- [ ] T026 [US2] In `GatekeeperPanel.tsx`, wire each violation row to
      `openInspector({ section: violation.section, kind: violation.kind, namespace: violation.namespace || null, name: violation.name }, paneId, true)`
      (contracts/protocol.md's UI navigation contract — normalize `""` namespace to `null` for
      cluster-scoped resources before calling `openInspector`, matching existing normalization
      convention), obtaining `paneId` and `openInspector` from the store the same way
      `GitOpsWarningBanner.tsx` does.
- [ ] T027 [P] [US2] Extend the `GatekeeperPanel.tsx` Vitest/RTL tests (T021's file) to cover:
      selecting a constraint reveals its violation list with correct fields, the truncation notice
      appears exactly when `violations_truncated` is true, and clicking a violation row calls
      `openInspector` with the expected `ObjectRef`.
- [ ] T028 [US2] Write the Playwright e2e spec `ui/tests/e2e/gatekeeper-panel.spec.ts` (per the
      task brief: "hyper-focused ... two scenarios total") against a scripted fake backend
      (existing e2e harness convention — mirror the setup used by another recent panel's e2e spec,
      e.g. Flux's or Hubble's if one exists, or the harness pattern documented in `ui/tests/e2e/`):
      **Scenario A** — inject a `GatekeeperOverviewUpdated` event with `state: 'Active'` and one
      constraint carrying a violation; open the Gatekeeper pane; assert the violation count and
      violating-resource row render; click the violating resource; assert the Inspector opens
      scoped to that object (namespace/kind/name match). **Scenario B** — inject
      `state: 'NotInstalled'`; open the pane; assert the explicit "not installed" message renders
      (no spinner, no empty list ambiguity).

**Checkpoint**: US1 + US2 together deliver the full "see it, drill into it, go fix it" flow
end-to-end; Playwright spec green.

## Phase 5: User Story 3 - Honest empty state when Gatekeeper isn't installed (Priority: P2)

**Goal**: `NotInstalled`, `InstalledNoConstraints`, and `PermissionDenied` all render distinct,
honest messages — no ambiguous blank/spinner/generic-error states.

**Independent Test**: Point Fathom at a cluster without Gatekeeper CRDs, open the pane, confirm
the explicit "not installed" message (quickstart.md Scenario 2). Separately verify the
zero-constraints and permission-denied states are visually distinct from it and from each other.

*(Backend support for all three states was already built in T010/T011/T013 as part of US1's scan
function — the scan function returns one unified `GatekeeperOverview` regardless of which state
applies, so there is no separate backend task here; this phase is UI-only.)*

- [ ] T029 [US3] In `GatekeeperPanel.tsx`, branch on `overview.state`: `NotInstalled` →
      `MetricsUnavailable` (or equivalent) with the exact copy "OPA Gatekeeper is not installed on
      this cluster"; `InstalledNoConstraints` → a distinct message, e.g. "Gatekeeper is installed
      but no constraints are defined"; `PermissionDenied` → a distinct message surfacing
      `overview.error` verbatim (Principle I — never replace backend errors with generic text);
      `Active` → the T019/T025 list view. Each state must be visually and textually
      distinguishable from the others (FR-002).
- [ ] T030 [P] [US3] Add Vitest/RTL coverage (extending T021/T027's file) asserting each of the
      four `overview.state` values renders its own distinct message/view, with explicit
      assertions that `NotInstalled` text differs from `InstalledNoConstraints` text and that
      `PermissionDenied` renders the injected `error` string verbatim.

**Checkpoint**: All three empty/edge states are covered; Scenario B of the Playwright spec (T028)
already exercises the `NotInstalled` path end-to-end.

## Phase 6: Polish & Cross-Cutting Concerns

- [ ] T031 [P] Update any panel-registry/routing file that lists available integration panes
      (wherever `FluxPanel`/`HubblePanel` are registered into the pane shell, e.g.
      `ui/src/components/shell/PaneHost.tsx` or a panes/registry module) to add
      `GatekeeperPanel` under `integrations/gatekeeper`, and add the corresponding nav entry
      (wherever Flux/Hubble appear in the integrations nav section) so the pane is reachable from
      the UI.
- [x] T032 [P] Add a doc comment at the top of `src/backend/gatekeeper.rs` matching `flux.rs`'s
      module-doc style, summarizing the discovery approach and explicitly noting the deliberate
      non-goals (no constraint authoring/editing, no template creation — FR-011) so future
      contributors don't accidentally scope-creep into Kubescape territory (per
      `docs/FEATURE_BACKLOG.md`).
- [ ] T033 Run the full verification suite end-to-end: `cargo build --workspace --all-targets`,
      `cargo clippy --all-targets -- -D warnings`, `cargo test`, `pnpm -C ui typecheck`,
      `pnpm -C ui lint`, `pnpm -C ui test`, and
      `pnpm -C ui exec playwright test gatekeeper-panel.spec.ts`; fix any failures before
      considering the feature done.

## Dependencies & Execution Order

- **Phase 1 (Setup)** → **Phase 2 (Foundational)**: no code depends on Phase 1 beyond the
  confirmation note; Phase 2 is the real blocking gate — all protocol types/routes must exist
  before any backend or frontend story work compiles.
- **Phase 3 (US1)** depends only on Phase 2.
- **Phase 4 (US2)** depends on Phase 3 (extends `summarize_constraint`, `GatekeeperPanel.tsx`, and
  the store already built in US1) — not independently implementable before US1's scan/panel exist,
  but independently *testable* once both are in place, per its own Independent Test above.
- **Phase 5 (US3)** depends on Phase 3 (needs `GatekeeperPanel.tsx` and the state-branching point)
  but not on Phase 4 — could be implemented in parallel with Phase 4 by a second contributor,
  since both only touch `GatekeeperPanel.tsx`'s rendering logic in different branches (minor merge
  overlap expected, not a hard sequencing dependency).
- **Phase 6 (Polish)** depends on Phases 3-5 being complete.

## Parallel Execution Examples

- Within Phase 2: T002, T003 (blocked by nothing), and T004 (same file as T003, sequential with
  it) can start together; T005-T008 depend on T003's types existing.
- Within Phase 3: T010 and T014/T015 (test-writing against the not-yet-implemented function) can
  be drafted in parallel by different contributors if T014/T015 are written against the
  data-model.md contract first, then wired up once T010-T013 land; realistically T010→T011→T012→T013
  are sequential (same file, each builds on the last), while T020/T021 (frontend tests) can proceed
  in parallel with T016-T019 once T018's store shape is agreed from data-model.md.
- Phase 4 and Phase 5 can run in parallel (see Dependencies above), each on its own branch, if two
  contributors are available.

## Implementation Strategy (MVP first)

**MVP = User Story 1 only** (Phases 1-3): a live, accurate constraint list with violation counts
and honest awaiting-status/zero-violations handling is independently valuable and testable even
before drill-down navigation exists — an operator can already tell "is anything violated right
now?" without leaving the app. User Story 2 (drill-down + Inspector link-out) is the natural next
increment, and User Story 3's remaining states (P2) round out the honest-empty-state guarantee.
Given both US1 and US2 are P1 in spec.md and share the bulk of the backend surface, they are
expected to ship together in practice, with US3's `PermissionDenied`/`InstalledNoConstraints`
polish following immediately after.
