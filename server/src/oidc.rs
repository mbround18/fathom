//! The hosted shell's OIDC relying party. The relying party itself is `web_auth::oidc` (spec 050,
//! T015); this wraps it with discovery from [`HostedConfig`].

use std::sync::Arc;

use secrecy::SecretString;
use web_auth::oidc::OidcProvider;
pub use web_auth::oidc::{
    AuthRequest, ClaimsConfig, HttpError, LoginState, OidcError, build_http_client, map_identity,
};

use crate::config::HostedConfig;
use crate::session::{Identity, Tokens};

/// [`OidcProvider`] configured from the hosted server's environment.
#[derive(Clone)]
pub struct OidcClient(Arc<OidcProvider>);

impl OidcClient {
    /// Discover provider metadata. Failure is fatal for startup (exit code 3, no port bound).
    pub async fn discover(cfg: &HostedConfig) -> Result<Self, OidcError> {
        let http = build_http_client(cfg.oidc.ca_file.as_deref())?;
        Self::discover_with(cfg, http).await
    }

    pub async fn discover_with(
        cfg: &HostedConfig,
        http: reqwest::Client,
    ) -> Result<Self, OidcError> {
        let redirect_url = url::Url::parse(&cfg.redirect_uri())
            .map_err(|e| OidcError::Setup(format!("redirect URI: {e}")))?;
        let mut post_logout_url = cfg.external_url.clone();
        post_logout_url.set_path("/");
        let oidc = web_auth::oidc::OidcConfig {
            issuer: cfg.oidc.issuer_raw.clone(),
            client_id: cfg.oidc.client_id.clone(),
            client_secret: cfg.oidc.client_secret.clone(),
            redirect_url,
            scopes: cfg.oidc.scopes.clone(),
            post_logout_url,
        };
        // `claims.groups_required` already follows the access model (impersonation needs groups).
        let provider = OidcProvider::discover(oidc, cfg.claims.clone(), http).await?;
        Ok(Self(Arc::new(provider)))
    }

    /// The provider this wraps.
    pub fn provider(&self) -> &OidcProvider {
        &self.0
    }

    pub fn issuer(&self) -> &str {
        self.0.issuer()
    }

    pub fn authorize(&self) -> AuthRequest {
        self.0.authorize()
    }

    pub async fn complete(
        &self,
        code: &str,
        returned_state: &str,
        login: &LoginState,
    ) -> Result<(Identity, Tokens), OidcError> {
        self.0.complete(code, returned_state, login).await
    }

    pub async fn refresh(&self, refresh_token: &SecretString) -> Result<Tokens, OidcError> {
        self.0.refresh(refresh_token).await
    }

    /// RP-initiated logout URL. In pass-through with ID-token bearers the ID token is the user's
    /// cluster credential, so the logout handler passes no hint then.
    pub fn end_session_url(&self, id_token_hint: Option<&SecretString>) -> Option<url::Url> {
        self.0.end_session_url(id_token_hint)
    }
}
