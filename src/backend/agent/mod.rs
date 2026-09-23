//! 013-agent-sdk-ops: the in-app AI-assist troubleshooting agent.
//!
//! # Safety invariant (Principle I / SC-002 / SC-003)
//!
//! No function anywhere under `src/backend/agent/` ever calls a `kube::Api` write
//! method (`create`/`patch`/`replace`/`delete`/…) or constructs a cluster-mutating
//! `BackendCmdV1` variant, and none of them contain any `auto_apply`/`auto_confirm`/
//! `bypass` shortcut. A `SuggestedAction` produced here is inert data: the *only* way
//! a cluster is ever mutated as a result of an agent conversation is for the frontend
//! to route the operator's confirmation through the exact same pre-existing
//! `ActionDescriptor`/`ConfirmDialog`/mutating-`BackendCmd` path a manual click uses
//! (see `ui/src/features/agent-ops/reviewSuggestedAction.ts`), recomputing the diff
//! fresh at confirm time. `AgentRecordExecutedAction` is purely local bookkeeping,
//! called only *after* that unmodified command has already run — see
//! `src/protocol/v1/mutation.rs`'s `agent_commands_never_write_cluster` test, which
//! proves this at the type level for every `Agent*` command.

pub(crate) mod anthropic;
pub(crate) mod credentials;
pub(crate) mod grounding;
pub(crate) mod ollama;
pub(crate) mod state_machine;

use crate::protocol::v1::AgentTurnAuthor;

/// One turn of conversation sent to a provider as context.
#[derive(Debug, Clone)]
pub(crate) struct Turn {
    pub author: AgentTurnAuthor,
    pub text: String,
}

/// A provider's reply to a turn. `suggested_actions` is populated only when the
/// provider's response can be parsed into one or more structured proposals; wiring
/// that parsing is Phase 5 (US3) work — an empty vec here is always valid and simply
/// means the assistant's reply was informational only.
#[derive(Debug, Clone, Default)]
pub(crate) struct TurnResult {
    pub text: String,
}

/// Abstraction over the two supported providers (research.md R1). Deliberately thin:
/// no crate-level "Claude Agent SDK" exists for Rust (only TS/Python), so both
/// implementations call their provider's plain HTTP API directly via `reqwest`
/// (already a workspace dependency) rather than pulling in a new SDK crate.
#[async_trait::async_trait]
pub(crate) trait AgentClient: Send + Sync {
    async fn send_turn(&self, system: &str, messages: &[Turn]) -> anyhow::Result<TurnResult>;
}

/// Generates a locally-unique id for messages/suggestions/executed actions. No `uuid`
/// crate dependency exists in this workspace; a monotonic-enough id is sufficient
/// since these ids are only ever compared within one persisted conversation.
pub(crate) fn next_local_id(prefix: &str) -> String {
    let nanos = k8s_openapi::chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    format!("{prefix}-{nanos}")
}
