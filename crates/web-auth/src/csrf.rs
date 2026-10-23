//! CSRF and open-redirect defenses (spec 031, moved here by spec 050).

use subtle::ConstantTimeEq;

/// A fresh 256-bit CSRF token (URL-safe base64), the same generator sessions use.
pub fn new_token() -> String {
    crate::session::random_token()
}

/// Reduce an external URL to its `scheme://host[:port]` origin string.
pub fn origin_of(url: &url::Url) -> String {
    url.origin().ascii_serialization()
}

/// `Origin` must be present and exactly equal to the configured origin.
pub fn origin_allowed(expected_origin: &str, origin: Option<&str>) -> bool {
    matches!(origin, Some(o) if o == expected_origin)
}

/// `Sec-Fetch-Site` gate for state-changing requests. Absent (older clients)
/// is accepted because the Origin check still applies; anything other than
/// `same-origin` is rejected.
pub fn fetch_site_allowed(value: Option<&str>) -> bool {
    match value {
        None => true,
        Some(v) => v.eq_ignore_ascii_case("same-origin"),
    }
}

/// Constant-time comparison of the session CSRF token with the request header.
pub fn csrf_token_valid(expected: &str, provided: Option<&str>) -> bool {
    let Some(p) = provided else { return false };
    if expected.is_empty() {
        return false;
    }
    expected.as_bytes().ct_eq(p.as_bytes()).into()
}

/// Full check for a state-changing request.
pub fn check_mutation(
    expected_origin: &str,
    origin: Option<&str>,
    fetch_site: Option<&str>,
    session_csrf: &str,
    header_csrf: Option<&str>,
) -> bool {
    origin_allowed(expected_origin, origin)
        && fetch_site_allowed(fetch_site)
        && csrf_token_valid(session_csrf, header_csrf)
}

/// Accept only same-origin absolute paths; anything else becomes `/`. The result is used
/// verbatim as a `Location` header, so it must be visible ASCII only (no CR/LF header
/// injection, and no byte `HeaderValue` would reject: that would panic in `Redirect::to`).
pub fn safe_return_path(raw: Option<&str>) -> String {
    let Some(p) = raw else { return "/".into() };
    let ok = p.starts_with('/')
        && !p.starts_with("//")
        && !p.starts_with("/\\")
        && !p.contains('\\')
        && p.chars().all(|c| c.is_ascii_graphic())
        && !p.contains("://");
    if ok { p.to_string() } else { "/".into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    const O: &str = "https://fathom.example.com";

    #[test]
    fn origin_exact_match() {
        assert!(origin_allowed(O, Some(O)));
        assert!(!origin_allowed(O, None));
        assert!(!origin_allowed(
            O,
            Some("https://fathom.example.com.evil.io")
        ));
        assert!(!origin_allowed(O, Some("http://fathom.example.com")));
        assert!(!origin_allowed(O, Some("https://fathom.example.com:8443")));
        assert!(!origin_allowed(O, Some("null")));
    }

    #[test]
    fn origin_of_strips_path_and_default_port() {
        let u = url::Url::parse("https://fathom.example.com:443/app/").unwrap();
        assert_eq!(origin_of(&u), O);
        let u = url::Url::parse("https://h.io:8443/x").unwrap();
        assert_eq!(origin_of(&u), "https://h.io:8443");
    }

    #[test]
    fn fetch_site() {
        assert!(fetch_site_allowed(Some("same-origin")));
        assert!(fetch_site_allowed(None));
        assert!(!fetch_site_allowed(Some("cross-site")));
        assert!(!fetch_site_allowed(Some("same-site")));
        assert!(!fetch_site_allowed(Some("none")));
    }

    #[test]
    fn csrf_compare() {
        assert!(csrf_token_valid("abc", Some("abc")));
        assert!(!csrf_token_valid("abc", Some("abd")));
        assert!(!csrf_token_valid("abc", Some("abcd")));
        assert!(!csrf_token_valid("abc", None));
        assert!(!csrf_token_valid("", Some("")));
    }

    #[test]
    fn mutation_combined() {
        assert!(check_mutation(
            O,
            Some(O),
            Some("same-origin"),
            "t",
            Some("t")
        ));
        assert!(!check_mutation(
            O,
            Some(O),
            Some("cross-site"),
            "t",
            Some("t")
        ));
        assert!(!check_mutation(O, None, None, "t", Some("t")));
        assert!(!check_mutation(O, Some(O), None, "t", Some("x")));
    }

    #[test]
    fn return_path() {
        assert_eq!(safe_return_path(Some("/clusters?a=1")), "/clusters?a=1");
        assert_eq!(safe_return_path(None), "/");
        for bad in [
            "//evil.io",
            "https://evil.io",
            "/\\evil.io",
            "evil.io",
            "/a\\b",
            "/x\r\nSet-Cookie: a=b",
            "/redir?u=http://evil.io",
            "javascript:alert(1)",
            "",
            // Non-ASCII and spaces would be invalid in a `Location` header value.
            "/caf\u{e9}",
            "/a b",
            "/x\u{7f}",
        ] {
            assert_eq!(safe_return_path(Some(bad)), "/", "{bad}");
        }
    }
}
