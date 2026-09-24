//! OIDC relying party for the hosted shell (spec 031): discovery (fail closed), auth-code + PKCE
//! S256, state/nonce, ID-token validation, claim -> Identity mapping, refresh rotation,
//! RP-initiated logout, custom CA.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use openidconnect::core::{CoreClient, CoreIdToken, CoreResponseType};
use openidconnect::{
    AuthenticationFlow, AuthorizationCode, ClientId, ClientSecret, CsrfToken, HttpRequest,
    HttpResponse, IssuerUrl, Nonce, OAuth2TokenResponse, PkceCodeChallenge, PkceCodeVerifier,
    RedirectUrl, RefreshToken, Scope, TokenResponse,
};
use openidconnect::{EndSessionUrl, ProviderMetadataWithLogout};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tokio::time::Instant;

use crate::config::{ClaimsConfig, HostedConfig};
use crate::session::{Identity, Tokens};

#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    #[error("OIDC discovery failed: {0}")]
    Discovery(String),
    #[error("OIDC client setup failed: {0}")]
    Setup(String),
    #[error("state mismatch")]
    StateMismatch,
    #[error("token exchange failed: {0}")]
    Exchange(String),
    #[error("ID token invalid: {0}")]
    IdToken(String),
    #[error("claim `{0}` missing or not a string")]
    MissingUsername(String),
    #[error("groups claim `{0}` is missing from the ID token; configure the IdP to emit it")]
    MissingGroups(String),
    /// The refresh token was rejected (`invalid_grant`, revoked): the session must end.
    #[error("refresh rejected: {0}")]
    RefreshRejected(String),
    /// Transient failure (network, 5xx): retry within the grace window.
    #[error("refresh temporarily failed: {0}")]
    RefreshTransient(String),
    #[error("no refresh token available")]
    NoRefreshToken,
}

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("http: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("http: {0}")]
    Http(#[from] http::Error),
}

/// Per-login secrets kept between `/auth/login` and `/auth/callback`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginState {
    pub state: String,
    pub nonce: String,
    pub pkce_verifier: String,
}

pub struct AuthRequest {
    pub url: url::Url,
    pub login: LoginState,
}

/// Build the reqwest client used for every IdP call (no redirects; optional extra CA).
pub fn build_http_client(ca_file: Option<&std::path::Path>) -> Result<reqwest::Client, OidcError> {
    let mut b = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15));
    if let Some(p) = ca_file {
        let pem =
            std::fs::read(p).map_err(|e| OidcError::Setup(format!("{}: {e}", p.display())))?;
        let cert = reqwest::Certificate::from_pem(&pem)
            .map_err(|e| OidcError::Setup(format!("{}: {e}", p.display())))?;
        b = b.add_root_certificate(cert);
    }
    b.build().map_err(|e| OidcError::Setup(e.to_string()))
}

async fn call(client: &reqwest::Client, request: HttpRequest) -> Result<HttpResponse, HttpError> {
    let response = client.execute(request.try_into()?).await?;
    let mut builder = http::Response::builder().status(response.status());
    for (n, v) in response.headers() {
        builder = builder.header(n, v);
    }
    Ok(builder.body(response.bytes().await?.to_vec())?)
}

#[derive(Clone)]
pub struct OidcClient {
    meta: ProviderMetadataWithLogout,
    http: reqwest::Client,
    client_id: String,
    client_secret: Option<SecretString>,
    redirect: RedirectUrl,
    scopes: Vec<String>,
    claims: ClaimsConfig,
    issuer: String,
    end_session: Option<EndSessionUrl>,
    post_logout: url::Url,
}

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
        // Verbatim: the discovery document's `issuer` must equal this exactly, trailing slash
        // included (Rauthy's has none, some IdPs' do). Never normalized.
        let issuer = IssuerUrl::new(cfg.oidc.issuer_raw.clone())
            .map_err(|e| OidcError::Discovery(e.to_string()))?;
        let h = http.clone();
        let meta = ProviderMetadataWithLogout::discover_async(issuer, &move |r: HttpRequest| {
            let h = h.clone();
            async move { call(&h, r).await }
        })
        .await
        .map_err(|e| OidcError::Discovery(format!("{} ({e})", cfg.oidc.issuer)))?;
        let end_session = meta.additional_metadata().end_session_endpoint.clone();
        let redirect =
            RedirectUrl::new(cfg.redirect_uri()).map_err(|e| OidcError::Setup(e.to_string()))?;
        let mut post_logout = cfg.external_url.clone();
        post_logout.set_path("/");
        Ok(Self {
            issuer: meta.issuer().to_string(),
            meta,
            http,
            client_id: cfg.oidc.client_id.clone(),
            client_secret: cfg.oidc.client_secret.clone(),
            redirect,
            scopes: cfg.oidc.scopes.clone(),
            claims: cfg.claims.clone(),
            end_session,
            post_logout,
        })
    }

    fn client(
        &self,
    ) -> CoreClient<
        openidconnect::EndpointSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointMaybeSet,
        openidconnect::EndpointMaybeSet,
    > {
        CoreClient::from_provider_metadata(
            self.meta.clone(),
            ClientId::new(self.client_id.clone()),
            self.client_secret
                .as_ref()
                .map(|s| ClientSecret::new(s.expose_secret().to_string())),
        )
        .set_redirect_uri(self.redirect.clone())
    }

    fn http_fn(
        &self,
    ) -> impl Fn(
        HttpRequest,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<HttpResponse, HttpError>> + Send>,
    > + '_ {
        move |r| {
            let h = self.http.clone();
            Box::pin(async move { call(&h, r).await })
        }
    }

    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// Start a login: authorization URL plus the secrets to remember until the callback.
    pub fn authorize(&self) -> AuthRequest {
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let client = self.client();
        let mut req = client
            .authorize_url(
                AuthenticationFlow::<CoreResponseType>::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .set_pkce_challenge(challenge);
        for s in &self.scopes {
            if s != "openid" {
                req = req.add_scope(Scope::new(s.clone()));
            }
        }
        let (url, state, nonce) = req.url();
        AuthRequest {
            url,
            login: LoginState {
                state: state.secret().clone(),
                nonce: nonce.secret().clone(),
                pkce_verifier: verifier.secret().clone(),
            },
        }
    }

    /// Complete the callback: verify state, exchange the code with the PKCE verifier, validate the
    /// ID token (signature, iss, aud, exp, nonce) and map claims to an `Identity`.
    pub async fn complete(
        &self,
        code: &str,
        returned_state: &str,
        login: &LoginState,
    ) -> Result<(Identity, Tokens), OidcError> {
        if !crate::csrf::csrf_token_valid(&login.state, Some(returned_state)) {
            return Err(OidcError::StateMismatch);
        }
        let client = self.client();
        let http = self.http_fn();
        let resp = client
            .exchange_code(AuthorizationCode::new(code.to_string()))
            .map_err(|e| OidcError::Exchange(e.to_string()))?
            .set_pkce_verifier(PkceCodeVerifier::new(login.pkce_verifier.clone()))
            .request_async(&http)
            .await
            .map_err(|e| OidcError::Exchange(e.to_string()))?;
        let id_token = resp
            .id_token()
            .ok_or_else(|| OidcError::IdToken("token response has no id_token".into()))?;
        let identity = self.identity_from(&client, id_token, &Nonce::new(login.nonce.clone()))?;
        Ok((identity, tokens_from(&resp, id_token, None)))
    }

    fn identity_from(
        &self,
        client: &impl HasVerifier,
        id_token: &CoreIdToken,
        nonce: &Nonce,
    ) -> Result<Identity, OidcError> {
        client.verify(id_token, nonce)?;
        let jwt = id_token.to_string();
        let payload = jwt
            .split('.')
            .nth(1)
            .ok_or_else(|| OidcError::IdToken("malformed JWT".into()))?;
        let bytes = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|e| OidcError::IdToken(e.to_string()))?;
        let claims: Value =
            serde_json::from_slice(&bytes).map_err(|e| OidcError::IdToken(e.to_string()))?;
        map_identity(&self.issuer, &claims, &self.claims)
    }

    /// Refresh-token grant. The (possibly rotated) refresh token replaces the old one; when the IdP
    /// does not rotate, the previous one is kept. `invalid_grant` => `RefreshRejected`.
    pub async fn refresh(&self, refresh_token: &SecretString) -> Result<Tokens, OidcError> {
        let client = self.client();
        let http = self.http_fn();
        let resp = client
            .exchange_refresh_token(&RefreshToken::new(
                refresh_token.expose_secret().to_string(),
            ))
            .map_err(|e| OidcError::Exchange(e.to_string()))?
            .request_async(&http)
            .await
            .map_err(|e| match e {
                openidconnect::RequestTokenError::ServerResponse(r) => {
                    let code = r.error().to_string();
                    if code.contains("invalid_grant")
                        || code.contains("invalid_client")
                        || code.contains("unauthorized_client")
                    {
                        OidcError::RefreshRejected(code)
                    } else {
                        OidcError::RefreshTransient(code)
                    }
                }
                other => OidcError::RefreshTransient(other.to_string()),
            })?;
        let id_token = resp
            .id_token()
            .ok_or_else(|| OidcError::IdToken("refresh response has no id_token".into()))?;
        // Nonce is not echoed on refresh; verify signature/iss/aud/exp only.
        client.verify_no_nonce(id_token)?;
        Ok(tokens_from(&resp, id_token, Some(refresh_token)))
    }

    /// RP-initiated logout URL, when the IdP advertises `end_session_endpoint`.
    pub fn end_session_url(&self, id_token_hint: &SecretString) -> Option<url::Url> {
        let mut u = self.end_session.as_ref()?.url().clone();
        u.query_pairs_mut()
            .append_pair("id_token_hint", id_token_hint.expose_secret())
            .append_pair("client_id", &self.client_id)
            .append_pair("post_logout_redirect_uri", self.post_logout.as_str());
        Some(u)
    }
}

// Small seam so verification code stays independent of the client's typestate parameters.
trait HasVerifier {
    fn verify(&self, t: &CoreIdToken, nonce: &Nonce) -> Result<(), OidcError>;
    fn verify_no_nonce(&self, t: &CoreIdToken) -> Result<(), OidcError>;
}

impl<A, B, C, D, E, F> HasVerifier for CoreClient<A, B, C, D, E, F>
where
    A: openidconnect::EndpointState,
    B: openidconnect::EndpointState,
    C: openidconnect::EndpointState,
    D: openidconnect::EndpointState,
    E: openidconnect::EndpointState,
    F: openidconnect::EndpointState,
{
    fn verify(&self, t: &CoreIdToken, nonce: &Nonce) -> Result<(), OidcError> {
        t.claims(&self.id_token_verifier(), nonce)
            .map(|_| ())
            .map_err(|e| OidcError::IdToken(e.to_string()))
    }
    fn verify_no_nonce(&self, t: &CoreIdToken) -> Result<(), OidcError> {
        t.claims(&self.id_token_verifier(), |_: Option<&Nonce>| Ok(()))
            .map(|_| ())
            .map_err(|e| OidcError::IdToken(e.to_string()))
    }
}

fn tokens_from<T: OAuth2TokenResponse>(
    resp: &T,
    id_token: &CoreIdToken,
    prev_refresh: Option<&SecretString>,
) -> Tokens {
    let lifetime = resp.expires_in().unwrap_or(Duration::from_secs(300));
    Tokens {
        id_token: SecretString::from(id_token.to_string()),
        access_token: SecretString::from(resp.access_token().secret().clone()),
        refresh_token: resp
            .refresh_token()
            .map(|r| SecretString::from(r.secret().clone()))
            .or_else(|| prev_refresh.cloned()),
        expires_at: Instant::now() + lifetime,
    }
}

/// Map validated ID-token claims to an `Identity` (pure; unit tested).
pub fn map_identity(
    issuer: &str,
    claims: &Value,
    cfg: &ClaimsConfig,
) -> Result<Identity, OidcError> {
    let sub = claims
        .get("sub")
        .and_then(Value::as_str)
        .ok_or_else(|| OidcError::IdToken("missing sub".into()))?
        .to_string();
    let username = claims
        .get(&cfg.username)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| OidcError::MissingUsername(cfg.username.clone()))?;
    let groups = match claims.get(&cfg.groups) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => return Err(OidcError::MissingGroups(cfg.groups.clone())),
    };
    Ok(Identity {
        issuer: issuer.to_string(),
        sub,
        username: format!("{}{}", cfg.username_prefix, username),
        groups: groups
            .into_iter()
            .map(|g| format!("{}{}", cfg.groups_prefix, g))
            .collect(),
        email: claims
            .get("email")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cc(up: &str, gp: &str) -> ClaimsConfig {
        ClaimsConfig {
            username: "preferred_username".into(),
            groups: "groups".into(),
            username_prefix: up.into(),
            groups_prefix: gp.into(),
        }
    }

    #[test]
    fn maps_default_claims() {
        let c = json!({"sub":"s1","preferred_username":"alice","groups":["dev","ops"],"email":"a@x.io"});
        let i = map_identity("https://idp", &c, &cc("", "")).unwrap();
        assert_eq!((i.username.as_str(), i.sub.as_str()), ("alice", "s1"));
        assert_eq!(i.groups, ["dev", "ops"]);
        assert_eq!(i.email.as_deref(), Some("a@x.io"));
    }

    #[test]
    fn applies_prefixes_and_custom_claims() {
        let c = json!({"sub":"s1","upn":"bob@corp","roles":["admin"]});
        let cfg = ClaimsConfig {
            username: "upn".into(),
            groups: "roles".into(),
            username_prefix: "oidc:".into(),
            groups_prefix: "oidc:".into(),
        };
        let i = map_identity("iss", &c, &cfg).unwrap();
        assert_eq!(i.username, "oidc:bob@corp");
        assert_eq!(i.groups, ["oidc:admin"]);
    }

    #[test]
    fn missing_groups_is_an_error_but_empty_is_fine() {
        let c = json!({"sub":"s","preferred_username":"a"});
        assert!(matches!(
            map_identity("i", &c, &cc("", "")),
            Err(OidcError::MissingGroups(_))
        ));
        let c = json!({"sub":"s","preferred_username":"a","groups":[]});
        assert!(
            map_identity("i", &c, &cc("", ""))
                .unwrap()
                .groups
                .is_empty()
        );
    }

    #[test]
    fn missing_username_or_sub_is_an_error() {
        let c = json!({"sub":"s","groups":[]});
        assert!(matches!(
            map_identity("i", &c, &cc("", "")),
            Err(OidcError::MissingUsername(_))
        ));
        let c = json!({"preferred_username":"a","groups":[]});
        assert!(matches!(
            map_identity("i", &c, &cc("", "")),
            Err(OidcError::IdToken(_))
        ));
    }

    #[test]
    fn single_string_group_accepted() {
        let c = json!({"sub":"s","preferred_username":"a","groups":"only"});
        assert_eq!(map_identity("i", &c, &cc("", "")).unwrap().groups, ["only"]);
    }
}
