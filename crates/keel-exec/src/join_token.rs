//! Join tokens (spec 055 contracts/join.md, T143): a strict compact JWS signed with Ed25519.
//!
//! The header is exactly `{"alg":"EdDSA","typ":"keel-join+jwt","kid":"<key id>"}`. Anything
//! else (another `alg`, `none`, a wrong `typ`, a missing `kid`, or any extra field such as
//! `jku`, `jwk`, `x5u` or `crit`) is refused before the signature is looked at, so algorithm
//! confusion is impossible. No JWT library: `ed25519-dalek` signs and `base64` encodes.
//!
//! The claims hold no secret and decode without the key ([`decode_unverified`]): that is what the
//! host's Ansible role and the runner read. Only the server verifies ([`verify`]).

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// `alg`: Ed25519 only.
pub const ALG: &str = "EdDSA";
/// `typ`: names what this token is, so no other JWT can stand in for it.
pub const TYP: &str = "keel-join+jwt";
/// `aud`: the only audience.
pub const AUDIENCE: &str = "keel-runner";
/// Default lifetime: 30 days.
pub const DEFAULT_TTL_SECS: i64 = 30 * 24 * 3600;
/// Longest lifetime: 365 days.
pub const MAX_TTL_SECS: i64 = 365 * 24 * 3600;
/// Longest token accepted at all, before any decoding.
pub const MAX_TOKEN_LEN: usize = 8 * 1024;

/// The JWS header, refused on any unknown field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    pub alg: String,
    pub typ: String,
    pub kid: String,
}

/// The token's claims. Connection settings only: nothing here widens what a host accepts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinClaims {
    /// Keel's public URL.
    pub iss: String,
    /// Always [`AUDIENCE`].
    pub aud: String,
    /// The workspace id.
    pub sub: String,
    /// 16 random bytes, hex.
    pub jti: String,
    pub iat: i64,
    pub exp: i64,
    /// `host:port` of the runner listener.
    #[serde(rename = "keel.server")]
    pub server: String,
    /// The full `@cert-authority * ssh-ed25519 …` line.
    #[serde(rename = "keel.host_ca")]
    pub host_ca: String,
    #[serde(rename = "keel.origin")]
    pub origin: String,
    #[serde(rename = "keel.rp_id")]
    pub rp_id: String,
    /// A suggested inventory name; the operator may change it on accept.
    #[serde(
        rename = "keel.inventory_name",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub inventory_name: Option<String>,
    /// Profiles to prefill on accept. Advisory only.
    #[serde(
        rename = "keel.profiles",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub profiles: Option<Vec<String>>,
}

/// Why a token was refused. Never carries the token.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JoinTokenError {
    #[error("the join token is not a compact JWS")]
    Malformed,
    #[error("the join token header is not exactly alg EdDSA, typ keel-join+jwt and a kid")]
    Header,
    #[error("the join token was signed by a key this Keel does not know")]
    UnknownKey,
    #[error("the join token signature does not verify")]
    Signature,
    #[error("the join token was issued by another Keel")]
    Issuer,
    #[error("the join token is not meant for a runner")]
    Audience,
    #[error("the join token has expired")]
    Expired,
    #[error("the join token's lifetime is out of bounds")]
    Lifetime,
}

/// A fresh random token id: 16 bytes, hex.
pub fn new_jti() -> String {
    hex(&rand::random::<[u8; 16]>())
}

/// SHA-256 of the whole token, the only form in which it is stored.
pub fn token_hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

/// The key id of a public key: SHA-256 of its 32 bytes, truncated to 16, hex.
pub fn kid_of(public: &[u8; 32]) -> String {
    let digest = Sha256::digest(public);
    hex(&digest[..16])
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Keel's join signing key. It signs join tokens and nothing else.
#[derive(Clone)]
pub struct JoinSigningKey {
    key: SigningKey,
}

impl fmt::Debug for JoinSigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JoinSigningKey")
            .field("kid", &self.kid())
            .finish_non_exhaustive()
    }
}

impl JoinSigningKey {
    pub fn generate() -> Self {
        Self::from_bytes(&rand::random::<[u8; 32]>())
    }

    pub fn from_bytes(secret: &[u8; 32]) -> Self {
        JoinSigningKey {
            key: SigningKey::from_bytes(secret),
        }
    }

    /// The secret seed, for sealing.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.key.to_bytes()
    }

    pub fn public_bytes(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }

    pub fn kid(&self) -> String {
        kid_of(&self.public_bytes())
    }

    pub fn verifying_key(&self) -> JoinVerifyingKey {
        JoinVerifyingKey {
            key: self.key.verifying_key(),
        }
    }

    /// Signs `claims` into a compact JWS.
    pub fn sign(&self, claims: &JoinClaims) -> String {
        let header = Header {
            alg: ALG.to_owned(),
            typ: TYP.to_owned(),
            kid: self.kid(),
        };
        let header = serde_json::to_vec(&header).expect("serialize a header");
        let claims = serde_json::to_vec(claims).expect("serialize claims");
        self.sign_parts(&header, &claims)
    }

    /// Signs raw header and claims bytes. Tests use it to build tokens the server must refuse.
    #[doc(hidden)]
    pub fn sign_parts(&self, header: &[u8], claims: &[u8]) -> String {
        let signing_input = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(header),
            URL_SAFE_NO_PAD.encode(claims)
        );
        let sig = self.key.sign(signing_input.as_bytes());
        format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(sig.to_bytes()))
    }
}

/// A public join key, as kept for verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinVerifyingKey {
    key: VerifyingKey,
}

impl JoinVerifyingKey {
    pub fn from_bytes(public: &[u8; 32]) -> Option<Self> {
        VerifyingKey::from_bytes(public)
            .ok()
            .map(|key| JoinVerifyingKey { key })
    }

    pub fn kid(&self) -> String {
        kid_of(&self.key.to_bytes())
    }
}

struct Parts<'a> {
    signing_input: &'a str,
    header: Header,
    claims: Vec<u8>,
    signature: Vec<u8>,
}

fn split(token: &str) -> Result<Parts<'_>, JoinTokenError> {
    if token.len() > MAX_TOKEN_LEN || !token.is_ascii() {
        return Err(JoinTokenError::Malformed);
    }
    let (signing_input, sig) = token.rsplit_once('.').ok_or(JoinTokenError::Malformed)?;
    let (header, claims) = signing_input
        .split_once('.')
        .ok_or(JoinTokenError::Malformed)?;
    if claims.contains('.') {
        return Err(JoinTokenError::Malformed);
    }
    let decode = |s: &str| {
        URL_SAFE_NO_PAD
            .decode(s)
            .map_err(|_| JoinTokenError::Malformed)
    };
    let header: Header =
        serde_json::from_slice(&decode(header)?).map_err(|_| JoinTokenError::Header)?;
    if header.alg != ALG || header.typ != TYP || header.kid.is_empty() {
        return Err(JoinTokenError::Header);
    }
    Ok(Parts {
        signing_input,
        header,
        claims: decode(claims)?,
        signature: decode(sig)?,
    })
}

/// The header and claims, without checking the signature. For the host side, which has no
/// prior trust in Keel and takes only connection settings from the token.
pub fn decode_unverified(token: &str) -> Result<(Header, JoinClaims), JoinTokenError> {
    let parts = split(token.trim())?;
    let claims = serde_json::from_slice(&parts.claims).map_err(|_| JoinTokenError::Malformed)?;
    Ok((parts.header, claims))
}

/// Verifies a token: strict header, a known `kid`, the signature, `iss`, `aud` and `exp`
/// (`now` in Unix seconds). The caller then checks `jti` against the workspace's active token.
pub fn verify(
    token: &str,
    keys: &[JoinVerifyingKey],
    issuer: &str,
    now: i64,
) -> Result<JoinClaims, JoinTokenError> {
    let parts = split(token)?;
    let key = keys
        .iter()
        .find(|k| k.kid() == parts.header.kid)
        .ok_or(JoinTokenError::UnknownKey)?;
    let sig = Signature::from_slice(&parts.signature).map_err(|_| JoinTokenError::Signature)?;
    key.key
        .verify_strict(parts.signing_input.as_bytes(), &sig)
        .map_err(|_| JoinTokenError::Signature)?;
    let claims: JoinClaims =
        serde_json::from_slice(&parts.claims).map_err(|_| JoinTokenError::Malformed)?;
    if claims.iss != issuer {
        return Err(JoinTokenError::Issuer);
    }
    if claims.aud != AUDIENCE {
        return Err(JoinTokenError::Audience);
    }
    if claims.exp <= claims.iat || claims.exp - claims.iat > MAX_TTL_SECS {
        return Err(JoinTokenError::Lifetime);
    }
    if now >= claims.exp {
        return Err(JoinTokenError::Expired);
    }
    Ok(claims)
}

#[cfg(test)]
mod tests {
    //! T137.
    use super::*;

    const ISS: &str = "https://keel.test";
    const NOW: i64 = 1_800_000_000;

    fn claims() -> JoinClaims {
        JoinClaims {
            iss: ISS.to_owned(),
            aud: AUDIENCE.to_owned(),
            sub: "0190f0a0-0000-7000-8000-000000000001".to_owned(),
            jti: new_jti(),
            iat: NOW,
            exp: NOW + DEFAULT_TTL_SECS,
            server: "keel.test:7831".to_owned(),
            host_ca: "@cert-authority * ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIA keel-host-ca"
                .to_owned(),
            origin: ISS.to_owned(),
            rp_id: "keel.test".to_owned(),
            inventory_name: Some("web-1".to_owned()),
            profiles: Some(vec!["debian".to_owned()]),
        }
    }

    fn header(json: &str, key: &JoinSigningKey) -> String {
        let claims = serde_json::to_vec(&claims()).unwrap();
        key.sign_parts(json.replace("KID", &key.kid()).as_bytes(), &claims)
    }

    fn check(token: &str, key: &JoinSigningKey) -> Result<JoinClaims, JoinTokenError> {
        verify(token, &[key.verifying_key()], ISS, NOW)
    }

    #[test]
    fn issue_and_verify_round_trip() {
        let key = JoinSigningKey::generate();
        let c = claims();
        let token = key.sign(&c);
        assert_eq!(check(&token, &key), Ok(c.clone()));
        // Restored from its sealed seed, the key verifies the same tokens.
        let again = JoinSigningKey::from_bytes(&key.to_bytes());
        assert_eq!(again.kid(), key.kid());
        assert_eq!(check(&token, &again), Ok(c));
        // kid: SHA-256 of the public key, truncated to 16 bytes, hex.
        assert_eq!(key.kid().len(), 32);
        assert!(key.kid().bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(new_jti().len(), 32);
    }

    #[test]
    fn the_header_is_exactly_what_is_signed() {
        let key = JoinSigningKey::generate();
        let token = key.sign(&claims());
        let first = token.split('.').next().unwrap();
        let json = String::from_utf8(URL_SAFE_NO_PAD.decode(first).unwrap()).unwrap();
        assert_eq!(
            json,
            format!(
                r#"{{"alg":"EdDSA","typ":"keel-join+jwt","kid":"{}"}}"#,
                key.kid()
            )
        );
    }

    #[test]
    fn a_strict_header_refuses_everything_else() {
        let key = JoinSigningKey::generate();
        for bad in [
            r#"{"alg":"none","typ":"keel-join+jwt","kid":"KID"}"#,
            r#"{"alg":"HS256","typ":"keel-join+jwt","kid":"KID"}"#,
            r#"{"alg":"EdDSA","typ":"JWT","kid":"KID"}"#,
            r#"{"alg":"EdDSA","kid":"KID"}"#,
            r#"{"alg":"EdDSA","typ":"keel-join+jwt"}"#,
            r#"{"alg":"EdDSA","typ":"keel-join+jwt","kid":""}"#,
            r#"{"alg":"EdDSA","typ":"keel-join+jwt","kid":"KID","jku":"https://x"}"#,
            r#"{"alg":"EdDSA","typ":"keel-join+jwt","kid":"KID","jwk":{}}"#,
            r#"{"alg":"EdDSA","typ":"keel-join+jwt","kid":"KID","x5u":"https://x"}"#,
            r#"{"alg":"EdDSA","typ":"keel-join+jwt","kid":"KID","crit":["exp"]}"#,
            r#"{"alg":"EdDSA","alg":"EdDSA","typ":"keel-join+jwt","kid":"KID"}"#,
        ] {
            assert_eq!(
                check(&header(bad, &key), &key),
                Err(JoinTokenError::Header),
                "{bad}"
            );
            assert!(decode_unverified(&header(bad, &key)).is_err(), "{bad}");
        }
        // `alg: none` with an empty signature is no better.
        let token = header(r#"{"alg":"none","typ":"keel-join+jwt","kid":"KID"}"#, &key);
        let unsigned = format!("{}.", token.rsplit_once('.').unwrap().0);
        assert_eq!(check(&unsigned, &key), Err(JoinTokenError::Header));
    }

    #[test]
    fn unknown_kid_and_bad_signature_are_refused() {
        let key = JoinSigningKey::generate();
        let other = JoinSigningKey::generate();
        // A token from another Keel: its kid is unknown here.
        assert_eq!(
            check(&other.sign(&claims()), &key),
            Err(JoinTokenError::UnknownKey)
        );
        // Another key claiming this kid: the signature fails.
        let forged = header(
            &format!(
                r#"{{"alg":"EdDSA","typ":"keel-join+jwt","kid":"{}"}}"#,
                key.kid()
            ),
            &other,
        );
        assert_eq!(check(&forged, &key), Err(JoinTokenError::Signature));
        // Tampered claims under a good signature.
        let token = key.sign(&claims());
        let mut parts: Vec<&str> = token.split('.').collect();
        let mut c = claims();
        c.sub = "another-workspace".to_owned();
        let swapped = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&c).unwrap());
        parts[1] = &swapped;
        assert_eq!(
            check(&parts.join("."), &key),
            Err(JoinTokenError::Signature)
        );
        // Not a JWS at all.
        for junk in [
            "",
            "a.b",
            "a.b.c.d",
            "é.é.é",
            &"a".repeat(MAX_TOKEN_LEN + 1),
        ] {
            assert!(check(junk, &key).is_err(), "{junk}");
        }
    }

    #[test]
    fn issuer_audience_and_expiry_are_checked() {
        let key = JoinSigningKey::generate();
        let token = key.sign(&claims());
        assert_eq!(
            verify(&token, &[key.verifying_key()], "https://other.test", NOW),
            Err(JoinTokenError::Issuer)
        );
        let mut c = claims();
        c.aud = "keel-web".to_owned();
        assert_eq!(check(&key.sign(&c), &key), Err(JoinTokenError::Audience));
        assert_eq!(
            verify(&token, &[key.verifying_key()], ISS, NOW + DEFAULT_TTL_SECS),
            Err(JoinTokenError::Expired)
        );
        let mut c = claims();
        c.exp = c.iat + MAX_TTL_SECS + 1;
        assert_eq!(check(&key.sign(&c), &key), Err(JoinTokenError::Lifetime));
    }

    #[test]
    fn claims_decode_without_the_key() {
        let key = JoinSigningKey::generate();
        let c = claims();
        let token = key.sign(&c);
        let (h, decoded) = decode_unverified(&format!("{token}\n")).unwrap();
        assert_eq!(h.kid, key.kid());
        assert_eq!(decoded, c);
        // Flat dotted claim names, as the role reads them.
        let payload = token.split('.').nth(1).unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
        assert_eq!(json["keel.server"], "keel.test:7831");
        assert!(
            json["keel.host_ca"]
                .as_str()
                .unwrap()
                .starts_with("@cert-authority * ssh-ed25519 ")
        );
        assert_eq!(json["keel.origin"], ISS);
        assert_eq!(json["keel.rp_id"], "keel.test");
        assert_eq!(json["aud"], AUDIENCE);
        // The optional claims are left out, not null.
        let mut bare = c;
        bare.inventory_name = None;
        bare.profiles = None;
        let token = key.sign(&bare);
        let payload = token.split('.').nth(1).unwrap();
        let text = String::from_utf8(URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
        assert!(!text.contains("keel.inventory_name") && !text.contains("keel.profiles"));
        assert_eq!(decode_unverified(&token).unwrap().1, bare);
    }

    #[test]
    fn the_signing_key_never_prints() {
        let key = JoinSigningKey::generate();
        let shown = format!("{key:?}");
        assert!(shown.contains(&key.kid()));
        assert!(!shown.contains(&hex(&key.to_bytes())));
    }
}
