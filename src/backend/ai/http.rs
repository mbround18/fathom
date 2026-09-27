//! `AiHttp`: the single outbound transport chokepoint for the AI-assist surface
//! (038-sdk-login-ollama, research R5, FR-011).
//!
//! Provider adapters and the assistant clients never hold a raw `reqwest::Client`; they
//! build an [`AiRequest`] from this wrapper and `send()` it. `send()` enforces offline
//! mode: when the flag is on, the destination host is resolved and *every* resolved
//! address must be loopback, otherwise the request is refused before any connection is
//! attempted (fail closed - a DNS failure is a refusal, and a look-alike hostname that
//! resolves to a public IP is refused too).
//!
//! Every attempt can be recorded ([`AiHttp::with_recorder`]) so tests can assert that a
//! full assistant turn issued zero non-loopback requests (SC-005).

use std::net::IpAddr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use reqwest::{Method, Response, Url};
use serde::Serialize;

/// One attempted request, as seen by the chokepoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedRequest {
    pub method: String,
    pub url: String,
    pub host: String,
    /// `true` when the guard let the request through to the network layer.
    pub allowed: bool,
    /// `true` when every resolved address of `host` is loopback.
    pub loopback: bool,
}

#[derive(Debug)]
pub(crate) enum AiHttpError {
    /// Offline mode refused a non-loopback destination. No network I/O happened.
    OfflineBlocked {
        host: String,
    },
    InvalidUrl(String),
    Transport(reqwest::Error),
}

impl std::fmt::Display for AiHttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiHttpError::OfflineBlocked { host } => write!(
                f,
                "Offline mode is on: the request to {host} was blocked because only loopback \
                 hosts are allowed. Turn off Offline mode to use hosted providers."
            ),
            AiHttpError::InvalidUrl(msg) => write!(f, "Invalid URL: {msg}"),
            AiHttpError::Transport(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for AiHttpError {}

impl AiHttpError {
    pub(crate) fn is_offline_blocked(&self) -> bool {
        matches!(self, AiHttpError::OfflineBlocked { .. })
    }
}

pub(crate) struct AiHttp {
    client: reqwest::Client,
    offline: AtomicBool,
    recorder: Option<Mutex<Vec<RecordedRequest>>>,
}

impl AiHttp {
    pub(crate) fn new() -> Self {
        // Never `unwrap_or_default()`: `Client::default()` panics when the system has no CA
        // certificates, which took the whole backend down at startup.
        let client = crate::backend::http_client::build_or_fallback(|| {
            reqwest::Client::builder()
                // No redirects: a redirect would be a second, unchecked destination.
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(120))
        });
        Self {
            client,
            offline: AtomicBool::new(false),
            recorder: None,
        }
    }

    /// Records every attempted request (allowed or refused). Used by tests.
    #[cfg(test)]
    pub(crate) fn with_recorder(mut self) -> Self {
        self.recorder = Some(Mutex::new(Vec::new()));
        self
    }

    pub(crate) fn set_offline(&self, offline: bool) {
        self.offline.store(offline, Ordering::SeqCst);
    }

    pub(crate) fn is_offline(&self) -> bool {
        self.offline.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(crate) fn recorded(&self) -> Vec<RecordedRequest> {
        self.recorder
            .as_ref()
            .map(|r| {
                r.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone()
            })
            .unwrap_or_default()
    }

    pub(crate) fn get(&self, url: &str) -> Result<AiRequest<'_>, AiHttpError> {
        self.request(Method::GET, url)
    }

    pub(crate) fn post(&self, url: &str) -> Result<AiRequest<'_>, AiHttpError> {
        self.request(Method::POST, url)
    }

    fn request(&self, method: Method, url: &str) -> Result<AiRequest<'_>, AiHttpError> {
        let parsed = Url::parse(url).map_err(|e| AiHttpError::InvalidUrl(format!("{url}: {e}")))?;
        if parsed.host_str().is_none() {
            return Err(AiHttpError::InvalidUrl(format!("{url}: no host")));
        }
        Ok(AiRequest {
            http: self,
            builder: self.client.request(method.clone(), parsed.clone()),
            method,
            url: parsed,
        })
    }

    async fn guard(&self, method: &Method, url: &Url) -> Result<(), AiHttpError> {
        let host = url.host_str().unwrap_or_default().to_owned();
        let port = url.port_or_known_default().unwrap_or(80);
        let offline = self.is_offline();
        // Resolution only matters when offline (or when recording, to report `loopback`).
        let loopback = if offline || self.recorder.is_some() {
            host_is_loopback(&host, port).await
        } else {
            false
        };
        let allowed = !offline || loopback;
        if let Some(rec) = &self.recorder {
            rec.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(RecordedRequest {
                    method: method.to_string(),
                    url: url.to_string(),
                    host: host.clone(),
                    allowed,
                    loopback,
                });
        }
        if allowed {
            Ok(())
        } else {
            Err(AiHttpError::OfflineBlocked { host })
        }
    }
}

/// A request under construction. Deliberately exposes only what adapters need, and the only
/// way to send it is through the guard.
pub(crate) struct AiRequest<'a> {
    http: &'a AiHttp,
    builder: reqwest::RequestBuilder,
    method: Method,
    url: Url,
}

impl AiRequest<'_> {
    pub(crate) fn header(mut self, key: &str, value: &str) -> Self {
        self.builder = self.builder.header(key, value);
        self
    }

    pub(crate) fn bearer_auth(mut self, token: &str) -> Self {
        self.builder = self.builder.bearer_auth(token);
        self
    }

    pub(crate) fn json<T: Serialize + ?Sized>(mut self, body: &T) -> Self {
        self.builder = self.builder.json(body);
        self
    }

    pub(crate) fn form<T: Serialize + ?Sized>(mut self, body: &T) -> Self {
        self.builder = self.builder.form(body);
        self
    }

    pub(crate) fn timeout(mut self, timeout: Duration) -> Self {
        self.builder = self.builder.timeout(timeout);
        self
    }

    pub(crate) async fn send(self) -> Result<Response, AiHttpError> {
        self.http.guard(&self.method, &self.url).await?;
        self.builder.send().await.map_err(AiHttpError::Transport)
    }
}

/// Resolve `host` and require every address to be loopback. Fails closed: an unresolvable
/// host is not loopback.
pub(crate) async fn host_is_loopback(host: &str, port: u16) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return ip.is_loopback();
    }
    match tokio::net::lookup_host((bare, port)).await {
        Ok(addrs) => {
            let addrs: Vec<_> = addrs.collect();
            !addrs.is_empty() && addrs.iter().all(|a| a.ip().is_loopback())
        }
        Err(_) => false,
    }
}

/// Synchronous, DNS-free classification of a user-supplied endpoint URL: loopback when the
/// host is a loopback IP literal or `localhost`. Used to decide whether a non-loopback
/// Ollama endpoint needs explicit LAN confirmation (the transport still re-checks by
/// resolving at request time).
pub(crate) fn endpoint_is_loopback(endpoint: &str) -> bool {
    let Ok(url) = Url::parse(endpoint) else {
        return false;
    };
    match url.host_str() {
        Some(host) => {
            let bare = host.trim_start_matches('[').trim_end_matches(']');
            if bare.eq_ignore_ascii_case("localhost") {
                return true;
            }
            bare.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn endpoint_loopback_classification() {
        assert!(endpoint_is_loopback("http://127.0.0.1:11434"));
        assert!(endpoint_is_loopback("http://localhost:11434"));
        assert!(endpoint_is_loopback("http://[::1]:11434"));
        assert!(!endpoint_is_loopback("http://192.168.1.20:11434"));
        assert!(!endpoint_is_loopback("http://ollama.example.com"));
        assert!(!endpoint_is_loopback("not a url"));
    }

    #[tokio::test]
    async fn offline_blocks_non_loopback_without_network_io() {
        let http = AiHttp::new().with_recorder();
        http.set_offline(true);
        // 192.0.2.1 is TEST-NET-1 (never routable); if the guard let it through the
        // send would hang until the connect timeout rather than fail instantly.
        let err = http
            .get("http://192.0.2.1/v1/models")
            .unwrap()
            .send()
            .await
            .expect_err("blocked");
        assert!(err.is_offline_blocked(), "{err}");
        let rec = http.recorded();
        assert_eq!(rec.len(), 1);
        assert!(!rec[0].allowed && !rec[0].loopback);
    }

    #[tokio::test]
    async fn offline_fails_closed_on_unresolvable_host() {
        let http = AiHttp::new().with_recorder();
        http.set_offline(true);
        let err = http
            .get("https://definitely-not-a-real-host.invalid/x")
            .unwrap()
            .send()
            .await
            .expect_err("blocked");
        assert!(err.is_offline_blocked(), "{err}");
    }

    #[tokio::test]
    async fn offline_allows_loopback_and_online_allows_all() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;
        let http = AiHttp::new().with_recorder();
        http.set_offline(true);
        let resp = http.get(&server.uri()).unwrap().send().await.unwrap();
        assert_eq!(resp.status(), 200);
        http.set_offline(false);
        let resp = http.get(&server.uri()).unwrap().send().await.unwrap();
        assert_eq!(resp.status(), 200);
        let rec = http.recorded();
        assert_eq!(rec.len(), 2);
        assert!(rec.iter().all(|r| r.allowed && r.loopback));
    }

    #[tokio::test]
    async fn redirects_are_not_followed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "http://192.0.2.1/"))
            .mount(&server)
            .await;
        let http = AiHttp::new();
        let resp = http.get(&server.uri()).unwrap().send().await.unwrap();
        assert_eq!(resp.status(), 302);
    }
}
