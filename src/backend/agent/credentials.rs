//! Agent auth-mode storage (T014). Follows the exact existing convention in
//! `src/backend/persistence.rs`: plain JSON at `~/.fathom/credentials.json`, stored
//! under the `"agent"` key alongside whatever other credentials already live there.
//! There is no encryption-at-rest anywhere in this codebase today — this module does
//! not invent any; it follows the established pattern faithfully (Principle VI is
//! about never *replaying* a mutation without a fresh confirmation, not about
//! inventing new crypto here).
//!
//! The raw key is never written to `tracing::*` anywhere in this module — every log
//! statement below logs only the mode discriminant, never `key`/`Anthropic { key }`.

use crate::backend::persistence;
use crate::protocol::v1::{AgentAuthMode, AgentAuthModeInput};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StoredAgentAuth {
    #[default]
    Unconfigured,
    Anthropic {
        key: String,
    },
    Ollama {
        endpoint: String,
        model: String,
    },
    Disabled,
}

/// Loads the persisted agent auth mode. A missing/corrupt `credentials.json` (or a
/// missing/malformed `"agent"` key within it) falls back to `Unconfigured` rather than
/// deleting or overwriting the file (Principle VI) — other credentials in that file
/// must be left untouched.
pub(crate) fn load_agent_auth() -> AgentAuthMode {
    let stored = load_stored().unwrap_or_default();
    match stored {
        StoredAgentAuth::Unconfigured => AgentAuthMode::Unconfigured,
        StoredAgentAuth::Anthropic { key } => AgentAuthMode::Anthropic {
            key_configured: !key.trim().is_empty(),
        },
        StoredAgentAuth::Ollama { endpoint, model } => AgentAuthMode::Ollama { endpoint, model },
        StoredAgentAuth::Disabled => AgentAuthMode::Disabled,
    }
}

/// Loads the raw stored key/endpoint for actually making a request. Not part of the
/// `AgentAuthMode` returned to the frontend (which only ever carries
/// `key_configured: bool`), used only by the backend handler that dispatches to
/// `AnthropicClient`/`OllamaClient`.
pub(crate) fn load_for_request() -> AgentAuthModeForRequest {
    match load_stored().unwrap_or_default() {
        StoredAgentAuth::Unconfigured => AgentAuthModeForRequest::Unconfigured,
        StoredAgentAuth::Anthropic { key } => AgentAuthModeForRequest::Anthropic { key },
        StoredAgentAuth::Ollama { endpoint, model } => {
            AgentAuthModeForRequest::Ollama { endpoint, model }
        }
        StoredAgentAuth::Disabled => AgentAuthModeForRequest::Disabled,
    }
}

pub(crate) enum AgentAuthModeForRequest {
    Unconfigured,
    Anthropic { key: String },
    Ollama { endpoint: String, model: String },
    Disabled,
}

/// Returns the raw stored Anthropic key, in direct response to `AgentRevealKey` only
/// (FR-004, Principle VI). Returns `None` when no key is configured.
pub(crate) fn reveal_key() -> Option<String> {
    match load_stored().unwrap_or_default() {
        StoredAgentAuth::Anthropic { key } if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

pub(crate) fn save_agent_auth(input: AgentAuthModeInput) -> Result<AgentAuthMode> {
    let stored = match &input {
        AgentAuthModeInput::Anthropic { key } => StoredAgentAuth::Anthropic { key: key.clone() },
        AgentAuthModeInput::Ollama { endpoint, model } => StoredAgentAuth::Ollama {
            endpoint: endpoint.clone(),
            model: model.clone(),
        },
        AgentAuthModeInput::Disabled => StoredAgentAuth::Disabled,
    };
    save_stored(&stored)?;
    tracing::info!("Saved agent auth mode: {}", stored_kind(&stored));
    Ok(load_agent_auth())
}

fn stored_kind(stored: &StoredAgentAuth) -> &'static str {
    match stored {
        StoredAgentAuth::Unconfigured => "unconfigured",
        StoredAgentAuth::Anthropic { .. } => "anthropic",
        StoredAgentAuth::Ollama { .. } => "ollama",
        StoredAgentAuth::Disabled => "disabled",
    }
}

fn load_stored() -> Result<StoredAgentAuth> {
    let creds = persistence::load_credentials()?;
    match creds.get("agent") {
        Some(value) => Ok(serde_json::from_value(value.clone()).unwrap_or_default()),
        None => Ok(StoredAgentAuth::default()),
    }
}

fn save_stored(stored: &StoredAgentAuth) -> Result<()> {
    let mut creds = persistence::load_credentials()?;
    creds["agent"] = serde_json::to_value(stored)?;
    persistence::save_credentials(&creds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // These tests exercise the real `~/.fathom/credentials.json` path (same convention
    // as the rest of persistence.rs's untested-in-isolation functions). No
    // `serial_test` dependency exists in this workspace, so a process-wide mutex
    // serializes access to that shared file instead, and each test restores the prior
    // file contents afterwards.
    static CREDENTIALS_FILE_LOCK: Mutex<()> = Mutex::new(());

    fn with_clean_credentials<F: FnOnce()>(f: F) {
        let _guard = CREDENTIALS_FILE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let path = persistence::credentials_config_path().expect("path");
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

    #[test]
    fn save_then_load_roundtrips_anthropic_key() {
        with_clean_credentials(|| {
            save_agent_auth(AgentAuthModeInput::Anthropic {
                key: "sk-ant-test-key".to_owned(),
            })
            .expect("save");

            let mode = load_agent_auth();
            assert_eq!(mode, AgentAuthMode::Anthropic { key_configured: true });

            let revealed = reveal_key().expect("key present");
            assert_eq!(revealed, "sk-ant-test-key");
        });
    }

    #[test]
    fn save_then_load_roundtrips_ollama() {
        with_clean_credentials(|| {
            save_agent_auth(AgentAuthModeInput::Ollama {
                endpoint: "http://localhost:11434".to_owned(),
                model: "llama3.1".to_owned(),
            })
            .expect("save");

            let mode = load_agent_auth();
            assert_eq!(
                mode,
                AgentAuthMode::Ollama {
                    endpoint: "http://localhost:11434".to_owned(),
                    model: "llama3.1".to_owned(),
                }
            );
            // Reveal is Anthropic-only; no key to leak for Ollama mode.
            assert!(reveal_key().is_none());
        });
    }

    #[test]
    fn corrupt_credentials_file_falls_back_to_unconfigured_without_deleting_file() {
        with_clean_credentials(|| {
            let path = persistence::credentials_config_path().expect("path");
            std::fs::create_dir_all(path.parent().unwrap()).ok();
            std::fs::write(&path, "{ this is not valid json").expect("write corrupt file");

            let mode = load_agent_auth();
            assert_eq!(mode, AgentAuthMode::Unconfigured);

            // The corrupt file must still exist untouched — this module never deletes
            // or silently rewrites a file it merely failed to parse (Principle VI).
            assert!(path.exists());
            let content = std::fs::read_to_string(&path).expect("still readable as text");
            assert_eq!(content, "{ this is not valid json");
        });
    }

    #[test]
    fn disabled_mode_roundtrips() {
        with_clean_credentials(|| {
            save_agent_auth(AgentAuthModeInput::Disabled).expect("save");
            assert_eq!(load_agent_auth(), AgentAuthMode::Disabled);
        });
    }
}
