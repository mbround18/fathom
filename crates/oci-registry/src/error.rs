//! The crate's one error type. `code()` is the stable string apps store and show.
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("invalid image reference: {0}")]
    InvalidReference(String),
    #[error("the image reference is not by digest")]
    NotByDigest,
    #[error("the registry refused the credentials or token")]
    AuthFailed,
    #[error("the manifest was not found")]
    NotFound,
    /// A 429 (or Docker Hub's `TOOMANYREQUESTS`). `retry_after` is the response's `Retry-After`,
    /// when present and parsable.
    #[error("the registry rate limited the request")]
    RateLimited { retry_after: Option<Duration> },
    #[error("the registry request timed out")]
    Timeout,
    #[error("could not reach the registry")]
    Network,
    #[error("bad registry response: {0}")]
    BadResponse(&'static str),
    #[error("the registry sent no valid Docker-Content-Digest")]
    NoDigestHeader,
    #[error("no image for this platform")]
    NoImageForPlatform,
}

impl RegistryError {
    pub fn code(&self) -> &'static str {
        match self {
            RegistryError::InvalidReference(_) => "invalid_reference",
            RegistryError::NotByDigest => "not_by_digest",
            RegistryError::AuthFailed => "auth_failed",
            RegistryError::NotFound => "not_found",
            RegistryError::RateLimited { .. } => "rate_limited",
            RegistryError::Timeout => "timeout",
            RegistryError::Network => "network",
            RegistryError::BadResponse(_) => "bad_response",
            RegistryError::NoDigestHeader => "no_digest_header",
            RegistryError::NoImageForPlatform => "no_image_for_platform",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable() {
        let cases = [
            (RegistryError::InvalidReference("x".into()), "invalid_reference"),
            (RegistryError::NotByDigest, "not_by_digest"),
            (RegistryError::AuthFailed, "auth_failed"),
            (RegistryError::NotFound, "not_found"),
            (
                RegistryError::RateLimited {
                    retry_after: Some(Duration::from_secs(3)),
                },
                "rate_limited",
            ),
            (RegistryError::Timeout, "timeout"),
            (RegistryError::Network, "network"),
            (RegistryError::BadResponse("x"), "bad_response"),
            (RegistryError::NoDigestHeader, "no_digest_header"),
            (RegistryError::NoImageForPlatform, "no_image_for_platform"),
        ];
        for (e, code) in cases {
            assert_eq!(e.code(), code);
        }
    }
}
