//! Hosted HTTP + WebSocket integration tests (spec 031, T028) against a mock IdP and a fake
//! per-user backend host.
mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use common::MockIdp;
use fathom::protocol::{BridgeError, PublicLimits, RequestId, SubscriptionInfo};
use fathom_server::http::{AppState, router};
use fathom_server::limits::LimitReached;
use fathom_server::oidc::OidcClient;
use fathom_server::session::Session;
use fathom_server::ws::{AttachError, TenantHost, UserConn};
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc::UnboundedSender;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const EXT: &str = "http://localhost:18080";

struct FakeHost {
    ready: AtomicBool,
    tabs: Arc<AtomicUsize>,
    max_tabs: usize,
}

struct FakeConn {
    user: String,
    frames: UnboundedSender<String>,
    tabs: Arc<AtomicUsize>,
}

impl Drop for FakeConn {
    fn drop(&mut self) {
        self.tabs.fetch_sub(1, Ordering::SeqCst);
    }
}

impl UserConn for FakeConn {
    fn send(&self, _cmd: fathom::protocol::BackendCmd) -> Result<RequestId, BridgeError> {
        Ok(RequestId(7))
    }
    fn subscription_info(&self) -> Result<SubscriptionInfo, BridgeError> {
        Ok(SubscriptionInfo {
            protocol_version: 1,
            backend_started: true,
            mode: None,
            identity: None,
            limits: None,
        })
    }
    fn subscribe(&self) -> Result<(), BridgeError> {
        let _ = self.frames.send(format!(
            r#"{{"op":"batch","batch":{{"envelopes":[],"dropped":0}},"user":"{}"}}"#,
            self.user
        ));
        Ok(())
    }
}

#[async_trait]
impl TenantHost for FakeHost {
    async fn ready(&self) -> Result<(), String> {
        if self.ready.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err("home cluster unreachable".into())
        }
    }
    fn home_context(&self) -> String {
        "in-cluster".into()
    }
    fn public_limits(&self) -> PublicLimits {
        PublicLimits {
            max_watches: 30,
            max_streams: 10,
            max_tabs: self.max_tabs as u32,
        }
    }
    async fn attach(
        &self,
        s: &Session,
        frames: UnboundedSender<String>,
    ) -> Result<Box<dyn UserConn>, AttachError> {
        if self.tabs.load(Ordering::SeqCst) >= self.max_tabs {
            return Err(AttachError::Limit(LimitReached {
                kind: fathom::protocol::LimitKind::Tabs,
                max: self.max_tabs as u32,
            }));
        }
        self.tabs.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeConn {
            user: s.identity.username.clone(),
            frames,
            tabs: self.tabs.clone(),
        }))
    }
}

struct Harness {
    idp: MockIdp,
    base: String,
    addr: std::net::SocketAddr,
    http: reqwest::Client,
    host: Arc<FakeHost>,
    _ui: tempfile_dir::Dir,
    codes: AtomicUsize,
}

/// Minimal temp dir (no extra dev-dependency).
mod tempfile_dir {
    pub struct Dir(pub std::path::PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

async fn harness(max_tabs: usize) -> Harness {
    let idp = MockIdp::start().await;
    let dir = std::env::temp_dir().join(format!(
        "fathom-ui-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(dir.join("index.html"), "<html>fathom</html>").unwrap();
    std::fs::write(dir.join("assets/app.js"), "console.log(1)").unwrap();
    let cfg = Arc::new(idp.config_with(EXT, Some(dir.to_str().unwrap())));
    let oidc = Arc::new(OidcClient::discover(&cfg).await.unwrap());
    let host = Arc::new(FakeHost {
        ready: AtomicBool::new(true),
        tabs: Arc::default(),
        max_tabs,
    });
    let app = AppState::new(cfg, oidc, host.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let r = router(app);
    tokio::spawn(async move {
        let _ = axum::serve(listener, r).await;
    });
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    Harness {
        idp,
        base: format!("http://{addr}"),
        addr,
        http,
        host,
        _ui: tempfile_dir::Dir(dir),
        codes: AtomicUsize::new(0),
    }
}

fn rand_suffix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

fn set_cookies(r: &reqwest::Response) -> Vec<String> {
    r.headers()
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .collect()
}

fn cookie_pair(all: &[String], name: &str) -> Option<String> {
    all.iter()
        .find(|c| c.starts_with(&format!("{name}=")))
        .map(|c| c.split(';').next().unwrap().to_owned())
}

impl Harness {
    /// Full login as `user`; returns the `name=value` session cookie and the raw callback response.
    async fn login_full(&self, user: &str) -> (String, reqwest::Response) {
        let r = self
            .http
            .get(format!("{}/auth/login?return=/x", self.base))
            .send()
            .await
            .unwrap();
        assert!(r.status().is_redirection());
        let loc = url::Url::parse(r.headers()["location"].to_str().unwrap()).unwrap();
        let q: HashMap<_, _> = loc.query_pairs().into_owned().collect();
        let login_cookie =
            cookie_pair(&set_cookies(&r), "__Host-fathom_login").expect("login cookie");
        self.idp.set(|b| b.username = user.into());
        let code = format!("code-{}", self.codes.fetch_add(1, Ordering::SeqCst));
        self.idp.grant_code(&code, &q["nonce"]);
        let cb = self
            .http
            .get(format!(
                "{}/auth/callback?code={code}&state={}",
                self.base, q["state"]
            ))
            .header("cookie", login_cookie)
            .send()
            .await
            .unwrap();
        let session = cookie_pair(&set_cookies(&cb), "__Host-fathom_session").unwrap_or_default();
        (session, cb)
    }

    async fn login(&self, user: &str) -> String {
        let (c, r) = self.login_full(user).await;
        assert!(
            r.status().is_redirection(),
            "callback failed: {}",
            r.status()
        );
        assert!(!c.is_empty());
        c
    }

    async fn session_info(&self, cookie: &str) -> serde_json::Value {
        self.http
            .get(format!("{}/api/session", self.base))
            .header("cookie", cookie)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    #[allow(clippy::result_large_err)]
    async fn ws(
        &self,
        cookie: Option<&str>,
        origin: &str,
    ) -> Result<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::Error,
    > {
        let mut req = format!("ws://{}/ws", self.addr)
            .into_client_request()
            .unwrap();
        req.headers_mut().insert("origin", origin.parse().unwrap());
        if let Some(c) = cookie {
            req.headers_mut().insert("cookie", c.parse().unwrap());
        }
        tokio_tungstenite::connect_async(req).await.map(|(s, _)| s)
    }
}

async fn next_text(
    ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
) -> serde_json::Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("timeout")
            .expect("closed")
            .unwrap()
        {
            Message::Text(t) => return serde_json::from_str(t.as_str()).unwrap(),
            Message::Ping(_) | Message::Pong(_) => {}
            other => panic!("unexpected frame {other:?}"),
        }
    }
}

fn status_of(e: tokio_tungstenite::tungstenite::Error) -> u16 {
    match e {
        tokio_tungstenite::tungstenite::Error::Http(r) => r.status().as_u16(),
        other => panic!("not an http error: {other:?}"),
    }
}

#[tokio::test]
async fn unauthenticated_requests_are_redirected_or_rejected() {
    let h = harness(3).await;
    let r = h.http.get(format!("{}/", h.base)).send().await.unwrap();
    assert!(r.status().is_redirection());
    assert_eq!(r.headers()["location"], "/auth/login?return=%2F");
    // Static assets are public.
    assert_eq!(
        h.http
            .get(format!("{}/assets/app.js", h.base))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let r = h
        .http
        .get(format!("{}/api/session", h.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    assert_eq!(r.headers()["cache-control"], "no-store");
    let b: serde_json::Value = r.json().await.unwrap();
    assert_eq!(b["loginUrl"], "/auth/login");
    // Health probes are open.
    assert_eq!(
        h.http
            .get(format!("{}/healthz", h.base))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
}

#[tokio::test]
async fn login_sets_hardened_cookies_and_serves_the_app() {
    let h = harness(3).await;
    let (session, cb) = h.login_full("alice").await;
    assert_eq!(cb.headers()["location"], "/x");
    let cookies = set_cookies(&cb);
    let s = cookies
        .iter()
        .find(|c| c.starts_with("__Host-fathom_session="))
        .unwrap();
    for a in ["HttpOnly", "Secure", "SameSite=Lax", "Path=/"] {
        assert!(s.contains(a), "{s}");
    }
    assert!(!s.contains("Domain"));
    let cleared = cookies
        .iter()
        .find(|c| c.starts_with("__Host-fathom_login="))
        .unwrap();
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
    let page = h
        .http
        .get(format!("{}/some/deep/link", h.base))
        .header("cookie", &session)
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), 200);
    assert_eq!(page.text().await.unwrap(), "<html>fathom</html>");
}

#[tokio::test]
async fn session_endpoint_is_token_free_and_headers_are_set() {
    let h = harness(3).await;
    let c = h.login("alice").await;
    let r = h
        .http
        .get(format!("{}/api/session", h.base))
        .header("cookie", &c)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["cache-control"], "no-store");
    assert!(
        r.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'")
    );
    assert_eq!(r.headers()["x-content-type-options"], "nosniff");
    let text = r.text().await.unwrap();
    assert!(
        !text.contains("at-1") && !text.contains("rt-1") && !text.contains("eyJ"),
        "{text}"
    );
    let b: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(b["username"], "alice");
    assert_eq!(b["mode"], "hosted");
    assert!(b["csrf"].as_str().unwrap().len() > 20);
    assert!(b["expiresAt"].as_str().unwrap().ends_with('Z'));
}

#[tokio::test]
async fn callback_with_bad_state_is_rejected_without_echo() {
    let h = harness(3).await;
    let r = h
        .http
        .get(format!("{}/auth/login", h.base))
        .send()
        .await
        .unwrap();
    let login_cookie = cookie_pair(&set_cookies(&r), "__Host-fathom_login").unwrap();
    let cb = h
        .http
        .get(format!(
            "{}/auth/callback?code=whatever&state=<script>forged</script>",
            h.base
        ))
        .header("cookie", login_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(cb.status(), 400);
    assert!(cookie_pair(&set_cookies(&cb), "__Host-fathom_session").is_none());
    assert!(!cb.text().await.unwrap().contains("forged"));
}

#[tokio::test]
async fn logout_requires_csrf_and_origin_then_ends_the_session() {
    let h = harness(3).await;
    let c = h.login("alice").await;
    let csrf = h.session_info(&c).await["csrf"]
        .as_str()
        .unwrap()
        .to_owned();
    let url = format!("{}/auth/logout", h.base);

    let no_csrf = h
        .http
        .post(&url)
        .header("cookie", &c)
        .header("origin", EXT)
        .send()
        .await
        .unwrap();
    assert_eq!(no_csrf.status(), 403);
    let bad_origin = h
        .http
        .post(&url)
        .header("cookie", &c)
        .header("origin", "https://evil.example")
        .header("x-csrf-token", &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(bad_origin.status(), 403);
    let cross_site = h
        .http
        .post(&url)
        .header("cookie", &c)
        .header("origin", EXT)
        .header("sec-fetch-site", "cross-site")
        .header("x-csrf-token", &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(cross_site.status(), 403);
    // Still signed in after the refusals.
    assert_eq!(
        h.http
            .get(format!("{}/api/session", h.base))
            .header("cookie", &c)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );

    let ok = h
        .http
        .post(&url)
        .header("cookie", &c)
        .header("origin", EXT)
        .header("x-csrf-token", &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
    assert!(
        set_cookies(&ok)
            .iter()
            .any(|c| c.starts_with("__Host-fathom_session=;") && c.contains("Max-Age=0"))
    );
    let b: serde_json::Value = ok.json().await.unwrap();
    assert!(
        b["endSessionUrl"]
            .as_str()
            .unwrap()
            .contains("id_token_hint=")
    );
    assert_eq!(
        h.http
            .get(format!("{}/api/session", h.base))
            .header("cookie", &c)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
}

#[tokio::test]
async fn ws_is_rejected_before_upgrade_without_session_origin_or_capacity() {
    let h = harness(1).await;
    assert_eq!(status_of(h.ws(None, EXT).await.unwrap_err()), 401);
    let c = h.login("alice").await;
    assert_eq!(
        status_of(h.ws(Some(&c), "https://evil.example").await.unwrap_err()),
        403
    );
    let _first = h.ws(Some(&c), EXT).await.expect("first tab");
    assert_eq!(status_of(h.ws(Some(&c), EXT).await.unwrap_err()), 429);
}

#[tokio::test]
async fn ws_frames_filter_and_isolation() {
    let h = harness(3).await;
    let (a, b) = (h.login("alice").await, h.login("bob").await);
    let mut wa = h.ws(Some(&a), EXT).await.unwrap();
    let mut wb = h.ws(Some(&b), EXT).await.unwrap();

    wa.send(Message::text(
        r#"{"op":"send","cmd":{"type":"Initialize"}}"#,
    ))
    .await
    .unwrap();
    assert_eq!(next_text(&mut wa).await["op"], "sent");
    wa.send(Message::text(
        r#"{"op":"send","cmd":{"type":"StartFleetPolling"}}"#,
    ))
    .await
    .unwrap();
    let denied = next_text(&mut wa).await;
    assert_eq!(denied["op"], "error");
    assert!(denied["error"]["type"] == "Forbidden", "{denied}");

    wa.send(Message::text(r#"{"op":"subscribe"}"#))
        .await
        .unwrap();
    let sub = next_text(&mut wa).await;
    assert_eq!(sub["op"], "subscribed");
    assert_eq!(sub["info"]["mode"], "hosted");
    assert_eq!(sub["info"]["identity"]["username"], "alice");
    assert_eq!(sub["info"]["limits"]["maxTabs"], 3);
    assert_eq!(next_text(&mut wa).await["user"], "alice");

    // Bob's activity never reaches Alice's socket, and vice versa.
    wb.send(Message::text(r#"{"op":"subscribe"}"#))
        .await
        .unwrap();
    assert_eq!(next_text(&mut wb).await["op"], "subscribed");
    assert_eq!(next_text(&mut wb).await["user"], "bob");
    assert!(
        tokio::time::timeout(Duration::from_millis(300), wa.next())
            .await
            .is_err(),
        "alice saw bob's frame"
    );
}

#[tokio::test]
async fn sign_out_closes_websockets_with_4401() {
    let h = harness(3).await;
    let c = h.login("alice").await;
    let csrf = h.session_info(&c).await["csrf"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut ws = h.ws(Some(&c), EXT).await.unwrap();
    ws.send(Message::text(r#"{"op":"subscribe"}"#))
        .await
        .unwrap();
    assert_eq!(next_text(&mut ws).await["op"], "subscribed");
    let _ = next_text(&mut ws).await;
    let r = h
        .http
        .post(format!("{}/auth/logout", h.base))
        .header("cookie", &c)
        .header("origin", EXT)
        .header("x-csrf-token", csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("no close")
        {
            Some(Ok(Message::Close(Some(f)))) => {
                assert_eq!(u16::from(f.code), 4401);
                break;
            }
            Some(Ok(_)) => continue,
            other => panic!("expected close 4401, got {other:?}"),
        }
    }
    // The tab permit is released once the socket is gone.
    for _ in 0..50 {
        if h.host.tabs.load(Ordering::SeqCst) == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("tab permit leaked");
}

#[tokio::test]
async fn readyz_reflects_host_readiness() {
    let h = harness(3).await;
    assert_eq!(
        h.http
            .get(format!("{}/readyz", h.base))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    h.host.ready.store(false, Ordering::SeqCst);
    let r = h
        .http
        .get(format!("{}/readyz", h.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503);
    let b: serde_json::Value = r.json().await.unwrap();
    assert_eq!(b["status"], "unready");
    assert!(b["reason"].as_str().unwrap().contains("unreachable"));
}
