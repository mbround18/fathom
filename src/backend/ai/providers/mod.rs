//! Provider adapters (038-sdk-login-ollama, FR-001). One trait; GitHub adds a device flow
//! on top, Claude/OpenAI are API-key validators. An official SDK or OAuth flow can replace
//! an adapter without any UI change.

pub(crate) mod claude;
pub(crate) mod github;
pub(crate) mod openai;

use reqwest::Response;
use secrecy::SecretString;

use crate::backend::ai::http::{AiHttp, AiHttpError};

/// What a successful credential validation tells us (never the secret).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ValidatedAccount {
    pub label: Option<String>,
}

/// Why a provider call failed. Bodies are the provider's own text, kept verbatim
/// (Principle I / FR-015).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderError {
    /// 401/403: the credential is invalid, revoked or expired.
    Rejected {
        status: u16,
        body: String,
    },
    /// 429, with the `retry-after` header when present.
    RateLimited {
        retry_after: Option<String>,
        body: String,
    },
    Http {
        status: u16,
        body: String,
    },
    /// Offline mode refused the request; nothing was sent.
    Offline(String),
    Network(String),
}

impl ProviderError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            ProviderError::Rejected { .. } => "credential_rejected",
            ProviderError::RateLimited { .. } => "rate_limited",
            ProviderError::Http { .. } => "provider_http_error",
            ProviderError::Offline(_) => "offline_blocked",
            ProviderError::Network(_) => "network_error",
        }
    }

    pub(crate) fn message_verbatim(&self) -> String {
        match self {
            ProviderError::Rejected { status, body } => format!("HTTP {status}: {body}"),
            ProviderError::RateLimited { retry_after, body } => match retry_after {
                Some(after) => format!("HTTP 429 (retry-after: {after}): {body}"),
                None => format!("HTTP 429: {body}"),
            },
            ProviderError::Http { status, body } => format!("HTTP {status}: {body}"),
            ProviderError::Offline(msg) | ProviderError::Network(msg) => msg.clone(),
        }
    }

    pub(crate) fn hint(&self) -> String {
        match self {
            ProviderError::Rejected { .. } => {
                "The provider rejected this credential. Check it and try again, or re-authenticate."
                    .to_owned()
            }
            ProviderError::RateLimited { retry_after, .. } => match retry_after {
                Some(after) => format!("Rate limited by the provider. Retry after {after}."),
                None => "Rate limited by the provider. Wait a moment and retry.".to_owned(),
            },
            ProviderError::Http { .. } => "The provider returned an unexpected error.".to_owned(),
            ProviderError::Offline(_) => {
                "Turn off Offline mode to sign in to hosted providers.".to_owned()
            }
            ProviderError::Network(_) => {
                "Could not reach the provider. Check your network connection and retry.".to_owned()
            }
        }
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message_verbatim())
    }
}

impl std::error::Error for ProviderError {}

impl From<AiHttpError> for ProviderError {
    fn from(err: AiHttpError) -> Self {
        if err.is_offline_blocked() {
            ProviderError::Offline(err.to_string())
        } else {
            ProviderError::Network(err.to_string())
        }
    }
}

/// Turn a non-2xx response into the matching [`ProviderError`], reading the body verbatim.
pub(crate) async fn error_from_response(resp: Response) -> ProviderError {
    let status = resp.status().as_u16();
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = resp.text().await.unwrap_or_default();
    match status {
        401 | 403 => ProviderError::Rejected { status, body },
        429 => ProviderError::RateLimited { retry_after, body },
        _ => ProviderError::Http { status, body },
    }
}

/// Common adapter interface (T007).
#[async_trait::async_trait]
pub(crate) trait ProviderAdapter: Send + Sync {
    /// Cheap authenticated call proving the credential works. Also used to detect a token
    /// that has since become invalid (NeedsReauth).
    async fn validate(
        &self,
        http: &AiHttp,
        secret: &SecretString,
    ) -> Result<ValidatedAccount, ProviderError>;
}
