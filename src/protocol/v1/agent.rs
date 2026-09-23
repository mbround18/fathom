//! Protocol types for 013-agent-sdk-ops: the in-app troubleshooting assistant.
//!
//! `SuggestedAction` and `ExecutedAction` are deliberately distinct types (FR-006): a
//! `SuggestedAction` is inert data the assistant proposed and MUST NOT be serialized or
//! treated identically to an `ExecutedAction`, which only ever exists once the operator
//! has confirmed the action through the app's ordinary confirmation dialog and the
//! resulting command has actually run. See `specs/013-agent-sdk-ops/data-model.md`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Identifies the object an assistant conversation is scoped to. `kind` is a plain
/// string (mirrors the rest of the protocol's `kind: String` convention, e.g.
/// `RelatedResourceRef`) rather than an enum, so extending the assistant to a new
/// object kind never requires a protocol-breaking enum change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct AgentObjectRef {
    pub kind: String,
    pub namespace: String,
    pub name: String,
}

/// The operator's chosen assistant auth mode (FR-003). Never carries the raw BYOK key —
/// only `AgentRevealKey`'s reply does, and only in direct response to that explicit
/// operator action (FR-004, Principle VI).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum AgentAuthMode {
    /// No auth mode chosen yet. The assistant is unusable (FR-003).
    Unconfigured,
    /// A self-provided Anthropic API key is stored. `key_configured` is always `true`
    /// when this variant is sent — kept as an explicit field (rather than a bare unit
    /// variant) so a future per-key metadata addition (e.g. a masked suffix) is
    /// additive, not a breaking enum change.
    Anthropic { key_configured: bool },
    /// A local Ollama endpoint/model. Selecting this mode means zero calls to any
    /// hosted LLM API are made while it is active (FR-003, SC-007).
    Ollama { endpoint: String, model: String },
    /// The operator explicitly disabled the assistant (FR-013). No assistant request
    /// is made in this state.
    Disabled,
}

/// Which already-streamed data sources grounded a given assistant answer (FR-012).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum GroundingSource {
    Events,
    Conditions,
    ContainerStatuses,
    /// `truncated` is `true` when the bounded log ring buffer (Principle V) meant not
    /// all available log lines were sent to the model (FR-018).
    Logs {
        truncated: bool,
    },
    Lifecycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum AgentTurnAuthor {
    Operator,
    Assistant,
}

/// A structured proposal the assistant made — inert until reviewed (FR-006, FR-007).
///
/// `action_kind` MUST match an existing `ActionDescriptor.id` already registered for
/// `target.kind` in the UI (`"delete" | "restart" | "scale" | ...`, contracts.md rule
/// 1); the backend does not validate this beyond string equality; the frontend treats
/// an unrecognized `action_kind` as informational-only (no review path is invented for
/// it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct SuggestedAction {
    pub id: String,
    pub action_kind: String,
    pub target: AgentObjectRef,
    /// Action-specific parameters (e.g. `{"replicas": 3}` for `scale`). Opaque to the
    /// backend state machine; interpreted only by the frontend's existing
    /// `ActionDescriptor.plan()` for `action_kind`.
    #[ts(type = "unknown")]
    pub params: serde_json::Value,
    pub rationale: String,
    pub status: SuggestedActionStatus,
}

/// See `specs/013-agent-sdk-ops/data-model.md`'s state-machine diagram. `Declined` and
/// `Superseded` are terminal — no transition function in
/// `src/backend/agent/state_machine.rs` accepts them as input and produces anything
/// other than a no-op/error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum SuggestedActionStatus {
    /// Shown to the operator; not yet reviewed.
    Proposed,
    /// The operator opened the confirmation dialog for this suggestion at least once.
    /// Opening review never itself mutates anything (FR-007) — it is a pure status
    /// change plus a client-side, fresh `plan()` read.
    Reviewed,
    /// The operator cancelled/closed the confirmation dialog, or explicitly dismissed
    /// the suggestion card. Terminal: the assistant may be asked again and propose a
    /// new suggestion, but this one is never auto-retried (FR-007, US3 AS4).
    Declined,
    /// A newer suggestion for the same target replaced this one. Terminal.
    Superseded,
}

/// The verbatim outcome of a confirmed, executed action (Principle I: backend errors
/// reach the operator verbatim, never replaced with a generic message).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum ActionOutcome {
    Success,
    Error { message: String },
}

/// The record of a `SuggestedAction` the operator reviewed and confirmed, and the
/// backend command that actually ran as a result (FR-010, FR-011). Constructing one
/// from anything but a `Reviewed` `SuggestedAction` is a programming error caught by
/// `src/backend/agent/state_machine.rs`'s tests, not something this type itself can
/// prevent structurally (Rust has no dependent types) — see data-model.md invariant 4.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ExecutedAction {
    pub id: String,
    pub suggested_action_id: String,
    pub command_summary: String,
    pub outcome: ActionOutcome,
    pub executed_at: String,
}

/// One turn in a conversation: an operator question, or assistant text with zero or
/// more suggested actions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct AgentMessage {
    pub id: String,
    pub author: AgentTurnAuthor,
    pub text: String,
    /// Empty for operator messages; for assistant messages, the data sources that
    /// grounded this specific answer (FR-012).
    pub grounding: Vec<GroundingSource>,
    pub suggested_actions: Vec<SuggestedAction>,
    pub created_at: String,
}

/// A per-object conversation, persisted locally (FR-017). See
/// `~/.fathom/agent_conversations.json`, keyed by `"<kind>/<namespace>/<name>"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Default)]
pub struct AgentConversation {
    pub messages: Vec<AgentMessage>,
    pub executed_actions: Vec<ExecutedAction>,
}

/// Input to `BackendCmdV1::AgentSetAuthMode` — distinct from `AgentAuthMode` because it
/// carries the raw key on the way *in* (never echoed back except via `AgentRevealKey`),
/// whereas `AgentAuthMode` is what gets sent back out to every other listener.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum AgentAuthModeInput {
    Anthropic { key: String },
    Ollama { endpoint: String, model: String },
    Disabled,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip<T>(value: &T)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(value).expect("serialize");
        let back: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(&back, value);
    }

    #[test]
    fn agent_object_ref_roundtrip() {
        roundtrip(&AgentObjectRef {
            kind: "Pod".to_owned(),
            namespace: "default".to_owned(),
            name: "api-0".to_owned(),
        });
    }

    #[test]
    fn agent_auth_mode_roundtrip() {
        roundtrip(&AgentAuthMode::Unconfigured);
        roundtrip(&AgentAuthMode::Anthropic {
            key_configured: true,
        });
        roundtrip(&AgentAuthMode::Ollama {
            endpoint: "http://localhost:11434".to_owned(),
            model: "llama3.1".to_owned(),
        });
        roundtrip(&AgentAuthMode::Disabled);
    }

    #[test]
    fn agent_auth_mode_input_roundtrip() {
        roundtrip(&AgentAuthModeInput::Anthropic {
            key: "sk-ant-test".to_owned(),
        });
        roundtrip(&AgentAuthModeInput::Ollama {
            endpoint: "http://localhost:11434".to_owned(),
            model: "llama3.1".to_owned(),
        });
        roundtrip(&AgentAuthModeInput::Disabled);
    }

    #[test]
    fn grounding_source_roundtrip() {
        roundtrip(&GroundingSource::Events);
        roundtrip(&GroundingSource::Conditions);
        roundtrip(&GroundingSource::ContainerStatuses);
        roundtrip(&GroundingSource::Logs { truncated: true });
        roundtrip(&GroundingSource::Logs { truncated: false });
        roundtrip(&GroundingSource::Lifecycle);
    }

    fn sample_target() -> AgentObjectRef {
        AgentObjectRef {
            kind: "Pod".to_owned(),
            namespace: "default".to_owned(),
            name: "api-0".to_owned(),
        }
    }

    fn sample_suggested_action(status: SuggestedActionStatus) -> SuggestedAction {
        SuggestedAction {
            id: "sugg-1".to_owned(),
            action_kind: "restart".to_owned(),
            target: sample_target(),
            params: serde_json::json!({}),
            rationale: "Pod is crash-looping on OOMKilled".to_owned(),
            status,
        }
    }

    #[test]
    fn suggested_action_status_roundtrip() {
        for status in [
            SuggestedActionStatus::Proposed,
            SuggestedActionStatus::Reviewed,
            SuggestedActionStatus::Declined,
            SuggestedActionStatus::Superseded,
        ] {
            roundtrip(&sample_suggested_action(status));
        }
    }

    #[test]
    fn action_outcome_roundtrip() {
        roundtrip(&ActionOutcome::Success);
        roundtrip(&ActionOutcome::Error {
            message: "pods \"api-0\" not found".to_owned(),
        });
    }

    #[test]
    fn executed_action_roundtrip() {
        roundtrip(&ExecutedAction {
            id: "exec-1".to_owned(),
            suggested_action_id: "sugg-1".to_owned(),
            command_summary: "RestartDeployment default/api".to_owned(),
            outcome: ActionOutcome::Success,
            executed_at: "2026-09-22T00:00:00Z".to_owned(),
        });
    }

    #[test]
    fn agent_message_roundtrip() {
        roundtrip(&AgentMessage {
            id: "msg-1".to_owned(),
            author: AgentTurnAuthor::Assistant,
            text: "This pod is crash-looping due to an OOMKilled event.".to_owned(),
            grounding: vec![
                GroundingSource::Events,
                GroundingSource::Logs { truncated: false },
            ],
            suggested_actions: vec![sample_suggested_action(SuggestedActionStatus::Proposed)],
            created_at: "2026-09-22T00:00:00Z".to_owned(),
        });
    }

    #[test]
    fn agent_conversation_roundtrip() {
        roundtrip(&AgentConversation {
            messages: vec![AgentMessage {
                id: "msg-1".to_owned(),
                author: AgentTurnAuthor::Operator,
                text: "What's wrong with this pod?".to_owned(),
                grounding: vec![],
                suggested_actions: vec![],
                created_at: "2026-09-22T00:00:00Z".to_owned(),
            }],
            executed_actions: vec![ExecutedAction {
                id: "exec-1".to_owned(),
                suggested_action_id: "sugg-1".to_owned(),
                command_summary: "RestartDeployment default/api".to_owned(),
                outcome: ActionOutcome::Success,
                executed_at: "2026-09-22T00:00:00Z".to_owned(),
            }],
        });
    }

    #[test]
    fn agent_conversation_default_is_empty() {
        let empty = AgentConversation::default();
        assert!(empty.messages.is_empty());
        assert!(empty.executed_actions.is_empty());
    }
}
