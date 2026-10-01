//! ID token validation (033 US1): signature (JWKS with rotation), `iss`, `aud`, `exp`, `nonce`
//! and clock-skew handling. Signature and claim checks are delegated to `openidconnect`; this
//! module owns the JWKS cache, the refetch-on-unknown-key rotation retry and the operator
//! facing messages.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use openidconnect::core::{CoreIdToken, CoreIdTokenVerifier, CoreJsonWebKeySet};
use openidconnect::{ClientId, IssuerUrl, Nonce};
use serde_json::Value;

use crate::protocol::v1::ClaimPair;

/// Tolerated difference between the IdP clock and ours.
pub const CLOCK_LEEWAY_SECS: i64 = 60;

/// The verified, non-secret facts we take from an ID token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedClaims {
    pub sub: String,
    pub username: String,
    pub email: Option<String>,
    /// Empty group names removed (Rauthy emits `[""]` in some tokens, research T004).
    pub groups: Vec<String>,
    pub exp: u64,
    pub iat: u64,
    /// Every claim the token carried, flattened to strings (045 FR-005): the guided path lets the
    /// operator pick the username and groups claims from what the provider actually emitted
    /// rather than from its documentation. No token material -- an ID token's payload is claims.
    pub all_claims: Vec<ClaimPair>,
}

/// Verification failed; `0` is the operator-facing message (verbatim reason included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError(pub String);

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ValidationError {}

pub struct Validator {
    http: reqwest::Client,
    jwks_uri: String,
    issuer: IssuerUrl,
    client_id: ClientId,
    username_claim: String,
    groups_claim: String,
    jwks: RwLock<CoreJsonWebKeySet>,
    /// Injected clock for tests (seconds since epoch).
    now: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl Validator {
    pub async fn new(
        http: reqwest::Client,
        jwks_uri: String,
        issuer: &str,
        client_id: &str,
        username_claim: &str,
        groups_claim: &str,
    ) -> Result<Self, ValidationError> {
        let jwks = fetch_jwks(&http, &jwks_uri).await?;
        Ok(Self {
            http,
            jwks_uri,
            issuer: IssuerUrl::new(issuer.to_owned())
                .map_err(|e| ValidationError(format!("invalid issuer URL: {e}")))?,
            client_id: ClientId::new(client_id.to_owned()),
            username_claim: username_claim.to_owned(),
            groups_claim: groups_claim.to_owned(),
            jwks: RwLock::new(jwks),
            now: Arc::new(|| chrono::Utc::now().timestamp()),
        })
    }

    #[must_use]
    pub fn with_clock(mut self, now: Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        self.now = now;
        self
    }

    /// Verify `jwt`. `nonce` is `Some` for the authorization-code and device flows and `None`
    /// for refresh (the IdP need not echo it). An unknown signing key triggers exactly one
    /// JWKS refetch (key rotation) before failing.
    pub async fn validate(
        &self,
        jwt: &str,
        nonce: Option<&str>,
    ) -> Result<VerifiedClaims, ValidationError> {
        match self.verify_once(jwt, nonce) {
            Err(VerifyFailure::UnknownKey) => {
                let fresh = fetch_jwks(&self.http, &self.jwks_uri).await?;
                *self.jwks.write().expect("jwks lock") = fresh;
                self.verify_once(jwt, nonce).map_err(VerifyFailure::message)
            }
            other => other.map_err(VerifyFailure::message),
        }
    }

    fn verify_once(&self, jwt: &str, nonce: Option<&str>) -> Result<VerifiedClaims, VerifyFailure> {
        use openidconnect::ClaimsVerificationError as E;
        use openidconnect::SignatureVerificationError as S;

        let now = (self.now)();
        let jwks = self.jwks.read().expect("jwks lock").clone();
        let clock = self.now.clone();
        let verifier = CoreIdTokenVerifier::new_public_client(
            self.client_id.clone(),
            self.issuer.clone(),
            jwks,
        )
        .set_time_fn(move || {
            chrono::DateTime::from_timestamp((clock)() - CLOCK_LEEWAY_SECS, 0).unwrap_or_default()
        })
        // `iat` is checked below with our own clock-skew message.
        .set_issue_time_verifier_fn(|_| Ok(()));

        let token: CoreIdToken = serde_json::from_value(Value::String(jwt.to_owned()))
            .map_err(|e| VerifyFailure::Other(format!("malformed ID token: {e}")))?;
        let expected = nonce.map(|n| Nonce::new(n.to_owned()));
        let claims = match &expected {
            Some(n) => token.claims(&verifier, n),
            None => token.claims(&verifier, |_: Option<&Nonce>| Ok(())),
        }
        .map_err(|e| match e {
            E::SignatureVerification(S::NoMatchingKey) => VerifyFailure::UnknownKey,
            E::SignatureVerification(s) => {
                VerifyFailure::Other(format!("bad ID token signature: {s}"))
            }
            E::Expired(_) => VerifyFailure::Other(format!(
                "ID token expired. If this keeps happening, check that this computer's clock is correct (leeway {CLOCK_LEEWAY_SECS}s)"
            )),
            E::InvalidIssuer(m) => VerifyFailure::Other(format!("ID token issuer mismatch: {m}")),
            E::InvalidAudience(m) => {
                VerifyFailure::Other(format!("ID token audience mismatch: {m}"))
            }
            E::InvalidNonce(m) => VerifyFailure::Other(format!("ID token nonce mismatch: {m}")),
            other => VerifyFailure::Other(format!("ID token rejected: {other}")),
        })?;

        let payload = payload_json(jwt).map_err(VerifyFailure::Other)?;
        let iat = payload["iat"].as_i64().unwrap_or(now);
        if iat - now > CLOCK_LEEWAY_SECS {
            return Err(VerifyFailure::Other(format!(
                "clock skew: the ID token was issued {}s in the future. Check that this computer's clock is correct",
                iat - now
            )));
        }

        let username = string_claim(&payload, &self.username_claim)
            .or_else(|| string_claim(&payload, "email"))
            .unwrap_or_else(|| claims.subject().as_str().to_owned());
        Ok(VerifiedClaims {
            sub: claims.subject().as_str().to_owned(),
            username,
            email: string_claim(&payload, "email"),
            groups: groups_from(&payload, &self.groups_claim),
            exp: u64::try_from(claims.expiration().timestamp()).unwrap_or(0),
            iat: u64::try_from(iat).unwrap_or(0),
            all_claims: claim_pairs(&payload),
        })
    }
}

enum VerifyFailure {
    UnknownKey,
    Other(String),
}

impl VerifyFailure {
    fn message(self) -> ValidationError {
        match self {
            Self::UnknownKey => {
                ValidationError("ID token signed by a key not in the issuer's JWKS".to_owned())
            }
            Self::Other(m) => ValidationError(m),
        }
    }
}

async fn fetch_jwks(
    http: &reqwest::Client,
    uri: &str,
) -> Result<CoreJsonWebKeySet, ValidationError> {
    let resp = http
        .get(uri)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| ValidationError(format!("fetching JWKS from {uri}: {e}")))?;
    if !resp.status().is_success() {
        return Err(ValidationError(format!(
            "fetching JWKS from {uri}: HTTP {}",
            resp.status()
        )));
    }
    resp.json::<CoreJsonWebKeySet>()
        .await
        .map_err(|e| ValidationError(format!("parsing JWKS from {uri}: {e}")))
}

/// Decode a JWT payload without verifying (callers verify first).
pub fn payload_json(jwt: &str) -> Result<Value, String> {
    use base64::Engine;
    let part = jwt.split('.').nth(1).ok_or("malformed JWT")?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(part.trim_end_matches('='))
        .map_err(|e| format!("malformed JWT payload: {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("malformed JWT payload: {e}"))
}

/// Flatten the token payload into name/value pairs. A non-string value is rendered as the
/// compact JSON it is, so an array claim is shown as an array rather than silently dropped --
/// FR-005 asks what the token carries, which includes the shape it carried it in.
fn claim_pairs(payload: &Value) -> Vec<ClaimPair> {
    payload
        .as_object()
        .map(|map| {
            map.iter()
                .map(|(name, value)| ClaimPair {
                    name: name.clone(),
                    value: match value {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    },
                })
                .collect()
        })
        .unwrap_or_default()
}

fn string_claim(payload: &Value, name: &str) -> Option<String> {
    payload
        .get(name)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Read a groups claim (array of strings, or a single string), dropping empty names.
pub fn groups_from(payload: &Value, claim: &str) -> Vec<String> {
    let mut out: Vec<String> = match payload.get(claim) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    };
    out.retain(|g| !g.trim().is_empty());
    out
}
