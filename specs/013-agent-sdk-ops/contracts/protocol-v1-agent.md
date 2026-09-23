# Protocol v1 Contract: Agent SDK Ops

New `BackendCmdV1` variants (`src/protocol/v1/cmd.rs`) and `BackendEventV1` variants
(`src/protocol/v1/event.rs` / `events.rs`). Types referenced here are defined in
`src/protocol/v1/agent.rs` (see data-model.md).

## Commands (UI -> backend)

| Command | Payload | `mutation_target()` |
|---|---|---|
| `AgentGetAuthMode` | `{}` | `ReadOnly` |
| `AgentSetAuthMode` | `{ mode: AgentAuthModeInput }` (`Anthropic{ key: String }` or `Ollama{ endpoint, model }` or `Disabled`) | `Local` |
| `AgentRevealKey` | `{}` | `ReadOnly` (returns the current BYOK key once, on explicit operator request, mirroring existing reveal-credential behavior) |
| `AgentOpenConversation` | `{ object: AgentObjectRef }` | `ReadOnly` |
| `AgentAsk` | `{ object: AgentObjectRef, question: String }` | `ReadOnly` |
| `AgentClearConversation` | `{ object: AgentObjectRef }` | `Local` |
| `AgentReviewSuggestedAction` | `{ object: AgentObjectRef, suggestion_id: String }` | `ReadOnly` (marks `Proposed -> Reviewed`; no cluster call) |
| `AgentDeclineSuggestedAction` | `{ object: AgentObjectRef, suggestion_id: String }` | `Local` |
| `AgentRecordExecutedAction` | `{ object: AgentObjectRef, suggestion_id: String, command_summary: String, outcome: ActionOutcome }` | `Local` |

`AgentAsk` never appears as a variant whose `mutation_target()` is `Namespaced` or
`ClusterScoped` — this is asserted by an exhaustive unit test (`mutation.rs` tests) that
every `Agent*` command is `ReadOnly` or `Local`, never a cluster-writing target,
guaranteeing SC-002 at the type level: **no `Agent*` command can write to the cluster.**
Cluster mutation only ever happens via the pre-existing `DeletePod`/`ScaleDeployment`/
`RestartDeployment`/`ApplyHelmRollback`/etc. commands, unmodified, dispatched by the
UI's existing action-runner after the operator confirms in the existing `ConfirmDialog`.

## Events (backend -> UI)

| Event | Payload |
|---|---|
| `AgentAuthModeUpdated` | `{ mode: AgentAuthMode }` |
| `AgentKeyRevealed` | `{ key: String }` (only ever sent in direct response to `AgentRevealKey`; never included in any other event, log line, or diagnostic export) |
| `AgentConversationUpdated` | `{ object: AgentObjectRef, conversation: AgentConversation }` |
| `AgentMessageAppended` | `{ object: AgentObjectRef, message: AgentMessage }` (streamed incrementally for a long assistant response — see below) |
| `AgentError` | `{ object: AgentObjectRef, message: String }` (verbatim provider/backend error, Principle I — missing key, unreachable Ollama endpoint, model not found, rate limit, etc.) |
| `AgentSuggestedActionStatusChanged` | `{ object: AgentObjectRef, suggestion_id: String, status: SuggestedActionStatus }` |

Streaming: rather than one event per model token (which would violate Principle V's
batching guidance), the backend batches streamed chunks and emits `AgentMessageAppended`
at most every ~80ms with the accumulated text so far for that message id, same batching
convention already used by `StartPodLogs`/log-tail streaming.

## Round-trip test requirement

Every new command and event variant gets a `serde_json` round-trip test in
`src/protocol/v1/agent.rs` (`#[cfg(test)]`), following the exact pattern already used for
existing variants in `cmd.rs`/`event.rs` — serialize, deserialize, assert equality.

## Confirmation-gate contract (Principle I, FR-007/008/009/010)

This is not a new UI component. The contract is:

1. `SuggestedAction.action_kind` MUST be a string that matches an existing
   `ActionDescriptor.id` already registered for `target.kind` in
   `ui/src/resources/*.tsx` (`"delete" | "restart" | "scale"`, plus Helm-specific ids
   for Helm releases). If the assistant proposes an `action_kind` with no matching
   descriptor for that kind, the UI renders the suggestion as informational-only (no
   "Review" button) — never invents a new confirmation path for it.
2. "Review" on a suggestion card calls that descriptor's existing `plan(rows)` with the
   row(s) freshly read from the live store (by `namespace/name` lookup at click time,
   never anything cached on the `SuggestedAction`), then opens the existing
   `ConfirmDialog` with that plan's `items` — byte-for-byte the same call a manual
   toolbar/row action makes.
3. Confirming sends the existing `BackendCmd` from that plan through the existing
   dispatch path — no new frontend mutation call is introduced for this purpose.
4. Only after that command's `CommandFinished`/error event returns does the UI call
   `AgentRecordExecutedAction` (Local-only) to persist the linked outcome.
