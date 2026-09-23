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
    base_url: String,
}

impl AnthropicClient {
    pub(crate) fn new(api_key: String) -> Self {
        Self {
            api_key,
            model: DEFAULT_MODEL.to_owned(),
            base_url: ANTHROPIC_API_URL.to_owned(),
        }
    }

    /// Test-only hook so an HTTP-layer test can point this client at a `wiremock`
    /// server instead of the real Anthropic API (T-verify: verbatim 401/429 error
    /// propagation, Principle I).
    #[cfg(test)]
    pub(crate) fn new_with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            api_key,
            model: DEFAULT_MODEL.to_owned(),
            base_url,
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
            .post(&self.base_url)
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

        Ok(TurnResult {
            text,
            suggested_actions: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Principle I: a 401 from the provider must reach the caller with its status and
    /// body text intact, plus the distinguishing hint — never swallowed or replaced
    /// with a generic message.
    #[tokio::test]
    async fn surfaces_401_verbatim() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "sk-ant-bad"))
            .respond_with(ResponseTemplate::new(401).set_body_string(
                r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#,
            ))
            .mount(&server)
            .await;

        let client = AnthropicClient::new_with_base_url(
            "sk-ant-bad".to_owned(),
            format!("{}/v1/messages", server.uri()),
        );
        let err = client
            .send_turn("system prompt", &[])
            .await
            .expect_err("401 must be an error");
        let msg = err.to_string();
        assert!(msg.contains("Anthropic rejected the API key"), "{msg}");
        assert!(msg.contains("401"), "{msg}");
        assert!(msg.contains("invalid x-api-key"), "{msg}");
    }

    /// Same for a 429 rate-limit response.
    #[tokio::test]
    async fn surfaces_429_verbatim() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(429).set_body_string(
                r#"{"type":"error","error":{"message":"rate limited, slow down"}}"#,
            ))
            .mount(&server)
            .await;

        let client = AnthropicClient::new_with_base_url(
            "sk-ant-test".to_owned(),
            format!("{}/v1/messages", server.uri()),
        );
        let err = client
            .send_turn("system prompt", &[])
            .await
            .expect_err("429 must be an error");
        let msg = err.to_string();
        assert!(msg.contains("Anthropic rate-limited this request"), "{msg}");
        assert!(msg.contains("429"), "{msg}");
        assert!(msg.contains("rate limited, slow down"), "{msg}");
    }

    /// An unreachable endpoint (nothing listening) must produce a distinct
    /// "could not reach" message rather than a generic/parse-style error.
    #[tokio::test]
    async fn surfaces_unreachable_endpoint_error() {
        // Port 0 bound-then-dropped-server trick isn't reliable across platforms; use a
        // reserved, almost-certainly-closed local port instead.
        let client = AnthropicClient::new_with_base_url(
            "sk-ant-test".to_owned(),
            "http://127.0.0.1:1/v1/messages".to_owned(),
        );
        let err = client
            .send_turn("system prompt", &[])
            .await
            .expect_err("unreachable endpoint must be an error");
        assert!(
            err.to_string().contains("Could not reach Anthropic API"),
            "{err}"
        );
    }
}
