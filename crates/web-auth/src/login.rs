//! Pending logins between the redirect to the provider and its callback, keyed by the value of
//! a short-lived login cookie. Generic over what is kept (`L`): the OIDC [`LoginState`] by
//! default, or a [`crate::provider::PendingLogin`].
//!
//! Login CSRF (T094): a login is single use, expires after [`DEFAULT_LOGIN_TTL`], and is bound to
//! the browser that started it by the login cookie. [`LoginStore::begin`] sets that cookie and
//! [`LoginStore::finish`] refuses a callback with no cookie, an unknown, replayed or expired one,
//! or a `state` that is not the one that cookie's login was started with (compared in constant
//! time). The store keeps only `sha256(cookie)`, like sessions do.
//!
//! [`LoginState`]: crate::oidc::LoginState

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use http::{HeaderMap, HeaderValue};
use tokio::time::Instant;

use crate::cookie::CookieConfig;
use crate::oidc::LoginState;
use crate::provider::PendingLogin;
use crate::session::session_handle;

/// How long a started login may take to come back.
pub const DEFAULT_LOGIN_TTL: Duration = Duration::from_secs(600);
/// Pending logins kept at once; the oldest is evicted past this.
pub const DEFAULT_LOGIN_CAP: usize = 2000;
/// The login cookie's name unless the app names its own with [`LoginStore::with_cookie`]
/// (Fathom: `__Host-fathom_login`, Depths: `__Host-depths_login`).
pub const DEFAULT_LOGIN_COOKIE: &str = "__Host-login";

/// A kept login that knows the `state` it was started with.
pub trait LoginSecret {
    fn state(&self) -> &str;
}

impl LoginSecret for LoginState {
    fn state(&self) -> &str {
        &self.state
    }
}

impl LoginSecret for PendingLogin {
    fn state(&self) -> &str {
        &self.state
    }
}

/// Why [`LoginStore::finish`] refused a callback. Each is a login-CSRF refusal to audit as
/// `denied`; none says more to the browser than "start again".
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LoginRefused {
    /// The callback carried no login cookie: not the browser that started the login.
    #[error("no login cookie")]
    NoCookie,
    /// The cookie names no pending login: never started here, already used, or evicted.
    #[error("unknown or already used login")]
    Unknown,
    #[error("login expired")]
    Expired,
    /// The `state` is not the one this browser's login was started with.
    #[error("state mismatch")]
    StateMismatch,
}

pub struct LoginStore<L = LoginState> {
    ttl: Duration,
    cap: usize,
    cookie: CookieConfig,
    /// Keyed by `sha256(login cookie)`.
    inner: Mutex<HashMap<String, (L, String, Instant)>>,
}

impl<L> Default for LoginStore<L> {
    fn default() -> Self {
        Self::new(DEFAULT_LOGIN_TTL, DEFAULT_LOGIN_CAP)
    }
}

impl<L> LoginStore<L> {
    pub fn new(ttl: Duration, cap: usize) -> Self {
        Self {
            ttl,
            cap: cap.max(1),
            cookie: CookieConfig::new(DEFAULT_LOGIN_COOKIE),
            inner: Mutex::default(),
        }
    }

    /// Name the login cookie (and whether it is `Secure`).
    pub fn with_cookie(mut self, cookie: CookieConfig) -> Self {
        self.cookie = cookie;
        self
    }

    pub fn cookie(&self) -> CookieConfig {
        self.cookie
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Keep `login` and the path to return to; the returned id is the login cookie's value.
    pub fn put(&self, login: L, return_to: String) -> String {
        let id = crate::session::random_token();
        let now = Instant::now();
        let mut m = self.inner.lock().unwrap();
        let ttl = self.ttl;
        m.retain(|_, (_, _, t)| now.duration_since(*t) < ttl);
        if m.len() >= self.cap {
            let oldest = m
                .iter()
                .min_by_key(|(_, (_, _, t))| *t)
                .map(|(k, _)| k.clone());
            if let Some(oldest) = oldest {
                m.remove(&oldest);
            }
        }
        m.insert(session_handle(&id), (login, return_to, now));
        id
    }

    /// Single use: the entry is removed whether or not the callback succeeds.
    pub fn take(&self, id: &str) -> Option<(L, String)> {
        self.take_checked(id).ok()
    }

    fn take_checked(&self, id: &str) -> Result<(L, String), LoginRefused> {
        let (l, r, t) = self
            .inner
            .lock()
            .unwrap()
            .remove(&session_handle(id))
            .ok_or(LoginRefused::Unknown)?;
        if Instant::now().duration_since(t) < self.ttl {
            Ok((l, r))
        } else {
            Err(LoginRefused::Expired)
        }
    }

    /// Start a login: keep it and return the `Set-Cookie` value binding it to this browser.
    pub fn begin(&self, login: L, return_to: String) -> HeaderValue {
        let id = self.put(login, return_to);
        self.cookie.set(&id, self.ttl.as_secs())
    }

    /// The `Set-Cookie` value that deletes the login cookie; send it with every callback answer.
    pub fn clear_cookie(&self) -> HeaderValue {
        self.cookie.clear()
    }
}

impl<L: LoginSecret> LoginStore<L> {
    /// Finish a callback: the request's login cookie must name a live login started with
    /// `state`. The login is consumed whatever the outcome.
    pub fn finish(&self, headers: &HeaderMap, state: &str) -> Result<(L, String), LoginRefused> {
        let id = self.cookie.get(headers).ok_or(LoginRefused::NoCookie)?;
        let (l, r) = self.take_checked(id)?;
        if crate::csrf::csrf_token_valid(l.state(), Some(state)) {
            Ok((l, r))
        } else {
            Err(LoginRefused::StateMismatch)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::header;

    fn ls(state: &str) -> LoginState {
        LoginState {
            state: state.into(),
            nonce: "n".into(),
            pkce_verifier: "v".into(),
        }
    }

    /// The `Cookie` header a browser would send back for a `Set-Cookie` value.
    fn jar(set_cookie: &HeaderValue) -> HeaderMap {
        let pair = set_cookie.to_str().unwrap().split(';').next().unwrap();
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, HeaderValue::from_str(pair).unwrap());
        h
    }

    fn store() -> LoginStore {
        LoginStore::default().with_cookie(CookieConfig::new("__Host-depths_login"))
    }

    #[test]
    fn single_use() {
        let s: LoginStore = LoginStore::default();
        let id = s.put(ls("s"), "/x".into());
        assert_eq!(s.take(&id).map(|(_, r)| r).as_deref(), Some("/x"));
        assert!(s.take(&id).is_none());
    }

    #[test]
    fn keeps_only_the_cookie_hash() {
        let s: LoginStore = LoginStore::default();
        let id = s.put(ls("s"), "/".into());
        assert!(s.take(&session_handle(&id)).is_none());
        assert!(
            s.take(&id).is_some(),
            "the failed lookup did not consume it"
        );
        let id = s.put(ls("s"), "/".into());
        assert!(s.inner.lock().unwrap().contains_key(&session_handle(&id)));
        assert!(!s.inner.lock().unwrap().contains_key(&id));
    }

    #[test]
    fn cookie_name_is_configurable() {
        let s = store();
        let c = s.begin(ls("s"), "/".into());
        let v = c.to_str().unwrap();
        assert!(v.starts_with("__Host-depths_login="), "{v}");
        assert!(v.contains("Max-Age=600") && v.contains("HttpOnly") && v.contains("Secure"));
        assert!(
            s.clear_cookie()
                .to_str()
                .unwrap()
                .starts_with("__Host-depths_login=;")
        );
        assert_eq!(
            LoginStore::<LoginState>::default().cookie().name,
            DEFAULT_LOGIN_COOKIE
        );
    }

    #[test]
    fn finish_happy_path() {
        let s = store();
        let c = s.begin(ls("st"), "/r".into());
        let (l, r) = s.finish(&jar(&c), "st").unwrap();
        assert_eq!((l.state.as_str(), r.as_str()), ("st", "/r"));
    }

    #[test]
    fn replayed_state_is_refused() {
        let s = store();
        let c = s.begin(ls("st"), "/".into());
        assert!(s.finish(&jar(&c), "st").is_ok());
        assert_eq!(s.finish(&jar(&c), "st").unwrap_err(), LoginRefused::Unknown);
    }

    #[test]
    fn state_without_the_login_cookie_is_refused() {
        let s = store();
        let c = s.begin(ls("st"), "/".into());
        assert_eq!(
            s.finish(&HeaderMap::new(), "st").unwrap_err(),
            LoginRefused::NoCookie
        );
        // A cookie under another name (Fathom's) is not this store's.
        let other = LoginStore::<LoginState>::default()
            .with_cookie(CookieConfig::new("__Host-fathom_login"));
        assert_eq!(
            other.finish(&jar(&c), "st").unwrap_err(),
            LoginRefused::NoCookie
        );
        // The login is still there for the right browser.
        assert!(s.finish(&jar(&c), "st").is_ok());
    }

    /// Login CSRF: an attacker's callback URL (their state) opened in the victim's browser
    /// (the victim's cookie) is refused, and consumes the victim's pending login.
    #[test]
    fn state_from_another_browsers_login_is_refused() {
        let s = store();
        let victim = s.begin(ls("victim-state"), "/".into());
        let _attacker = s.begin(ls("attacker-state"), "/".into());
        assert_eq!(
            s.finish(&jar(&victim), "attacker-state").unwrap_err(),
            LoginRefused::StateMismatch
        );
        assert_eq!(
            s.finish(&jar(&victim), "victim-state").unwrap_err(),
            LoginRefused::Unknown
        );
    }

    #[test]
    fn unknown_cookie_is_refused() {
        let s = store();
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_static("__Host-depths_login=forged"),
        );
        assert_eq!(s.finish(&h, "st").unwrap_err(), LoginRefused::Unknown);
    }

    #[tokio::test(start_paused = true)]
    async fn expires_after_ttl() {
        let s: LoginStore<u8> = LoginStore::new(Duration::from_secs(10), 10);
        let id = s.put(1, "/".into());
        tokio::time::advance(Duration::from_secs(11)).await;
        assert!(s.take(&id).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn expired_state_is_refused() {
        let s = store();
        let c = s.begin(ls("st"), "/".into());
        tokio::time::advance(DEFAULT_LOGIN_TTL + Duration::from_secs(1)).await;
        assert_eq!(s.finish(&jar(&c), "st").unwrap_err(), LoginRefused::Expired);
    }

    #[tokio::test(start_paused = true)]
    async fn evicts_the_oldest_at_the_cap() {
        let s: LoginStore<u8> = LoginStore::new(Duration::from_secs(600), 2);
        let a = s.put(1, "/a".into());
        tokio::time::advance(Duration::from_millis(1)).await;
        let b = s.put(2, "/b".into());
        tokio::time::advance(Duration::from_millis(1)).await;
        let c = s.put(3, "/c".into());
        assert!(s.take(&a).is_none());
        assert!(s.take(&b).is_some());
        assert!(s.take(&c).is_some());
    }
}
