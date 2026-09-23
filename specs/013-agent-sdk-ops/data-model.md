# Phase 1 Data Model: Agent SDK Ops

All types live in `src/protocol/v1/agent.rs`, derive `Serialize`/`Deserialize`/`TS`
(the existing `ts-rs` pattern), and are exported to `ui/src/protocol/generated/` the
same way every other protocol type is.

## ObjectRef

```rust
pub struct AgentObjectRef {
    pub kind: String,       // "Pod" for v1; extensible
    pub namespace: String,
    pub name: String,
}
```

## AgentAuthMode

```rust
pub enum AgentAuthMode {
    Unconfigured,
    Anthropic { key_configured: bool },      // never carries the raw key to the UI
    Ollama { endpoint: String, model: String },
}
```
The raw BYOK key never appears in this type; it lives only in `credentials.json` and in
the backend process's memory for the duration of a request.

## GroundingSource

```rust
pub enum GroundingSource {
    Events,
    Conditions,
    ContainerStatuses,
    Logs { truncated: bool },
    Lifecycle,
}
```

## AssistantMessage / turn content

```rust
pub enum AgentTurnAuthor { Operator, Assistant }

pub struct AgentMessage {
    pub id: String,               // uuid-v4 string
    pub author: AgentTurnAuthor,
    pub text: String,             // operator question, or assistant informational text
    pub grounding: Vec<GroundingSource>,   // empty for operator messages
    pub suggested_actions: Vec<SuggestedAction>, // usually empty
    pub created_at: String,       // RFC3339
}
```

## SuggestedAction (State: Suggested — inert)

```rust
pub struct SuggestedAction {
    pub id: String,                 // uuid-v4, stable for this proposal
    pub action_kind: String,        // "delete" | "restart" | "scale" | "helm-rollback" | ...
                                     // MUST be one of the app's existing ActionDescriptor ids
    pub target: AgentObjectRef,
    pub params: serde_json::Value,  // e.g. {"replicas": 3} for scale; opaque, action-specific
    pub rationale: String,          // assistant's stated reason, shown on the card
    pub status: SuggestedActionStatus,
}

pub enum SuggestedActionStatus {
    Proposed,   // shown, not reviewed
    Reviewed,   // confirm dialog opened at least once (may still be cancelled)
    Declined,   // operator cancelled/closed the dialog, or explicitly dismissed the card
    Superseded, // a newer suggestion for the same target replaced it
}
```
A `SuggestedAction` NEVER contains a diff-preview or affected-object snapshot — that is
computed fresh, client-side, by the existing `ActionDescriptor.plan()` at review-open
time (FR-009), reading current store state, not anything cached on this struct.

## ExecutedAction (State: Executed — the only thing that can mutate the cluster)

```rust
pub struct ExecutedAction {
    pub id: String,                  // uuid-v4, distinct from the SuggestedAction id
    pub suggested_action_id: String, // FK to the SuggestedAction it originated from
    pub command_summary: String,     // human-readable, e.g. "DeletePod default/foo"
    pub outcome: ActionOutcome,
    pub executed_at: String,
}

pub enum ActionOutcome {
    Success,
    Error { message: String },  // verbatim backend error, per Principle I
}
```
Constructing an `ExecutedAction` is only ever done by the backend handler that receives
the *result* of a command the operator explicitly confirmed and sent through the normal
`BackendCmdV1` dispatch (the same handler path as a manual action) — see contracts.md.
There is no function anywhere that takes a `SuggestedAction` and produces an
`ExecutedAction` without an intervening operator-confirmed command round-trip.

## AgentConversation (persisted, per-object)

```rust
pub struct AgentConversation {
    pub object: AgentObjectRef,
    pub messages: Vec<AgentMessage>,
    pub executed_actions: Vec<ExecutedAction>,
}
```
Persisted at `~/.fathom/agent_conversations.json` as `{ "<kind>/<namespace>/<name>": AgentConversation }`.
Loaded on `AgentOpenConversation`; never auto-replayed (FR-017 / US4 AS3).

## State Machine (the safety-critical piece — see backend/agent/state_machine.rs)

```
[no suggestion]
     |  assistant proposes
     v
Proposed  --operator opens review-->  Reviewed
   |  \                                  |
   |   \--operator dismisses card-->  Declined (terminal)
   |                                     |  operator cancels dialog
   |                                     v
   |                                  Declined (terminal)
   |                                     |  operator confirms dialog
   |                                     v
   |                          [backend command dispatched: normal path]
   |                                     |
   |                          command result arrives
   |                                     v
   |                       ExecutedAction created (Success | Error), linked by id
   v
 a NEW suggestion for the same target --> old one becomes Superseded (terminal)
```

Invariants enforced by unit tests (fixture-only, no cluster/LLM):
1. No transition function can construct an `ExecutedAction` from a `Proposed` or
   `Declined` `SuggestedAction` — only from `Reviewed`.
2. Opening review (`Proposed`/`Reviewed` -> `Reviewed`) never itself calls a mutating
   command — it is a pure state read.
3. `Declined` and `Superseded` are terminal: no transition function accepts them as
   input and produces anything other than a no-op/error.
4. Every `ExecutedAction` has a `suggested_action_id` that must resolve to a
   `SuggestedAction` that was `Reviewed` at the time the confirming command was sent —
   enforced by the backend command handler requiring the id to look up a `Reviewed`
   entry, erroring (not silently ignoring) otherwise.

## BackendCmdV1 / BackendEventV1 additions (see contracts/protocol-v1-agent.md)

New commands are classified in `mutation.rs`:
- `AgentGetAuthMode`, `AgentAsk`, `AgentOpenConversation`, `AgentClearConversation`,
  `AgentReviewSuggestedAction`, `AgentDeclineSuggestedAction`, `AgentRevealKey` →
  `MutationTarget::ReadOnly` (informational reads / local UI-state reads; `AgentAsk`
  calls an external model API but writes no cluster or persisted state other than
  appending to the in-memory conversation, matching how e.g. `TestPrometheusConnection`
  is classified `ReadOnly` today).
- `AgentSetAuthMode { mode }` → `MutationTarget::Local` (writes `credentials.json` /
  app-local state only, same class as `SetMcpAutoStart`).
- `AgentRecordExecutedAction { suggestion_id, command_summary, outcome }` →
  `MutationTarget::Local`. **No new mutation command is introduced.** Confirming a
  suggested action means the frontend calls the operator's *existing*, unmodified
  action dispatch for that resource — the exact same `DeletePod`/`ScaleDeployment`/
  `ApplyHelmRollback`/etc. command a manual click would send, through the exact same
  `runAction` path, with no agent-specific parameter or flag. There is no backend
  awareness that a given `DeletePod` came from a suggestion — this is what makes "no
  parallel or agent-specific mutation path exists" true by construction rather than by
  convention. Once that command's normal result event arrives, the frontend separately
  sends `AgentRecordExecutedAction` (a purely local, non-cluster-mutating command) so
  the conversation history can show the linked outcome. If this local record-keeping
  step is never reached (e.g. app crashes mid-flight), the cluster mutation itself is
  unaffected — only conversation history is out of sync, self-healing next time the
  object is refreshed.
