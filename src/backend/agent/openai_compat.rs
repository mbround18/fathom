//! Chat-completions client for OpenAI and GitHub Models (038 T018). Both speak the same
//! `/chat/completions` protocol with a bearer token. All traffic goes through [`AiHttp`].

use std::sync::Arc;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use super::{AgentClient, Turn, TurnFailure, TurnResult};
use crate::backend::ai::http::AiHttp;
use crate::protocol::v1::{AgentTurnAuthor, Provider};

pub(crate) const OPENAI_CHAT_URL: &str = "https://api.openai.com/v1/chat/completions";
pub(crate) const OPENAI_MODEL: &str = "gpt-4o-mini";
pub(crate) const GITHUB_CHAT_URL: &str = "https://models.github.ai/inference/chat/completions";
pub(crate) const GITHUB_MODEL: &str = "openai/gpt-4o-mini";

pub(crate) struct OpenAiCompatClient {
    http: Arc<AiHttp>,
    provider: Provider,
    url: String,
    token: SecretString,
    model: String,
}

impl OpenAiCompatClient {
    pub(crate) fn new(
        http: Arc<AiHttp>,
        provider: Provider,
        url: String,
        token: SecretString,
        model: String,
    ) -> Self {
        Self {
            http,
            provider,
            url,
            token,
            model,
        }
    }

    fn label(&self) -> &'static str {
        match self.provider {
            Provider::Github => "GitHub Models",
            _ => "OpenAI",
        }
    }
}

#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    messages: Vec<Message>,
}

#[derive(Serialize)]
struct Message {
    role: &'static str,
    content: String,
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: Option<ChoiceMessage>,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    #[serde(default)]
    content: Option<String>,
}

#[async_trait::async_trait]
impl AgentClient for OpenAiCompatClient {
    async fn send_turn(&self, system: &str, messages: &[Turn]) -> anyhow::Result<TurnResult> {
        let label = self.label();
        let mut chat = vec![Message {
            role: "system",
            content: system.to_owned(),
        }];
        chat.extend(messages.iter().map(|t| Message {
            role: match t.author {
                AgentTurnAuthor::Operator => "user",
                AgentTurnAuthor::Assistant => "assistant",
            },
            content: t.text.clone(),
        }));
        let body = Request {
            model: &self.model,
            messages: chat,
        };
        let resp = self
            .http
            .post(&self.url)
            .map_err(|e| anyhow::anyhow!("Could not reach {label}: {e}"))?
            .bearer_auth(self.token.expose_secret())
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Could not reach {label}: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let hint = match status.as_u16() {
                401 => format!("{label} rejected the credential (invalid, revoked or expired). "),
                429 => format!("{label} rate-limited this request. "),
                _ => String::new(),
            };
            let message = format!("{hint}{label} API error {status}: {text}");
            if status.as_u16() == 401 {
                return Err(anyhow::Error::new(TurnFailure::AuthRejected {
                    provider: self.provider,
                    message,
                }));
            }
            anyhow::bail!("{message}");
        }
        let parsed: Response = resp
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Could not parse {label} response: {e}"))?;
        let text = parsed
            .choices
            .into_iter()
            .filter_map(|c| c.message.and_then(|m| m.content))
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

    fn client(server: &MockServer, provider: Provider) -> OpenAiCompatClient {
        OpenAiCompatClient::new(
            Arc::new(AiHttp::new()),
            provider,
            format!("{}/chat/completions", server.uri()),
            SecretString::from("tok".to_owned()),
            "m".to_owned(),
        )
    }

    #[tokio::test]
    async fn returns_reply_text() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("authorization", "Bearer tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"choices":[{"message":{"role":"assistant","content":"hi"}}]}),
            ))
            .mount(&server)
            .await;
        let out = client(&server, Provider::Openai)
            .send_turn("sys", &[])
            .await
            .unwrap();
        assert_eq!(out.text, "hi");
    }

    #[tokio::test]
    async fn unauthorized_is_typed_and_verbatim() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string(r#"{"error":"bad token"}"#))
            .mount(&server)
            .await;
        let err = client(&server, Provider::Github)
            .send_turn("sys", &[])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("bad token"));
        assert!(matches!(
            err.downcast_ref::<TurnFailure>(),
            Some(TurnFailure::AuthRejected {
                provider: Provider::Github,
                ..
            })
        ));
    }
}
