# Tasks: Agent SDK Ops (AI-Assist Troubleshooting)

**Input**: plan.md, research.md, data-model.md, contracts/protocol-v1-agent.md, quickstart.md
**Tests**: explicitly requested (see spec/plan Testing section) — included throughout.

## Phase 1: Setup

- [x] T001 Create `src/backend/agent/` module directory with `mod.rs`, `anthropic.rs`, _(note: actions.rs became action_parse.rs; also openai_compat.rs, credentials.rs, persistence.rs)_
      `ollama.rs`, `grounding.rs`, `actions.rs`, `state_machine.rs` (empty stubs +
      `pub(crate)` re-exports) and register `mod agent;` in `src/backend.rs`.
- [x] T002 Create `ui/src/features/agent-ops/` directory with `index.ts` barrel file. _(closed as superseded: the UI lives in `ui/src/components/panels/agent-ops/` and imports its modules directly, per the repo's panel convention; no `features/` tree or barrel exists anywhere in the UI)_
- [x] T003 [P] Add `agent-conversations` fixture helper stub to _(closed as superseded: there is no `ui/tests/e2e/fixtures/` dir; the agent-ops specs build `AgentConversationUpdated` batches inline with `ui/tests/fixtures/builders`, like every other e2e spec)_
      `ui/tests/e2e/fixtures/` (mirrors existing fake-backend fixture pattern) for
      US1-US4 specs to import.

## Phase 2: Foundational (blocking prerequisites)

- [x] T004 Define protocol types in `src/protocol/v1/agent.rs`: `AgentObjectRef`,
      `AgentAuthMode`, `GroundingSource`, `AgentTurnAuthor`, `AgentMessage`,
      `SuggestedAction`, `SuggestedActionStatus` (`Proposed | Reviewed | Declined |
      Superseded`, exactly these four, per data-model.md), `ExecutedAction`,
      `ActionOutcome`, `AgentConversation`. All derive `Debug, Clone, PartialEq,
      Serialize, Deserialize, TS` per the existing `ts-rs` convention; register the
      module in `src/protocol/v1/mod.rs` and re-export from `src/protocol/mod.rs`.
- [x] T005 Add round-trip serde tests for every type in T004 in
      `src/protocol/v1/agent.rs` (`#[cfg(test)] mod tests`), following the exact
      pattern of existing round-trip tests in `src/protocol/v1/cmd.rs`.
- [x] T006 Add new `BackendCmdV1` variants per contracts/protocol-v1-agent.md
      (`AgentGetAuthMode`, `AgentSetAuthMode`, `AgentRevealKey`,
      `AgentOpenConversation`, `AgentAsk`, `AgentClearConversation`,
      `AgentReviewSuggestedAction`, `AgentDeclineSuggestedAction`,
      `AgentRecordExecutedAction`) in `src/protocol/v1/cmd.rs`.
- [x] T007 Add new `BackendEventV1` variants (`AgentAuthModeUpdated`, _(closed as superseded: `AgentAuthModeUpdated`/`AgentKeyRevealed`/`AgentConversationUpdated` exist; appended messages and suggestion-status changes arrive as a whole `AgentConversationUpdated`, and provider errors surface verbatim as the assistant's reply text or a failed `CommandFinished`, so `AgentMessageAppended`/`AgentError`/`AgentSuggestedActionStatusChanged` are unnecessary)_
      `AgentKeyRevealed`, `AgentConversationUpdated`, `AgentMessageAppended`,
      `AgentError`, `AgentSuggestedActionStatusChanged`) in
      `src/protocol/v1/event.rs` / `events.rs`.
- [x] T008 Extend `src/protocol/v1/mutation.rs`'s exhaustive match: every `Agent*`
      command from T006 classifies as `MutationTarget::ReadOnly` (the `AgentAsk`,
      `AgentGetAuthMode`, `AgentRevealKey`, `AgentOpenConversation`,
      `AgentReviewSuggestedAction` arms) or `MutationTarget::Local` (`AgentSetAuthMode`,
      `AgentClearConversation`, `AgentDeclineSuggestedAction`,
      `AgentRecordExecutedAction` arms) — never `Namespaced`/`ClusterScoped`.
- [x] T009 Add a `mutation.rs` unit test asserting, for every `Agent*` command
      variant, `matches!(cmd.mutation_target(), MutationTarget::ReadOnly |
      MutationTarget::Local)` — the type-level proof backing SC-002.
- [x] T010 Implement `AgentClient` trait in `src/backend/agent/mod.rs`:
      `async fn send_turn(&self, system: &str, messages: &[Turn]) -> Result<TurnResult>`
      (see research.md R1), with `TurnResult` carrying assistant text plus zero or more
      proposed tool calls that map to `SuggestedAction`.
- [x] T011 [P] Implement `anthropic.rs`: `AnthropicClient` (implements `AgentClient`)
      calling `POST https://api.anthropic.com/v1/messages` via `reqwest`, using the
      BYOK key read from `credentials.json` at request time only (never cached beyond
      the request's lifetime in a long-lived struct field that could be logged).
- [x] T012 [P] Implement `ollama.rs`: `OllamaClient` (implements `AgentClient`) calling
      `POST {endpoint}/api/chat` via `reqwest`.
- [x] T013 Implement `grounding.rs`: `fn collect_pod_grounding(state: &BackendState,
      object: &AgentObjectRef) -> (String, Vec<GroundingSource>)` reading only from
      already-populated in-memory pod events/conditions/container-statuses/log-tail
      state (no new K8s API calls per research.md R3); truncates per Principle V ring
      buffers and returns `GroundingSource::Logs { truncated: true }` when truncation
      occurred (FR-018).
- [x] T014 Implement credential storage: extend `src/backend/persistence.rs` (or add _(note: key stored via keyring in credentials.rs, not credentials.json)_
      `src/backend/agent/credentials.rs`) with `load_agent_auth()`/`save_agent_auth()`
      reading/writing the `agent` key inside `credentials.json` via the existing
      `load_credentials()`/`save_credentials()` functions (research.md R4) — masked by
      default, key only ever returned in full in direct response to `AgentRevealKey`.
- [x] T015 Add unit tests for T014: save then load round-trips the key; an unreadable/
      corrupt `credentials.json` falls back to `AgentAuthMode::Unconfigured` without
      deleting the file (Principle VI); assert the key never appears in any
      `tracing::*` call by grepping the module's log statements in the test (or by
      construction — no `tracing::` call in the module takes the key).
- [x] T016 Implement the Suggested/Executed state machine in
      `src/backend/agent/state_machine.rs`: `fn review(suggestion: &mut
      SuggestedAction)` (`Proposed|Reviewed -> Reviewed`, no I/O), `fn decline(...)`
      (`-> Declined`, terminal), `fn supersede(...)` (`-> Superseded`, terminal), and a
      pure function proving an `ExecutedAction` can only be constructed from a
      `Reviewed` status (return `Result<ExecutedAction, StateError>`, erroring on
      `Proposed`/`Declined`/`Superseded` input).
- [x] T017 Add fixture-only unit tests for T016 covering every transition in the
      data-model.md diagram, including: reviewing never returns any command/mutation
      value (only a status change), declining/superseded are terminal (attempting any
      transition from them returns `StateError`, never panics or silently succeeds),
      and constructing an `ExecutedAction` from anything but `Reviewed` errors.
- [x] T018 Generate/refresh TypeScript bindings (`ts-rs`) so `ui/src/protocol/
      generated/` picks up all T004/T006/T007 types (run the existing generation
      command/test used by prior features, e.g. `cargo test export_bindings` or
      equivalent per repo convention) and commit the generated `.ts` files.

**Checkpoint**: protocol + state machine compile, `cargo test` green for Phase 2 — ready
for user stories.

## Phase 3: User Story 1 — Ask the assistant what's wrong with a pod (P1)

**Goal**: Read-only grounded diagnosis panel; zero mutating commands possible.
**Independent test**: quickstart.md US1.

- [x] T019 [P] [US1] Implement backend command handlers for `AgentOpenConversation` _(note: errors surface via command failure, no AgentError event)_
      and `AgentAsk` in `src/backend/agent/mod.rs`: on `AgentAsk`, build the system
      prompt (scope: "answer only using the provided grounding data; decline
      off-topic questions" per FR-014), call `grounding::collect_pod_grounding`,
      dispatch to the configured `AgentClient`, and emit `AgentMessageAppended`
      (batched, per contracts.md) then a final `AgentConversationUpdated`. On any
      client error, emit `AgentError` with the verbatim provider error message
      (Principle I) — never a generic string.
- [x] T020 [US1] Wire `AgentGetAuthMode` handler (reads T014 storage, returns
      `AgentAuthMode` with `Unconfigured` default) — required so the panel can gate
      question submission even before US2's setup UI exists.
- [x] T021 [P] [US1] Add `ui/src/features/agent-ops/AgentPanel.tsx`: renders message _(note: testids differ (e.g. agent-ops-submit, agent-ops-suggested-action-card))_
      list (`data-testid="agent-ops-panel"`, `agent-ops-message`), a question input
      disabled when `AgentAuthMode` is `Unconfigured`
      (`data-testid="agent-ops-question-input"`), and a grounding-sources chip list per
      assistant message (`data-testid="agent-ops-grounding-sources"`) per FR-012.
- [x] T022 [US1] Wire `AgentPanel` into the pod inspector (mount point identified in
      plan.md — add as an additional collapsible pane at medium+ width per Principle
      IV, and a sheet/modal trigger at phone width), sending `AgentOpenConversation`
      on mount and `AgentAsk` on submit.
- [x] T023 [US1] Add zustand store slice `ui/src/store/agent.ts` (or extend the
      existing store per its current pattern) handling
      `AgentConversationUpdated`/`AgentMessageAppended`/`AgentError` events.
- [x] T024 [P] [US1] Vitest component tests:
      `ui/src/features/agent-ops/AgentPanel.test.tsx` — question input disabled when
      unconfigured; renders grounding chips; renders `AgentError` verbatim (no generic
      fallback string).
- [x] T025 [US1] Playwright E2E `ui/tests/e2e/us1-agent-ops-diagnosis.spec.ts` per _(done: us1-agent-ops-diagnosis.spec.ts, a CrashLoopBackOff PodDetails fixture, grounding chips asserted, zero cluster-mutating commands via CLUSTER_MUTATING_COMMANDS mirroring mutation.rs. It surfaced and fixed a real gap: AgentPanel rendered no grounding sources (FR-012/FR-018))_
      quickstart.md US1: seeds a `CrashLoopBackOff` pod fixture, asks a question,
      asserts response cites fixture data, asserts grounding sources shown, and
      **asserts the fake backend's received-command log contains zero
      cluster-mutating commands** for the whole scenario.
- [x] T026 [US1] Playwright E2E `ui/tests/e2e/us1-agent-ops-healthy-pod.spec.ts`: _(done: us1-agent-ops-healthy-pod.spec.ts)_
      healthy pod fixture (all conditions `True`, no warning events) → assistant states
      pod appears healthy, proposes no `SuggestedAction`.

**Checkpoint**: US1 independently demoable — diagnosis works read-only end-to-end.

## Phase 4: User Story 2 — Operator configures auth mode (P1)

**Goal**: BYOK key entry or local Ollama selection gates and enables the assistant.
**Independent test**: quickstart.md US2.

- [x] T027 [P] [US2] Implement `AgentSetAuthMode` and `AgentRevealKey` handlers in
      `src/backend/agent/mod.rs`, persisting via T014 and emitting
      `AgentAuthModeUpdated` / `AgentKeyRevealed`.
- [x] T028 [US2] Extend `OllamaClient`/`AnthropicClient` (T011/T012) error paths to _(done: Anthropic missing key (refused before any request), 401 (typed AuthRejected, then NeedsReauth) and 404 model-not-found each yield a distinct message; tests in agent/anthropic.rs and ai/tests.rs. Ollama unreachable/404 were already covered in agent/ollama.rs. Errors surface as the reply text or a failed command, since AgentError was superseded (T007))_
      distinguish: missing/invalid key, unreachable endpoint, model not found — each
      surfaced as a distinct `AgentError` message (FR-015), not a generic failure.
- [x] T029 [P] [US2] Add `ui/src/features/agent-ops/AuthSetup.tsx`: setup prompt with
      "Enter Anthropic API key" (`data-testid="agent-ops-byok-key-input"`,
      masked-by-default with an explicit "reveal" control) and "Use local Ollama"
      (`data-testid="agent-ops-ollama-endpoint-input"`,
      `data-testid="agent-ops-ollama-model-input"`) options, plus a disable toggle
      (`data-testid="agent-ops-disable-toggle"`, FR-013).
- [x] T030 [US2] Wire `AuthSetup` into `AgentPanel` (T021) as the gating view when
      `AgentAuthMode::Unconfigured`.
- [x] T031 [P] [US2] Vitest tests: key input masked by default and only shows plaintext _(done: authSetup.test.tsx masks by default; agentPanel.test.tsx covers the disable toggle and the unconfigured gate. The gate itself was missing (FR-003/FR-013) and is now implemented in AgentPanel)_
      after explicit reveal click; disable toggle prevents question submission.
- [x] T032 [US2] Playwright E2E `ui/tests/e2e/us2-agent-ops-auth-setup.spec.ts` per _(done: us2-agent-ops-auth-setup.spec.ts covers all 5 scenarios. Ollama isolation is asserted at the command boundary (no hosted-provider commands); the fake has no hosted-API mock, and network isolation is covered by the Rust tests. All four new agent-ops specs also pass on WebKit (webkit-phone and webkit-laptop, 16/16, 2026-09-26) after libevent-2.1-7t64 was installed)_
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

- [x] T033 [US3] Extend `AgentAsk` handling (T019) so the model's tool-call output (per
      T010's `TurnResult`) can populate `AgentMessage.suggested_actions`, constructing
      `SuggestedAction { status: Proposed, .. }` — never anything else on creation.
- [x] T034 [P] [US3] Implement `AgentReviewSuggestedAction` and
      `AgentDeclineSuggestedAction` handlers calling `state_machine::review`/`decline`
      (T016) and emitting `AgentSuggestedActionStatusChanged` — confirm neither handler
      calls any cluster-mutating command (grep-verifiable: neither function imports or
      calls anything from the `kube` client).
- [x] T035 [P] [US3] Implement `AgentRecordExecutedAction` handler: requires the
      referenced `suggestion_id` to currently be `Reviewed` (else returns an error
      event, does not silently accept), then appends an `ExecutedAction` via T016's
      pure constructor and persists the updated `AgentConversation`.
- [x] T036 [US3] Add `ui/src/features/agent-ops/SuggestedActionCard.tsx`
      (`data-testid="agent-ops-suggestion"`, `data-testid="agent-ops-suggestion-
      review"`): renders the proposal as a visually distinct card (not inline message
      text) naming `target`/`action_kind`/`rationale`; "Review" is disabled if
      `action_kind` has no matching `ActionDescriptor` for `target.kind` in
      `ui/src/resources/*.tsx` (contracts.md rule 1).
- [x] T037 [US3] Implement the review adapter
      `ui/src/features/agent-ops/reviewSuggestedAction.ts`: on "Review", sends
      `AgentReviewSuggestedAction`, looks up the live row for `target` from the store,
      calls the matching `ActionDescriptor.plan([row])` fresh (contracts.md rule 2),
      and opens the existing `ConfirmDialog` (`ui/src/components/ConfirmDialog.tsx`)
      unmodified with that plan's items.
- [x] T038 [US3] On `ConfirmDialog` confirm from T037, dispatch the plan's existing
      `BackendCmd` through the existing `runAction`/dispatch path unchanged (contracts.md
      rule 3); on cancel/close, send `AgentDeclineSuggestedAction`.
- [x] T039 [US3] On the confirmed command's result event, call
      `AgentRecordExecutedAction` (contracts.md rule 4) with the verbatim
      success/error outcome (FR-011); render the outcome inline under the (now
      resolved) suggestion card.
- [x] T040 [P] [US3] Vitest test: `reviewSuggestedAction.ts` never calls a mutating
      command directly — only `plan()` + dialog-open — asserted via a mock dispatch
      spy that records zero calls before confirm.
- [x] T041 [US3] Playwright E2E `ui/tests/e2e/us3-agent-ops-confirm-suggestion.spec.ts` _(note: spec is us3-agent-ops-confirm.spec.ts; fresh-state check covered in unit test)_
      per quickstart.md US3, asserting: card distinct from text and zero mutating
      commands on appearance (AS1); "Review" opens the *existing*
      `[data-testid="confirm-dialog"]` populated fresh at click time, verified by
      mutating the fixture's pod state between suggestion-generation and review-click
      and asserting the dialog reflects the *new* state (AS2/FR-009); confirm executes
      through the identical command and reports outcome in-conversation (AS3);
      cancel leaves cluster state untouched and marks declined, no auto-retry (AS4).
- [x] T042 [US3] Playwright E2E _(done: us3-agent-ops-permission-denied.spec.ts shows the verbatim toast, AgentRecordExecutedAction{Error}, and one ScaleDeployment only)_
      `ui/tests/e2e/us3-agent-ops-permission-denied.spec.ts`: fake backend scripts a
      permission-error result for the confirmed command; assert the same verbatim
      error surfaces as a manual attempt would (AS5), and the assistant does not retry
      or route around it.
- [x] T043 [US3] Code-path audit task (SC-002/SC-003, spec User Story 3 AS6):
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

- [x] T044 [US4] Implement `~/.fathom/agent_conversations.json` load/save in
      `src/backend/agent/mod.rs` (or `persistence.rs`), keyed by
      `"<kind>/<namespace>/<name>"`, following the existing `load_json_file`/
      `save_json_file` convention; loaded on `AgentOpenConversation`, written after
      every `AgentMessageAppended`/`AgentRecordExecutedAction`.
- [x] T045 [US4] Implement `AgentClearConversation` handler: permanently removes the
      object's entry from the persisted file and emits an empty
      `AgentConversationUpdated`.
- [x] T046 [P] [US4] Add `ui/src/features/agent-ops/AgentPanel.tsx` "Clear history"
      control (`data-testid="agent-ops-clear-history"`) with its own confirm step
      (reuse `ConfirmDialog` again, generic text-only variant already supported).
- [x] T047 [US4] Playwright E2E
      `ui/tests/e2e/us4-agent-ops-conversation-history.spec.ts` per quickstart.md US4:
      conversation + one executed action survive a fake-backend remount; assert zero
      `AgentAsk` commands are sent automatically on reopen (FR-017/AS3); clearing
      removes history permanently (does not reappear on remount).

## Phase 7: Polish & Cross-Cutting

- [x] T048 [P] Update `docs/MVP.md`'s Tier E / Agent SDK Ops status row to shipped,
      referencing 013-agent-sdk-ops (separate small doc commit per task brief).
- [x] T049 [P] `cargo clippy --all-targets -- -D warnings` clean across _(done: cargo fmt/clippy clean; cargo test --workspace 927 passed, 0 failed; pnpm typecheck clean, lint 0 errors (27 pre-existing warnings), Vitest 1288/1288; Chromium e2e 2433 passed, 2 failed (a node-actions phone/tablet request-id race, since fixed and 75/75 green on re-run; perf-10k keystroke budget missed at load avg ~70), 190 skipped)_
      `src/protocol/v1/agent.rs`, `src/backend/agent/**`, and touched files.
- [x] T050 [P] `pnpm -C ui lint` and `pnpm -C ui typecheck` clean across _(done: cargo fmt/clippy clean; cargo test --workspace 927 passed, 0 failed; pnpm typecheck clean, lint 0 errors (27 pre-existing warnings), Vitest 1288/1288; Chromium e2e 2433 passed, 2 failed (a node-actions phone/tablet request-id race, since fixed and 75/75 green on re-run; perf-10k keystroke budget missed at load avg ~70), 190 skipped)_
      `ui/src/features/agent-ops/**` and touched files.
- [x] T051 Full verification pass: `cargo build`, `cargo test`, `pnpm -C ui test` _(done: cargo fmt/clippy clean; cargo test --workspace 927 passed, 0 failed; pnpm typecheck clean, lint 0 errors (27 pre-existing warnings), Vitest 1288/1288; Chromium e2e 2433 passed, 2 failed (a node-actions phone/tablet request-id race, since fixed and 75/75 green on re-run; perf-10k keystroke budget missed at load avg ~70), 190 skipped)_
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
