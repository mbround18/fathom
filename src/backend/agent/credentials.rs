//! 013 agent auth-mode API, now a thin shim over the 038 secure store and settings
//! (`crate::backend::ai`). The wire contract (`AgentAuthMode`, `AgentRevealKey`) is
//! unchanged; only the storage moved: the Anthropic key lives in the OS credential store
//! and the non-secret selection in `~/.fathom/config.json`. The legacy plaintext
//! `credentials.json` is migrated at startup (`ai::migrate`) and never written again.
//!
//! Mapping notes: 013's `Disabled` and `Unconfigured` both mean "no active provider" now.

use anyhow::{Result, bail};
use secrecy::{ExposeSecret, SecretString};

use crate::backend::ai::AiRuntime;
use crate::backend::ai::http::endpoint_is_loopback;
use crate::protocol::v1::{AgentAuthMode, AgentAuthModeInput, Provider};

/// The current mode as the 013 UI understands it. The key itself is never returned here
/// (only `key_configured`).
pub(crate) async fn load_agent_auth(rt: &AiRuntime) -> AgentAuthMode {
    rt.refresh_presence().await;
    let s = rt.session();
    match s.settings.active_provider {
        Some(Provider::Claude) => AgentAuthMode::Anthropic {
            key_configured: s.present.contains(&Provider::Claude),
        },
        Some(Provider::Ollama) => AgentAuthMode::Ollama {
            endpoint: s.settings.ollama_endpoint.clone(),
            model: s.settings.ollama_model.clone().unwrap_or_default(),
        },
        // GitHub / OpenAI have no 013 representation.
        _ => AgentAuthMode::Unconfigured,
    }
}

/// Raw stored Anthropic key, in direct response to `AgentRevealKey` only (013 FR-004,
/// Principle VI). `None` when no key is configured or the store cannot be read.
pub(crate) async fn reveal_key(rt: &AiRuntime) -> Option<String> {
    match rt.store().get_async(Provider::Claude).await {
        Ok(Some(secret)) if !secret.expose_secret().trim().is_empty() => {
            Some(secret.expose_secret().to_owned())
        }
        _ => None,
    }
}

pub(crate) async fn save_agent_auth(
    rt: &AiRuntime,
    input: AgentAuthModeInput,
) -> Result<AgentAuthMode> {
    match input {
        AgentAuthModeInput::Anthropic { key } => {
            let key = key.trim().to_owned();
            if key.is_empty() {
                bail!("Anthropic API key is empty");
            }
            rt.store()
                .save_verified_async(Provider::Claude, SecretString::from(key))
                .await
                .map_err(|e| anyhow::anyhow!("{} {}", e.message(), e.hint()))?;
            {
                let mut s = rt.session();
                s.present.insert(Provider::Claude);
                s.needs_reauth.remove(&Provider::Claude);
                s.settings.active_provider = Some(Provider::Claude);
            }
        }
        AgentAuthModeInput::Ollama { endpoint, model } => {
            if !endpoint_is_loopback(&endpoint) && !rt.settings().allow_lan_ollama {
                bail!(
                    "The Ollama endpoint {endpoint} is not loopback. Confirm LAN use in Settings -> AI Assist first."
                );
            }
            let mut s = rt.session();
            s.settings.ollama_endpoint = endpoint;
            s.settings.ollama_model = Some(model).filter(|m| !m.trim().is_empty());
            s.settings.active_provider = Some(Provider::Ollama);
        }
        AgentAuthModeInput::Disabled => {
            rt.session().settings.active_provider = None;
        }
    }
    rt.persist_settings();
    tracing::info!("Saved agent auth mode");
    Ok(load_agent_auth(rt).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ai::{AiRuntime, Endpoints, http::AiHttp, store::testing::Fixture};
    use std::sync::Arc;

    fn runtime() -> (AiRuntime, Fixture, tempfile::TempDir) {
        let fx = Fixture::new();
        let dir = tempfile::tempdir().unwrap();
        let rt = AiRuntime::new(
            Arc::clone(&fx.store),
            Arc::new(AiHttp::new()),
            Endpoints::default(),
            Some(dir.path().join("config.json")),
            None,
        );
        (rt, fx, dir)
    }

    #[tokio::test]
    async fn unconfigured_by_default() {
        let (rt, _fx, _dir) = runtime();
        assert!(matches!(
            load_agent_auth(&rt).await,
            AgentAuthMode::Unconfigured
        ));
        assert_eq!(reveal_key(&rt).await, None);
    }

    #[tokio::test]
    async fn anthropic_key_goes_to_keyring_and_reveals_on_demand() {
        let (rt, fx, dir) = runtime();
        let mode = save_agent_auth(
            &rt,
            AgentAuthModeInput::Anthropic {
                key: "sk-ant-x".into(),
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            mode,
            AgentAuthMode::Anthropic {
                key_configured: true
            }
        ));
        assert_eq!(
            fx.keyring.raw("ai-assist/claude").as_deref(),
            Some("sk-ant-x")
        );
        assert_eq!(reveal_key(&rt).await.as_deref(), Some("sk-ant-x"));
        // Never in the on-disk config.
        let cfg = std::fs::read_to_string(dir.path().join("config.json")).unwrap();
        assert!(!cfg.contains("sk-ant-x"));
    }

    #[tokio::test]
    async fn ollama_and_disabled_roundtrip() {
        let (rt, _fx, _dir) = runtime();
        let mode = save_agent_auth(
            &rt,
            AgentAuthModeInput::Ollama {
                endpoint: "http://127.0.0.1:11434".into(),
                model: "llama3.2".into(),
            },
        )
        .await
        .unwrap();
        assert!(matches!(mode, AgentAuthMode::Ollama { ref model, .. } if model == "llama3.2"));
        let mode = save_agent_auth(&rt, AgentAuthModeInput::Disabled)
            .await
            .unwrap();
        assert!(matches!(mode, AgentAuthMode::Unconfigured));
    }

    #[tokio::test]
    async fn lan_ollama_needs_confirmation() {
        let (rt, _fx, _dir) = runtime();
        let res = save_agent_auth(
            &rt,
            AgentAuthModeInput::Ollama {
                endpoint: "http://192.168.1.50:11434".into(),
                model: "m".into(),
            },
        )
        .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn locked_store_surfaces_an_error_and_keeps_state() {
        use crate::backend::ai::store::testing::Mode;
        let (rt, fx, _dir) = runtime();
        fx.keyring.set_mode(Mode::Locked);
        let res = save_agent_auth(&rt, AgentAuthModeInput::Anthropic { key: "k".into() }).await;
        assert!(res.is_err());
        assert!(rt.settings().active_provider.is_none());
    }
}
