//! Session and login cookies: always `HttpOnly; SameSite=Lax; Path=/`, `Secure` unless the
//! app runs on plain-HTTP localhost. Fathom names them `__Host-fathom_session` and
//! `__Host-fathom_login`; a `__Host-` name needs `secure: true`.

use http::{HeaderMap, HeaderValue, header};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CookieConfig {
    pub name: &'static str,
    pub secure: bool,
}

impl CookieConfig {
    pub const fn new(name: &'static str) -> Self {
        Self { name, secure: true }
    }

    /// This cookie's value in the request, if sent.
    pub fn get<'a>(&self, headers: &'a HeaderMap) -> Option<&'a str> {
        cookie_value(headers, self.name)
    }

    /// A `Set-Cookie` value for `value`, kept for `max_age` seconds.
    pub fn set(&self, value: &str, max_age: u64) -> HeaderValue {
        let secure = if self.secure { " Secure;" } else { "" };
        HeaderValue::from_str(&format!(
            "{}={value}; Path=/;{secure} HttpOnly; SameSite=Lax; Max-Age={max_age}",
            self.name
        ))
        .expect("cookie value is ascii")
    }

    /// A `Set-Cookie` value that deletes the cookie.
    pub fn clear(&self) -> HeaderValue {
        self.set("", 0)
    }
}

/// The value of the cookie named exactly `name` in the request's `Cookie` headers.
pub fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|s| s.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: CookieConfig = CookieConfig::new("__Host-fathom_session");

    #[test]
    fn parsing_picks_exact_name() {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_static(
                "a=1; __Host-fathom_session=abc; x-__Host-fathom_session=evil",
            ),
        );
        assert_eq!(SESSION.get(&h), Some("abc"));
        assert_eq!(cookie_value(&h, "missing"), None);
    }

    #[test]
    fn set_carries_required_attributes() {
        let v = SESSION.set("id", 60);
        let s = v.to_str().unwrap();
        for a in [
            "__Host-fathom_session=id",
            "Path=/",
            "Secure",
            "HttpOnly",
            "SameSite=Lax",
            "Max-Age=60",
        ] {
            assert!(s.contains(a), "{s}");
        }
        assert!(!s.contains("Domain"));
        assert!(SESSION.clear().to_str().unwrap().contains("Max-Age=0"));
    }

    #[test]
    fn insecure_drops_only_secure() {
        let c = CookieConfig {
            name: "depths_session",
            secure: false,
        };
        let s = c.set("id", 1);
        let s = s.to_str().unwrap();
        assert!(!s.contains("Secure"), "{s}");
        assert!(s.contains("HttpOnly") && s.contains("SameSite=Lax"), "{s}");
    }
}
