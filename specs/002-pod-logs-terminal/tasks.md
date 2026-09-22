---

description: "Task list for Pod Logs & Terminal"
---

# Tasks: Pod Logs & Terminal

**Input**: Design documents from `/specs/002-pod-logs-terminal/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/protocol.md, quickstart.md

**Tests**: Included per constitution Principle III ("Tested at the Boundaries", NON-NEGOTIABLE).

## Format: `[ID] [P?] [Story] Description`

## Phase 1: Setup

- [ ] T001 Add `xterm` and `@xterm/addon-fit` to `ui/package.json` dependencies; run `pnpm -C ui install`
- [ ] T002 [P] Create `ui/tests/promo/assets/` directory (new location for the staged Terminal screenshot, per research.md R6)

**Checkpoint**: Dependencies installed, ready for foundational backend/protocol work.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Shared protocol/state changes both Logs UI and Terminal depend on. MUST complete before either user story.

- [ ] T003 Move the `next_stream_id` counter off `LogStreamState` onto `BackendState::next_stream_id(&mut self) -> StreamId` in `src/backend/state.rs`, updating `src/backend/pods.rs::start_logs` to call it (research.md R2)
- [ ] T004 Add `ExecSessionState { attached: Option<AttachedProcess>, relay_task: Option<JoinHandle<()>>, stdin_writer: Option<Box<dyn AsyncWrite + Unpin + Send>>, resize_tx: Option<TerminalSizeSender> }` with a `.stop()` method to `src/backend/state.rs`, mirroring `LogStreamState` (data-model.md)
- [ ] T005 Add `StartPodExec { namespace: String, name: String, container: Option<String> }`, `SendPodExecInput { data: String }`, `ResizePodExec { cols: u16, rows: u16 }`, `StopPodExec` variants to `BackendCmdV1` in `src/protocol/v1/cmd.rs` (contracts/protocol.md)
- [ ] T006 Add `PodExecReady { stream_id: StreamId }` and `PodExecOutputChunk { stream_id: StreamId, data: String }` variants to `StreamEvent` in `src/protocol/v1/event.rs`; reuse existing `StreamClosed` for session end/error (contracts/protocol.md)
- [ ] T006a [P] Add serde round-trip tests for all 6 new variants (`StartPodExec`, `SendPodExecInput`, `ResizePodExec`, `StopPodExec`, `PodExecReady`, `PodExecOutputChunk`) in `src/protocol/v1/cmd.rs`/`event.rs`, per Constitution Principle III (NON-NEGOTIABLE: "every protocol command and event variant MUST have a serialization round-trip test")
- [ ] T007 [P] Classify the four new exec commands as non-mutating in `src/protocol/v1/mutation.rs` (Constitution Principle I gate — read/attach only, not a cluster-mutating action)
- [ ] T008 Regenerate ts-rs TypeScript bindings for the new cmd/event variants into `ui/src/protocol/generated/` (`cargo test` runs the ts-rs export, per existing repo convention)
- [ ] T009 [P] Add `ExecStreamState` (`kind: 'exec'`, `status: 'attaching' | 'active' | 'ended' | 'error'`, `reason?`, `container?`) as a variant of the stream entry type in `ui/src/store/index.ts` (data-model.md)

**Checkpoint**: Protocol and state scaffolding exist; both user stories can now proceed.

---

## Phase 3: User Story 1 - Tail a pod's logs without leaving the app (Priority: P1) 🎯 MVP

**Goal**: A working "Logs" tab in the Inspector that live-tails an already-existing backend log stream.

**Independent Test**: Select a running pod, open its Logs tab, see lines appear live — no dependency on User Story 2.

### Tests for User Story 1

- [ ] T010 [P] [US1] Vitest test in `ui/src/store/handlers/stream.test.ts` (or adjacent existing test file) asserting that switching the selected container restarts the log stream (new `stream_id`, old one's `StreamClosed` ignored/superseded)
- [ ] T011 [P] [US1] Playwright scenario in `ui/tests/live/` (or `ui/tests/e2e/`, matching existing convention) driving the fake backend: select a pod, open Logs tab, assert `StartPodLogs` is sent, assert injected `PodLogsChunk` events render as lines, assert `StopPodLogs` is sent when the tab closes

### Implementation for User Story 1

- [ ] T012 [US1] Add a "Logs" `TabsTrigger`/`TabsContent` to `ui/src/components/Inspector.tsx`, gated to render only when the inspected resource is a Pod (FR-001)
- [ ] T013 [US1] Create `ui/src/components/panels/PodLogsTab.tsx`: on mount (and on container change) send `StartPodLogs{namespace, name, container}`; on unmount send `StopPodLogs` (FR-002, FR-005)
- [ ] T014 [US1] In `PodLogsTab.tsx`, render a container picker when the pod has more than one container, defaulting to the pod's first container (FR-003)
- [ ] T015 [US1] In `PodLogsTab.tsx`, render `store.streams[stream_id]`'s `RingBuffer<string>` content with auto-scroll-to-bottom by default, stopping auto-scroll on manual scroll-up, with an explicit "jump to latest" control to resume (FR-004)
- [ ] T016 [US1] In `PodLogsTab.tsx`, render a distinct "stream ended" indicator when a `StreamClosed` event arrives, instead of leaving the pane looking merely idle (FR-006)

**Checkpoint**: User Story 1 fully functional and independently testable/demoable.

---

## Phase 4: User Story 2 - Open an interactive shell inside a running container (Priority: P2)

**Goal**: A working "Terminal" tab in the Inspector with a real xterm.js-rendered `kube exec` shell session.

**Independent Test**: Select a running pod, open its Terminal tab, type `ls`, see real container output — no dependency on User Story 1.

### Tests for User Story 2

- [ ] T017 [P] [US2] `cargo test` unit tests in `src/backend/exec.rs` for the exec-output relay and error-mapping logic (attach failure → mapped to a specific error reason string), mirroring `flux.rs`/`hubble.rs` test conventions; MUST include an explicit sub-case simulating a `Forbidden`/RBAC-denied API response mapped to a distinct "permission denied" reason string, not lumped into a generic "attach failed" message (spec Acceptance Scenario 2.6, FR-013)
- [ ] T018 [P] [US2] Playwright scenario in `ui/tests/live/` (or `ui/tests/e2e/`) driving the fake backend: open Terminal tab, assert `StartPodExec` sent, assert typed input triggers `SendPodExecInput`, assert injected `PodExecOutputChunk` events render in the terminal, assert opening a second session against a different pod sends `StopPodExec` for the first before `StartPodExec` for the second (FR-012 singleton)

### Implementation for User Story 2

- [ ] T019 [US2] Create `src/backend/exec.rs`: `start_exec(state, sender, namespace, name, container)` builds `AttachParams::interactive_tty()` (`.container(name)` if given), calls `Api::exec(name, ["/bin/bash"], &ap)`; on error, retries once with `Api::exec(name, ["/bin/sh"], &ap)`; if that also errors, surfaces it as the FR-013 "no usable shell" error (research.md R1). On success, extracts `stdin()`/`stdout()`/`terminal_size()`, stores them in `ExecSessionState`, spawns the stdout-relay task emitting `PodExecOutputChunk`/`StreamClosed`, emits `PodExecReady` once attached (research.md R1, R2)
- [ ] T020 [US2] In `src/backend/exec.rs`, implement `send_input`/`resize`/`stop` writing directly to the stored `stdin_writer`/`resize_tx` and calling `ExecSessionState::stop()` respectively (research.md R2)
- [ ] T021 [US2] Wire `StartPodExec`/`SendPodExecInput`/`ResizePodExec`/`StopPodExec` to `exec.rs` functions in `src/backend/cmd/workloads.rs`, with `StartPodExec` calling `ExecSessionState::stop()` on any existing session first (FR-012)
- [ ] T022 [US2] Base64-encode/decode `PodExecOutputChunk.data` and `SendPodExecInput.data` at the backend boundary (research.md R4)
- [ ] T023 [US2] [P] Add `PodExecReady`/`PodExecOutputChunk` handling to `ui/src/store/handlers/stream.ts`, updating the `kind: 'exec'` stream entry's `status`/`reason` (T009's type)
- [ ] T024 [US2] Add a "Terminal" `TabsTrigger`/`TabsContent` to `ui/src/components/Inspector.tsx`, gated to Pod resources in a running state only (FR-007)
- [ ] T025 [US2] Create `ui/src/components/panels/PodTerminalTab.tsx`: container picker when >1 container shown *before* the session starts (FR-008), instantiate `xterm.Terminal` with `{ scrollback: 1000 }` (research.md R5) + `FitAddon`, send `StartPodExec` on mount / `StopPodExec` on unmount
- [ ] T026 [US2] In `PodTerminalTab.tsx`, wire `Terminal.onData` to base64-encode and send `SendPodExecInput`, and wire `FitAddon`'s resize/`ResizeObserver` to send `ResizePodExec{cols, rows}` (FR-009, FR-010)
- [ ] T027 [US2] In `PodTerminalTab.tsx`, render distinct "attaching" / "active" / "ended" / "error" states (error state shows the `StreamClosed.reason` text verbatim for attach failures / RBAC denial) (FR-013)

**Checkpoint**: User Stories 1 AND 2 both independently functional.

---

## Phase 5: User Story 3 - See both capabilities in the promo brochure (Priority: P3)

**Goal**: The promo brochure showcases both Logs (live) and Terminal (staged) without ever driving a live shell against the demo cluster.

**Independent Test**: Regenerate the brochure; confirm a real live Logs screenshot and a Terminal showcase page, with zero cluster-affecting commands during the unattended capture run.

- [ ] T028 [US3] Add a `wide-logs` scene to `ui/tests/promo/capture.spec.ts`: open a pod's Logs tab against the live demo cluster, wait for real streamed lines, screenshot (FR-014)
- [ ] T029 [US3] Capture one staged Terminal screenshot by hand (interactively, against the existing `web` Deployment pod in `dev/sandbox/workloads.yaml` — `nginxinc/nginx-unprivileged:stable-alpine`, has `/bin/sh` — never the shared live demo cluster used for automated capture) and save it to `ui/tests/promo/assets/terminal-shot.png` (research.md R6, FR-015)
- [ ] T030 [US3] Add Logs (live shot) and Terminal (staged asset) entries to `SHOT_META`/pages in `ui/tests/promo/compose.spec.ts`, following the brochure's existing branded header/footer/watermark/page-numbering treatment (FR-016)
- [ ] T031 [US3] Run `FATHOM_LIVE_CONTEXT=<demo-context> pnpm -C ui promo`, confirm zero writes reported by the existing `WRITE_CMD`/`expectNoWrites` guard (unchanged), regenerate `promo-out/fathom-promo.pdf` (SC-004)

**Checkpoint**: All three user stories independently functional; brochure regenerated.

---

## Phase 6: Polish & Cross-Cutting Concerns

- [ ] T032 [P] `cargo clippy --all-targets -- -D warnings` and `cargo test` clean
- [ ] T033 [P] `pnpm -C ui typecheck` and `pnpm -C ui lint` clean
- [ ] T034 Run all `quickstart.md` validation scenarios end-to-end against a real cluster, confirming SC-001 through SC-004
- [ ] T035 Verify GPG commit signing is active (`git config commit.gpgsign`/`user.signingkey`, global + no local override) before committing, per standing repo rule

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies.
- **Foundational (Phase 2)**: Depends on Setup — BLOCKS both user stories (protocol/state shared by both).
- **User Story 1 (Phase 3)**: Depends on Foundational (T003, T009) only — independent of User Story 2.
- **User Story 2 (Phase 4)**: Depends on Foundational (T003–T009) only — independent of User Story 1.
- **User Story 3 (Phase 5)**: Depends on both User Story 1 (needs the Logs tab to exist for the live scene) and User Story 2 (needs the Terminal tab to exist to capture its staged screenshot).
- **Polish (Phase 6)**: Depends on all desired user stories being complete.

### Parallel Opportunities

- T002 (Setup) is parallel with T001.
- T007 and T009 (Foundational) are parallel with the rest of Phase 2's sequential protocol/state work.
- Once Phase 2 completes, **User Story 1 (Phase 3) and User Story 2 (Phase 4) can be built fully in parallel** by different developers — they touch disjoint files (`PodLogsTab.tsx` vs. `PodTerminalTab.tsx`/`exec.rs`) except for shared edits to `Inspector.tsx` (adding two separate tabs — low conflict risk).
- Within each story, [P]-marked test tasks can run in parallel with each other before implementation.

## Implementation Strategy

**MVP = User Story 1 only** (Phase 1 → 2 → 3, stop and validate). This ships real value (live log tailing) fastest since all backend plumbing already exists.

**Incremental delivery**: Setup + Foundational → US1 (MVP, demo) → US2 (Terminal, demo) → US3 (promo brochure, marketing-facing) → Polish.
