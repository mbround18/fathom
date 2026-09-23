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

pub(crate) mod action_parse;
pub(crate) mod anthropic;
pub(crate) mod credentials;
pub(crate) mod grounding;
pub(crate) mod ollama;
pub(crate) mod persistence;
pub(crate) mod state_machine;

use crate::protocol::v1::AgentTurnAuthor;

pub(crate) use action_parse::ParsedAction;

/// Brief instructions appended to every provider's system prompt (Phase 3, US1) so the
/// model knows about the `fathom-action` fenced-block convention for proposing a
/// concrete corrective action. Parsing that convention out of the raw reply happens in
/// `src/backend/cmd/agent.rs::handle_ask` via `action_parse::parse_suggested_actions`.
pub(crate) const ACTION_PROPOSAL_INSTRUCTIONS: &str = "If you want to propose a concrete \
corrective action the operator could take, emit it as a fenced code block tagged \
`fathom-action` containing JSON {\"action_kind\":...,\"target\":{\"kind\":...,\"namespace\":...,\"name\":...},\"params\":{...},\"rationale\":...}. \
Only propose actions you are confident are safe and directly relevant. Never claim you \
have performed an action — you can only propose it for human review.";

/// One turn of conversation sent to a provider as context.
#[derive(Debug, Clone)]
pub(crate) struct Turn {
    pub author: AgentTurnAuthor,
    pub text: String,
}

/// A provider's reply to a turn. `suggested_actions` is populated only when the raw
/// `text` contains one or more parseable ```fathom-action fenced blocks (see
/// `action_parse`); an empty vec here is always valid and simply means the assistant's
/// reply was informational only. Providers themselves never populate this field —
/// `text` always carries the raw, unstripped reply; parsing happens centrally in
/// `src/backend/cmd/agent.rs::handle_ask`.
#[derive(Debug, Clone, Default)]
pub(crate) struct TurnResult {
    pub text: String,
    /// Reserved for a future provider that returns structured tool-call output
    /// directly rather than text-embedded fenced blocks; both current clients leave
    /// this empty and `handle_ask` parses `text` itself via `action_parse`.
    #[allow(dead_code)]
    pub suggested_actions: Vec<ParsedAction>,
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
    let nanos = k8s_openapi::chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_default();
    format!("{prefix}-{nanos}")
}
