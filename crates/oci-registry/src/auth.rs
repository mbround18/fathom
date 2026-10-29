//! Credentials, `WWW-Authenticate: Bearer` parsing and the token cache (contract rule 2).
//!
//! The HTTP side of the token request lives in `client.rs`; this module holds only data and pure
//! logic, so it is unit-tested without a server.
use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use std::time::Duration;

use secrecy::SecretString;
use serde::Deserialize;
use tokio::time::Instant;

use crate::error::RegistryError;

/// Registry credentials. `Debug` never prints the password.
#[derive(Clone)]
pub enum Credentials {
    Anonymous,
    Basic {
        username: String,
        password: SecretString,
    },
}

impl Credentials {
    /// The username part of the token cache key; empty when anonymous.
    pub(crate) fn username(&self) -> &str {
        match self {
            Credentials::Anonymous => "",
            Credentials::Basic { username, .. } => username,
        }
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Credentials::Anonymous => f.write_str("Anonymous"),
            Credentials::Basic { username, .. } => f
                .debug_struct("Basic")
                .field("username", username)
                .field("password", &"[redacted]")
                .finish(),
        }
    }
}

/// A parsed `WWW-Authenticate: Bearer realm="…",service="…",scope="…"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Challenge {
    pub realm: String,
    pub service: Option<String>,
}

/// Parse a `Bearer` challenge. Anything else (no header, `Basic`, no realm) is `None`.
pub(crate) fn parse_bearer(header: &str) -> Option<Challenge> {
    let header = header.trim();
    let (scheme, rest) = header.split_once(char::is_whitespace)?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let params = parse_params(rest)?;
    let realm = params.get("realm")?.clone();
    if realm.is_empty() {
        return None;
    }
    Some(Challenge {
        realm,
        service: params.get("service").cloned(),
    })
}

/// `k="v", k2=v2` with quoted values that may hold commas and `\"`.
fn parse_params(s: &str) -> Option<HashMap<String, String>> {
    let mut out = HashMap::new();
    let mut chars = s.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace() || *c == ',') {
            chars.next();
        }
        if chars.peek().is_none() {
            return Some(out);
        }
        let mut key = String::new();
        while let Some(&c) = chars.peek() {
            if c == '=' {
                break;
            }
            key.push(c);
            chars.next();
        }
        chars.next()?; // '='
        let mut value = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            loop {
                match chars.next()? {
                    '\\' => value.push(chars.next()?),
                    '"' => break,
                    c => value.push(c),
                }
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c == ',' {
                    break;
                }
                value.push(c);
                chars.next();
            }
        }
        out.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
    }
}

/// The token endpoint's reply. Docker Hub sends both `token` and `access_token`.
#[derive(Deserialize)]
struct TokenResponse {
    token: Option<String>,
    access_token: Option<String>,
    expires_in: Option<u64>,
}

/// Token responses are at most 64 KiB.
pub(crate) const TOKEN_BODY_LIMIT: usize = 64 * 1024;
const DEFAULT_EXPIRES_IN: u64 = 60;

/// Parse a token body into the token and how long it lives.
pub(crate) fn parse_token(body: &[u8]) -> Result<(String, Duration), RegistryError> {
    let r: TokenResponse = serde_json::from_slice(body)
        .map_err(|_| RegistryError::BadResponse("malformed token response"))?;
    let token = r
        .token
        .filter(|t| !t.is_empty())
        .or(r.access_token.filter(|t| !t.is_empty()))
        .ok_or(RegistryError::BadResponse("token response without a token"))?;
    let secs = r
        .expires_in
        .filter(|s| *s > 0)
        .unwrap_or(DEFAULT_EXPIRES_IN);
    Ok((token, Duration::from_secs(secs)))
}

/// (realm, service, scope, username).
pub(crate) type TokenKey = (String, String, String, String);

struct CachedToken {
    token: String,
    expires: Instant,
}

/// Tokens per (realm, service, scope, username), and the challenge each registry last sent so a
/// later call can find its token before making a request. In memory, per `Client`.
#[derive(Default)]
pub(crate) struct TokenCache {
    challenges: Mutex<HashMap<String, Challenge>>,
    tokens: Mutex<HashMap<TokenKey, CachedToken>>,
}

impl TokenCache {
    pub fn key(challenge: &Challenge, scope: &str, creds: &Credentials) -> TokenKey {
        (
            challenge.realm.clone(),
            challenge.service.clone().unwrap_or_default(),
            scope.to_string(),
            creds.username().to_string(),
        )
    }

    pub fn challenge_for(&self, registry: &str) -> Option<Challenge> {
        lock(&self.challenges).get(registry).cloned()
    }

    pub fn remember_challenge(&self, registry: &str, c: Challenge) {
        lock(&self.challenges).insert(registry.to_string(), c);
    }

    /// A token that has not expired.
    pub fn get(&self, key: &TokenKey) -> Option<String> {
        let mut tokens = lock(&self.tokens);
        match tokens.get(key) {
            Some(t) if Instant::now() < t.expires => Some(t.token.clone()),
            Some(_) => {
                tokens.remove(key);
                None
            }
            None => None,
        }
    }

    pub fn put(&self, key: TokenKey, token: String, ttl: Duration) {
        lock(&self.tokens).insert(
            key,
            CachedToken {
                token,
                expires: Instant::now() + ttl,
            },
        );
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_the_password() {
        let c = Credentials::Basic {
            username: "bob".into(),
            password: SecretString::from("s3cr3t-value".to_string()),
        };
        let d = format!("{c:?}");
        assert_eq!(d, "Basic { username: \"bob\", password: \"[redacted]\" }");
        assert_eq!(format!("{:?}", Credentials::Anonymous), "Anonymous");
    }

    #[test]
    fn parses_docker_hub_challenge() {
        let c = parse_bearer(
            r#"Bearer realm="https://auth.docker.io/token",service="registry.docker.io",scope="repository:mbround18/keel:pull""#,
        )
        .unwrap();
        assert_eq!(c.realm, "https://auth.docker.io/token");
        assert_eq!(c.service.as_deref(), Some("registry.docker.io"));
    }

    #[test]
    fn parses_quoted_commas_and_unquoted_values() {
        let c = parse_bearer(
            r#"bearer scope="repository:a:pull,push", realm=https://r.example/t , service="s\"x""#,
        )
        .unwrap();
        assert_eq!(c.realm, "https://r.example/t");
        assert_eq!(c.service.as_deref(), Some("s\"x"));
    }

    #[test]
    fn refuses_non_bearer_and_missing_realm() {
        assert_eq!(parse_bearer(r#"Basic realm="registry""#), None);
        assert_eq!(parse_bearer(r#"Bearer service="x""#), None);
        assert_eq!(parse_bearer(r#"Bearer realm="#), None);
        assert_eq!(parse_bearer(r#"Bearer realm="unterminated"#), None);
        assert_eq!(parse_bearer(""), None);
    }

    #[test]
    fn token_body() {
        let (t, ttl) = parse_token(br#"{"token":"abc","expires_in":300}"#).unwrap();
        assert_eq!((t.as_str(), ttl), ("abc", Duration::from_secs(300)));
        let (t, ttl) = parse_token(br#"{"access_token":"xyz"}"#).unwrap();
        assert_eq!((t.as_str(), ttl), ("xyz", Duration::from_secs(60)));
        assert!(parse_token(br#"{"expires_in":3}"#).is_err());
        assert!(parse_token(b"not json").is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn cache_expires() {
        let cache = TokenCache::default();
        let ch = Challenge {
            realm: "https://auth/token".into(),
            service: Some("svc".into()),
        };
        let anon = TokenCache::key(&ch, "repository:a:pull", &Credentials::Anonymous);
        let bob = TokenCache::key(
            &ch,
            "repository:a:pull",
            &Credentials::Basic {
                username: "bob".into(),
                password: SecretString::from("p".to_string()),
            },
        );
        cache.put(anon.clone(), "t1".into(), Duration::from_secs(60));
        assert_eq!(cache.get(&anon).as_deref(), Some("t1"));
        assert_eq!(cache.get(&bob), None, "keyed per username");
        tokio::time::advance(Duration::from_secs(59)).await;
        assert_eq!(cache.get(&anon).as_deref(), Some("t1"));
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(cache.get(&anon), None);
    }
}
