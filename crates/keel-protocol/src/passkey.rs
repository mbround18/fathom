//! Passkey wire types, the same shapes as Depths (spec 054) so `crates/web-passkeys` and the web
//! dialogs work against both. The passkey commands themselves arrive with T028.

use keel_core::uuid::Uuid;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::types::B64Url;

/// One passkey ceremony. Single-use, bound to the session and its purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct CeremonyId(#[ts(type = "string")] pub Uuid);

/// `response` of an assertion (`AuthenticatorAssertionResponse`), binary fields base64url.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssertionResponse {
    pub authenticator_data: B64Url,
    #[serde(rename = "clientDataJSON")]
    #[ts(rename = "clientDataJSON")]
    pub client_data_json: B64Url,
    pub signature: B64Url,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub user_handle: Option<B64Url>,
}

/// The client extension results the server asks for. Nothing else is carried.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PasskeyExtensions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cred_props: Option<CredProps>,
}

/// The `credProps` extension result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CredProps {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub rk: Option<bool>,
}

/// The only WebAuthn credential type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum PublicKeyType {
    #[serde(rename = "public-key")]
    PublicKey,
}

/// A `PublicKeyCredential` from `navigator.credentials.get`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssertionCredential {
    pub id: B64Url,
    pub raw_id: B64Url,
    pub response: AssertionResponse,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub kind: PublicKeyType,
    #[serde(default)]
    #[ts(as = "Option<PasskeyExtensions>", optional)]
    pub extensions: PasskeyExtensions,
}

/// A fresh check carried on `Confirm` (step-up).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct PasskeyAssertion {
    pub ceremony: CeremonyId,
    pub credential: AssertionCredential,
}

/// Where this session stands on passkeys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Verification {
    /// This session proved it is the person within the step-up window.
    pub verified: bool,
    /// When it last did, RFC 3339 UTC.
    pub at: Option<String>,
    /// How long a check covers risky actions, in seconds.
    pub window_secs: u32,
    /// The person has at least one passkey.
    pub enrolled: bool,
    /// The person must verify before step-up actions.
    pub required: bool,
    /// A recovery code was accepted: the next registration needs no prior check.
    #[serde(default)]
    pub recovering: bool,
}
