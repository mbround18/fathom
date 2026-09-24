//! Caller identity (spec 036 R10, FR-017).
//!
//! Identity is accepted only from the API server's front-proxy headers and only when the TLS
//! layer has verified the peer's client certificate against the requestheader CA (it signals
//! that with the [`PeerVerified`] request extension). Anything else yields `None`, which every
//! caller must treat as an empty scope (fail closed). Query parameters and `Authorization`
//! are never consulted.

use axum::http::HeaderMap;
use std::collections::BTreeMap;

/// Request extension set by the TLS acceptor when the peer presented a client certificate
/// that chains to the requestheader CA. Absent means unverified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerVerified(pub bool);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerIdentity {
    pub user: String,
    pub groups: Vec<String>,
    /// `X-Remote-Extra-<key>` values (keys as sent, lower-cased by HTTP).
    pub extra: BTreeMap<String, Vec<String>>,
}

impl CallerIdentity {
    /// Collision-free cache key over user, groups (order-insensitive) and extra.
    pub fn key(&self) -> String {
        let mut groups = self.groups.clone();
        groups.sort();
        let mut k = format!("u:{}", self.user.escape_default());
        for g in groups {
            k.push_str("\u{0}g:");
            k.push_str(&g.escape_default().to_string());
        }
        for (name, vals) in &self.extra {
            let mut vals = vals.clone();
            vals.sort();
            for v in vals {
                k.push_str(&format!("\u{0}x:{}={}", name, v.escape_default()));
            }
        }
        k
    }
}

/// Parse identity from front-proxy headers. `peer_verified` must come from the TLS layer.
pub fn identity_from_headers(headers: &HeaderMap, peer_verified: bool) -> Option<CallerIdentity> {
    if !peer_verified {
        return None;
    }
    let user = headers
        .get("x-remote-user")?
        .to_str()
        .ok()?
        .trim()
        .to_string();
    if user.is_empty() {
        return None;
    }
    let groups = headers
        .get_all("x-remote-group")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(str::to_string)
        .collect();
    let mut extra: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, value) in headers {
        if let Some(k) = name.as_str().strip_prefix("x-remote-extra-")
            && let Ok(v) = value.to_str()
        {
            extra.entry(k.to_string()).or_default().push(v.to_string());
        }
    }
    Some(CallerIdentity {
        user,
        groups,
        extra,
    })
}
