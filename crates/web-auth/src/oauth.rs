//! What the plain OAuth2 providers ([`crate::github`], [`crate::discord`]) share: the HTTP
//! client, PKCE, capped JSON reads and the rejected-versus-unreachable classification.
//!
//! Nothing here logs, and no error carries a response body: provider responses can echo tokens.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use http::{HeaderMap, StatusCode, header};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::time::Instant;
use url::Url;

use crate::provider::{PendingLogin, ProviderError, Result};
use crate::session::{Session, Tokens};

/// Largest provider response read; a bigger one is refused, not buffered.
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(10);
/// Bounds on a provider's `Retry-After`.
const MIN_RETRY: Duration = Duration::from_secs(1);
const MAX_RETRY: Duration = Duration::from_secs(3600);
const DEFAULT_RETRY: Duration = Duration::from_secs(30);
/// An access token this close to expiry is refreshed before use.
pub(crate) const EXPIRY_MARGIN: Duration = Duration::from_secs(60);

/// Timeouts for [`build_oauth_client`].
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub connect: Duration,
    pub total: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: CONNECT_TIMEOUT,
            total: TOTAL_TIMEOUT,
        }
    }
}

/// The client for provider token and user endpoints: timeouts, no redirects followed, and
/// HTTPS only unless `allow_http` (the in-process fakes in tests).
pub fn build_oauth_client(t: Timeouts, allow_http: bool) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(t.connect)
        .timeout(t.total)
        .redirect(reqwest::redirect::Policy::none())
        .https_only(!allow_http)
        .user_agent(concat!("web-auth/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| ProviderError::Failed(format!("http client: {e}")))
}

/// A fresh login: `state`, a PKCE verifier and its S256 challenge.
pub(crate) fn new_login(redirect: &Url) -> (PendingLogin, String) {
    let verifier = crate::session::random_token();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (
        PendingLogin {
            state: crate::session::random_token(),
            nonce: None,
            pkce_verifier: verifier,
            redirect: redirect.clone(),
        },
        challenge,
    )
}

/// `state` first, in constant time, before anything reaches the provider.
pub(crate) fn check_state(pending: &PendingLogin, state: &str) -> Result<()> {
    if crate::csrf::csrf_token_valid(&pending.state, Some(state)) {
        Ok(())
    } else {
        Err(ProviderError::StateMismatch)
    }
}

/// The callback this provider was configured with, and only that one.
pub(crate) fn check_redirect(configured: &Url, got: &Url) -> Result<()> {
    if configured == got {
        Ok(())
    } else {
        Err(ProviderError::Failed(format!(
            "callback {got} is not the registered redirect URI {configured}"
        )))
    }
}

/// `<public_url>/auth/callback/<provider>`, the redirect URI to register with the provider.
pub fn callback_url(public_url: &Url, provider: &str) -> Result<Url> {
    let base = public_url.as_str().trim_end_matches('/');
    Url::parse(&format!("{base}/auth/callback/{provider}"))
        .map_err(|e| ProviderError::Failed(format!("public URL: {e}")))
}

pub(crate) fn form(pairs: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

/// Send, mapping transport failures (connect, timeout, reset) to unreachable.
pub(crate) async fn send(req: reqwest::RequestBuilder) -> Result<reqwest::Response> {
    req.send().await.map_err(|e| {
        ProviderError::Transient(if e.is_timeout() {
            "provider timed out".into()
        } else if e.is_connect() {
            "provider unreachable".into()
        } else {
            "provider request failed".into()
        })
    })
}

/// Read at most [`MAX_RESPONSE_BYTES`] of the body.
pub(crate) async fn read_capped(resp: &mut reqwest::Response) -> Result<Vec<u8>> {
    let too_big = || ProviderError::Failed("provider response too large".into());
    if resp
        .content_length()
        .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
    {
        return Err(too_big());
    }
    let mut out = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|_| ProviderError::Transient("provider response interrupted".into()))?
    {
        if out.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(too_big());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// A JSON body from a successful response, or the classified error for any other.
pub(crate) async fn json<T: serde::de::DeserializeOwned>(mut resp: reqwest::Response) -> Result<T> {
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = read_capped(&mut resp).await?;
    if !status.is_success() {
        return Err(classify(status, &headers, &body));
    }
    serde_json::from_slice(&body)
        .map_err(|_| ProviderError::Failed("provider response is not the expected JSON".into()))
}

/// Rejected (the provider said no: the session ends) versus unreachable (keep it, retry).
pub(crate) fn classify(status: StatusCode, headers: &HeaderMap, body: &[u8]) -> ProviderError {
    if status == StatusCode::TOO_MANY_REQUESTS {
        return ProviderError::RateLimited(retry_after(headers, body));
    }
    if status.is_server_error() {
        return ProviderError::Transient(format!("provider answered HTTP {}", status.as_u16()));
    }
    if status.is_redirection() {
        return ProviderError::Failed("provider answered with a redirect (not followed)".into());
    }
    if status.is_client_error() {
        return match oauth_error_code(body) {
            Some(code) if is_try_later(&code) => ProviderError::Transient(code),
            Some(code) => ProviderError::Rejected(code),
            None => ProviderError::Rejected(format!("HTTP {}", status.as_u16())),
        };
    }
    ProviderError::Failed(format!("unexpected HTTP {}", status.as_u16()))
}

fn is_try_later(code: &str) -> bool {
    code == "temporarily_unavailable" || code == "server_error"
}

/// The OAuth `error` field, reduced to a short identifier so no provider text is echoed.
pub(crate) fn oauth_error_code(body: &[u8]) -> Option<String> {
    #[derive(Deserialize)]
    struct E {
        error: serde_json::Value,
    }
    let e: E = serde_json::from_slice(body).ok()?;
    let s = e.error.as_str()?;
    let clean: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(64)
        .collect();
    (!clean.is_empty()).then_some(clean)
}

/// `Retry-After` (seconds) or the JSON `retry_after` (Discord, fractional seconds), clamped.
pub(crate) fn retry_after(headers: &HeaderMap, body: &[u8]) -> Duration {
    let from_header = headers
        .get(header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<f64>().ok());
    let from_body = || {
        serde_json::from_slice::<serde_json::Value>(body)
            .ok()?
            .get("retry_after")?
            .as_f64()
    };
    match from_header.or_else(from_body) {
        Some(s) if s.is_finite() && s >= 0.0 => {
            Duration::from_secs_f64(s.min(MAX_RETRY.as_secs_f64())).clamp(MIN_RETRY, MAX_RETRY)
        }
        _ => DEFAULT_RETRY,
    }
}

/// An OAuth token response. GitHub answers some errors with 200 and an `error` field.
#[derive(Deserialize)]
pub(crate) struct TokenResponse {
    pub access_token: Option<String>,
    pub token_type: Option<String>,
    pub expires_in: Option<u64>,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
    pub error: Option<serde_json::Value>,
}

impl TokenResponse {
    /// Tokens from a successful exchange; `prev_refresh` is kept when the provider does not
    /// rotate it. A response without a bearer access token is rejected.
    pub fn into_tokens(
        self,
        prev_refresh: Option<&SecretString>,
        default_lifetime: Duration,
    ) -> Result<(Tokens, Option<String>)> {
        if let Some(e) = self.error {
            let body = serde_json::json!({ "error": e }).to_string();
            let code = oauth_error_code(body.as_bytes()).unwrap_or_else(|| "error".into());
            return Err(if is_try_later(&code) {
                ProviderError::Transient(code)
            } else {
                ProviderError::Rejected(code)
            });
        }
        let at = self
            .access_token
            .filter(|t| !t.is_empty())
            .ok_or_else(|| ProviderError::Failed("token response has no access_token".into()))?;
        if !self
            .token_type
            .as_deref()
            .is_some_and(|t| t.eq_ignore_ascii_case("bearer"))
        {
            return Err(ProviderError::Failed(
                "token response is not a bearer token".into(),
            ));
        }
        // Ten years caps the lifetime so the Instant addition cannot overflow.
        let lifetime = self
            .expires_in
            .map(Duration::from_secs)
            .unwrap_or(default_lifetime)
            .min(Duration::from_secs(10 * 365 * 86_400));
        let tokens = Tokens {
            id_token: SecretString::from(String::new()),
            access_token: SecretString::from(at),
            refresh_token: self
                .refresh_token
                .filter(|t| !t.is_empty())
                .map(SecretString::from)
                .or_else(|| prev_refresh.cloned()),
            expires_at: Instant::now() + lifetime,
        };
        Ok((tokens, self.scope))
    }
}

/// POST a form to a token endpoint and read the token response.
pub(crate) async fn token_request(
    http: &reqwest::Client,
    url: &Url,
    body: String,
) -> Result<TokenResponse> {
    let resp = send(
        http.post(url.clone())
            .header(header::ACCEPT, "application/json")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body),
    )
    .await?;
    json(resp).await
}

/// GET a JSON resource with the bearer token.
pub(crate) async fn get_json<T: serde::de::DeserializeOwned>(
    http: &reqwest::Client,
    url: Url,
    token: &SecretString,
    extra: &[(&str, &str)],
) -> Result<T> {
    let mut req = http
        .get(url)
        .bearer_auth(token.expose_secret())
        .header(header::ACCEPT, "application/json");
    for (k, v) in extra {
        req = req.header(*k, *v);
    }
    json(send(req).await?).await
}

/// A provider user id: decimal digits only (Discord snowflake, GitHub numeric id), never empty,
/// never zero, at most a u64.
pub(crate) fn numeric_subject(raw: &str) -> Result<String> {
    let ok = !raw.is_empty()
        && raw.len() <= 20
        && raw.bytes().all(|b| b.is_ascii_digit())
        && raw.parse::<u64>().is_ok_and(|n| n > 0);
    if ok {
        Ok(raw.to_string())
    } else {
        Err(ProviderError::Refused(
            "the provider returned no usable numeric user id".into(),
        ))
    }
}

/// A display name the UI can show: non-empty, at most 100 characters, no control characters.
pub(crate) fn display_name(raw: Option<&str>) -> Result<String> {
    match raw {
        Some(s)
            if !s.is_empty() && s.chars().count() <= 100 && !s.chars().any(char::is_control) =>
        {
            Ok(s.to_string())
        }
        _ => Err(ProviderError::Refused(
            "the provider returned no usable username".into(),
        )),
    }
}

/// The session's access token, unless it has expired (or is about to).
pub(crate) fn live_access_token(s: &Session) -> Option<&SecretString> {
    (Instant::now() + EXPIRY_MARGIN < s.tokens.expires_at).then_some(&s.tokens.access_token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

    fn h(retry: Option<&'static str>) -> HeaderMap {
        let mut m = HeaderMap::new();
        if let Some(v) = retry {
            m.insert(header::RETRY_AFTER, HeaderValue::from_static(v));
        }
        m
    }

    #[test]
    fn classification() {
        use ProviderError as P;
        let none = h(None);
        assert!(matches!(
            classify(StatusCode::UNAUTHORIZED, &none, b""),
            P::Rejected(_)
        ));
        assert!(matches!(
            classify(StatusCode::FORBIDDEN, &none, b""),
            P::Rejected(_)
        ));
        assert!(matches!(
            classify(StatusCode::BAD_REQUEST, &none, br#"{"error":"invalid_grant"}"#),
            P::Rejected(c) if c == "invalid_grant"
        ));
        assert!(matches!(
            classify(
                StatusCode::BAD_REQUEST,
                &none,
                br#"{"error":"temporarily_unavailable"}"#
            ),
            P::Transient(_)
        ));
        for s in [500, 502, 503, 504] {
            let s = StatusCode::from_u16(s).unwrap();
            assert!(matches!(classify(s, &none, b""), P::Transient(_)), "{s}");
        }
        assert!(matches!(
            classify(StatusCode::FOUND, &none, b""),
            P::Failed(_)
        ));
        assert!(matches!(
            classify(StatusCode::TOO_MANY_REQUESTS, &h(Some("7")), b""),
            P::RateLimited(d) if d == Duration::from_secs(7)
        ));
    }

    #[test]
    fn retry_after_sources_and_bounds() {
        assert_eq!(
            retry_after(&h(None), br#"{"retry_after": 2.5}"#),
            Duration::from_millis(2500)
        );
        assert_eq!(retry_after(&h(Some("0")), b""), MIN_RETRY);
        assert_eq!(retry_after(&h(Some("999999")), b""), MAX_RETRY);
        assert_eq!(retry_after(&h(Some("soon")), b"{}"), DEFAULT_RETRY);
        assert_eq!(
            retry_after(&h(None), br#"{"retry_after": -1}"#),
            DEFAULT_RETRY
        );
    }

    #[test]
    fn error_codes_are_sanitized() {
        assert_eq!(
            oauth_error_code(br#"{"error":"bad<script>code at-xyz"}"#).as_deref(),
            Some("badscriptcodeatxyz")
        );
        assert_eq!(oauth_error_code(br#"{"error":{"x":1}}"#), None);
        assert_eq!(oauth_error_code(b"not json"), None);
    }

    #[test]
    fn subjects_must_be_numeric() {
        assert_eq!(
            numeric_subject("140642000969007104").unwrap(),
            "140642000969007104"
        );
        for bad in [
            "",
            "0",
            "alice",
            "12a",
            "-1",
            " 1",
            "1.0",
            "99999999999999999999999",
        ] {
            assert!(
                matches!(numeric_subject(bad), Err(ProviderError::Refused(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn display_names() {
        assert_eq!(display_name(Some("alice")).unwrap(), "alice");
        for bad in [None, Some(""), Some("a\nb"), Some(&*"x".repeat(101))] {
            assert!(display_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn pkce_challenge_is_s256_of_the_verifier() {
        let r = Url::parse("https://a.test/cb").unwrap();
        let (p, challenge) = new_login(&r);
        assert_eq!(
            challenge,
            URL_SAFE_NO_PAD.encode(Sha256::digest(p.pkce_verifier.as_bytes()))
        );
        assert_ne!(p.state, p.pkce_verifier);
        assert!(p.nonce.is_none());
    }

    #[test]
    fn callback_urls() {
        for base in ["https://depth.example.test", "https://depth.example.test/"] {
            let u = Url::parse(base).unwrap();
            assert_eq!(
                callback_url(&u, "discord").unwrap().as_str(),
                "https://depth.example.test/auth/callback/discord"
            );
        }
        let u = Url::parse("https://h.test/depths/").unwrap();
        assert_eq!(
            callback_url(&u, "github").unwrap().as_str(),
            "https://h.test/depths/auth/callback/github"
        );
    }

    #[test]
    fn token_response_errors_and_types() {
        let tr = |v: serde_json::Value| serde_json::from_value::<TokenResponse>(v).unwrap();
        let d = Duration::from_secs(60);
        assert!(matches!(
            tr(serde_json::json!({"error":"bad_verification_code"})).into_tokens(None, d),
            Err(ProviderError::Rejected(c)) if c == "bad_verification_code"
        ));
        assert!(matches!(
            tr(serde_json::json!({"access_token":"a","token_type":"mac"})).into_tokens(None, d),
            Err(ProviderError::Failed(_))
        ));
        let prev = SecretString::from("rt-old".to_string());
        let (t, _) = tr(serde_json::json!({"access_token":"a","token_type":"bearer"}))
            .into_tokens(Some(&prev), d)
            .unwrap();
        assert_eq!(t.refresh_token.unwrap().expose_secret(), "rt-old");
    }

    #[test]
    fn default_timeouts() {
        let t = Timeouts::default();
        assert_eq!(t.connect, Duration::from_secs(5));
        assert_eq!(t.total, Duration::from_secs(10));
    }
}
