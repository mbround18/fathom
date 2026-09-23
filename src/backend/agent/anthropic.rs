//! Anthropic Messages API client (research.md R1). BYOK: the raw key is read from
//! `credentials.json` at request time only and is never stored on this struct beyond
//! the single request that needs it (Principle VI) — never logged.

use super::{AgentClient, Turn, TurnResult};
use crate::protocol::v1::AgentTurnAuthor;
use serde::{Deserialize, Serialize};

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MODEL: &str = "claude-3-5-sonnet-latest";

pub(crate) struct AnthropicClient {
    api_key: String,
    model: String,
}

impl AnthropicClient {
    pub(crate) fn new(api_key: String) -> Self {
        Self {
            api_key,
            model: DEFAULT_MODEL.to_owned(),
        }
    }
}

#[derive(Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: Vec<AnthropicMessage>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: &'static str,
    content: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    #[serde(default)]
    content: Vec<AnthropicContentBlock>,
}

#[derive(Deserialize)]
struct AnthropicContentBlock {
    #[serde(default)]
    text: Option<String>,
}

#[async_trait::async_trait]
impl AgentClient for AnthropicClient {
    async fn send_turn(&self, system: &str, messages: &[Turn]) -> anyhow::Result<TurnResult> {
        if self.api_key.trim().is_empty() {
            anyhow::bail!("Anthropic API key is not configured");
        }

        let body = AnthropicRequest {
            model: &self.model,
            max_tokens: 1024,
            system,
            messages: messages
                .iter()
                .map(|turn| AnthropicMessage {
                    role: match turn.author {
                        AgentTurnAuthor::Operator => "user",
                        AgentTurnAuthor::Assistant => "assistant",
                    },
                    content: turn.text.clone(),
                })
                .collect(),
        };

        let client = reqwest::Client::new();
        let resp = client
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Could not reach Anthropic API: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            // Surface the provider's own error body verbatim (Principle I) rather than
            // a generic "request failed" string, so auth/rate-limit/model errors are
            // distinguishable to the operator (FR-015).
            let text = resp.text().await.unwrap_or_default();
            let hint = match status.as_u16() {
                401 => "Anthropic rejected the API key (invalid or revoked). ",
                429 => "Anthropic rate-limited this request. ",
                404 => "Anthropic model not found. ",
                _ => "",
            };
            anyhow::bail!("{hint}Anthropic API error {status}: {text}");
        }

        let parsed: AnthropicResponse = resp
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Could not parse Anthropic response: {e}"))?;

        let text = parsed
            .content
            .into_iter()
            .filter_map(|block| block.text)
            .collect::<Vec<_>>()
            .join("\n");

        Ok(TurnResult { text })
    }
}
