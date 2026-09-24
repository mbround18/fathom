---
description: "Task list for Local Mutation Audit-Log Viewer"
---

# Tasks: Local Mutation Audit-Log Viewer

**Input**: Design documents from `/specs/026-mutation-audit-log-viewer/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md

**Tests**: Constitution Principle III (NON-NEGOTIABLE) requires boundary tests for every user
story, protocol round-trips, and UI decision logic; the task brief additionally requires a
Playwright E2E spec. Test tasks are included throughout.

**Organization**: Tasks are grouped by user story (spec.md P1/P2/P3) for independent
implementation and testing.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no unresolved dependency)
- **[Story]**: US1, US2, US3 — maps to spec.md priorities

## Path Conventions

Single project (Tauri desktop app): Rust backend at `src/`, frontend at `ui/src/`, Rust
integration tests at `tests/`, Playwright specs at `ui/tests/e2e/`.

---

## Phase 1: Setup

**Purpose**: Confirm dependencies and scaffolding needed before any protocol/backend work.

- [x] T001 Confirm whether a UUID-generation dependency is already available in the workspace
      (`grep -n uuid Cargo.toml Cargo.lock`); if absent, add `uuid` (v4, `serde` feature) to
      `Cargo.toml` as the minimal additive dependency for `AuditLogEntry.id` generation, or reuse
      the existing `src/protocol/v1/ids.rs` `RequestId`-style generator if it already produces a
      suitable unique string — document the choice as a one-line comment in `src/protocol/v1/audit.rs` (created in T004).
- [x] T002 [P] Confirm the existing `SectionKey` helper module (`src/protocol/v1/route.rs`,
      `mod names` block) so the new `audit_log()` constructor (T007) matches the existing naming/
      style (`SectionKey::new("audit-log")`, `Global` scope per `research.md` R4).

**Checkpoint**: Dependency and convention decisions confirmed; no code changes yet beyond a
possible `Cargo.toml` addition.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Protocol types, backend persistence, and the audit-write hook that every user story
depends on. **No user story can be implemented or tested until this phase is complete.**

- [x] T003 Create `src/protocol/v1/audit.rs` defining `AuditLogEntry` with exactly the fields from
      `data-model.md`: `id: String` (UUID v4, always present), `timestamp: String` (RFC 3339 UTC,
      always present), `action: AuditActionKind` (always present), `action_label: String` (always
      present), `context: String` (always present), `target_kind: Option<String>`,
      `target_namespace: Option<String>`, `target_name: Option<String>` (each independently
      optional per data-model.md), `outcome: AuditOutcome` (always present), `request_id:
      Option<String>`. Define `AuditOutcome { Success, Failure { reason: String } }` — enforce
      via a constructor/validation helper that `Failure` is never constructed with an empty
      `reason` string when an error message is available (data-model.md validation rule: "never
      leave `Failure` reason-less if avoidable"). Derive `Serialize`, `Deserialize`, `Debug`,
      `Clone`, `PartialEq`. Register the module in `src/protocol/v1/mod.rs`.
- [x] T004 [P] In `src/protocol/v1/audit.rs`, define the exhaustive `AuditActionKind` enum (no
      wildcard arm) covering every mutating `BackendCmdV1` variant category from
      `contracts/protocol-v1-audit-log.md`: `YamlApply`, `HelmUpgrade`, `HelmRollback`,
      `HelmUninstall`, `NodeCordon`, `NodeUncordon`, `NodeDrain`, `NodeTaint`, `RbacChange`,
      `Delete`, `BulkDelete`, `Scale`, `Restart`, `PortForwardChange`, `McpToggle`,
      `TeamProfilePublish`, `ContextConfigChange` — cross-check against the full match arms in
      `src/protocol/v1/mutation.rs`'s `mutation_target()` to ensure every
      `MutationTarget::{Namespaced, ClusterScoped, Local}`-producing `BackendCmdV1` variant maps
      to one of these kinds (add any missing kind discovered during this cross-check). Derive
      `Serialize`, `Deserialize`, `Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`.
- [x] T005 [US-shared] In `src/protocol/v1/mutation.rs`, add
      `impl BackendCmdV1 { pub fn audit_summary(&self) -> Option<AuditActionSummary> }` (mirrors
      the exhaustive, no-wildcard-arm style of `mutation_target()`, per `research.md` R1 and
      `contracts/protocol-v1-audit-log.md`). Define `AuditActionSummary { kind: AuditActionKind,
      label: String, target_kind: Option<String>, target_namespace: Option<String>, target_name:
      Option<String> }` in `src/protocol/v1/audit.rs` (T003). Return `None` for every
      `MutationTarget::ReadOnly` and `MutationTarget::ContextSwitch` variant (FR-001 scope: only
      mutating actions are recorded); return `Some(..)` for every `Namespaced`/`ClusterScoped`/
      `Local` variant, extracting the target kind/namespace/name from the command's own fields
      where available (`None` where not, per data-model.md's independent-optionality rule).
- [x] T006 [P] Add `pub fn audit_log_path() -> Result<PathBuf>` to `src/backend/persistence.rs`
      (`fathom_config_dir()?.join("audit-log.json")`, matching the existing `*_path()` helper
      style at lines ~70-122).
- [x] T007 Create `src/backend/audit_log.rs` implementing:
      `load_entries() -> Vec<AuditLogEntry>` (missing/corrupt file → empty vec, logged via
      `tracing::warn!`, file left untouched — same pattern as `load_cluster_settings`/
      `load_ui_config`, Constitution Principle VI);
      `append_entry(entry: AuditLogEntry) -> Result<()>` (load, push, truncate to the newest 500
      entries per `research.md` R3 — oldest-first eviction — then save via
      `save_json_file`/`audit_log_path()`); wrap the container as `{ "entries": [...] }` per
      `data-model.md`'s `AuditLogStore` shape. Any I/O/serialization error in `append_entry` MUST
      be caught and logged (`tracing::warn!`), never propagated as a panic or as the calling
      command's own error (FR-011).
- [x] T008 In `src/backend.rs`, inside `run_cmd` (~line 169-207): before `cmd` is moved into
      `cmd::dispatch_cmd(cmd, &mut state, sender)`, capture
      `let audit_summary = cmd.audit_summary();` and the current cluster context name (from
      `state`, same source `finished_route`/existing context-aware handlers use) and the request
      id (from `id`/`events::current_request_id_string()`). After `ok`/`error` are computed and
      before/alongside the `events::emit(... CommandFinished ...)` call, if
      `audit_summary.is_some()`, construct an `AuditLogEntry` (UUID from T001, `timestamp` =
      `chrono::Utc::now()` RFC3339 or equivalent already-available time source, `outcome` =
      `Success` if `ok` else `Failure { reason: error.clone().unwrap_or_else(|| "unknown error"
      .to_string()) }`) and call `audit_log::append_entry(entry)`, logging (not propagating) any
      error. This write MUST happen without delaying or altering the `CommandFinished` emission
      itself (FR-011, SC-003 <100ms budget).
- [x] T009 [P] Add `SectionKey::audit_log()` to `src/protocol/v1/route.rs`'s helper module
      (`SectionKey::new("audit-log")`), following the existing `overview()`/`nodes()` pattern
      (lines ~92-117).
- [x] T010 Add `BackendCmdV1::RefreshAuditLog` to `src/protocol/v1/cmd.rs`; wire its
      `mutation_target()` arm in `src/protocol/v1/mutation.rs` to `MutationTarget::ReadOnly`
      (it only reads `audit-log.json`, matching contracts/protocol-v1-audit-log.md), and its
      `section()` arm to `SectionKey::audit_log()` (Global scope, `context: None`, per
      `research.md` R4 — the list spans all contexts by design).
- [x] T011 Add a handler for `RefreshAuditLog` in the appropriate `src/backend/cmd/` module
      (alongside other read-only list-refresh handlers, e.g. near events/pods dispatch): call
      `audit_log::load_entries()` (T007) and emit the result via the existing generic list-event
      mechanism (`BackendEventV1::ResourceList`-equivalent) or, if no existing variant fits
      `Vec<AuditLogEntry>` cleanly, add exactly one new `BackendEventV1::AuditLogLoaded(Vec<AuditLogEntry>)`
      variant in `src/protocol/v1/event.rs`, routed via `Route::global(SectionKey::audit_log())`.
- [x] T012 [P] Regenerate/update TypeScript protocol bindings for the new
      `AuditLogEntry`/`AuditActionKind`/`AuditOutcome`/`RefreshAuditLog`/audit-log event types
      under `ui/src/protocol/generated/` (run the project's existing binding-generation step —
      check `Makefile.toml`/`build.rs` for the exact command, e.g. `cargo make generate-bindings`
      or equivalent) so the frontend has typed access.
- [x] T013 [P] Rust unit tests for `audit_summary()` (T005) in `src/protocol/v1/mutation.rs`'s
      `#[cfg(test)]` module (or a new `tests/` file if the existing test module structure prefers
      that): assert `None` for representative `ReadOnly`/`ContextSwitch` commands, and `Some(..)`
      with the correct `AuditActionKind` + target fields for at least one representative command
      per category (YAML apply, Helm upgrade, node cordon, RBAC change, a `Local` mutation).
- [x] T014 [P] Rust unit/integration test for `src/backend/audit_log.rs` (T007) in `tests/` (new
      file, e.g. `tests/audit_log_persistence.rs`): round-trip append/load against a temp
      `~/.fathom`-equivalent directory (use the existing test pattern for isolating
      `fathom_config_dir()` in tests, or inject a path override if one already exists for other
      persistence tests); assert retention truncation — appending 501 entries leaves exactly the
      newest 500, oldest evicted first (data-model.md / research.md R3); assert a
      corrupt/missing file yields an empty vec without deleting/overwriting an existing unreadable
      file (Constitution Principle VI).

**Checkpoint**: Protocol types, persistence, and the audit-write hook exist and are tested.
Backend can now record and serve audit history; no UI yet. All user stories can proceed.

---

## Phase 3: User Story 1 - Review recent mutating actions across all contexts (Priority: P1) 🎯 MVP

**Goal**: A user can perform a mutating action and see it appear in a browsable audit-log viewer
pane with type/target/context/timestamp/outcome.

**Independent Test**: Perform a mutating action (e.g. cordon a node), open the audit-log viewer,
confirm the action appears with target resource, cluster context, timestamp, and outcome.

### Tests for User Story 1

- [ ] T015 [P] [US1] Playwright E2E spec `ui/tests/e2e/audit-log.spec.ts`: scripted-fake-backend
      scenario that (a) triggers one mutating action (e.g. a node cordon command dispatched
      through the existing fake-backend harness used by other E2E specs — follow the pattern in
      an existing mutation-flow spec, e.g. from spec 006/008), (b) opens the Audit Log viewer
      pane, (c) asserts the resulting entry shows the correct target resource name, outcome
      (success), and a timestamp, per Constitution Principle III (drives the UI by injecting
      events and asserting rendered state, not by hitting a real cluster). This is Part 1 of the
      "hyper-focused" spec required by the task brief; Part 2 (filter check) is T023 in Phase 4.
- [ ] T016 [P] [US1] Frontend unit test for the `AuditLogLoaded`/list-response store handler
      (`ui/src/store/handlers/`, new or existing test file) asserting entries are stored
      reverse-chronologically and re-render on refresh.

### Implementation for User Story 1

- [ ] T017 [US1] Add a Zustand store slice for audit-log entries (`ui/src/store/`, new file or
      extension of an existing list-store pattern) holding `AuditLogEntry[]`, a `loading` flag,
      and a `refresh()` action that dispatches `RefreshAuditLog` and applies the response via the
      handler from T016.
- [ ] T018 [US1] Create `ui/src/components/audit/AuditLogPane.tsx`: a read-only list/table view
      (reverse-chronological) rendering, per entry, `action_label`, `target_kind`/
      `target_namespace`/`target_name` (formatted as a resource descriptor), `context`,
      `timestamp` (localized, human-readable), and `outcome` (visually distinguishable
      success/failure, per spec.md Acceptance Scenario 1.3 — "clearly distinguishable 'failed'
      outcome"). Follow the existing mobile-first, column-priority pattern (Constitution
      Principle IV): on narrow widths, drop `context`/`request_id`-equivalent columns first, keep
      `action_label`/target/outcome/timestamp.
- [ ] T019 [US1] Add empty states to `AuditLogPane.tsx`: "no actions recorded yet" (empty
      history) distinct from later "no matching entries" (Phase 4) per spec.md Edge Cases.
- [ ] T020 [US1] Wire a new navigation entry (`ui/src/components/nav/` or `ui/src/layout/`,
      following the existing nav-entry pattern) to open `AuditLogPane` as a new pane in the
      three-pane shell, calling `refresh()` (T017) on mount.
- [ ] T021 [US1] On completion of any mutating command (existing `CommandFinished` handling in
      the frontend bridge/store, `ui/src/store/handlers/`), trigger `refresh()` (T017) for the
      audit-log slice if the Audit Log pane is mounted/visible, so a newly completed mutation
      appears without a manual refresh (spec.md SC-001: within 2 seconds).

**Checkpoint**: User Story 1 is independently functional — perform a mutation, see it in the
viewer with correct fields.

---

## Phase 4: User Story 2 - Filter and search the history (Priority: P2)

**Goal**: A user can filter/search the audit-log history by action type, outcome, context, time
range, and free-text resource-name search.

**Independent Test**: With a history containing multiple action types/outcomes, apply a filter
and confirm only matching entries remain; clear it and confirm the full list returns.

### Tests for User Story 2

- [ ] T022 [P] [US2] Frontend unit tests for the filter/search reducer/selector (new file, e.g.
      `ui/src/store/auditLogFilter.test.ts`): action-type filter, outcome filter, context filter,
      time-range filter, and case-insensitive free-text search against `target_name` each
      narrow the visible set correctly in isolation and in combination; clearing all filters
      restores the full (retention-bounded) list. Exercise the `AuditLogFilter` shape exactly as
      specified in `data-model.md` (`actionKinds: AuditActionKind[]`, `outcome: "all" | "success"
      | "failure"` default `"all"`, `context: string | null` default `null`, `timeRange: { from,
      to }` default unset, `searchText: string`).
- [ ] T023 [US2] Extend `ui/tests/e2e/audit-log.spec.ts` (from T015) with the required second
      scenario: with at least two distinct entries present (from the T015 setup plus one
      additional differing action/outcome), apply one filter or search term and assert the
      visible entry count/content narrows correctly, then clear it and assert the full list
      reappears. This completes the "one end-to-end scenario plus one filter check" required by
      the task brief.

### Implementation for User Story 2

- [ ] T024 [P] [US2] Add the `AuditLogFilter` state shape (data-model.md) to the audit-log store
      slice (T017): `actionKinds`, `outcome`, `context`, `timeRange`, `searchText`, all
      transient/session-only (never persisted to disk or `localStorage`, per spec.md's Key
      Entity note that filter/search state is not persisted).
- [ ] T025 [US2] Implement filter/search application logic (pure function, unit-tested by T022)
      that derives the visible entry list from the full store slice + `AuditLogFilter`, applied
      entirely client-side (research.md R4: 500 entries is small enough for in-memory filtering).
- [ ] T026 [US2] Add filter controls to `AuditLogPane.tsx` (T018): action-type multi-select,
      outcome toggle (all/success/failure), context select, time-range picker, and a text search
      input, using existing shadcn/ui form components consistent with other filter UIs in the
      app (check `ui/src/components/panels/` for an existing filter-bar pattern to match).
- [ ] T027 [US2] Add the "no matching entries" empty state (distinct from "no history exists",
      T019) to `AuditLogPane.tsx` when filters/search yield zero results, per spec.md Edge Cases.
- [ ] T028 [US2] Add a visible "clear filters" control that resets `AuditLogFilter` to defaults.

**Checkpoint**: User Stories 1 and 2 both work independently; filtering/search is fully
client-side and does not affect persisted data.

---

## Phase 5: User Story 3 - Trust local retention limits (Priority: P3)

**Goal**: The viewer and its behavior make clear the history is local-only, bounded, and never
includes cluster-side (Kubernetes API server) audit data.

**Independent Test**: Exceed the retention bound and confirm oldest entries drop; confirm no
entry corresponds to a Kubernetes-API-server-originated event Fathom didn't initiate.

### Tests for User Story 3

- [ ] T029 [P] [US3] Extend `tests/audit_log_persistence.rs` (T014) if not already fully covered:
      explicit assertion that after exceeding 500 entries, the oldest entries are gone and the
      newest 500 remain in order (already covered by T014; this task is a checkpoint to confirm
      coverage rather than new code if T014 already asserts it — otherwise add the missing
      assertion here).
- [ ] T030 [P] [US3] Frontend unit/E2E-level check (extend `AuditLogPane` tests or the E2E spec)
      that the viewer never renders a "source: cluster audit log" or equivalent — i.e., there is
      no code path that fetches or displays Kubernetes API server audit events (a negative/
      structural check: no such command exists in the protocol, and no such data source is wired
      into `AuditLogPane.tsx`).

### Implementation for User Story 3

- [ ] T031 [US3] Add a short, permanent explanatory note/tooltip in `AuditLogPane.tsx` (e.g. near
      the pane header) stating the history is local-only, bounded to the most recent 500 actions,
      and is not Kubernetes' own audit log — satisfying spec.md User Story 3's "so I don't mistake
      it for a complete or cluster-wide compliance record."

**Checkpoint**: All three user stories independently functional and tested.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Final quality gates across all stories.

- [x] T032 [P] Run `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`; fix any
      warnings introduced by T003-T014.
- [ ] T033 [P] Run `pnpm -C ui lint` and `pnpm -C ui typecheck`; fix any issues introduced by
      T012, T017-T028, T031.
- [ ] T034 Run `cargo build --workspace --all-targets` and `cargo test`; confirm all new and
      existing tests pass.
- [ ] T035 Run `pnpm -C ui test` (Vitest) and the full Playwright suite including
      `ui/tests/e2e/audit-log.spec.ts`; confirm all pass.
- [ ] T036 Execute the manual validation steps in `quickstart.md` end-to-end at least once against
      a real dev cluster to confirm the automated tests match observed behavior.
- [ ] T037 Update any relevant top-level docs that enumerate shipped features (e.g.
      `docs/MVP.md` if it exists and is the convention used by prior specs 001-015) to mark
      026-mutation-audit-log-viewer as shipped, following the same pattern as the
      015-mcp-agent-exposure commit history (`docs(026-mutation-audit-log-viewer): mark ... shipped`).

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies.
- **Foundational (Phase 2)**: Depends on Setup. **BLOCKS all user stories** — protocol types,
  persistence, and the audit-write hook must exist before any story can be built or tested.
- **User Story 1 (Phase 3)**: Depends on Foundational only.
- **User Story 2 (Phase 4)**: Depends on Foundational + User Story 1's `AuditLogPane.tsx`/store
  slice existing (T017, T018) to attach filter UI to — not independently buildable before US1's
  pane exists, though its filter *logic* (T022, T024, T025) could be developed in parallel.
- **User Story 3 (Phase 5)**: Depends on Foundational (retention logic, T007) and, for T031,
  User Story 1's pane (T018) existing to attach the note to.
- **Polish (Phase 6)**: Depends on all desired user stories being complete.

### Parallel Opportunities

- T001-T002 (Setup) in parallel.
- T004, T006, T009, T012, T013, T014 (Foundational, marked [P]) in parallel once their
  individual prerequisites (e.g. T003 before T004) land.
- T015, T016 (US1 tests) in parallel; T017-T021 (US1 impl) are mostly sequential (same files).
- T022, T024 (US2) in parallel with each other; T023/T026-T028 depend on US1's pane existing.
- T029, T030 (US3 tests) in parallel.
- T032, T033 (Polish) in parallel.

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1 (Setup) and Phase 2 (Foundational) — this alone makes every mutating action
   recorded and queryable via `RefreshAuditLog`, even before any UI exists.
2. Complete Phase 3 (User Story 1) — a working, read-only, unfiltered viewer pane. This is a
   demoable MVP satisfying the feature's core value.
3. **STOP and VALIDATE** against `quickstart.md`'s User Story 1 steps before proceeding.

### Incremental Delivery

1. Setup + Foundational → backend fully records/serves history (no UI).
2. + User Story 1 → MVP viewer pane.
3. + User Story 2 → filter/search.
4. + User Story 3 → retention-trust messaging + verified bound behavior.
5. + Polish → full CI gate green, quickstart re-validated.
