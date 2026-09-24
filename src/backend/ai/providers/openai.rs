//! OpenAI key validation: `GET /v1/models` with the operator's key.

use secrecy::{ExposeSecret, SecretString};

use super::{ProviderAdapter, ProviderError, ValidatedAccount, error_from_response};
use crate::backend::ai::http::AiHttp;

pub(crate) const DEFAULT_BASE: &str = "https://api.openai.com";

pub(crate) struct OpenAiAdapter {
    base: String,
}

impl OpenAiAdapter {
    pub(crate) fn new(base: impl Into<String>) -> Self {
        Self { base: base.into() }
    }
}

#[async_trait::async_trait]
impl ProviderAdapter for OpenAiAdapter {
    async fn validate(
        &self,
        http: &AiHttp,
        secret: &SecretString,
    ) -> Result<ValidatedAccount, ProviderError> {
        let resp = http
            .get(&format!("{}/v1/models", self.base.trim_end_matches('/')))?
            .bearer_auth(secret.expose_secret())
            .send()
            .await?;
        if resp.status().is_success() {
            Ok(ValidatedAccount { label: None })
        } else {
            Err(error_from_response(resp).await)
        }
    }
}
