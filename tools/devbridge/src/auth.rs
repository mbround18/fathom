//! WebSocket handshake checks: the `Origin` allow-list and the `?token=` secret.

/// Only pages served from the local machine over plain HTTP may connect:
/// `http://localhost[:port]` and `http://127.0.0.1[:port]`. Anything else (another web
/// page, `https`, a look-alike host, a missing header) is refused, so a random site in
/// the operator's browser cannot drive the cluster.
pub fn origin_allowed(origin: Option<&str>) -> bool {
    let Some(rest) = origin.and_then(|o| o.strip_prefix("http://")) else {
        return false;
    };
    let (host, port) = match rest.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (rest, None),
    };
    let host_ok = host == "localhost" || host == "127.0.0.1";
    let port_ok =
        port.is_none_or(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()));
    host_ok && port_ok
}

/// The value of the `token` query parameter, URL-decoded.
pub fn token_from_query(query: Option<&str>) -> Option<String> {
    query?
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == "token")
        .and_then(|(_, value)| urlencoding::decode(value).ok())
        .map(|v| v.into_owned())
}

/// Constant-time (for equal lengths) comparison of the presented token to the expected one.
pub fn token_matches(presented: Option<&str>, expected: &str) -> bool {
    let Some(presented) = presented else {
        return false;
    };
    let (a, b) = (presented.as_bytes(), expected.as_bytes());
    if a.len() != b.len() || b.is_empty() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Why a handshake was refused (becomes the HTTP status of the rejection).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeError {
    /// 403: `Origin` missing or not local.
    BadOrigin,
    /// 401: `token` missing or wrong.
    BadToken,
}

/// Check one upgrade request's `Origin` header and query string.
pub fn check_handshake(
    origin: Option<&str>,
    query: Option<&str>,
    expected_token: &str,
) -> Result<(), HandshakeError> {
    if !origin_allowed(origin) {
        return Err(HandshakeError::BadOrigin);
    }
    if !token_matches(token_from_query(query).as_deref(), expected_token) {
        return Err(HandshakeError::BadToken);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_origins_are_allowed() {
        for o in [
            "http://localhost:5173",
            "http://127.0.0.1:4173",
            "http://localhost",
            "http://127.0.0.1",
        ] {
            assert!(origin_allowed(Some(o)), "{o}");
        }
    }

    #[test]
    fn everything_else_is_refused() {
        for o in [
            None,
            Some(""),
            Some("null"),
            Some("https://localhost:5173"),
            Some("http://evil.example"),
            Some("http://localhost.evil.example:5173"),
            Some("http://127.0.0.1.nip.io:80"),
            Some("http://localhost:"),
            Some("http://localhost:5173/path"),
            Some("http://localhost:abc"),
            Some("http://[::1]:5173"),
            Some("http://user@localhost:5173"),
        ] {
            assert!(!origin_allowed(o), "{o:?}");
        }
    }

    #[test]
    fn token_parsing_and_matching() {
        assert_eq!(token_from_query(Some("token=abc")), Some("abc".into()));
        assert_eq!(
            token_from_query(Some("x=1&token=a%2Bb%3D")),
            Some("a+b=".into())
        );
        assert_eq!(token_from_query(Some("tok=abc")), None);
        assert_eq!(token_from_query(None), None);

        assert!(token_matches(Some("secret"), "secret"));
        assert!(!token_matches(Some("secreT"), "secret"));
        assert!(!token_matches(Some("secre"), "secret"));
        assert!(!token_matches(None, "secret"));
        assert!(!token_matches(Some(""), ""));
    }

    #[test]
    fn handshake_checks_origin_then_token() {
        let good = Some("http://localhost:5173");
        assert_eq!(check_handshake(good, Some("token=t"), "t"), Ok(()));
        assert_eq!(
            check_handshake(Some("http://evil.example"), Some("token=t"), "t"),
            Err(HandshakeError::BadOrigin)
        );
        assert_eq!(
            check_handshake(good, Some("token=x"), "t"),
            Err(HandshakeError::BadToken)
        );
        assert_eq!(
            check_handshake(good, None, "t"),
            Err(HandshakeError::BadToken)
        );
    }
}
