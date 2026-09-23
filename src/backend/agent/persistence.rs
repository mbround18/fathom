//! Agent conversation persistence (Phase 6/T044, US4). Follows the exact existing
//! convention in `src/backend/persistence.rs`: plain JSON under `~/.fathom/`. Unlike
//! `credentials.rs` (which subkeys into a shared `credentials.json`), this whole file
//! belongs to this one feature, so it is stored as its own file,
//! `~/.fathom/agent_conversations.json`, deserialized directly into a typed
//! `HashMap<String, AgentConversation>` keyed by `"<kind>/<namespace>/<name>"`
//! (data-model.md), rather than staying an untyped `serde_json::Value`.
//!
//! A missing or corrupt `agent_conversations.json` falls back to an empty map
//! (FR-017: history is inert/best-effort local durability, never load-bearing for
//! correctness) without deleting or overwriting the file (Principle VI) — the
//! operator's on-disk history is never silently destroyed by a parse failure.

use crate::backend::persistence::{ensure_config_dir, fathom_config_dir};
use crate::protocol::v1::AgentConversation;
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;

/// Get the path to `agent_conversations.json`.
pub(crate) fn agent_conversations_config_path() -> Result<PathBuf> {
    Ok(fathom_config_dir()?.join("agent_conversations.json"))
}

/// Loads all persisted agent conversations, keyed by `"<kind>/<namespace>/<name>"`. A
/// missing file yields an empty map. A corrupt/unparseable file also yields an empty
/// map, and is left untouched on disk (Principle VI) — this function never deletes or
/// rewrites a file it merely failed to parse.
pub(crate) fn load_agent_conversations() -> HashMap<String, AgentConversation> {
    match load_agent_conversations_inner() {
        Ok(map) => map,
        Err(e) => {
            tracing::warn!(
                "Failed to load agent_conversations.json; starting with empty history: {}",
                e
            );
            HashMap::new()
        }
    }
}

fn load_agent_conversations_inner() -> Result<HashMap<String, AgentConversation>> {
    let path = agent_conversations_config_path()?;
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let content = std::fs::read_to_string(&path)?;
    let map = serde_json::from_str(&content)?;
    Ok(map)
}

/// Persists the full set of agent conversations to `agent_conversations.json`. Callers
/// (`src/backend/cmd/agent.rs`) treat a write failure as best-effort: it is logged via
/// `tracing::error!` there and never blocks or fails the in-memory state update / event
/// emission, since disk is durability-only, not the source of truth for a running
/// session (FR-017).
pub(crate) fn save_agent_conversations(
    conversations: &HashMap<String, AgentConversation>,
) -> Result<()> {
    ensure_config_dir()?;
    let path = agent_conversations_config_path()?;
    let content = serde_json::to_string_pretty(conversations)?;
    std::fs::write(&path, content)?;
    tracing::debug!("Saved {} agent conversation(s) to disk", conversations.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::v1::{AgentMessage, AgentTurnAuthor};
    use std::sync::Mutex;

    // Same isolation convention as `agent/credentials.rs`'s tests: exercise the real
    // `~/.fathom/agent_conversations.json` path, serialized by a process-wide mutex,
    // restoring whatever was there beforehand.
    static AGENT_CONVERSATIONS_FILE_LOCK: Mutex<()> = Mutex::new(());

    fn with_clean_file<F: FnOnce()>(f: F) {
        let _guard = AGENT_CONVERSATIONS_FILE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let path = agent_conversations_config_path().expect("path");
        let existed = path.exists();
        let backup = if existed {
            Some(std::fs::read_to_string(&path).expect("read backup"))
        } else {
            None
        };
        f();
        match backup {
            Some(content) => {
                std::fs::write(&path, content).ok();
            }
            None => {
                std::fs::remove_file(&path).ok();
            }
        }
    }

    fn sample_conversation() -> AgentConversation {
        AgentConversation {
            messages: vec![AgentMessage {
                id: "msg-1".to_owned(),
                author: AgentTurnAuthor::Operator,
                text: "why is this pod crashlooping?".to_owned(),
                grounding: Vec::new(),
                suggested_actions: Vec::new(),
                created_at: "2026-01-01T00:00:00Z".to_owned(),
            }],
            executed_actions: Vec::new(),
        }
    }

    #[test]
    fn save_then_load_roundtrips_a_conversation() {
        with_clean_file(|| {
            let mut map = HashMap::new();
            map.insert("Pod/default/my-pod".to_owned(), sample_conversation());

            save_agent_conversations(&map).expect("save");
            let loaded = load_agent_conversations();

            assert_eq!(loaded, map);
        });
    }

    #[test]
    fn missing_file_falls_back_to_empty_map() {
        with_clean_file(|| {
            let path = agent_conversations_config_path().expect("path");
            std::fs::remove_file(&path).ok();

            let loaded = load_agent_conversations();
            assert!(loaded.is_empty());
        });
    }

    #[test]
    fn corrupt_file_falls_back_to_empty_map_without_deleting_it() {
        with_clean_file(|| {
            let path = agent_conversations_config_path().expect("path");
            std::fs::create_dir_all(path.parent().unwrap()).ok();
            std::fs::write(&path, "{ this is not valid json").expect("write corrupt file");

            let loaded = load_agent_conversations();
            assert!(loaded.is_empty());

            // The corrupt file must still exist untouched.
            assert!(path.exists());
            let content = std::fs::read_to_string(&path).expect("still readable as text");
            assert_eq!(content, "{ this is not valid json");
        });
    }
}
