//! OIDC identity types, spec `033-rauthy-oidc-teams-users`.
//!
//! Non-secret only: no type here (nor any event/command carrying one) may hold an access
//! token, ID token, refresh token, client secret or admin API key in a *response*. Secrets
//! live in the OS credential store (`src/backend/oidc/store.rs`), never in these structs.
//! The one write-only exception is `BackendCmdV1::IdpSetAdminKey`, whose payload is never
//! echoed or logged.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The issuer id a guided-path test sign-in runs under (spec `045-guided-oidc-setup`, FR-007).
///
/// A draft is not a saved issuer, but it goes through the same sign-in machinery, which keys
/// cancellation and progress on the issuer id. So the draft borrows a reserved id instead of its
/// own: a saved issuer id is a lowercase slug, so the colon makes a collision unrepresentable and
/// a test sign-in cannot cancel a real issuer's in-flight one. It is here rather than in the
/// backend because `OidcSignInProgress` and `OidcError` reach the UI carrying it, and the UI has
/// to recognise them as the draft's.
pub const OIDC_DRAFT_SESSION_KEY: &str = "draft:045";

fn default_scopes() -> Vec<String> {
    ["openid", "profile", "email", "groups"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn default_username_claim() -> String {
    "preferred_username".to_owned()
}

fn default_groups_claim() -> String {
    "groups".to_owned()
}

/// One configured OIDC issuer (persisted, non-secret; `config.json` key `oidc.issuers`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct OidcIssuerConfig {
    /// Slug, unique per config; keys the credential-store entries.
    pub id: String,
    pub issuer_url: String,
    /// Public client id (no client secret is ever configured: PKCE public client).
    pub client_id: String,
    #[serde(default = "default_scopes")]
    pub scopes: Vec<String>,
    /// PEM bundle for a private CA (path only; contents are read at use).
    #[serde(default)]
    pub ca_bundle_path: Option<String>,
    #[serde(default = "default_username_claim")]
    pub username_claim: String,
    #[serde(default = "default_groups_claim")]
    pub groups_claim: String,
    /// `--oidc-username-prefix` equivalent, e.g. `oidc:`.
    #[serde(default)]
    pub username_prefix: Option<String>,
    /// `--oidc-groups-prefix` equivalent, e.g. `oidc:`.
    #[serde(default)]
    pub groups_prefix: Option<String>,
    /// Base URL of the IdP admin REST API (Rauthy), for the read-only directory.
    #[serde(default)]
    pub admin_api_base: Option<String>,
}

/// A provider's OIDC discovery document, as fetched from `/.well-known/openid-configuration`.
///
/// Defined here rather than in the backend because 045 renders it before any issuer is saved:
/// the operator is shown the real document, not a summary of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Discovery {
    /// The issuer string exactly as the IdP reports it (tokens carry this in `iss`).
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub device_authorization_endpoint: Option<String>,
    pub jwks_uri: String,
    #[serde(default)]
    pub end_session_endpoint: Option<String>,
    /// What the provider says it supports. A provider may omit either list, which is not an
    /// error -- it means "not advertised", not "none".
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    #[serde(default)]
    pub claims_supported: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum OidcSignInMethod {
    Browser,
    DeviceCode,
}

/// Progress of an in-flight sign-in (`OidcSignInProgress`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum OidcSignInStage {
    /// Browser opened / loopback listener waiting for the redirect.
    Awaiting,
    /// Device-code flow: show the code and URI to the operator.
    DeviceCode {
        user_code: String,
        verification_uri: String,
        /// Unix seconds.
        expires_at: u64,
    },
    /// Code exchanged; validating the ID token.
    Validating,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum OidcIdentityState {
    SignedIn,
    Refreshing,
    /// `reason` is the verbatim IdP / refresh error.
    SignedOut {
        reason: Option<String>,
    },
}

/// Who the operator is signed in as, per issuer. Keyed by (`issuer_id`, `sub`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct OidcIdentity {
    pub issuer_id: String,
    pub sub: String,
    pub username: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub groups: Vec<String>,
    /// Unix seconds at which the current ID token expires.
    pub expires_at: u64,
    pub state: OidcIdentityState,
}

/// How a kubeconfig context authenticates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum ContextAuthMode {
    #[default]
    Kubeconfig,
    Oidc,
}

/// Persisted per-context auth choice (`config.json` key `oidc.contexts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ContextAuthConfig {
    pub context_name: String,
    #[serde(default)]
    pub mode: ContextAuthMode,
    #[serde(default)]
    pub issuer_id: Option<String>,
}

fn default_viewers() -> String {
    "fathom-viewers".to_owned()
}
fn default_editors() -> String {
    "fathom-editors".to_owned()
}
fn default_admins() -> String {
    "fathom-admins".to_owned()
}

/// Fathom-level role groups (R9). Gate Fathom-only features; never cluster access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FathomRoleMap {
    #[serde(default = "default_viewers")]
    pub viewers_group: String,
    #[serde(default = "default_editors")]
    pub editors_group: String,
    #[serde(default = "default_admins")]
    pub admins_group: String,
}

impl Default for FathomRoleMap {
    fn default() -> Self {
        Self {
            viewers_group: default_viewers(),
            editors_group: default_editors(),
            admins_group: default_admins(),
        }
    }
}

/// The additive `config.json` `oidc` object. Unreadable => defaults, file never deleted.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
pub struct OidcConfig {
    #[serde(default)]
    pub issuers: Vec<OidcIssuerConfig>,
    #[serde(default)]
    pub contexts: Vec<ContextAuthConfig>,
    #[serde(default)]
    pub role_map: FathomRoleMap,
}

impl OidcConfig {
    /// Semantic checks serde's shape validation can't express. Invalid config is never
    /// applied (defaults are used and the original blob is preserved, Principle VI).
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::BTreeSet::new();
        for issuer in &self.issuers {
            let id = issuer.id.as_str();
            if id.is_empty()
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
            {
                return Err(format!("issuer id {id:?} must be a lowercase slug"));
            }
            if !seen.insert(id) {
                return Err(format!("duplicate issuer id {id:?}"));
            }
            if !(issuer.issuer_url.starts_with("https://")
                || issuer.issuer_url.starts_with("http://"))
            {
                return Err(format!("issuer {id:?}: issuer_url must be http(s)"));
            }
            if issuer.client_id.trim().is_empty() {
                return Err(format!("issuer {id:?}: client_id is empty"));
            }
        }
        for ctx in &self.contexts {
            if ctx.mode == ContextAuthMode::Oidc {
                match &ctx.issuer_id {
                    Some(issuer) if seen.contains(issuer.as_str()) => {}
                    Some(issuer) => {
                        return Err(format!(
                            "context {:?} references unknown issuer {issuer:?}",
                            ctx.context_name
                        ));
                    }
                    None => {
                        return Err(format!(
                            "context {:?} uses oidc without an issuer_id",
                            ctx.context_name
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Parse and validate a raw `config.json -> oidc` value.
    pub fn from_value(value: serde_json::Value) -> Result<Self, String> {
        let cfg: OidcConfig = serde_json::from_value(value).map_err(|e| e.to_string())?;
        cfg.validate()?;
        Ok(cfg)
    }
}

// ---------------------------------------------------------------------------
// 045-guided-oidc-setup: the draft an operator builds before anything is saved,
// and what Fathom learns about the cluster while they build it.
// ---------------------------------------------------------------------------

/// A whole unsaved issuer, carried inline so a sign-in can run before a save.
///
/// `OidcSignIn { issuer_id, .. }` resolves its config out of *saved* config, and the credential
/// store keys on the issuer id, so a draft cannot go through that path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct IssuerDraft {
    pub config: OidcIssuerConfig,
    pub method: OidcSignInMethod,
}

/// One claim as the provider actually emitted it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ClaimPair {
    pub name: String,
    pub value: String,
}

/// What a draft sign-in proved. Carries no token material of any kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DraftSignInResult {
    pub username: String,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub claims: Vec<ClaimPair>,
    /// Unix seconds at which the ID token this sign-in produced expires.
    #[ts(type = "number")]
    pub expires_at: i64,
}

/// Which authentication style an API server is configured with.
///
/// `Legacy` and `Structured` are mutually exclusive: `--authentication-config` cannot be combined
/// with the `--oidc-*` flags, which is why the repo's own lab script had to delete one to add the
/// other. Representing them as one enum makes writing both unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum ApiServerAuthStyle {
    /// `--oidc-issuer-url` and friends.
    Legacy,
    /// `--authentication-config=<file>`.
    Structured,
    /// Read successfully, and the API server has no OIDC authentication configured.
    None,
    /// Could not be determined. Never an assumed pass; `reason` says why.
    Unknown,
}

/// The API server's authentication configuration as Fathom read it.
///
/// `checked` plus `reason` is the three-state "stated unknown" shape `detect_kubeadm` uses: a
/// successful read, a definite negative, and an undetermined read carrying the API server's own
/// words. An `Option<String>` field means *absent*, never empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ApiServerAuthRead {
    pub style: ApiServerAuthStyle,
    #[serde(default)]
    pub issuer: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub username_claim: Option<String>,
    #[serde(default)]
    pub groups_claim: Option<String>,
    #[serde(default)]
    pub username_prefix: Option<String>,
    #[serde(default)]
    pub groups_prefix: Option<String>,
    /// The kubeconfig context this was read from.
    pub context: String,
    /// False when the read could not be completed at all.
    pub checked: bool,
    /// Verbatim reason from the API server when the read was denied or impossible.
    #[serde(default)]
    pub reason: Option<String>,
}

/// One field on which the cluster and the draft disagree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct AuthDifference {
    pub field: String,
    /// `None` means the cluster does not set this field at all.
    #[serde(default)]
    pub cluster: Option<String>,
    /// `None` means the draft does not set it.
    #[serde(default)]
    pub draft: Option<String>,
}

/// The read plus what it means for the draft being configured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ApiServerAuthComparison {
    pub read: ApiServerAuthRead,
    #[serde(default)]
    pub differences: Vec<AuthDifference>,
    /// Whether Fathom may change this API server: false without a provenance record, and always
    /// false in hosted.
    pub writable: bool,
    #[serde(default)]
    pub not_writable_reason: Option<String>,
}

/// Where an issuer in the list came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
pub enum IssuerOrigin {
    /// Read from this installation's saved config.
    #[default]
    Saved,
    /// Projected from the hosted server's validated startup configuration. Not editable here.
    SeededFromEnvironment,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(v: T) {
        let json = serde_json::to_string(&v).unwrap();
        let back: T = serde_json::from_str(&json).unwrap();
        assert_eq!(back, v, "{json}");
    }

    pub(crate) fn issuer() -> OidcIssuerConfig {
        OidcIssuerConfig {
            id: "rauthy".into(),
            issuer_url: "https://auth.localtest.me/auth/v1/".into(),
            client_id: "fathom-desktop".into(),
            scopes: default_scopes(),
            ca_bundle_path: Some("/etc/ca.pem".into()),
            username_claim: "preferred_username".into(),
            groups_claim: "groups".into(),
            username_prefix: Some("oidc:".into()),
            groups_prefix: Some("oidc:".into()),
            admin_api_base: Some("https://auth.localtest.me/auth/v1".into()),
        }
    }

    #[test]
    fn issuer_and_config_roundtrip() {
        rt(issuer());
        rt(OidcConfig {
            issuers: vec![issuer()],
            contexts: vec![ContextAuthConfig {
                context_name: "prod".into(),
                mode: ContextAuthMode::Oidc,
                issuer_id: Some("rauthy".into()),
            }],
            role_map: FathomRoleMap::default(),
        });
        rt(OidcConfig::default());
    }

    #[test]
    fn validate_rejects_bad_config() {
        let ok = OidcConfig {
            issuers: vec![issuer()],
            ..OidcConfig::default()
        };
        assert!(ok.validate().is_ok());
        let mut dup = ok.clone();
        dup.issuers.push(issuer());
        assert!(dup.validate().unwrap_err().contains("duplicate"));
        let mut bad_id = ok.clone();
        bad_id.issuers[0].id = "Bad Id".into();
        assert!(bad_id.validate().is_err());
        let mut bad_url = ok.clone();
        bad_url.issuers[0].issuer_url = "ftp://x".into();
        assert!(bad_url.validate().is_err());
        let mut dangling = ok.clone();
        dangling.contexts.push(ContextAuthConfig {
            context_name: "c".into(),
            mode: ContextAuthMode::Oidc,
            issuer_id: Some("nope".into()),
        });
        assert!(dangling.validate().unwrap_err().contains("unknown issuer"));
        let mut missing = ok;
        missing.contexts.push(ContextAuthConfig {
            context_name: "c".into(),
            mode: ContextAuthMode::Oidc,
            issuer_id: None,
        });
        assert!(missing.validate().is_err());
    }

    #[test]
    fn minimal_issuer_gets_defaults() {
        let i: OidcIssuerConfig =
            serde_json::from_str(r#"{"id":"a","issuer_url":"https://i","client_id":"c"}"#).unwrap();
        assert_eq!(i.scopes, default_scopes());
        assert_eq!(i.username_claim, "preferred_username");
        assert_eq!(i.groups_claim, "groups");
        assert!(i.username_prefix.is_none());
        let c: OidcConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(c.role_map.admins_group, "fathom-admins");
    }

    #[test]
    fn stage_state_method_roundtrip() {
        rt(OidcSignInMethod::Browser);
        rt(OidcSignInMethod::DeviceCode);
        rt(OidcSignInStage::Awaiting);
        rt(OidcSignInStage::DeviceCode {
            user_code: "ABCD-1234".into(),
            verification_uri: "https://auth/device".into(),
            expires_at: 1_800_000_000,
        });
        rt(OidcSignInStage::Validating);
        rt(OidcSignInStage::Done);
        rt(OidcIdentityState::SignedIn);
        rt(OidcIdentityState::Refreshing);
        rt(OidcIdentityState::SignedOut { reason: None });
        rt(OidcIdentityState::SignedOut {
            reason: Some("invalid_grant".into()),
        });
        rt(ContextAuthMode::Kubeconfig);
        rt(ContextAuthMode::Oidc);
    }

    #[test]
    fn identity_roundtrip() {
        rt(OidcIdentity {
            issuer_id: "rauthy".into(),
            sub: "u1".into(),
            username: "alice".into(),
            email: Some("alice@example.com".into()),
            groups: vec!["fathom-viewers".into()],
            expires_at: 1_800_000_000,
            state: OidcIdentityState::SignedIn,
        });
    }
}
