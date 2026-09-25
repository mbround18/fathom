---
description: "Task list for Pod Exec Session Gate (Confirmation + RBAC Pre-Check)"
---

# Tasks: Pod Exec Session Gate (Confirmation + RBAC Pre-Check)

**Input**: Design documents from `/specs/024-pod-exec-terminal/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/protocol.md, quickstart.md

**Tests**: Requested by the task brief (protocol round-trip tests, backend unit tests, and a
Playwright E2E spec). Included below.

**Organization**: Tasks are grouped by user story. Both stories extend the same existing files
(`mutation.rs`, `exec.rs`, `PodTerminalTab.tsx`), so Phase 2 (Foundational) carries the shared
protocol plumbing both stories need, per data-model.md.

## Phase 1: Setup

- [x] T001 Confirm `Cargo.toml`/`ui/package.json` need no new dependency (research.md R3) — no
      action expected; sanity-check only, no file changes.

## Phase 2: Foundational (blocking prerequisites for both user stories)

**Purpose**: Protocol/state changes both stories depend on.

- [x] T002 Add `CheckPodExecAccess { namespace: String, name: String, container: Option<String> }`
      variant to `BackendCmdV1` in `src/protocol/v1/cmd.rs` (contracts/protocol.md)
- [x] T003 [P] Add a serde round-trip test for `CheckPodExecAccess` in `src/protocol/v1/cmd.rs`
      (Constitution Principle III), alongside the existing `start_pod_exec_roundtrip` etc. tests
- [x] T004 Add `PodExecAccessChecked { namespace: String, name: String, container: Option<String>, allowed: bool, reason: Option<String> }`
      as a new `ResourceReadEvent` variant in `src/protocol/v1/event.rs` (data-model.md); `reason`
      MUST be `Some` whenever the underlying review call itself errored (contracts/protocol.md
      invariant), `None` is only valid for a plain server-side deny with no explanation
- [x] T005 [P] Add a serde round-trip test for `PodExecAccessChecked` in `src/protocol/v1/event.rs`
- [x] T006 In `src/protocol/v1/mutation.rs`, move `C::StartPodExec { .. }` out of the `ReadOnly`
      match arm (currently lines ~63-66) into a new `Namespaced` arm keyed on its `namespace`
      field (research.md R1); add `C::CheckPodExecAccess { .. }` to the existing `ReadOnly` arm.
      Keep `SendPodExecInput`/`ResizePodExec`/`StopPodExec` in `ReadOnly`, unchanged.
- [x] T007 In `src/protocol/v1/mutation.rs`, add `C::CheckPodExecAccess { .. }` to the
      `section()` match's `section_keys::workloads_pods()` arm (next to the other pod-exec
      commands, currently lines ~339-342)
- [x] T008 [P] Add a unit test in `src/protocol/v1/mutation.rs` asserting
      `BackendCmdV1::StartPodExec { namespace: "ns".into(), name: "n".into(), container: None }.mutation_target()`
      equals `MutationTarget::Namespaced { namespaces: vec!["ns".into()] }` (not `ReadOnly`),
      per contracts/protocol.md's reclassification contract
- [x] T009 [P] Add/verify a `tools/devbridge/src/guard.rs` test asserting `StartPodExec` is now
      blocked by the sandbox guard for a namespace that isn't allow-listed (mirrors the existing
      `DeletePod`-style guard test in that file) — confirms the "consumers affected" note in
      contracts/protocol.md
- [x] T010 [P] Add an `ExecAccessCheckState` type (`allowed: boolean`, `reason?: string`,
      `rev: number`) plus a store slice keyed by `` `${namespace}/${name}/${container ?? ''}` ``
      in `ui/src/store/index.ts` (data-model.md "Store: exec-access-check slice")
- [x] T011 Add `PodExecAccessChecked` handling to `ui/src/store/handlers/stream.ts` (or a new
      handler if that file is scoped strictly to stream events — check its existing scope first),
      writing into T010's slice and bumping `rev`

**Checkpoint**: Protocol builds, round-trips pass, mutation classification test passes — ready
for both user stories.

---

## Phase 3: User Story 1 - Confirm before opening a shell into a container (Priority: P1)

**Goal**: An explicit confirmation, naming namespace/pod/container, is required before any
`StartPodExec` is sent.

**Independent Test**: Open the Terminal tab for a running pod, see the confirmation before any
shell input is possible; confirm proceeds to the existing terminal experience; decline sends
nothing and leaves state unchanged (spec.md Acceptance Scenarios 1-4).

- [x] T012 [US1] In `ui/src/components/panels/PodTerminalTab.tsx`, extend the `Phase` union with
      `'checking' | 'denied' | 'confirming'` between `'picking'` and `'attaching'` (data-model.md
      "PodTerminalTab phase union")
- [x] T013 [US1] In `PodTerminalTab.tsx`, replace the current auto-start `useEffect` (today:
      sends `StartPodExec` as soon as `started` is true) with: once a container is resolved, send
      `CheckPodExecAccess { namespace, name, container }` and enter `'checking'`; do NOT send
      `StartPodExec` from this effect anymore
- [x] T014 [US1] In `PodTerminalTab.tsx`, read the T010 store slice for the current
      namespace/name/container key; when a fresh result (`rev` newer than last-seen) arrives with
      `allowed: true`, transition to `'confirming'`; on `allowed: false`, transition to `'denied'`
      (US2 covers the denied-state UI; this task only needs the phase transition itself to exist
      so US1's flow is complete end-to-end)
- [x] T015 [US1] In `PodTerminalTab.tsx`, render `ConfirmDialog` (`ui/src/components/ConfirmDialog.tsx`)
      when `phase === 'confirming'`: `items: [{ namespace, name }]`, `detail` naming the chosen
      container, `destructive: true`, `confirmLabel: "Open terminal"`; `onConfirm` sends
      `StartPodExec` and transitions to `'attaching'` (existing flow, unchanged from there);
      `onCancel` returns to `'picking'` (if >1 container) or an idle not-started state (if 1
      container — add a minimal idle state with an explicit "Open Terminal" trigger, since the
      component can no longer auto-start for the single-container case either, per FR-002/FR-004
      applying uniformly)
- [x] T016 [US1] In `PodTerminalTab.tsx`, ensure reopening the tab (component remount, or pod/name
      change while mounted) resets to the pre-check flow rather than reusing a stale `'confirming'`/
      `'denied'` result — confirmation must never be skipped on a subsequent open (spec.md
      Acceptance Scenario 4; data-model.md state-transition diagram's "picking --(container
      resolved)--> checking" applies fresh every time)
- [x] T018 [US1] Playwright scenario in `ui/tests/e2e/pod-exec-gate.spec.ts`: open Terminal tab
      against the fake backend, inject `PodExecAccessChecked { allowed: true }`, assert
      `ConfirmDialog` (`data-testid="confirm-dialog"`) appears naming the pod before any
      `StartPodExec` is sent; assert clicking confirm sends `StartPodExec` and, after injecting
      `PodExecReady`/`PodExecOutputChunk`, the output renders in the `xterm.js` pane
      (`data-testid="pod-terminal-xterm"`); assert clicking cancel sends no `StartPodExec` at all
      (quickstart.md Scenario 1)

**Checkpoint**: User Story 1 independently complete and testable — every exec open requires
confirmation, decline sends nothing.

---

## Phase 4: User Story 2 - Block exec attempts the operator isn't permitted to make (Priority: P2)

**Goal**: A live, proactive permission check pre-empts the confirmation entirely when the
operator lacks `pods/exec`.

**Independent Test**: As an operator without `pods/exec` in the namespace, open the Terminal
tab and see a permission-denied message immediately, no confirmation shown, no attach sent; as
a permitted operator, the flow is unaffected (spec.md Acceptance Scenarios 1-3).

- [x] T019 [US2] In `src/backend/exec.rs`, add `pub(super) async fn check_access(state: &BackendState, sender: &Sender<BackendEventEnvelope>, namespace: String, name: String, container: Option<String>) -> anyhow::Result<()>`:
      build one `SelfSubjectAccessReview` via `Api::<SelfSubjectAccessReview>::all(client)` +
      `PostParams::default()` (mirroring `handle_refresh_rbac_self_access` in
      `src/backend/rbac_access.rs`) with `ResourceAttributes { group: Some(String::new()), resource: Some("pods".into()), subresource: Some("exec".into()), verb: Some("create".into()), namespace: Some(namespace.clone()), name: Some(name.clone()), ..Default::default() }`;
      on success emit `PodExecAccessChecked { allowed: status.allowed, reason: status.reason.or(status.evaluation_error), .. }`;
      on the API call itself erroring, emit `allowed: false, reason: Some(err.to_string())`
      (contracts/protocol.md's error-mapping invariant)
- [x] T020 [P] [US2] Unit tests in `src/backend/exec.rs` for `check_access`'s `ResourceAttributes`
      construction (asserts `group` is empty-string/core, not `"rbac.authorization.k8s.io"` —
      the exact bug this spec must avoid per research.md R2) and its error-to-`allowed:false`
      mapping, mirroring the existing `describe_attach_failure` test conventions in this file
- [x] T021 [US2] Wire `BackendCmd::CheckPodExecAccess { namespace, name, container }` to
      `exec::check_access(...)` in `src/backend/cmd/workloads.rs`, next to the existing
      `StartPodExec`/`SendPodExecInput`/`ResizePodExec`/`StopPodExec` wiring
- [x] T022 [US2] In `ui/src/components/panels/PodTerminalTab.tsx`, render the `'denied'` phase:
      a permission-denied panel (reuse the existing error-panel styling/`ShieldWarningIcon` used
      today for `stream?.reason?.toLowerCase().includes('permission')`) showing the
      `PodExecAccessChecked.reason` verbatim when present, and a generic "You don't have
      permission to open a shell in this container" fallback when `reason` is absent (per
      contracts/protocol.md: `reason` may legitimately be `None`); no `ConfirmDialog` is ever
      shown in this phase; provide the same "Choose container" recovery affordance as the
      existing `error`/`ended` panels when the pod has multiple containers
- [x] T023 [US2] In `PodTerminalTab.tsx`, ensure a `CheckPodExecAccess` call that itself never
      resolves (e.g. the review call errors before emitting — should not happen per T019, but
      guard defensively) does not leave the UI stuck in `'checking'` forever; treat a
      `StreamClosed`-free timeout or an explicit error path the same as `'denied'` with a
      check-failed reason (spec.md Acceptance Scenario 2.3 / FR-006 "fail safe")
- [x] T024 [P] [US2] Playwright scenario in `ui/tests/e2e/pod-exec-gate.spec.ts`: open Terminal
      tab against the fake backend, inject `PodExecAccessChecked { allowed: false, reason: "..." }`,
      assert the permission-denied message renders (containing the injected reason) and that no
      `ConfirmDialog` and no `StartPodExec` ever appear in the fake backend's command log
      (quickstart.md Scenario 2); keep this and T018 to exactly two scenarios total in the file,
      per the task brief's "hyper-focused... not a full terminal-emulator conformance suite"

**Checkpoint**: Both user stories independently complete; full gate (pre-check + confirmation)
in place end to end.

---

## Phase 5: Polish & Cross-Cutting

- [x] T025 [P] Update `specs/002-pod-logs-terminal/spec.md`'s User Story 2 acceptance scenarios
      with a note (not a rewrite) pointing at this spec for the confirmation/RBAC-pre-check
      behavior now layered on top, so a future reader of spec 002 isn't confused by the
      no-longer-accurate "opens immediately" framing
- [x] T026 Run `cargo build --workspace --all-targets`, `cargo clippy --all-targets -- -D warnings`,
      `cargo test`, `pnpm -C ui typecheck`, `pnpm -C ui lint`, `pnpm -C ui test`, and the new
      `ui/tests/e2e/pod-exec-gate.spec.ts`; fix any failures

## Dependencies & Execution Order

- **Phase 1 (Setup)**: No dependencies.
- **Phase 2 (Foundational)**: Depends on Phase 1. **Blocks both user stories** (both need the
  `CheckPodExecAccess`/`PodExecAccessChecked` protocol variants and the `mutation_target()`
  reclassification).
- **User Story 1 (Phase 3)**: Depends on Phase 2. Independently testable/shippable on its own
  (confirmation gate alone, without the RBAC pre-check denying anything, is already the primary
  safety fix per spec.md's priority ordering).
- **User Story 2 (Phase 4)**: Depends on Phase 2. Builds on US1's phase state machine (T012's
  `'denied'` phase, T014's transition) but is additive — US1's confirmation flow works correctly
  whether or not US2 has landed (an `allowed: true`-only backend still satisfies US1's contract).
- **Polish (Phase 5)**: Depends on both user stories being complete.

## Parallel Execution Examples

- Phase 2: T003, T005, T008, T009, T010 can run in parallel (distinct files/sections) once T002/
  T004/T006/T007 land.
- Phase 3: T018 (Playwright) can be drafted in parallel with T012-T016 (component work), then
  finalized once the component lands.
- Phase 4: T020 and T024 are `[P]` against T019/T021-T023 respectively (different files).

## Implementation Strategy

**MVP = User Story 1 only** (the confirmation gate). It closes the primary safety gap (no
confirmation at all today) and is fully independently testable. User Story 2 (the proactive
RBAC pre-check) is valuable defense-in-depth layered on top and should ship immediately after,
but a build that only has US1 is already a correct, shippable improvement over today's
behavior — an unpermitted operator still gets blocked at attach time by the API server itself
(spec 002's existing FR-013 behavior), just without the earlier, friendlier pre-check.
