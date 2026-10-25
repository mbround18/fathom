//! GitHub OAuth App sign-in (spec 050, research R5, T071).
//!
//! - Authorize at `github.com/login/oauth/authorize` with `state` and PKCE S256, exchange at
//!   `github.com/login/oauth/access_token`.
//! - Scopes `read:user user:email`. Identity from `GET /user`: the subject is the numeric `id`,
//!   never the login (logins can be renamed and re-registered). The email is the primary
//!   *verified* one from `GET /user/emails`, or none. No org or team calls.
//! - Classic OAuth tokens do not expire: [`IdentityProvider::refresh`] calls `GET /user`, and a
//!   401 (the person revoked the app) is rejected.
//!
//! The endpoints are fixed; only this crate's tests can point them elsewhere.

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use std::time::Duration;
use url::Url;

use crate::oauth::{self, Timeouts};
use crate::provider::{
    AuthRedirect, Completed, IdentityProvider, PendingLogin, ProviderError, Refreshed, Result,
};
use crate::session::{Identity, Provider, Session};

pub const AUTHORIZE_URL: &str = "https://github.com/login/oauth/authorize";
pub const TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
pub const API_URL: &str = "https://api.github.com";
/// The identity's issuer; part of [`Identity::user_key`], so it never changes.
pub const ISSUER: &str = "https://github.com";
pub const SCOPES: &str = "read:user user:email";
const API_VERSION: (&str, &str) = ("x-github-api-version", "2022-11-28");
/// Classic OAuth tokens do not expire; the session's own lifetime bounds them.
const NO_EXPIRY: Duration = Duration::from_secs(10 * 365 * 86_400);

/// A GitHub OAuth App. `Debug` redacts the secret.
#[derive(Clone)]
pub struct GithubConfig {
    pub client_id: String,
    pub client_secret: SecretString,
    /// The registered callback, normally [`oauth::callback_url`]`(public_url, "github")`.
    pub redirect_url: Url,
}

impl std::fmt::Debug for GithubConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GithubConfig")
            .field("client_id", &self.client_id)
            .field("client_secret", &"[redacted]")
            .field("redirect_url", &self.redirect_url.as_str())
            .finish()
    }
}

/// Where the provider's endpoints live. Production uses the constants above.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub authorize: Url,
    pub token: Url,
    /// API base, without a trailing slash.
    pub api: Url,
}

impl Endpoints {
    fn github() -> Self {
        Self {
            authorize: Url::parse(AUTHORIZE_URL).expect("constant"),
            token: Url::parse(TOKEN_URL).expect("constant"),
            api: Url::parse(API_URL).expect("constant"),
        }
    }
}

#[derive(Clone)]
pub struct GithubProvider {
    cfg: GithubConfig,
    ep: Endpoints,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct User {
    /// A JSON number; a string (or anything else) is refused.
    #[serde(default)]
    id: serde_json::Value,
    login: Option<String>,
}

#[derive(Deserialize)]
struct Email {
    email: String,
    primary: bool,
    verified: bool,
}

impl GithubProvider {
    pub fn new(cfg: GithubConfig) -> Result<Self> {
        if cfg.client_id.is_empty() || cfg.client_secret.expose_secret().is_empty() {
            return Err(ProviderError::Failed(
                "GitHub client id and secret are required".into(),
            ));
        }
        Ok(Self {
            cfg,
            ep: Endpoints::github(),
            http: oauth::build_oauth_client(Timeouts::default(), false)?,
        })
    }

    /// Point at a fake GitHub (this crate's tests only), with the client the test built.
    #[cfg(feature = "insecure-test-endpoints")]
    pub fn with_endpoints(cfg: GithubConfig, ep: Endpoints, http: reqwest::Client) -> Self {
        Self { cfg, ep, http }
    }

    fn api(&self, path: &str) -> Url {
        let base = self.ep.api.as_str().trim_end_matches('/');
        Url::parse(&format!("{base}{path}")).expect("API base is a URL")
    }

    async fn user(&self, token: &SecretString) -> Result<(String, String)> {
        let u: User = oauth::get_json(&self.http, self.api("/user"), token, &[API_VERSION]).await?;
        // `as_u64` refuses strings, negatives and fractions; numeric_subject refuses 0.
        let sub =
            oauth::numeric_subject(&u.id.as_u64().map(|n| n.to_string()).unwrap_or_default())?;
        let login = oauth::display_name(u.login.as_deref())?;
        Ok((sub, login))
    }

    /// The primary verified email. A token without `user:email` (403/404) has none.
    async fn email(&self, token: &SecretString) -> Result<Option<String>> {
        let r: Result<Vec<Email>> =
            oauth::get_json(&self.http, self.api("/user/emails"), token, &[API_VERSION]).await;
        match r {
            Ok(list) => Ok(list
                .into_iter()
                .find(|e| e.primary && e.verified)
                .map(|e| e.email)),
            Err(ProviderError::Rejected(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn identity(sub: String, login: String, email: Option<String>) -> Identity {
        Identity {
            provider: Provider::Github,
            issuer: ISSUER.into(),
            sub,
            username: login,
            groups: vec![],
            email,
        }
    }
}

#[async_trait]
impl IdentityProvider for GithubProvider {
    fn id(&self) -> &'static str {
        "github"
    }

    async fn authorize(&self, redirect: &Url) -> Result<AuthRedirect> {
        oauth::check_redirect(&self.cfg.redirect_url, redirect)?;
        let (pending, challenge) = oauth::new_login(redirect);
        let mut url = self.ep.authorize.clone();
        url.query_pairs_mut()
            .append_pair("client_id", &self.cfg.client_id)
            .append_pair("redirect_uri", redirect.as_str())
            .append_pair("scope", SCOPES)
            .append_pair("state", &pending.state)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");
        Ok(AuthRedirect { url, pending })
    }

    async fn complete(&self, pending: PendingLogin, code: &str, state: &str) -> Result<Completed> {
        oauth::check_state(&pending, state)?;
        oauth::check_redirect(&self.cfg.redirect_url, &pending.redirect)?;
        let body = oauth::form(&[
            ("client_id", &self.cfg.client_id),
            ("client_secret", self.cfg.client_secret.expose_secret()),
            ("code", code),
            ("redirect_uri", pending.redirect.as_str()),
            ("code_verifier", &pending.pkce_verifier),
        ]);
        let (tokens, _) = oauth::token_request(&self.http, &self.ep.token, body)
            .await?
            .into_tokens(None, NO_EXPIRY)?;
        let (sub, login) = self.user(&tokens.access_token).await?;
        let email = self.email(&tokens.access_token).await?;
        Ok(Completed {
            identity: Self::identity(sub, login, email),
            tokens,
        })
    }

    /// `GET /user` with the session's token. The login may have changed (a rename); the
    /// subject must not ([`IdentityProvider::revalidate`] rejects that). The email is kept.
    async fn refresh(&self, s: &Session) -> Result<Refreshed> {
        let (sub, login) = self.user(&s.tokens.access_token).await?;
        Ok(Refreshed {
            tokens: s.tokens.clone(),
            identity: Some(Self::identity(sub, login, s.identity.email.clone())),
        })
    }

    /// GitHub has no sign-out endpoint for OAuth Apps.
    fn end_session_url(&self, _s: &Session) -> Option<Url> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> GithubConfig {
        GithubConfig {
            client_id: "cid".into(),
            client_secret: SecretString::from("CLIENT-SECRET".to_string()),
            redirect_url: Url::parse("https://d.test/auth/callback/github").unwrap(),
        }
    }

    #[test]
    fn production_endpoints_are_fixed_https() {
        let p = GithubProvider::new(cfg()).unwrap();
        assert_eq!(p.ep.authorize.as_str(), AUTHORIZE_URL);
        assert_eq!(p.ep.token.as_str(), TOKEN_URL);
        assert_eq!(p.api("/user").as_str(), "https://api.github.com/user");
    }

    #[test]
    fn config_debug_hides_the_secret() {
        assert!(!format!("{:?}", cfg()).contains("CLIENT-SECRET"));
    }

    #[test]
    fn empty_credentials_refuse() {
        let mut c = cfg();
        c.client_secret = SecretString::from(String::new());
        assert!(GithubProvider::new(c).is_err());
    }

    #[tokio::test]
    async fn authorize_refuses_an_unregistered_callback() {
        let p = GithubProvider::new(cfg()).unwrap();
        let other = Url::parse("https://evil.test/auth/callback/github").unwrap();
        assert!(matches!(
            p.authorize(&other).await,
            Err(ProviderError::Failed(_))
        ));
        let a = p.authorize(&cfg().redirect_url).await.unwrap();
        let q: std::collections::HashMap<_, _> = a.url.query_pairs().into_owned().collect();
        assert_eq!(q["scope"], SCOPES);
        assert!(!a.url.as_str().contains("CLIENT-SECRET"));
    }
}
