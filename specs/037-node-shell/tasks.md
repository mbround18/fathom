# Tasks: Node Shell

Format: `[ID] [P?] [Story] description (path)`. [P] = parallelizable (different files). Tests precede implementation within each story.

## Phase 1: Setup
- [x] T001 Confirm state of 024/025 on main; record which of the confirm dialog, SSAR helper and terminal component exist (specs/037-node-shell/research.md R7)
- [x] T002 [P] Extract shared `TerminalView` from `ui/src/components/panels/PodTerminalTab.tsx` if not already separable

## Phase 2: Foundational (blocks all stories)
- [x] T003 Add protocol variants (`CheckNodeShellAccess`, `StartNodeShell`, `StopNodeShell`, `ListNodeShellOrphans`, `DeleteNodeShellOrphans`, settings get/set) and events (`NodeShellAccessChecked`, `NodeShellStatus`, `NodeShellOrphans`) (src/protocol/v1/cmd.rs, event.rs)
- [x] T004 Classify in `mutation_target()` per contract (src/protocol/v1/mutation.rs)
- [x] T005 [P] Serde round-trip tests for every new variant; regenerate TS bindings and drift check
- [x] T006 [P] Unit tests for mutation classification and devbridge guard refusing `StartNodeShell` (tools/devbridge/src/guard.rs)
- [x] T007 Create `src/backend/node_shell.rs` skeleton: session struct, phase enum, `build_pod()` with unit tests asserting privileged, host namespaces, nodeName, tolerations, labels, deadline, pull secret
- [x] T008 Generalize exec target selection so a node-shell pod can be attached (src/backend/exec.rs), with tests that pod exec behavior is unchanged
- [x] T009 Wire dispatch in src/backend/cmd.rs; extend the fake backend used by Playwright

## Phase 3: US1 - Open a shell on a node (P1)
- [x] T010 [P] [US1] Playwright: confirm names node/ns/pod/image/privileges; decline sends nothing; reopen re-confirms (ui/tests/e2e/node-shell.spec.ts)
- [x] T011 [P] [US1] Backend tests: create -> watch to Running -> attach state machine incl. stale session discard
- [x] T012 [US1] Implement create, single-pod watch, phase events, attach (src/backend/node_shell.rs)
- [x] T013 [US1] `NodeShellPanel.tsx` with confirm dialog, progress phases, terminal; "Node shell" action in Node inspector; phone-width layout
- [x] T014 [US1] Vitest for phase-to-UI mapping

## Phase 4: US2 - Guaranteed cleanup (P1)
- [x] T015 [P] [US2] Backend tests, one per end path: stop, tab close, terminal exit, supersede, context switch, app exit, delete-fails-leftover, NotFound-as-success
- [x] T016 [P] [US2] Playwright: leftover pod message with retry; "removed" only after backend event
- [x] T017 [US2] Implement delete-with-captured-client and bounded exit/context-switch hook (src/backend/node_shell.rs, Tauri exit hook)
- [x] T018 [US2] Implement orphan sweep list/delete (labels, terminal-phase pods) and UI list with per-pod confirmation
- [x] T019 [US2] Test that the deadline field is present on every created pod; run quickstart step 4 manually

## Phase 5: US3 - RBAC-aware failures (P2)
- [x] T020 [P] [US3] Backend tests: access check per permission (allowed/denied/error) and fail-safe on error
- [x] T021 [P] [US3] Playwright: denied list with no dialog; admission error verbatim; pending timeout shows conditions and cancel deletes
- [x] T022 [US3] Implement `CheckNodeShellAccess` via the SSAR helper (src/backend/rbac_access.rs reuse)
- [x] T023 [US3] Map create/admission/pull/unschedulable/timeout to Failed with verbatim detail and cleanup; UI states

## Phase 6: US4 - Settings (P3)
- [x] T024 [P] [US4] Persistence tests: additive load, missing keys default, invalid file preserved (src/backend/persistence.rs)
- [x] T025 [US4] Settings fields in Cluster Settings; per-session override in confirmation
- [X] T026 [US4] Playwright for settings persistence and override (ui/tests/e2e/node-shell.spec.ts)

## Phase 7: Polish
- [x] T027 [P] Disable action with reason for Windows nodes; test (view helper unit-tested in Vitest; no Playwright test)
- [X] T028 [P] Accessibility, keyboard, reduced-motion pass; verify no horizontal scroll at supported widths (keyboard, labels and reduced-motion asserted in node-shell.spec.ts; no horizontal scroll checked at all widths)
- [x] T029 Update docs/MVP.md Node Shell status and section text
- [ ] T030 Run quickstart on Tier 1 platforms; full CI gates (PARTIAL, not done: Linux only; live kind run via an ignored test; Ubuntu 26/Win11/macOS not run)
- [ ] T031 Brochure screenshots of the new screens (final step per project convention)

## Dependencies
Phase 1 -> 2 -> 3. Phases 5 and 6 can run in parallel after T012 (shared node_shell.rs); 4 depends on 3. Polish last. Suggested fan-out: agent A (protocol T003-T006), agent B (backend T007-T009, T012, T017, T018, T022, T023), agent C (UI T013, T016, T021, T025), then integration.

## MVP
Phases 1-4 (US1 + US2): a shell that always cleans up. US3 and US4 follow.
