//! Local Ollama chat client (research.md R1). Selecting this mode means zero calls to
//! any hosted LLM API are made (FR-003, SC-007) — this client only ever talks to the
//! operator-provided local `endpoint`.

use super::{AgentClient, Turn, TurnResult};
use crate::protocol::v1::AgentTurnAuthor;
use serde::{Deserialize, Serialize};

pub(crate) struct OllamaClient {
    endpoint: String,
    model: String,
}

impl OllamaClient {
    pub(crate) fn new(endpoint: String, model: String) -> Self {
        Self { endpoint, model }
    }
}

#[derive(Serialize)]
struct OllamaRequest<'a> {
    model: &'a str,
    messages: Vec<OllamaMessage>,
    stream: bool,
}

#[derive(Serialize)]
struct OllamaMessage {
    role: &'static str,
    content: String,
}

#[derive(Deserialize)]
struct OllamaResponse {
    #[serde(default)]
    message: Option<OllamaResponseMessage>,
}

#[derive(Deserialize)]
struct OllamaResponseMessage {
    #[serde(default)]
    content: String,
}

#[async_trait::async_trait]
impl AgentClient for OllamaClient {
    async fn send_turn(&self, system: &str, messages: &[Turn]) -> anyhow::Result<TurnResult> {
        if self.endpoint.trim().is_empty() {
            anyhow::bail!("Ollama endpoint is not configured");
        }

        let mut chat_messages = vec![OllamaMessage {
            role: "system",
            content: system.to_owned(),
        }];
        chat_messages.extend(messages.iter().map(|turn| OllamaMessage {
            role: match turn.author {
                AgentTurnAuthor::Operator => "user",
                AgentTurnAuthor::Assistant => "assistant",
            },
            content: turn.text.clone(),
        }));

        let body = OllamaRequest {
            model: &self.model,
            messages: chat_messages,
            stream: false,
        };

        let url = format!("{}/api/chat", self.endpoint.trim_end_matches('/'));
        let client = reqwest::Client::new();
        let resp = client.post(&url).json(&body).send().await.map_err(|e| {
            anyhow::anyhow!(
                "Could not reach local Ollama endpoint {}: {e}",
                self.endpoint
            )
        })?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let hint = if status.as_u16() == 404 {
                format!("Ollama model \"{}\" not found. ", self.model)
            } else {
                String::new()
            };
            anyhow::bail!("{hint}Ollama API error {status}: {text}");
        }

        let parsed: OllamaResponse = resp
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Could not parse Ollama response: {e}"))?;

        Ok(TurnResult {
            text: parsed.message.map(|m| m.content).unwrap_or_default(),
        })
    }
}
