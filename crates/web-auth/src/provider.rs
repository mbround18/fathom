//! The seam every sign-in provider implements (spec 050): OIDC ([`OidcProvider`]), GitHub
//! ([`crate::github`]) and Discord ([`crate::discord`]). `tests/common/mod.rs` holds the
//! conformance test each implementation must pass.

use std::time::Duration;

use async_trait::async_trait;
use openidconnect::RedirectUrl;
use url::Url;

use crate::oidc::{LoginState, OidcError, OidcProvider};
use crate::session::{Identity, Session, Tokens};

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// The callback's `state` is not the one this login was started with.
    #[error("state mismatch")]
    StateMismatch,
    /// The provider signed the person in but their identity is unusable (a required claim is
    /// missing): a sign-in error to show, not an outage.
    #[error("sign-in refused: {0}")]
    Refused(String),
    /// The provider answered no (revoked grant, bad code): the session, if any, must end.
    #[error("rejected by the provider: {0}")]
    Rejected(String),
    /// The provider could not be reached (connect error, timeout, 5xx) or asked to try later.
    #[error("provider temporarily unavailable: {0}")]
    Transient(String),
    /// The provider rate-limited us (429): unreachable for `retry_after`.
    #[error("provider rate-limited the request; retry after {0:?}")]
    RateLimited(Duration),
    /// Anything else: a malformed, oversized or unexpected answer, or a misconfiguration.
    #[error("{0}")]
    Failed(String),
}

impl ProviderError {
    /// The provider could not answer now. A revalidation that fails this way keeps the session
    /// and retries; every other failure ends it (fail closed).
    pub fn is_unreachable(&self) -> bool {
        matches!(self, Self::Transient(_) | Self::RateLimited(_))
    }

    /// How long the provider asked us to wait, when it said.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RateLimited(d) => Some(*d),
            _ => None,
        }
    }
}

/// The outcome of [`IdentityProvider::revalidate`], for the server's refresh loop (T072).
#[derive(Debug)]
pub enum Revalidation {
    /// Still good: store `tokens` (and the identity, if re-read) on the session.
    Valid(Refreshed),
    /// The provider said no (revoked grant, 401/403, the account is now someone else, or an
    /// answer that cannot be trusted): end the session now.
    Rejected(ProviderError),
    /// The provider could not be reached: keep the session until it expires, retry after
    /// `retry_after` (the provider's, when it gave one), and warn.
    Unreachable {
        error: ProviderError,
        retry_after: Option<Duration>,
    },
}

pub type Result<T> = std::result::Result<T, ProviderError>;

/// Secrets kept between [`IdentityProvider::authorize`] and [`IdentityProvider::complete`].
/// `Debug` is redacted.
#[derive(Clone)]
pub struct PendingLogin {
    pub state: String,
    /// OIDC only; OAuth providers without ID tokens have none.
    pub nonce: Option<String>,
    pub pkce_verifier: String,
    /// The callback the login was started with; the code exchange must send the same one.
    pub redirect: Url,
}

impl std::fmt::Debug for PendingLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingLogin")
            .field("secrets", &"[redacted]")
            .field("redirect", &self.redirect.as_str())
            .finish()
    }
}

/// Where to send the browser, and what to remember until it comes back.
#[derive(Debug, Clone)]
pub struct AuthRedirect {
    pub url: Url,
    pub pending: PendingLogin,
}

#[derive(Debug, Clone)]
pub struct Completed {
    pub identity: Identity,
    pub tokens: Tokens,
}

#[derive(Debug, Clone)]
pub struct Refreshed {
    pub tokens: Tokens,
    /// The identity as the provider sees it now (OIDC re-reads the groups), when it says.
    pub identity: Option<Identity>,
}

#[async_trait]
pub trait IdentityProvider: Send + Sync {
    /// `oidc`, `github` or `discord`: matches [`crate::session::Provider::as_str`].
    fn id(&self) -> &'static str;
    /// Start a login whose callback is `redirect`.
    async fn authorize(&self, redirect: &Url) -> Result<AuthRedirect>;
    /// Finish it: check `state`, exchange `code`, and say who signed in.
    async fn complete(&self, pending: PendingLogin, code: &str, state: &str) -> Result<Completed>;
    /// Revalidate the session's provider token, refreshing it first if it has expired.
    /// [`ProviderError::is_unreachable`] errors keep the session; any other error ends it.
    async fn refresh(&self, s: &Session) -> Result<Refreshed>;
    /// [`Self::refresh`], classified for the refresh loop. An identity whose subject differs
    /// from the session's is rejected.
    async fn revalidate(&self, s: &Session) -> Revalidation {
        match self.refresh(s).await {
            Ok(r) => match &r.identity {
                Some(i) if i.sub != s.identity.sub || i.provider != s.identity.provider => {
                    Revalidation::Rejected(ProviderError::Rejected(
                        "the provider now names a different account".into(),
                    ))
                }
                _ => Revalidation::Valid(r),
            },
            Err(e) if e.is_unreachable() => Revalidation::Unreachable {
                retry_after: e.retry_after(),
                error: e,
            },
            Err(e) => Revalidation::Rejected(e),
        }
    }
    /// Where to send the browser to sign out at the provider too, if it supports that.
    fn end_session_url(&self, s: &Session) -> Option<Url>;
}

impl From<OidcError> for ProviderError {
    fn from(e: OidcError) -> Self {
        match e {
            OidcError::StateMismatch => ProviderError::StateMismatch,
            OidcError::MissingUsername(_) | OidcError::MissingGroups(_) => {
                ProviderError::Refused(e.to_string())
            }
            OidcError::RefreshRejected(_) | OidcError::NoRefreshToken => {
                ProviderError::Rejected(e.to_string())
            }
            OidcError::RefreshTransient(_) => ProviderError::Transient(e.to_string()),
            other => ProviderError::Failed(other.to_string()),
        }
    }
}

fn redirect_url(u: &Url) -> Result<RedirectUrl> {
    RedirectUrl::new(u.to_string()).map_err(|e| ProviderError::Failed(e.to_string()))
}

#[async_trait]
impl IdentityProvider for OidcProvider {
    fn id(&self) -> &'static str {
        "oidc"
    }

    async fn authorize(&self, redirect: &Url) -> Result<AuthRedirect> {
        let req = self.authorize_with(redirect_url(redirect)?);
        Ok(AuthRedirect {
            url: req.url,
            pending: PendingLogin {
                state: req.login.state,
                nonce: Some(req.login.nonce),
                pkce_verifier: req.login.pkce_verifier,
                redirect: redirect.clone(),
            },
        })
    }

    async fn complete(&self, pending: PendingLogin, code: &str, state: &str) -> Result<Completed> {
        let redirect = redirect_url(&pending.redirect)?;
        let login = LoginState {
            state: pending.state,
            nonce: pending
                .nonce
                .ok_or_else(|| ProviderError::Failed("OIDC login without a nonce".into()))?,
            pkce_verifier: pending.pkce_verifier,
        };
        let (identity, tokens) = self.complete_with(redirect, code, state, &login).await?;
        Ok(Completed { identity, tokens })
    }

    /// The refresh-token grant, then the refreshed ID token's claims mapped again. Claims that
    /// no longer map (a required groups claim gone) end the session.
    async fn refresh(&self, s: &Session) -> Result<Refreshed> {
        let rt = s
            .tokens
            .refresh_token
            .as_ref()
            .ok_or(OidcError::NoRefreshToken)?;
        let tokens = OidcProvider::refresh(self, rt).await?;
        let identity = self
            .identity_from_verified(secrecy::ExposeSecret::expose_secret(&tokens.id_token))
            .map_err(|e| ProviderError::Rejected(e.to_string()))?;
        Ok(Refreshed {
            tokens,
            identity: Some(identity),
        })
    }

    /// Sends the ID token as `id_token_hint`. An app whose ID token is a live credential
    /// elsewhere (Fathom's pass-through) calls [`OidcProvider::end_session_url`] without one.
    fn end_session_url(&self, s: &Session) -> Option<Url> {
        OidcProvider::end_session_url(self, Some(&s.tokens.id_token))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_login_debug_is_redacted() {
        let p = PendingLogin {
            state: "STATE-SECRET".into(),
            nonce: Some("NONCE-SECRET".into()),
            pkce_verifier: "PKCE-SECRET".into(),
            redirect: Url::parse("https://a.test/cb").unwrap(),
        };
        let d = format!("{p:?}");
        assert!(!d.contains("SECRET"), "{d}");
        assert!(d.contains("https://a.test/cb"));
    }

    #[test]
    fn oidc_errors_map_to_provider_errors() {
        use ProviderError as P;
        assert!(matches!(
            P::from(OidcError::StateMismatch),
            P::StateMismatch
        ));
        assert!(matches!(
            P::from(OidcError::MissingGroups("g".into())),
            P::Refused(_)
        ));
        assert!(matches!(
            P::from(OidcError::RefreshRejected("invalid_grant".into())),
            P::Rejected(_)
        ));
        assert!(matches!(P::from(OidcError::NoRefreshToken), P::Rejected(_)));
        assert!(matches!(
            P::from(OidcError::RefreshTransient("x".into())),
            P::Transient(_)
        ));
        assert!(matches!(
            P::from(OidcError::Exchange("x".into())),
            P::Failed(_)
        ));
    }
}
