//! Discord OAuth2 sign-in (spec 050, research R13, T088). Plain OAuth2, not Discord's OIDC.
//!
//! - Authorize at `discord.com/oauth2/authorize`, exchange and refresh at
//!   `discord.com/api/oauth2/token`. `state` always, checked in constant time before any
//!   exchange; PKCE S256 is sent too, but nothing relies on it alone (the client secret and
//!   `state` carry that).
//! - The redirect URI is exactly `<public_url>/auth/callback/discord`.
//! - Scope `identify`, plus `email` when asked for. Identity from `GET /users/@me`: the subject
//!   is the user id snowflake. An email is kept only when Discord says it is verified. No guild
//!   calls.
//! - Refresh: the refresh token when the access token has expired, then `GET /users/@me`.
//!   401/`invalid_grant` is rejected (end the session); 429 and 5xx are unreachable, and a 429
//!   carries Discord's `retry_after`.
//!
//! The endpoints are fixed; only this crate's tests can point them elsewhere.

use std::time::Duration;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use url::Url;

use crate::oauth::{self, Timeouts};
use crate::provider::{
    AuthRedirect, Completed, IdentityProvider, PendingLogin, ProviderError, Refreshed, Result,
};
use crate::session::{Identity, Provider, Session, Tokens};

pub const AUTHORIZE_URL: &str = "https://discord.com/oauth2/authorize";
pub const TOKEN_URL: &str = "https://discord.com/api/oauth2/token";
pub const API_URL: &str = "https://discord.com/api/v10";
/// The identity's issuer; part of [`Identity::user_key`], so it never changes.
pub const ISSUER: &str = "https://discord.com";
/// Discord's documented access-token lifetime, used when a response omits `expires_in`.
const DEFAULT_LIFETIME: Duration = Duration::from_secs(604_800);

/// A Discord OAuth2 application. `Debug` redacts the secret.
#[derive(Clone)]
pub struct DiscordConfig {
    pub client_id: String,
    pub client_secret: SecretString,
    /// The app's external URL; the redirect URI is `<public_url>/auth/callback/discord`.
    pub public_url: Url,
    /// Ask for the `email` scope too. Sign-in needs only `identify`.
    pub request_email: bool,
}

impl std::fmt::Debug for DiscordConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscordConfig")
            .field("client_id", &self.client_id)
            .field("client_secret", &"[redacted]")
            .field("public_url", &self.public_url.as_str())
            .field("request_email", &self.request_email)
            .finish()
    }
}

/// Where the provider's endpoints live. Production uses the constants above.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub authorize: Url,
    pub token: Url,
    /// API base (`.../api/v10`), without a trailing slash.
    pub api: Url,
}

impl Endpoints {
    fn discord() -> Self {
        Self {
            authorize: Url::parse(AUTHORIZE_URL).expect("constant"),
            token: Url::parse(TOKEN_URL).expect("constant"),
            api: Url::parse(API_URL).expect("constant"),
        }
    }
}

#[derive(Clone)]
pub struct DiscordProvider {
    cfg: DiscordConfig,
    redirect: Url,
    ep: Endpoints,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct User {
    /// A snowflake, sent as a string of digits.
    #[serde(default)]
    id: serde_json::Value,
    username: Option<String>,
    email: Option<String>,
    verified: Option<bool>,
}

impl DiscordProvider {
    pub fn new(cfg: DiscordConfig) -> Result<Self> {
        Self::build(
            cfg,
            Endpoints::discord(),
            oauth::build_oauth_client(Timeouts::default(), false)?,
        )
    }

    /// Point at a fake Discord (this crate's tests only), with the client the test built.
    #[cfg(feature = "insecure-test-endpoints")]
    pub fn with_endpoints(
        cfg: DiscordConfig,
        ep: Endpoints,
        http: reqwest::Client,
    ) -> Result<Self> {
        Self::build(cfg, ep, http)
    }

    fn build(cfg: DiscordConfig, ep: Endpoints, http: reqwest::Client) -> Result<Self> {
        if cfg.client_id.is_empty() || cfg.client_secret.expose_secret().is_empty() {
            return Err(ProviderError::Failed(
                "Discord client id and secret are required".into(),
            ));
        }
        let redirect = oauth::callback_url(&cfg.public_url, "discord")?;
        Ok(Self {
            cfg,
            redirect,
            ep,
            http,
        })
    }

    /// The redirect URI to register in the Discord developer portal.
    pub fn redirect_url(&self) -> &Url {
        &self.redirect
    }

    pub fn scopes(&self) -> &'static str {
        if self.cfg.request_email {
            "identify email"
        } else {
            "identify"
        }
    }

    fn api(&self, path: &str) -> Url {
        let base = self.ep.api.as_str().trim_end_matches('/');
        Url::parse(&format!("{base}{path}")).expect("API base is a URL")
    }

    async fn token(&self, grant: &[(&str, &str)], prev: Option<&SecretString>) -> Result<Tokens> {
        let mut pairs: Vec<(&str, &str)> = grant.to_vec();
        pairs.push(("client_id", &self.cfg.client_id));
        pairs.push(("client_secret", self.cfg.client_secret.expose_secret()));
        let (tokens, scope) = oauth::token_request(&self.http, &self.ep.token, oauth::form(&pairs))
            .await?
            .into_tokens(prev, DEFAULT_LIFETIME)?;
        // The person can untick scopes on Discord's consent screen; `identify` is required.
        if let Some(scope) = scope
            && !scope.split_whitespace().any(|s| s == "identify")
        {
            return Err(ProviderError::Refused(
                "the Discord grant does not include the identify scope".into(),
            ));
        }
        Ok(tokens)
    }

    async fn me(&self, token: &SecretString) -> Result<Identity> {
        let u: User = oauth::get_json(&self.http, self.api("/users/@me"), token, &[]).await?;
        let sub = oauth::numeric_subject(u.id.as_str().unwrap_or_default())?;
        let username = oauth::display_name(u.username.as_deref())?;
        // An unverified address is someone's claim, not an identity: never keep it.
        let email = (self.cfg.request_email && u.verified == Some(true))
            .then_some(u.email)
            .flatten()
            .filter(|e| !e.is_empty());
        Ok(Identity {
            provider: Provider::Discord,
            issuer: ISSUER.into(),
            sub,
            username,
            groups: vec![],
            email,
        })
    }
}

#[async_trait]
impl IdentityProvider for DiscordProvider {
    fn id(&self) -> &'static str {
        "discord"
    }

    async fn authorize(&self, redirect: &Url) -> Result<AuthRedirect> {
        oauth::check_redirect(&self.redirect, redirect)?;
        let (pending, challenge) = oauth::new_login(redirect);
        let mut url = self.ep.authorize.clone();
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.cfg.client_id)
            .append_pair("scope", self.scopes())
            .append_pair("redirect_uri", redirect.as_str())
            .append_pair("state", &pending.state)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");
        Ok(AuthRedirect { url, pending })
    }

    async fn complete(&self, pending: PendingLogin, code: &str, state: &str) -> Result<Completed> {
        oauth::check_state(&pending, state)?;
        oauth::check_redirect(&self.redirect, &pending.redirect)?;
        let tokens = self
            .token(
                &[
                    ("grant_type", "authorization_code"),
                    ("code", code),
                    ("redirect_uri", pending.redirect.as_str()),
                    ("code_verifier", &pending.pkce_verifier),
                ],
                None,
            )
            .await?;
        let identity = self.me(&tokens.access_token).await?;
        Ok(Completed { identity, tokens })
    }

    async fn refresh(&self, s: &Session) -> Result<Refreshed> {
        let tokens = match oauth::live_access_token(s) {
            Some(_) => s.tokens.clone(),
            None => {
                let rt = s.tokens.refresh_token.as_ref().ok_or_else(|| {
                    ProviderError::Rejected("access token expired and no refresh token".into())
                })?;
                self.token(
                    &[
                        ("grant_type", "refresh_token"),
                        ("refresh_token", rt.expose_secret()),
                    ],
                    Some(rt),
                )
                .await?
            }
        };
        let identity = self.me(&tokens.access_token).await?;
        Ok(Refreshed {
            tokens,
            identity: Some(identity),
        })
    }

    /// Discord has no browser sign-out endpoint.
    fn end_session_url(&self, _s: &Session) -> Option<Url> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(email: bool) -> DiscordConfig {
        DiscordConfig {
            client_id: "cid".into(),
            client_secret: SecretString::from("CLIENT-SECRET".to_string()),
            public_url: Url::parse("https://depth.example.test").unwrap(),
            request_email: email,
        }
    }

    #[test]
    fn production_endpoints_are_fixed_https() {
        let p = DiscordProvider::new(cfg(true)).unwrap();
        assert_eq!(p.ep.authorize.as_str(), AUTHORIZE_URL);
        assert_eq!(p.ep.token.as_str(), TOKEN_URL);
        assert_eq!(
            p.api("/users/@me").as_str(),
            "https://discord.com/api/v10/users/@me"
        );
    }

    #[test]
    fn redirect_is_exactly_the_discord_callback() {
        let p = DiscordProvider::new(cfg(true)).unwrap();
        assert_eq!(
            p.redirect_url().as_str(),
            "https://depth.example.test/auth/callback/discord"
        );
    }

    #[test]
    fn scopes_need_only_identify() {
        assert_eq!(
            DiscordProvider::new(cfg(true)).unwrap().scopes(),
            "identify email"
        );
        assert_eq!(
            DiscordProvider::new(cfg(false)).unwrap().scopes(),
            "identify"
        );
    }

    #[test]
    fn config_debug_hides_the_secret() {
        assert!(!format!("{:?}", cfg(true)).contains("CLIENT-SECRET"));
    }

    #[tokio::test]
    async fn authorize_refuses_another_callback() {
        let p = DiscordProvider::new(cfg(true)).unwrap();
        for bad in [
            "https://depth.example.test/auth/callback",
            "https://depth.example.test/auth/callback/discord/",
            "https://evil.test/auth/callback/discord",
        ] {
            let u = Url::parse(bad).unwrap();
            assert!(p.authorize(&u).await.is_err(), "{bad}");
        }
        let a = p.authorize(&p.redirect_url().clone()).await.unwrap();
        assert!(!a.url.as_str().contains("CLIENT-SECRET"));
        assert!(a.url.as_str().starts_with(AUTHORIZE_URL));
    }
}
