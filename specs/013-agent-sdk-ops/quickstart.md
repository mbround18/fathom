# Quickstart: Agent SDK Ops

## Prerequisites
- `cargo build` succeeds at repo root.
- `pnpm -C ui install` has been run.
- For a live (non-fake-backend) manual check: either an Anthropic API key, or a locally
  running Ollama (`ollama serve`, with a model pulled, e.g. `ollama pull llama3.1`).

## US1 — Grounded diagnosis (P1)
1. `pnpm -C ui e2e:serve` (fake backend) in one terminal.
2. `pnpm -C ui e2e -g "agent-ops"` (or run `ui/tests/e2e/us1-agent-ops-*.spec.ts`
   directly) — the fake backend fixture seeds a pod in `CrashLoopBackOff` with events
   and a log tail, asks the assistant panel a question, and asserts:
   - the response text references data present in the fixture (event reason, log line),
   - `data-testid="agent-ops-grounding-sources"` lists the sources used,
   - no `DeletePod`/`ScaleDeployment`/etc. command appears in the fake backend's
     received-command log for the duration of the test.

## US2 — Auth mode setup (P1)
1. Same fake-backend E2E harness. `us2-agent-ops-auth-setup.spec.ts` drives: open panel
   with no auth configured -> setup prompt shown, question input disabled -> enter a
   BYOK key -> fake backend acknowledges `AgentSetAuthMode` -> question succeeds ->
   switch to Ollama mode -> fake backend simulates an unreachable endpoint ->
   `agent-ops-error` shows a specific remediation message.

## US3 — Suggested action confirm/execute (P2)
1. `us3-agent-ops-confirm-suggestion.spec.ts`: fake backend scripts an assistant
   response containing one `SuggestedAction` (`restart`, target = the pod under test).
   Test asserts:
   - the suggestion renders as `data-testid="agent-ops-suggestion"`, distinct from
     message text, and no mutating command was sent when it appeared;
   - clicking `agent-ops-suggestion-review` opens `[data-testid="confirm-dialog"]`
     (the *existing* component, not a new one) listing the pod;
   - confirming sends the pre-existing `RestartDeployment`/pod-restart command exactly
     as a manual restart would;
   - cancelling sends no command and marks the suggestion declined.

## US4 — Conversation persistence (P3)
1. `us4-agent-ops-conversation-history.spec.ts`: ask a question, confirm an action,
   "restart the app" (re-mount the fake-backend harness), reopen the panel on the same
   pod, assert prior messages + the executed-action outcome are shown, and assert zero
   `AgentAsk` commands were sent automatically on reopen.

## Backend-only validation (no UI, no live LLM)
- `cargo test -p fathom agent::` runs the state-machine + protocol round-trip tests in
  `src/backend/agent/state_machine.rs` and `src/protocol/v1/agent.rs` against fixture
  data only.
- `cargo test -p fathom mutation::agent` asserts every `Agent*` command classifies as
  `ReadOnly` or `Local` (never cluster-writing) — the type-level proof of SC-002.
