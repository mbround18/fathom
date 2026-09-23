# Tasks: Agent SDK Ops (AI-Assist Troubleshooting)

**Input**: plan.md, research.md, data-model.md, contracts/protocol-v1-agent.md, quickstart.md
**Tests**: explicitly requested (see spec/plan Testing section) — included throughout.

## Phase 1: Setup

- [ ] T001 Create `src/backend/agent/` module directory with `mod.rs`, `anthropic.rs`,
      `ollama.rs`, `grounding.rs`, `actions.rs`, `state_machine.rs` (empty stubs +
      `pub(crate)` re-exports) and register `mod agent;` in `src/backend.rs`.
- [ ] T002 Create `ui/src/features/agent-ops/` directory with `index.ts` barrel file.
- [ ] T003 [P] Add `agent-conversations` fixture helper stub to
      `ui/tests/e2e/fixtures/` (mirrors existing fake-backend fixture pattern) for
      US1-US4 specs to import.

## Phase 2: Foundational (blocking prerequisites)

- [ ] T004 Define protocol types in `src/protocol/v1/agent.rs`: `AgentObjectRef`,
      `AgentAuthMode`, `GroundingSource`, `AgentTurnAuthor`, `AgentMessage`,
      `SuggestedAction`, `SuggestedActionStatus` (`Proposed | Reviewed | Declined |
      Superseded`, exactly these four, per data-model.md), `ExecutedAction`,
      `ActionOutcome`, `AgentConversation`. All derive `Debug, Clone, PartialEq,
      Serialize, Deserialize, TS` per the existing `ts-rs` convention; register the
      module in `src/protocol/v1/mod.rs` and re-export from `src/protocol/mod.rs`.
- [ ] T005 Add round-trip serde tests for every type in T004 in
      `src/protocol/v1/agent.rs` (`#[cfg(test)] mod tests`), following the exact
      pattern of existing round-trip tests in `src/protocol/v1/cmd.rs`.
- [ ] T006 Add new `BackendCmdV1` variants per contracts/protocol-v1-agent.md
      (`AgentGetAuthMode`, `AgentSetAuthMode`, `AgentRevealKey`,
      `AgentOpenConversation`, `AgentAsk`, `AgentClearConversation`,
      `AgentReviewSuggestedAction`, `AgentDeclineSuggestedAction`,
      `AgentRecordExecutedAction`) in `src/protocol/v1/cmd.rs`.
- [ ] T007 Add new `BackendEventV1` variants (`AgentAuthModeUpdated`,
      `AgentKeyRevealed`, `AgentConversationUpdated`, `AgentMessageAppended`,
      `AgentError`, `AgentSuggestedActionStatusChanged`) in
      `src/protocol/v1/event.rs` / `events.rs`.
- [ ] T008 Extend `src/protocol/v1/mutation.rs`'s exhaustive match: every `Agent*`
      command from T006 classifies as `MutationTarget::ReadOnly` (the `AgentAsk`,
      `AgentGetAuthMode`, `AgentRevealKey`, `AgentOpenConversation`,
      `AgentReviewSuggestedAction` arms) or `MutationTarget::Local` (`AgentSetAuthMode`,
      `AgentClearConversation`, `AgentDeclineSuggestedAction`,
      `AgentRecordExecutedAction` arms) — never `Namespaced`/`ClusterScoped`.
- [ ] T009 Add a `mutation.rs` unit test asserting, for every `Agent*` command
      variant, `matches!(cmd.mutation_target(), MutationTarget::ReadOnly |
      MutationTarget::Local)` — the type-level proof backing SC-002.
- [ ] T010 Implement `AgentClient` trait in `src/backend/agent/mod.rs`:
      `async fn send_turn(&self, system: &str, messages: &[Turn]) -> Result<TurnResult>`
      (see research.md R1), with `TurnResult` carrying assistant text plus zero or more
      proposed tool calls that map to `SuggestedAction`.
- [ ] T011 [P] Implement `anthropic.rs`: `AnthropicClient` (implements `AgentClient`)
      calling `POST https://api.anthropic.com/v1/messages` via `reqwest`, using the
      BYOK key read from `credentials.json` at request time only (never cached beyond
      the request's lifetime in a long-lived struct field that could be logged).
- [ ] T012 [P] Implement `ollama.rs`: `OllamaClient` (implements `AgentClient`) calling
      `POST {endpoint}/api/chat` via `reqwest`.
- [ ] T013 Implement `grounding.rs`: `fn collect_pod_grounding(state: &BackendState,
      object: &AgentObjectRef) -> (String, Vec<GroundingSource>)` reading only from
      already-populated in-memory pod events/conditions/container-statuses/log-tail
      state (no new K8s API calls per research.md R3); truncates per Principle V ring
      buffers and returns `GroundingSource::Logs { truncated: true }` when truncation
      occurred (FR-018).
- [ ] T014 Implement credential storage: extend `src/backend/persistence.rs` (or add
      `src/backend/agent/credentials.rs`) with `load_agent_auth()`/`save_agent_auth()`
      reading/writing the `agent` key inside `credentials.json` via the existing
      `load_credentials()`/`save_credentials()` functions (research.md R4) — masked by
      default, key only ever returned in full in direct response to `AgentRevealKey`.
- [ ] T015 Add unit tests for T014: save then load round-trips the key; an unreadable/
      corrupt `credentials.json` falls back to `AgentAuthMode::Unconfigured` without
      deleting the file (Principle VI); assert the key never appears in any
      `tracing::*` call by grepping the module's log statements in the test (or by
      construction — no `tracing::` call in the module takes the key).
- [ ] T016 Implement the Suggested/Executed state machine in
      `src/backend/agent/state_machine.rs`: `fn review(suggestion: &mut
      SuggestedAction)` (`Proposed|Reviewed -> Reviewed`, no I/O), `fn decline(...)`
      (`-> Declined`, terminal), `fn supersede(...)` (`-> Superseded`, terminal), and a
      pure function proving an `ExecutedAction` can only be constructed from a
      `Reviewed` status (return `Result<ExecutedAction, StateError>`, erroring on
      `Proposed`/`Declined`/`Superseded` input).
- [ ] T017 Add fixture-only unit tests for T016 covering every transition in the
      data-model.md diagram, including: reviewing never returns any command/mutation
      value (only a status change), declining/superseded are terminal (attempting any
      transition from them returns `StateError`, never panics or silently succeeds),
      and constructing an `ExecutedAction` from anything but `Reviewed` errors.
- [ ] T018 Generate/refresh TypeScript bindings (`ts-rs`) so `ui/src/protocol/
      generated/` picks up all T004/T006/T007 types (run the existing generation
      command/test used by prior features, e.g. `cargo test export_bindings` or
      equivalent per repo convention) and commit the generated `.ts` files.

**Checkpoint**: protocol + state machine compile, `cargo test` green for Phase 2 — ready
for user stories.

## Phase 3: User Story 1 — Ask the assistant what's wrong with a pod (P1)

**Goal**: Read-only grounded diagnosis panel; zero mutating commands possible.
**Independent test**: quickstart.md US1.

- [ ] T019 [P] [US1] Implement backend command handlers for `AgentOpenConversation`
      and `AgentAsk` in `src/backend/agent/mod.rs`: on `AgentAsk`, build the system
      prompt (scope: "answer only using the provided grounding data; decline
      off-topic questions" per FR-014), call `grounding::collect_pod_grounding`,
      dispatch to the configured `AgentClient`, and emit `AgentMessageAppended`
      (batched, per contracts.md) then a final `AgentConversationUpdated`. On any
      client error, emit `AgentError` with the verbatim provider error message
      (Principle I) — never a generic string.
- [ ] T020 [US1] Wire `AgentGetAuthMode` handler (reads T014 storage, returns
      `AgentAuthMode` with `Unconfigured` default) — required so the panel can gate
      question submission even before US2's setup UI exists.
- [ ] T021 [P] [US1] Add `ui/src/features/agent-ops/AgentPanel.tsx`: renders message
      list (`data-testid="agent-ops-panel"`, `agent-ops-message`), a question input
      disabled when `AgentAuthMode` is `Unconfigured`
      (`data-testid="agent-ops-question-input"`), and a grounding-sources chip list per
      assistant message (`data-testid="agent-ops-grounding-sources"`) per FR-012.
- [ ] T022 [US1] Wire `AgentPanel` into the pod inspector (mount point identified in
      plan.md — add as an additional collapsible pane at medium+ width per Principle
      IV, and a sheet/modal trigger at phone width), sending `AgentOpenConversation`
      on mount and `AgentAsk` on submit.
- [ ] T023 [US1] Add zustand store slice `ui/src/store/agent.ts` (or extend the
      existing store per its current pattern) handling
      `AgentConversationUpdated`/`AgentMessageAppended`/`AgentError` events.
- [ ] T024 [P] [US1] Vitest component tests:
      `ui/src/features/agent-ops/AgentPanel.test.tsx` — question input disabled when
      unconfigured; renders grounding chips; renders `AgentError` verbatim (no generic
      fallback string).
- [ ] T025 [US1] Playwright E2E `ui/tests/e2e/us1-agent-ops-diagnosis.spec.ts` per
      quickstart.md US1: seeds a `CrashLoopBackOff` pod fixture, asks a question,
      asserts response cites fixture data, asserts grounding sources shown, and
      **asserts the fake backend's received-command log contains zero
      cluster-mutating commands** for the whole scenario.
- [ ] T026 [US1] Playwright E2E `ui/tests/e2e/us1-agent-ops-healthy-pod.spec.ts`:
      healthy pod fixture (all conditions `True`, no warning events) → assistant states
      pod appears healthy, proposes no `SuggestedAction`.

**Checkpoint**: US1 independently demoable — diagnosis works read-only end-to-end.

## Phase 4: User Story 2 — Operator configures auth mode (P1)

**Goal**: BYOK key entry or local Ollama selection gates and enables the assistant.
**Independent test**: quickstart.md US2.

- [ ] T027 [P] [US2] Implement `AgentSetAuthMode` and `AgentRevealKey` handlers in
      `src/backend/agent/mod.rs`, persisting via T014 and emitting
      `AgentAuthModeUpdated` / `AgentKeyRevealed`.
- [ ] T028 [US2] Extend `OllamaClient`/`AnthropicClient` (T011/T012) error paths to
      distinguish: missing/invalid key, unreachable endpoint, model not found — each
      surfaced as a distinct `AgentError` message (FR-015), not a generic failure.
- [ ] T029 [P] [US2] Add `ui/src/features/agent-ops/AuthSetup.tsx`: setup prompt with
      "Enter Anthropic API key" (`data-testid="agent-ops-byok-key-input"`,
      masked-by-default with an explicit "reveal" control) and "Use local Ollama"
      (`data-testid="agent-ops-ollama-endpoint-input"`,
      `data-testid="agent-ops-ollama-model-input"`) options, plus a disable toggle
      (`data-testid="agent-ops-disable-toggle"`, FR-013).
- [ ] T030 [US2] Wire `AuthSetup` into `AgentPanel` (T021) as the gating view when
      `AgentAuthMode::Unconfigured`.
- [ ] T031 [P] [US2] Vitest tests: key input masked by default and only shows plaintext
      after explicit reveal click; disable toggle prevents question submission.
- [ ] T032 [US2] Playwright E2E `ui/tests/e2e/us2-agent-ops-auth-setup.spec.ts` per
      quickstart.md US2 (all 5 acceptance scenarios): no-auth gate, BYOK success,
      Ollama-only-mode network isolation assertion (assert the fake backend records
      zero calls to any hosted-API mock while Ollama mode is active), unreachable
      Ollama error, disable-then-no-requests.

**Checkpoint**: US1+US2 together are the full "first working conversation with no
forced cloud dependency" slice per plan.md Summary.

## Phase 5: User Story 3 — Suggested action confirm/execute (P2)

**Goal**: Agent-proposed actions are inert until routed through the existing
confirmation dialog and existing mutation path.
**Independent test**: quickstart.md US3.

- [ ] T033 [US3] Extend `AgentAsk` handling (T019) so the model's tool-call output (per
      T010's `TurnResult`) can populate `AgentMessage.suggested_actions`, constructing
      `SuggestedAction { status: Proposed, .. }` — never anything else on creation.
- [ ] T034 [P] [US3] Implement `AgentReviewSuggestedAction` and
      `AgentDeclineSuggestedAction` handlers calling `state_machine::review`/`decline`
      (T016) and emitting `AgentSuggestedActionStatusChanged` — confirm neither handler
      calls any cluster-mutating command (grep-verifiable: neither function imports or
      calls anything from the `kube` client).
- [ ] T035 [P] [US3] Implement `AgentRecordExecutedAction` handler: requires the
      referenced `suggestion_id` to currently be `Reviewed` (else returns an error
      event, does not silently accept), then appends an `ExecutedAction` via T016's
      pure constructor and persists the updated `AgentConversation`.
- [ ] T036 [US3] Add `ui/src/features/agent-ops/SuggestedActionCard.tsx`
      (`data-testid="agent-ops-suggestion"`, `data-testid="agent-ops-suggestion-
      review"`): renders the proposal as a visually distinct card (not inline message
      text) naming `target`/`action_kind`/`rationale`; "Review" is disabled if
      `action_kind` has no matching `ActionDescriptor` for `target.kind` in
      `ui/src/resources/*.tsx` (contracts.md rule 1).
- [ ] T037 [US3] Implement the review adapter
      `ui/src/features/agent-ops/reviewSuggestedAction.ts`: on "Review", sends
      `AgentReviewSuggestedAction`, looks up the live row for `target` from the store,
      calls the matching `ActionDescriptor.plan([row])` fresh (contracts.md rule 2),
      and opens the existing `ConfirmDialog` (`ui/src/components/ConfirmDialog.tsx`)
      unmodified with that plan's items.
- [ ] T038 [US3] On `ConfirmDialog` confirm from T037, dispatch the plan's existing
      `BackendCmd` through the existing `runAction`/dispatch path unchanged (contracts.md
      rule 3); on cancel/close, send `AgentDeclineSuggestedAction`.
- [ ] T039 [US3] On the confirmed command's result event, call
      `AgentRecordExecutedAction` (contracts.md rule 4) with the verbatim
      success/error outcome (FR-011); render the outcome inline under the (now
      resolved) suggestion card.
- [ ] T040 [P] [US3] Vitest test: `reviewSuggestedAction.ts` never calls a mutating
      command directly — only `plan()` + dialog-open — asserted via a mock dispatch
      spy that records zero calls before confirm.
- [ ] T041 [US3] Playwright E2E `ui/tests/e2e/us3-agent-ops-confirm-suggestion.spec.ts`
      per quickstart.md US3, asserting: card distinct from text and zero mutating
      commands on appearance (AS1); "Review" opens the *existing*
      `[data-testid="confirm-dialog"]` populated fresh at click time, verified by
      mutating the fixture's pod state between suggestion-generation and review-click
      and asserting the dialog reflects the *new* state (AS2/FR-009); confirm executes
      through the identical command and reports outcome in-conversation (AS3);
      cancel leaves cluster state untouched and marks declined, no auto-retry (AS4).
- [ ] T042 [US3] Playwright E2E
      `ui/tests/e2e/us3-agent-ops-permission-denied.spec.ts`: fake backend scripts a
      permission-error result for the confirmed command; assert the same verbatim
      error surfaces as a manual attempt would (AS5), and the assistant does not retry
      or route around it.
- [ ] T043 [US3] Code-path audit task (SC-002/SC-003, spec User Story 3 AS6):
      grep/document in a short comment atop `src/backend/agent/mod.rs` that no
      function in `src/backend/agent/` calls a `kube::Api` write method, a
      cluster-mutating `BackendCmdV1` constructor, or contains any of
      `auto_apply`/`auto_confirm`/`bypass` — add a `cargo clippy`-visible `#![forbid]`-
      style module doc comment recording this invariant for future reviewers.

**Checkpoint**: US1+US2+US3 deliver the full confirmation-gated action loop — the
feature's core safety property is demonstrable end-to-end.

## Phase 6: User Story 4 — Conversation persistence (P3)

**Goal**: Conversations survive restart; never auto-replay.
**Independent test**: quickstart.md US4.

- [ ] T044 [US4] Implement `~/.fathom/agent_conversations.json` load/save in
      `src/backend/agent/mod.rs` (or `persistence.rs`), keyed by
      `"<kind>/<namespace>/<name>"`, following the existing `load_json_file`/
      `save_json_file` convention; loaded on `AgentOpenConversation`, written after
      every `AgentMessageAppended`/`AgentRecordExecutedAction`.
- [ ] T045 [US4] Implement `AgentClearConversation` handler: permanently removes the
      object's entry from the persisted file and emits an empty
      `AgentConversationUpdated`.
- [ ] T046 [P] [US4] Add `ui/src/features/agent-ops/AgentPanel.tsx` "Clear history"
      control (`data-testid="agent-ops-clear-history"`) with its own confirm step
      (reuse `ConfirmDialog` again, generic text-only variant already supported).
- [ ] T047 [US4] Playwright E2E
      `ui/tests/e2e/us4-agent-ops-conversation-history.spec.ts` per quickstart.md US4:
      conversation + one executed action survive a fake-backend remount; assert zero
      `AgentAsk` commands are sent automatically on reopen (FR-017/AS3); clearing
      removes history permanently (does not reappear on remount).

## Phase 7: Polish & Cross-Cutting

- [ ] T048 [P] Update `docs/MVP.md`'s Tier E / Agent SDK Ops status row to shipped,
      referencing 013-agent-sdk-ops (separate small doc commit per task brief).
- [ ] T049 [P] `cargo clippy --all-targets -- -D warnings` clean across
      `src/protocol/v1/agent.rs`, `src/backend/agent/**`, and touched files.
- [ ] T050 [P] `pnpm -C ui lint` and `pnpm -C ui typecheck` clean across
      `ui/src/features/agent-ops/**` and touched files.
- [ ] T051 Full verification pass: `cargo build`, `cargo test`, `pnpm -C ui test`
      (Vitest), full `pnpm -C ui e2e` suite (or at minimum the `agent-ops` specs) —
      all green before merge.

## Dependencies

- Phase 1 → Phase 2 → {Phase 3, Phase 4} (US1/US2 share Phase 2 foundation, can proceed
  in parallel by different implementers since they touch mostly disjoint files, but
  both are needed together for a usable MVP) → Phase 5 (US3 depends on US1's `AgentAsk`
  plumbing existing) → Phase 6 (US4 depends on US1's conversation shape) → Phase 7.

## Parallel Execution Examples

- Within Phase 2: T011 and T012 (Anthropic vs Ollama clients) in parallel; T005 in
  parallel with T006/T007 once T004 lands.
- Within Phase 3: T021 (panel UI) in parallel with T019 (backend handler) once T004/T006
  contracts are fixed; T024 (Vitest) in parallel with T025/T026 (Playwright).
- Phase 3 and Phase 4 backend handlers (T019 vs T027) touch the same
  `src/backend/agent/mod.rs` file — NOT parallel-safe; sequence T019 before T027 in
  practice despite both being "P1".

## Suggested MVP Scope

User Story 1 alone (Phase 1–3) is the smallest independently-shippable slice: read-only
diagnosis, zero mutation risk — but per plan.md, US2 (auth mode) must ship in the *same*
release slice as US1 since FR-003's "no forced cloud dependency" promise and the literal
fact that US1 cannot function without an auth mode configured (US1 depends on US2's
`AgentGetAuthMode`/`AgentSetAuthMode` plumbing existing, even if US2's full setup *UI*
polish lands slightly after). Treat Phases 3+4 as the true MVP pair.
