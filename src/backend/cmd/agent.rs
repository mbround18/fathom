//! Agent commands: 013-agent-sdk-ops's in-app troubleshooting assistant.
//!
//! # Safety invariant (Principle I / SC-002 / SC-003)
//!
//! Nothing in this file ever constructs or dispatches a cluster-mutating
//! `BackendCmd`. `AgentReviewSuggestedAction`/`AgentDeclineSuggestedAction` only ever
//! flip a `SuggestedActionStatus` via `crate::backend::agent::state_machine`, and
//! `AgentRecordExecutedAction` only ever records the outcome of a command the
//! *frontend* already ran through the ordinary, unmodified action-dispatch path (see
//! `ui/src/features/agent-ops/reviewSuggestedAction.ts`) — this handler never runs
//! that command itself. See `src/protocol/v1/mutation.rs`'s
//! `agent_commands_never_write_cluster` test for the type-level proof.

use crossbeam_channel::Sender;

use crate::backend::agent::{self, Turn, credentials, grounding, state_machine};
use crate::backend::state::BackendState;
use crate::protocol::{
    ActionOutcome, AgentAuthMode, AgentConversation, AgentMessage, AgentObjectRef, AgentTurnAuthor,
    BackendCmd, BackendEvent, BackendEventEnvelope, Correlation, Route, SystemEvent, section_keys,
};

fn conversation_key(object: &AgentObjectRef) -> String {
    format!("{}/{}/{}", object.kind, object.namespace, object.name)
}

/// Persists the full in-memory conversation map to `~/.fathom/agent_conversations.json`
/// (US4/FR-017). Best-effort: a write failure is logged and never blocks or fails the
/// caller — the in-memory state and the `AgentConversationUpdated` event emitted
/// alongside every call site of this function remain the source of truth for the
/// running session; disk is durability-only.
fn persist_conversations(state: &BackendState) {
    if let Err(err) = agent::persistence::save_agent_conversations(&state.agent_conversations) {
        tracing::error!("Failed to save agent_conversations.json: {}", err);
    }
}

fn emit_conversation_updated(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    object: AgentObjectRef,
    conversation: AgentConversation,
) {
    crate::backend::events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::global(section_keys::agent()),
        Correlation::None,
        BackendEvent::ResourceRead(
            crate::protocol::ResourceReadEvent::AgentConversationUpdated {
                object,
                conversation,
            },
        ),
    );
}

fn emit_auth_mode(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    mode: AgentAuthMode,
) {
    crate::backend::events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::global(section_keys::agent()),
        Correlation::None,
        BackendEvent::System(SystemEvent::AgentAuthModeUpdated { mode }),
    );
}

/// Handles `Agent*` commands; returns the command back if it belongs to another family
/// (mirrors `cmd/integrations.rs`'s shape).
pub(super) async fn handle(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::AgentGetAuthMode => {
            let mode = credentials::load_agent_auth(&state.ai).await;
            emit_auth_mode(state, sender, mode);
        }
        BackendCmd::AgentSetAuthMode { mode } => {
            let updated = credentials::save_agent_auth(&state.ai, mode).await?;
            emit_auth_mode(state, sender, updated);
            // The shared 038 status stream must reflect the change too.
            state.ai.emit_status(&super::ai::emitter(state, sender));
        }
        BackendCmd::AgentRevealKey => {
            let key = credentials::reveal_key(&state.ai).await.unwrap_or_default();
            crate::backend::events::emit(
                state.cluster.seq.as_ref(),
                sender,
                Route::global(section_keys::agent()),
                Correlation::None,
                BackendEvent::System(SystemEvent::AgentKeyRevealed { key }),
            );
        }
        BackendCmd::AgentOpenConversation { object } => {
            let conversation = state
                .agent_conversations
                .entry(conversation_key(&object))
                .or_default()
                .clone();
            emit_conversation_updated(state, sender, object, conversation);
        }
        BackendCmd::AgentAsk { object, question } => {
            handle_ask(state, sender, object, question).await;
        }
        BackendCmd::AgentClearConversation { object } => {
            state.agent_conversations.remove(&conversation_key(&object));
            persist_conversations(state);
            emit_conversation_updated(state, sender, object, AgentConversation::default());
        }
        BackendCmd::AgentReviewSuggestedAction {
            object,
            suggestion_id,
        } => {
            handle_transition(state, sender, object, &suggestion_id, state_machine::review);
        }
        BackendCmd::AgentDeclineSuggestedAction {
            object,
            suggestion_id,
        } => {
            handle_transition(
                state,
                sender,
                object,
                &suggestion_id,
                state_machine::decline,
            );
        }
        BackendCmd::AgentRecordExecutedAction {
            object,
            suggestion_id,
            command_summary,
            outcome,
        } => {
            handle_record_executed(
                state,
                sender,
                object,
                suggestion_id,
                command_summary,
                outcome,
            );
        }
        other => return Ok(Some(other)),
    }
    Ok(None)
}

fn handle_transition(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    object: AgentObjectRef,
    suggestion_id: &str,
    transition: fn(&mut crate::protocol::SuggestedAction) -> Result<(), state_machine::StateError>,
) {
    let key = conversation_key(&object);
    let Some(conversation) = state.agent_conversations.get_mut(&key) else {
        return;
    };
    for message in &mut conversation.messages {
        for suggestion in &mut message.suggested_actions {
            if suggestion.id == suggestion_id
                && let Err(err) = transition(suggestion)
            {
                tracing::warn!("Agent suggested-action transition rejected: {err}");
            }
        }
    }
    let conversation = conversation.clone();
    persist_conversations(state);
    emit_conversation_updated(state, sender, object, conversation);
}

fn handle_record_executed(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    object: AgentObjectRef,
    suggestion_id: String,
    command_summary: String,
    outcome: ActionOutcome,
) {
    let key = conversation_key(&object);
    let Some(conversation) = state.agent_conversations.get_mut(&key) else {
        return;
    };

    let suggestion = conversation
        .messages
        .iter()
        .flat_map(|m| m.suggested_actions.iter())
        .find(|s| s.id == suggestion_id)
        .cloned();

    let Some(suggestion) = suggestion else {
        tracing::warn!("AgentRecordExecutedAction: no such suggestion {suggestion_id}");
        return;
    };

    let executed_at = chrono::Utc::now().to_rfc3339();
    match state_machine::build_executed_action(
        agent::next_local_id("exec"),
        &suggestion,
        command_summary,
        outcome,
        executed_at,
    ) {
        Ok(executed) => conversation.executed_actions.push(executed),
        Err(err) => {
            tracing::warn!("AgentRecordExecutedAction rejected: {err}");
        }
    }

    let conversation = conversation.clone();
    persist_conversations(state);
    emit_conversation_updated(state, sender, object, conversation);
}

/// Builds the system prompt, collects grounding data, dispatches to the configured
/// provider, and appends both turns to the conversation. On any client error, the
/// error's verbatim message becomes the assistant's reply text (Principle I) — never
/// replaced with a generic string.
async fn handle_ask(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    object: AgentObjectRef,
    question: String,
) {
    let key = conversation_key(&object);

    let operator_message = AgentMessage {
        id: agent::next_local_id("msg"),
        author: AgentTurnAuthor::Operator,
        text: question.clone(),
        grounding: Vec::new(),
        suggested_actions: Vec::new(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    state
        .agent_conversations
        .entry(key.clone())
        .or_default()
        .messages
        .push(operator_message.clone());

    let (grounding_text, grounding_sources) =
        grounding::collect_pod_grounding(state, &object).await;

    let system_prompt = format!(
        "You are Fathom's in-app Kubernetes troubleshooting assistant. Answer only \
         using the grounding data provided below; if the question is unrelated to this \
         object or the data given, say so rather than guessing. Do not claim to have \
         taken any action — you may only describe a possible next step for the \
         operator to review and confirm.\n\n{}\n\nGrounding data for {}/{}/{}:\n{}",
        agent::ACTION_PROPOSAL_INSTRUCTIONS,
        object.kind,
        object.namespace,
        object.name,
        grounding_text
    );

    let history: Vec<Turn> = state
        .agent_conversations
        .get(&key)
        .map(|c| {
            c.messages
                .iter()
                .map(|m| Turn {
                    author: m.author,
                    text: m.text.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    let raw_reply_text = match dispatch_to_provider(state, sender, &system_prompt, &history).await {
        Ok(result) => result.text,
        Err(err) => err.to_string(),
    };

    // Parse the fathom-action convention out of the raw reply (Phase 3, US1:
    // informational only — no review/execute wiring here, see mod-level safety doc).
    // A parse failure is never surfaced to the operator: malformed blocks are simply
    // skipped inside `parse_suggested_actions`.
    let parsed_actions = agent::action_parse::parse_suggested_actions(&raw_reply_text);
    let suggested_actions = parsed_actions
        .into_iter()
        .map(|parsed| crate::protocol::SuggestedAction {
            id: agent::next_local_id("sugg"),
            action_kind: parsed.action_kind,
            target: parsed.target,
            params: parsed.params,
            rationale: parsed.rationale,
            status: crate::protocol::SuggestedActionStatus::Proposed,
        })
        .collect();
    let reply_text = agent::action_parse::strip_action_blocks(&raw_reply_text);

    let assistant_message = AgentMessage {
        id: agent::next_local_id("msg"),
        author: AgentTurnAuthor::Assistant,
        text: reply_text,
        grounding: grounding_sources,
        suggested_actions,
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    let conversation = {
        let conversation = state.agent_conversations.entry(key).or_default();
        conversation.messages.push(assistant_message);
        conversation.clone()
    };

    persist_conversations(state);
    emit_conversation_updated(state, sender, object, conversation);
}

/// Resolve the effective provider (offline forces Ollama), send the turn, and react to
/// typed failures: a 401 flips the provider to NeedsReauth, a missing Ollama model clears
/// the selection. The error text stays the provider's verbatim message (Principle I).
async fn dispatch_to_provider(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    system_prompt: &str,
    history: &[Turn],
) -> anyhow::Result<agent::TurnResult> {
    let client = state.ai.turn_client().await?;
    match client.send_turn(system_prompt, history).await {
        Ok(result) => Ok(result),
        Err(err) => {
            let em = super::ai::emitter(state, sender);
            match err.downcast_ref::<agent::TurnFailure>() {
                Some(agent::TurnFailure::AuthRejected { provider, .. }) => {
                    state.ai.mark_needs_reauth(*provider, &em);
                }
                Some(agent::TurnFailure::OllamaModelMissing { model, .. }) => {
                    state.ai.model_missing(model, &em).await;
                }
                None => {}
            }
            Err(err)
        }
    }
}
