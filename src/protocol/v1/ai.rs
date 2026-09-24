//! Protocol types for 038-sdk-login-ollama: provider login, secure credential status,
//! Ollama detection/model pick and offline mode.
//!
//! No type in this file that travels *out* of the backend carries a secret. The only
//! secret-bearing type is [`ApiKeyInput`], which is inbound-only and redacts itself in
//! `Debug` so it can never reach logs through `{:?}` formatting of a command.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A model provider the AI-assist surface can use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Github,
    Claude,
    Openai,
    Ollama,
}

impl Provider {
    pub const ALL: [Provider; 4] = [
        Provider::Github,
        Provider::Claude,
        Provider::Openai,
        Provider::Ollama,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Github => "github",
            Provider::Claude => "claude",
            Provider::Openai => "openai",
            Provider::Ollama => "ollama",
        }
    }

    /// Hosted providers need the network beyond loopback and a stored credential.
    pub fn is_hosted(self) -> bool {
        !matches!(self, Provider::Ollama)
    }
}

/// An API key on its way *in* (`AiProviderSubmitApiKey`). Never serialized outward by any
/// event; `Debug` prints a fixed placeholder so tracing a command cannot leak it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(transparent)]
#[ts(type = "string")]
pub struct ApiKeyInput(pub String);

impl std::fmt::Debug for ApiKeyInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKeyInput([REDACTED])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BlockedReason {
    Offline,
    StoreLocked,
    StoreUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum ProviderState {
    NotConfigured,
    Connecting,
    Connected,
    NeedsReauth,
    Blocked { reason: BlockedReason },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ProviderStatus {
    pub provider: Provider,
    pub state: ProviderState,
    pub account_label: Option<String>,
    /// RFC 3339.
    pub last_checked_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LoginFlowKind {
    DeviceCode,
    ApiKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum LoginPhase {
    Pending,
    Polling,
    Succeeded,
    Denied,
    Expired,
    Failed { message: String },
}

/// Transient login flow state. `user_code` is the GitHub device *user* code (meant to be
/// shown to the operator), not a credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct LoginFlow {
    pub provider: Provider,
    pub kind: LoginFlowKind,
    pub user_code: Option<String>,
    pub verification_uri: Option<String>,
    /// RFC 3339.
    pub expires_at: Option<String>,
    pub interval_s: Option<u32>,
    pub phase: LoginPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum InstalledHint {
    Unknown,
    Found,
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct OllamaModel {
    pub name: String,
    pub size_bytes: u64,
    pub family: Option<String>,
    pub parameter_size: Option<String>,
    pub modified_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct OllamaState {
    pub endpoint: String,
    pub reachable: bool,
    pub version: Option<String>,
    pub installed_hint: InstalledHint,
    pub models: Vec<OllamaModel>,
    pub checked_at: String,
    pub error: Option<String>,
}

/// Non-secret AI-assist settings persisted in `~/.fathom/config.json` under `ai_assist`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
pub struct AiAssistSettings {
    pub active_provider: Option<Provider>,
    pub ollama_endpoint: String,
    pub ollama_model: Option<String>,
    pub offline_mode: bool,
    pub allow_lan_ollama: bool,
    pub file_store_optin: bool,
}

pub const DEFAULT_OLLAMA_ENDPOINT: &str = "http://127.0.0.1:11434";

impl Default for AiAssistSettings {
    fn default() -> Self {
        Self {
            active_provider: None,
            ollama_endpoint: DEFAULT_OLLAMA_ENDPOINT.to_owned(),
            ollama_model: None,
            offline_mode: false,
            allow_lan_ollama: false,
            file_store_optin: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum CredentialMigrationOutcome {
    Migrated,
    Failed { message: String },
    NothingToMigrate,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip<T>(value: &T)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(value).unwrap();
        let back: T = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, value);
    }

    #[test]
    fn provider_wire_names() {
        assert_eq!(
            serde_json::to_string(&Provider::Github).unwrap(),
            "\"github\""
        );
        assert_eq!(
            serde_json::to_string(&Provider::Openai).unwrap(),
            "\"openai\""
        );
        for p in Provider::ALL {
            roundtrip(&p);
            assert_eq!(
                serde_json::to_string(&p).unwrap(),
                format!("\"{}\"", p.as_str())
            );
        }
    }

    #[test]
    fn status_and_flow_roundtrip() {
        for state in [
            ProviderState::NotConfigured,
            ProviderState::Connecting,
            ProviderState::Connected,
            ProviderState::NeedsReauth,
            ProviderState::Blocked {
                reason: BlockedReason::StoreLocked,
            },
        ] {
            roundtrip(&ProviderStatus {
                provider: Provider::Claude,
                state,
                account_label: Some("octocat".into()),
                last_checked_at: None,
            });
        }
        for phase in [
            LoginPhase::Pending,
            LoginPhase::Polling,
            LoginPhase::Succeeded,
            LoginPhase::Denied,
            LoginPhase::Expired,
            LoginPhase::Failed {
                message: "boom".into(),
            },
        ] {
            roundtrip(&LoginFlow {
                provider: Provider::Github,
                kind: LoginFlowKind::DeviceCode,
                user_code: Some("ABCD-1234".into()),
                verification_uri: Some("https://github.com/login/device".into()),
                expires_at: Some("2026-01-01T00:00:00Z".into()),
                interval_s: Some(5),
                phase,
            });
        }
    }

    #[test]
    fn ollama_state_roundtrip() {
        roundtrip(&OllamaState {
            endpoint: DEFAULT_OLLAMA_ENDPOINT.into(),
            reachable: true,
            version: Some("0.5.0".into()),
            installed_hint: InstalledHint::Found,
            models: vec![OllamaModel {
                name: "llama3.2:latest".into(),
                size_bytes: 2_000_000_000,
                family: Some("llama".into()),
                parameter_size: Some("3.2B".into()),
                modified_at: "2026-01-01T00:00:00Z".into(),
            }],
            checked_at: "2026-01-01T00:00:00Z".into(),
            error: None,
        });
    }

    #[test]
    fn settings_defaults_load_from_old_or_empty_files() {
        let s: AiAssistSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(s, AiAssistSettings::default());
        assert_eq!(s.ollama_endpoint, "http://127.0.0.1:11434");
        let s: AiAssistSettings = serde_json::from_str(r#"{"offline_mode":true}"#).unwrap();
        assert!(s.offline_mode && s.active_provider.is_none());
        roundtrip(&AiAssistSettings {
            active_provider: Some(Provider::Ollama),
            ollama_model: Some("m".into()),
            ..AiAssistSettings::default()
        });
    }

    #[test]
    fn api_key_input_is_transparent_and_redacted() {
        let k = ApiKeyInput("sk-secret-123".into());
        assert_eq!(serde_json::to_string(&k).unwrap(), "\"sk-secret-123\"");
        roundtrip(&k);
        let dbg = format!("{k:?}");
        assert!(!dbg.contains("sk-secret-123"), "{dbg}");
    }

    #[test]
    fn migration_outcome_roundtrip() {
        roundtrip(&CredentialMigrationOutcome::Migrated);
        roundtrip(&CredentialMigrationOutcome::NothingToMigrate);
        roundtrip(&CredentialMigrationOutcome::Failed {
            message: "x".into(),
        });
    }
}
