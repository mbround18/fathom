//! GitHub login via the OAuth device flow (RFC 8628), 038-sdk-login-ollama R2.
//!
//! Public client id, **no client secret** (FR-002). The polling driver honours the
//! server-provided `interval` (and `slow_down`) and `expires_in`, and is cancellable.
//! The resulting token is used against the GitHub Models inference API.

use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::{ProviderAdapter, ProviderError, ValidatedAccount, error_from_response};
use crate::backend::ai::http::AiHttp;
use crate::protocol::v1::LoginPhase;

pub(crate) const DEFAULT_OAUTH_BASE: &str = "https://github.com";
pub(crate) const DEFAULT_API_BASE: &str = "https://api.github.com";
/// Minimal scope: access to GitHub Models.
pub(crate) const SCOPE: &str = "models:read";
const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// Consecutive transient network errors tolerated while polling before giving up.
const MAX_CONSECUTIVE_NETWORK_ERRORS: u32 = 3;

pub(crate) struct DeviceCode {
    pub device_code: SecretString,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in_s: u64,
    pub interval_s: u64,
}

#[derive(Debug)]
pub(crate) enum PollStep {
    Pending,
    SlowDown { interval_s: Option<u64> },
    Denied,
    Expired,
    Token(SecretString),
}

#[derive(Debug)]
pub(crate) enum DeviceOutcome {
    Token(SecretString),
    Denied,
    Expired,
    Failed(String),
    Cancelled,
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: Option<String>,
    user_code: Option<String>,
    verification_uri: Option<String>,
    expires_in: Option<u64>,
    interval: Option<u64>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
    interval: Option<u64>,
}

pub(crate) struct GithubAdapter {
    client_id: String,
    oauth_base: String,
    api_base: String,
    /// One "second" of the protocol. `1s` in production; tests shrink it.
    time_unit: Duration,
}

impl GithubAdapter {
    pub(crate) fn new(
        client_id: impl Into<String>,
        oauth_base: impl Into<String>,
        api_base: impl Into<String>,
    ) -> Self {
        Self {
            client_id: client_id.into(),
            oauth_base: oauth_base.into(),
            api_base: api_base.into(),
            time_unit: Duration::from_secs(1),
        }
    }

    pub(crate) fn with_time_unit(mut self, unit: Duration) -> Self {
        self.time_unit = unit;
        self
    }

    fn oauth(&self, path: &str) -> String {
        format!("{}{path}", self.oauth_base.trim_end_matches('/'))
    }

    /// Step 1: request a device + user code.
    pub(crate) async fn start(&self, http: &AiHttp) -> Result<DeviceCode, ProviderError> {
        let resp = http
            .post(&self.oauth("/login/device/code"))?
            .header("accept", "application/json")
            .form(&[("client_id", self.client_id.as_str()), ("scope", SCOPE)])
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let parsed: DeviceCodeResponse =
            serde_json::from_str(&text).map_err(|_| ProviderError::Http {
                status: status.as_u16(),
                body: text.clone(),
            })?;
        if let Some(err) = parsed.error {
            let body = match parsed.error_description {
                Some(desc) => format!("{err}: {desc}"),
                None => err,
            };
            return Err(ProviderError::Http {
                status: status.as_u16(),
                body,
            });
        }
        match (
            parsed.device_code,
            parsed.user_code,
            parsed.verification_uri,
        ) {
            (Some(device_code), Some(user_code), Some(verification_uri)) => Ok(DeviceCode {
                device_code: SecretString::from(device_code),
                user_code,
                verification_uri,
                expires_in_s: parsed.expires_in.unwrap_or(900),
                interval_s: parsed.interval.unwrap_or(5).max(1),
            }),
            _ => Err(ProviderError::Http {
                status: status.as_u16(),
                body: text,
            }),
        }
    }

    /// Step 2 (repeated): exchange the device code for a token.
    pub(crate) async fn poll_once(
        &self,
        http: &AiHttp,
        device_code: &SecretString,
    ) -> Result<PollStep, ProviderError> {
        let resp = http
            .post(&self.oauth("/login/oauth/access_token"))?
            .header("accept", "application/json")
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("device_code", device_code.expose_secret()),
                ("grant_type", GRANT_TYPE),
            ])
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let parsed: TokenResponse =
            serde_json::from_str(&text).map_err(|_| ProviderError::Http {
                status: status.as_u16(),
                body: text.clone(),
            })?;
        if let Some(token) = parsed.access_token {
            return Ok(PollStep::Token(SecretString::from(token)));
        }
        match parsed.error.as_deref() {
            Some("authorization_pending") => Ok(PollStep::Pending),
            Some("slow_down") => Ok(PollStep::SlowDown {
                interval_s: parsed.interval,
            }),
            Some("access_denied") => Ok(PollStep::Denied),
            Some("expired_token") => Ok(PollStep::Expired),
            Some(other) => Err(ProviderError::Http {
                status: status.as_u16(),
                body: match parsed.error_description {
                    Some(desc) => format!("{other}: {desc}"),
                    None => other.to_owned(),
                },
            }),
            None => Err(ProviderError::Http {
                status: status.as_u16(),
                body: text,
            }),
        }
    }

    /// Cancellable polling driver honouring `interval`, `slow_down` and expiry. `on_phase`
    /// is told about `Polling` ticks so the UI can show liveness.
    pub(crate) async fn drive(
        &self,
        http: &AiHttp,
        device: &DeviceCode,
        cancel: &CancellationToken,
        on_phase: &(dyn Fn(LoginPhase, u64) + Send + Sync),
    ) -> DeviceOutcome {
        let deadline = Instant::now() + self.time_unit * device.expires_in_s as u32;
        let mut interval_s = device.interval_s.max(1);
        let mut network_errors = 0u32;
        loop {
            tokio::select! {
                () = cancel.cancelled() => return DeviceOutcome::Cancelled,
                () = tokio::time::sleep(self.time_unit * interval_s as u32) => {}
            }
            if Instant::now() >= deadline {
                return DeviceOutcome::Expired;
            }
            let step = tokio::select! {
                () = cancel.cancelled() => return DeviceOutcome::Cancelled,
                step = self.poll_once(http, &device.device_code) => step,
            };
            match step {
                Ok(PollStep::Token(t)) => return DeviceOutcome::Token(t),
                Ok(PollStep::Denied) => return DeviceOutcome::Denied,
                Ok(PollStep::Expired) => return DeviceOutcome::Expired,
                Ok(PollStep::Pending) => {
                    network_errors = 0;
                    on_phase(LoginPhase::Polling, interval_s);
                }
                Ok(PollStep::SlowDown { interval_s: new }) => {
                    network_errors = 0;
                    // RFC 8628 3.5: add 5 seconds (GitHub also sends the new interval).
                    interval_s = new.unwrap_or(interval_s + 5).max(interval_s + 1);
                    on_phase(LoginPhase::Polling, interval_s);
                }
                Err(ProviderError::Network(msg)) => {
                    network_errors += 1;
                    if network_errors >= MAX_CONSECUTIVE_NETWORK_ERRORS {
                        return DeviceOutcome::Failed(msg);
                    }
                }
                Err(ProviderError::Offline(msg)) => return DeviceOutcome::Failed(msg),
                Err(other) => return DeviceOutcome::Failed(other.message_verbatim()),
            }
        }
    }
}

#[async_trait::async_trait]
impl ProviderAdapter for GithubAdapter {
    /// `GET /user` with the token: proves it works and yields the account login.
    async fn validate(
        &self,
        http: &AiHttp,
        secret: &SecretString,
    ) -> Result<ValidatedAccount, ProviderError> {
        let resp = http
            .get(&format!("{}/user", self.api_base.trim_end_matches('/')))?
            .bearer_auth(secret.expose_secret())
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "fathom")
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        #[derive(Deserialize)]
        struct User {
            login: Option<String>,
        }
        let user: User = resp
            .json()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;
        Ok(ValidatedAccount { label: user.login })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    const UNIT: Duration = Duration::from_millis(20);

    /// Replies from a scripted list (last entry repeats).
    struct Script(Mutex<Vec<serde_json::Value>>);
    impl Respond for Script {
        fn respond(&self, _: &Request) -> ResponseTemplate {
            let mut q = self.0.lock().unwrap();
            let next = if q.len() > 1 {
                q.remove(0)
            } else {
                q[0].clone()
            };
            ResponseTemplate::new(200).set_body_json(next)
        }
    }

    async fn device_server(script: Vec<serde_json::Value>, expires_in: u64) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/login/device/code"))
            .and(body_string_contains("client_id=cid"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "device_code": "dev-secret",
                "user_code": "ABCD-1234",
                "verification_uri": "https://github.com/login/device",
                "expires_in": expires_in,
                "interval": 1
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/login/oauth/access_token"))
            .respond_with(Script(Mutex::new(script)))
            .mount(&server)
            .await;
        server
    }

    fn adapter(server: &MockServer) -> GithubAdapter {
        GithubAdapter::new("cid", server.uri(), server.uri()).with_time_unit(UNIT)
    }

    async fn run(script: Vec<serde_json::Value>, expires_in: u64) -> DeviceOutcome {
        let server = device_server(script, expires_in).await;
        let gh = adapter(&server);
        let http = AiHttp::new();
        let dc = gh.start(&http).await.unwrap();
        assert_eq!(dc.user_code, "ABCD-1234");
        assert_eq!(dc.verification_uri, "https://github.com/login/device");
        gh.drive(&http, &dc, &CancellationToken::new(), &|_, _| {})
            .await
    }

    #[tokio::test]
    async fn pending_then_success() {
        let out = run(
            vec![
                serde_json::json!({"error":"authorization_pending"}),
                serde_json::json!({"error":"authorization_pending"}),
                serde_json::json!({"access_token":"gho_tok","token_type":"bearer"}),
            ],
            600,
        )
        .await;
        match out {
            DeviceOutcome::Token(t) => assert_eq!(t.expose_secret(), "gho_tok"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn slow_down_increases_interval() {
        let server = device_server(
            vec![
                serde_json::json!({"error":"slow_down","interval":3}),
                serde_json::json!({"access_token":"gho_tok"}),
            ],
            600,
        )
        .await;
        let gh = adapter(&server);
        let http = AiHttp::new();
        let dc = gh.start(&http).await.unwrap();
        let seen = Mutex::new(Vec::new());
        let out = gh
            .drive(&http, &dc, &CancellationToken::new(), &|p, i| {
                seen.lock().unwrap().push((p, i));
            })
            .await;
        assert!(matches!(out, DeviceOutcome::Token(_)));
        // The interval reported after slow_down is the server's larger value.
        assert_eq!(seen.lock().unwrap()[0].1, 3);
    }

    #[tokio::test]
    async fn denied_is_reported() {
        let out = run(vec![serde_json::json!({"error":"access_denied"})], 600).await;
        assert!(matches!(out, DeviceOutcome::Denied), "{out:?}");
    }

    #[tokio::test]
    async fn server_reported_expiry() {
        let out = run(vec![serde_json::json!({"error":"expired_token"})], 600).await;
        assert!(matches!(out, DeviceOutcome::Expired), "{out:?}");
    }

    #[tokio::test]
    async fn local_deadline_expiry_when_always_pending() {
        // expires_in = 3 units, interval = 1 unit: pending forever -> Expired.
        let out = run(
            vec![serde_json::json!({"error":"authorization_pending"})],
            3,
        )
        .await;
        assert!(matches!(out, DeviceOutcome::Expired), "{out:?}");
    }

    #[tokio::test]
    async fn unknown_error_is_verbatim_failure() {
        let out = run(
            vec![serde_json::json!({"error":"unsupported_grant_type","error_description":"nope"})],
            600,
        )
        .await;
        match out {
            DeviceOutcome::Failed(m) => assert!(m.contains("unsupported_grant_type: nope"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn cancel_stops_polling() {
        let server = device_server(
            vec![serde_json::json!({"error":"authorization_pending"})],
            600,
        )
        .await;
        let gh = adapter(&server);
        let http = AiHttp::new();
        let dc = gh.start(&http).await.unwrap();
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(UNIT * 3).await;
            c2.cancel();
        });
        let out = gh.drive(&http, &dc, &cancel, &|_, _| {}).await;
        assert!(matches!(out, DeviceOutcome::Cancelled), "{out:?}");
    }

    #[tokio::test]
    async fn persistent_network_errors_fail_after_retries() {
        let gh = GithubAdapter::new("cid", "http://127.0.0.1:1", "http://127.0.0.1:1")
            .with_time_unit(UNIT);
        let http = AiHttp::new();
        let dc = DeviceCode {
            device_code: SecretString::from("d".to_owned()),
            user_code: "U".into(),
            verification_uri: "v".into(),
            expires_in_s: 600,
            interval_s: 1,
        };
        let out = gh
            .drive(&http, &dc, &CancellationToken::new(), &|_, _| {})
            .await;
        assert!(matches!(out, DeviceOutcome::Failed(_)), "{out:?}");
    }

    #[tokio::test]
    async fn start_error_body_is_verbatim() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/login/device/code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "error":"device_flow_disabled","error_description":"Device Flow must be explicitly enabled"
            })))
            .mount(&server)
            .await;
        let gh = adapter(&server);
        let err = gh.start(&AiHttp::new()).await.err().expect("error");
        assert!(
            err.message_verbatim()
                .contains("Device Flow must be explicitly enabled"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn validate_returns_login_and_maps_401() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .and(wiremock::matchers::header("authorization", "Bearer good"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"login":"octocat"})),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(
                ResponseTemplate::new(401).set_body_string(r#"{"message":"Bad credentials"}"#),
            )
            .mount(&server)
            .await;
        let gh = adapter(&server);
        let http = AiHttp::new();
        let ok = gh
            .validate(&http, &SecretString::from("good".to_owned()))
            .await
            .unwrap();
        assert_eq!(ok.label.as_deref(), Some("octocat"));
        let err = gh
            .validate(&http, &SecretString::from("bad".to_owned()))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ProviderError::Rejected { status: 401, .. }),
            "{err:?}"
        );
        assert!(err.message_verbatim().contains("Bad credentials"));
    }
}
