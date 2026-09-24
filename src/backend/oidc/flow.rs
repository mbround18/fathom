//! Sign-in flows (033 US1): OIDC discovery, Authorization Code + PKCE through the system browser
//! and a loopback listener, device-code fallback, and the refresh grant.
//!
//! Decisions from the T004 lab spike (research.md): the listener binds IPv4 `127.0.0.1` only
//! (Rauthy rejects `[::1]` redirect URIs), the redirect is always `http://127.0.0.1:<port>/callback`
//! built here, `state` is checked, and the client is a public PKCE client (no secret).

use std::sync::Arc;
use std::time::Duration;

use openidconnect::{CsrfToken, Nonce, PkceCodeChallenge};
use serde::Deserialize;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::protocol::v1::OidcIssuerConfig;

/// How long the loopback listener waits for the browser redirect.
pub const BROWSER_TIMEOUT: Duration = Duration::from_secs(120);

/// Opens a URL in the system browser. Injected so tests can drive the "browser" themselves.
pub type Opener = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// The platform default: `xdg-open` / `open` / `cmd /c start`.
pub fn system_opener() -> Opener {
    Arc::new(|url: &str| {
        let mut cmd = if cfg!(target_os = "macos") {
            std::process::Command::new("open")
        } else if cfg!(target_os = "windows") {
            let mut c = std::process::Command::new("cmd");
            c.args(["/c", "start", ""]);
            c
        } else {
            std::process::Command::new("xdg-open")
        };
        cmd.arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("could not open the system browser: {e}"))
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovery {
    /// The issuer string exactly as the IdP reports it (tokens carry this in `iss`).
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub device_authorization_endpoint: Option<String>,
    pub jwks_uri: String,
    pub end_session_endpoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowError {
    /// The loopback listener could not be bound or the browser could not be opened; the caller
    /// falls back to the device-code flow when the IdP offers it.
    LocalUnavailable(String),
    Cancelled,
    Timeout,
    /// Verbatim IdP or transport error.
    Failed(String),
}

impl std::fmt::Display for FlowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LocalUnavailable(m) | Self::Failed(m) => f.write_str(m),
            Self::Cancelled => f.write_str("sign-in cancelled"),
            Self::Timeout => f.write_str("sign-in timed out waiting for the browser"),
        }
    }
}
impl std::error::Error for FlowError {}

/// Tokens as returned by the token endpoint. Never logged, never serialised.
pub struct TokenResponse {
    pub id_token: String,
    pub refresh_token: Option<String>,
}

impl std::fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenResponse(<redacted>)")
    }
}

/// A token-endpoint failure with its OAuth `error` code kept for the device-code poll loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenError {
    pub code: String,
    pub message: String,
}

/// Build the HTTP client, adding a private-CA bundle when configured (T018). A bad bundle is a
/// hard error with the path in the message rather than a silent fallback to system roots.
pub fn http_client(cfg: &OidcIssuerConfig) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30));
    if let Some(path) = cfg.ca_bundle_path.as_deref().filter(|p| !p.is_empty()) {
        let pem =
            std::fs::read(path).map_err(|e| format!("cannot read the CA bundle {path}: {e}"))?;
        let certs = reqwest::Certificate::from_pem_bundle(&pem)
            .map_err(|e| format!("the CA bundle {path} is not valid PEM: {e}"))?;
        if certs.is_empty() {
            return Err(format!("the CA bundle {path} contains no certificates"));
        }
        for cert in certs {
            builder = builder.add_root_certificate(cert);
        }
    }
    builder
        .build()
        .map_err(|e| format!("cannot build the HTTP client: {e}"))
}

fn transport_hint(e: &reqwest::Error) -> String {
    let text = format!("{e:?}");
    if text.contains("UnknownIssuer") || text.contains("certificate") {
        format!("{e} (TLS certificate not trusted: set a CA bundle for this issuer)")
    } else {
        e.to_string()
    }
}

pub async fn discover(http: &reqwest::Client, issuer_url: &str) -> Result<Discovery, FlowError> {
    #[derive(Deserialize)]
    struct Doc {
        issuer: String,
        authorization_endpoint: String,
        token_endpoint: String,
        jwks_uri: String,
        #[serde(default)]
        device_authorization_endpoint: Option<String>,
        #[serde(default)]
        end_session_endpoint: Option<String>,
    }
    let url = format!(
        "{}/.well-known/openid-configuration",
        issuer_url.trim_end_matches('/')
    );
    let resp = http
        .get(&url)
        .send()
        .await
        .map_err(|e| FlowError::Failed(format!("discovery {url}: {}", transport_hint(&e))))?;
    if !resp.status().is_success() {
        return Err(FlowError::Failed(format!(
            "discovery {url}: HTTP {}",
            resp.status()
        )));
    }
    let doc: Doc = resp
        .json()
        .await
        .map_err(|e| FlowError::Failed(format!("discovery {url}: invalid document: {e}")))?;
    if doc.issuer.trim_end_matches('/') != issuer_url.trim_end_matches('/') {
        return Err(FlowError::Failed(format!(
            "discovery {url}: the document's issuer {:?} does not match the configured issuer {issuer_url:?}",
            doc.issuer
        )));
    }
    Ok(Discovery {
        issuer: doc.issuer,
        authorization_endpoint: doc.authorization_endpoint,
        token_endpoint: doc.token_endpoint,
        device_authorization_endpoint: doc.device_authorization_endpoint,
        jwks_uri: doc.jwks_uri,
        end_session_endpoint: doc.end_session_endpoint,
    })
}

fn form_body(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

async fn post_form(
    http: &reqwest::Client,
    url: &str,
    fields: &[(&str, &str)],
) -> Result<Value, TokenError> {
    let resp = http
        .post(url)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("accept", "application/json")
        .body(form_body(fields))
        .send()
        .await
        .map_err(|e| TokenError {
            code: "transport".into(),
            message: format!("{url}: {}", transport_hint(&e)),
        })?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if status.is_success() {
        return Ok(body);
    }
    let code = body["error"].as_str().unwrap_or("error").to_owned();
    let message = match body["error_description"].as_str() {
        Some(d) => format!("{code}: {d}"),
        None if body.is_null() => format!("HTTP {status}"),
        None => code.clone(),
    };
    Err(TokenError { code, message })
}

fn token_response(body: &Value) -> Result<TokenResponse, TokenError> {
    let id_token = body["id_token"].as_str().ok_or_else(|| TokenError {
        code: "no_id_token".into(),
        message:
            "the token response has no id_token (is the `openid` scope enabled for this client?)"
                .into(),
    })?;
    Ok(TokenResponse {
        id_token: id_token.to_owned(),
        refresh_token: body["refresh_token"].as_str().map(str::to_owned),
    })
}

/// Redeem a refresh token. The response carries a rotated refresh token, which the caller must
/// persist before dropping the old one.
pub async fn refresh(
    http: &reqwest::Client,
    disc: &Discovery,
    cfg: &OidcIssuerConfig,
    refresh_token: &str,
) -> Result<TokenResponse, TokenError> {
    let body = post_form(
        http,
        &disc.token_endpoint,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", &cfg.client_id),
        ],
    )
    .await?;
    token_response(&body)
}

/// Result of a completed browser flow. `nonce` must be passed to the validator.
pub struct BrowserResult {
    pub tokens: TokenResponse,
    pub nonce: String,
}

/// Authorization Code + PKCE through the system browser.
pub async fn browser_flow(
    http: &reqwest::Client,
    disc: &Discovery,
    cfg: &OidcIssuerConfig,
    opener: &Opener,
    cancel: &CancellationToken,
    timeout: Duration,
) -> Result<BrowserResult, FlowError> {
    // IPv4 loopback only: Rauthy's redirect_uri validation forbids `[::1]` (research T004).
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| {
        FlowError::LocalUnavailable(format!("cannot bind the loopback listener: {e}"))
    })?;
    let port = listener
        .local_addr()
        .map_err(|e| FlowError::LocalUnavailable(e.to_string()))?
        .port();
    let redirect = format!("http://127.0.0.1:{port}/callback");

    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let state = CsrfToken::new_random();
    let nonce = Nonce::new_random();
    let scope = cfg.scopes.join(" ");
    let sep = if disc.authorization_endpoint.contains('?') {
        '&'
    } else {
        '?'
    };
    let url = format!(
        "{}{sep}{}",
        disc.authorization_endpoint,
        form_body(&[
            ("response_type", "code"),
            ("client_id", &cfg.client_id),
            ("redirect_uri", &redirect),
            ("scope", &scope),
            ("state", state.secret()),
            ("nonce", nonce.secret()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
        ])
    );
    opener(&url).map_err(FlowError::LocalUnavailable)?;

    let code = tokio::select! {
        () = cancel.cancelled() => return Err(FlowError::Cancelled),
        () = tokio::time::sleep(timeout) => return Err(FlowError::Timeout),
        r = wait_for_callback(&listener, state.secret()) => r?,
    };

    let body = post_form(
        http,
        &disc.token_endpoint,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect),
            ("client_id", &cfg.client_id),
            ("code_verifier", verifier.secret()),
        ],
    )
    .await
    .map_err(|e| FlowError::Failed(e.message))?;
    let tokens = token_response(&body).map_err(|e| FlowError::Failed(e.message))?;
    Ok(BrowserResult {
        tokens,
        nonce: nonce.secret().clone(),
    })
}

async fn wait_for_callback(
    listener: &TcpListener,
    expected_state: &str,
) -> Result<String, FlowError> {
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|e| FlowError::Failed(format!("loopback listener: {e}")))?;
        let mut buf = vec![0u8; 8192];
        let mut len = 0;
        // Read the request head only.
        while len < buf.len() {
            let Ok(n) = stream.read(&mut buf[len..]).await else {
                break;
            };
            if n == 0 {
                break;
            }
            len += n;
            if buf[..len].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let head = String::from_utf8_lossy(&buf[..len]).into_owned();
        let target = head
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("/");
        let Ok(parsed) = url_parse(target) else {
            respond(&mut stream, 400, "Bad request").await;
            continue;
        };
        if parsed.0 != "/callback" {
            respond(&mut stream, 404, "Not found").await;
            continue;
        }
        let q = &parsed.1;
        let get = |k: &str| q.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        if let Some(err) = get("error") {
            respond(&mut stream, 400, "Sign-in failed. You can close this tab.").await;
            return Err(FlowError::Failed(match get("error_description") {
                Some(d) => format!("{err}: {d}"),
                None => err,
            }));
        }
        if get("state").as_deref() != Some(expected_state) {
            // Not our redirect (stale tab or forged): keep waiting for the real one.
            respond(&mut stream, 400, "State mismatch").await;
            continue;
        }
        let Some(code) = get("code") else {
            respond(&mut stream, 400, "Missing code").await;
            continue;
        };
        respond(
            &mut stream,
            200,
            "Signed in to Fathom. You can close this tab.",
        )
        .await;
        return Ok(code);
    }
}

fn url_parse(target: &str) -> Result<(String, Vec<(String, String)>), ()> {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let dec = |s: &str| {
        urlencoding::decode(&s.replace('+', " "))
            .map(std::borrow::Cow::into_owned)
            .map_err(|_| ())
    };
    let mut pairs = Vec::new();
    for kv in query.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        pairs.push((dec(k)?, dec(v)?));
    }
    Ok((path.to_owned(), pairs))
}

async fn respond(stream: &mut tokio::net::TcpStream, status: u16, message: &str) {
    let reason = if status == 200 { "OK" } else { "Error" };
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>Fathom</title><body style=\"font-family:sans-serif;padding:2rem\">{message}</body>"
    );
    let resp = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// What the operator must do to finish a device-code sign-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCodePrompt {
    pub user_code: String,
    pub verification_uri: String,
    /// Unix seconds.
    pub expires_at: u64,
}

/// Device authorization grant (RFC 8628). `on_prompt` is called once with the code to show.
pub async fn device_flow(
    http: &reqwest::Client,
    disc: &Discovery,
    cfg: &OidcIssuerConfig,
    cancel: &CancellationToken,
    on_prompt: impl Fn(DeviceCodePrompt),
) -> Result<TokenResponse, FlowError> {
    let Some(endpoint) = disc.device_authorization_endpoint.as_deref() else {
        return Err(FlowError::Failed(
            "this issuer does not advertise a device authorization endpoint".into(),
        ));
    };
    let scope = cfg.scopes.join(" ");
    let start = post_form(
        http,
        endpoint,
        &[("client_id", &cfg.client_id), ("scope", &scope)],
    )
    .await
    .map_err(|e| FlowError::Failed(e.message))?;
    let device_code = start["device_code"]
        .as_str()
        .ok_or_else(|| {
            FlowError::Failed("device authorization response has no device_code".into())
        })?
        .to_owned();
    let expires_in = start["expires_in"].as_u64().unwrap_or(300);
    let mut interval = Duration::from_secs(start["interval"].as_u64().unwrap_or(5).max(1));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    on_prompt(DeviceCodePrompt {
        user_code: start["user_code"].as_str().unwrap_or_default().to_owned(),
        verification_uri: start["verification_uri_complete"]
            .as_str()
            .or_else(|| start["verification_uri"].as_str())
            .unwrap_or_default()
            .to_owned(),
        expires_at: now + expires_in,
    });

    let deadline = tokio::time::Instant::now() + Duration::from_secs(expires_in);
    loop {
        tokio::select! {
            () = cancel.cancelled() => return Err(FlowError::Cancelled),
            () = tokio::time::sleep(interval) => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(FlowError::Timeout);
        }
        match post_form(
            http,
            &disc.token_endpoint,
            &[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", &device_code),
                ("client_id", &cfg.client_id),
            ],
        )
        .await
        {
            Ok(body) => return token_response(&body).map_err(|e| FlowError::Failed(e.message)),
            Err(e) if e.code == "authorization_pending" => {}
            Err(e) if e.code == "slow_down" => interval += Duration::from_secs(5),
            Err(e) => return Err(FlowError::Failed(e.message)),
        }
    }
}

/// Which flow to run first. Browser unless the caller asked for device code; a browser flow that
/// cannot start locally falls back to device code when the issuer supports it.
pub fn fallback_method(
    requested: crate::protocol::v1::OidcSignInMethod,
    disc: &Discovery,
    local_error: Option<&FlowError>,
) -> crate::protocol::v1::OidcSignInMethod {
    use crate::protocol::v1::OidcSignInMethod as M;
    match (requested, local_error) {
        (M::Browser, Some(FlowError::LocalUnavailable(_)))
            if disc.device_authorization_endpoint.is_some() =>
        {
            M::DeviceCode
        }
        (m, _) => m,
    }
}
