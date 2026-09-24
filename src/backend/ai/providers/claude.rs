//! Claude (Anthropic) key validation: `GET /v1/models` with the operator's key.

use secrecy::{ExposeSecret, SecretString};

use super::{ProviderAdapter, ProviderError, ValidatedAccount, error_from_response};
use crate::backend::ai::http::AiHttp;

pub(crate) const DEFAULT_BASE: &str = "https://api.anthropic.com";
pub(crate) const ANTHROPIC_VERSION: &str = "2023-06-01";

pub(crate) struct ClaudeAdapter {
    base: String,
}

impl ClaudeAdapter {
    pub(crate) fn new(base: impl Into<String>) -> Self {
        Self { base: base.into() }
    }
}

#[async_trait::async_trait]
impl ProviderAdapter for ClaudeAdapter {
    async fn validate(
        &self,
        http: &AiHttp,
        secret: &SecretString,
    ) -> Result<ValidatedAccount, ProviderError> {
        let resp = http
            .get(&format!("{}/v1/models", self.base.trim_end_matches('/')))?
            .header("x-api-key", secret.expose_secret())
            .header("anthropic-version", ANTHROPIC_VERSION)
            .send()
            .await?;
        if resp.status().is_success() {
            Ok(ValidatedAccount { label: None })
        } else {
            Err(error_from_response(resp).await)
        }
    }
}
