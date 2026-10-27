//! Registration and authentication ceremonies over webauthn-rs, with user verification required.
//!
//! Ceremony state ([`PasskeyRegistration`], [`PasskeyAuthentication`]) is returned to the caller,
//! which keeps it in memory, bound to the session and single use, with a short expiry (054 R10).
//! This crate does not persist it.
//!
//! [`Ceremonies::bound_request`] and [`Ceremonies::verify_bound_assertion`] serve assertions over
//! a challenge the application chose, such as Keel's override challenge (spec 055 R20): webauthn-rs
//! only issues random challenges, so these use the pure [`crate::verify::verify_assertion`]. The
//! sign counter is not checked for bound assertions; their challenge is single use by construction.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use webauthn_rs::prelude::{
    AuthenticationResult, Base64UrlSafeData, CreationChallengeResponse, PasskeyAuthentication,
    PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential,
    RequestChallengeResponse, Url, Uuid, WebauthnError,
};
use webauthn_rs::{Webauthn, WebauthnBuilder};

use crate::store::{PasskeyStore, StoredPasskey};
use crate::verify::{VerifyError, verify_assertion};

/// The longest passkey name, in characters.
pub const MAX_NAME_CHARS: usize = 64;

/// The person a ceremony is for.
#[derive(Debug, Clone)]
pub struct PasskeyUser {
    /// A stable, opaque user handle (WebAuthn `user.id`). Never an email or a provider id.
    pub handle: Uuid,
    /// The account name the authenticator shows.
    pub name: String,
    /// The display name the authenticator shows.
    pub display_name: String,
}

/// Why a ceremony failed.
#[derive(Debug, thiserror::Error)]
pub enum CeremonyError<E: std::error::Error + 'static = std::convert::Infallible> {
    /// webauthn-rs refused the response (wrong origin, challenge, counter, signature, ...).
    #[error("webauthn: {0}")]
    Webauthn(#[from] WebauthnError),
    /// The relying-party configuration is invalid.
    #[error("invalid relying party configuration")]
    Config,
    /// The name is empty, longer than 64 characters, or already used by this person.
    #[error("invalid passkey name")]
    InvalidName,
    /// The credential is not one of this person's passkeys.
    #[error("unknown credential")]
    UnknownCredential,
    /// A bound assertion did not verify.
    #[error("assertion: {0}")]
    Assertion(#[from] VerifyError),
    /// The passkey's key is neither ES256 nor EdDSA, so it cannot be pinned or verified.
    #[error("unsupported credential key")]
    UnsupportedKey,
    /// The store failed.
    #[error("passkey store: {0}")]
    Store(#[source] E),
}

/// The relying party: webauthn-rs plus the exact origin and RP id it was built for.
#[derive(Debug)]
pub struct Ceremonies {
    webauthn: Webauthn,
    origin: String,
    rp_id: String,
}

/// Trims a passkey name and checks it is 1 to 64 characters and not already used by `existing`.
pub fn validate_name(name: &str, existing: &[StoredPasskey]) -> Option<String> {
    let name = name.trim();
    let n = name.chars().count();
    if n == 0 || n > MAX_NAME_CHARS || existing.iter().any(|p| p.name == name) {
        return None;
    }
    Some(name.to_owned())
}

/// The credential id as stored: base64url without padding.
pub fn credential_id_string(raw: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(raw)
}

impl Ceremonies {
    /// A relying party for `rp_id` (a host name) at `origin` (`https://host[:port]`). Subdomains
    /// and other ports are not accepted.
    pub fn new(rp_id: &str, origin: &str, rp_name: &str) -> Result<Self, CeremonyError> {
        let url = Url::parse(origin).map_err(|_| CeremonyError::Config)?;
        let webauthn = WebauthnBuilder::new(rp_id, &url)
            .map_err(|_| CeremonyError::Config)?
            .rp_name(rp_name)
            .build()
            .map_err(|_| CeremonyError::Config)?;
        Ok(Self {
            webauthn,
            origin: url.origin().ascii_serialization(),
            rp_id: rp_id.to_owned(),
        })
    }

    /// The exact origin assertions must carry.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// The relying-party id.
    pub fn rp_id(&self) -> &str {
        &self.rp_id
    }

    /// Starts registering a passkey for `user`, excluding the ones they already have.
    pub fn start_registration(
        &self,
        user: &PasskeyUser,
        existing: &[StoredPasskey],
    ) -> Result<(CreationChallengeResponse, PasskeyRegistration), CeremonyError> {
        let exclude = existing
            .iter()
            .map(|p| p.credential.cred_id().clone())
            .collect::<Vec<_>>();
        let exclude = (!exclude.is_empty()).then_some(exclude);
        Ok(self.webauthn.start_passkey_registration(
            user.handle,
            &user.name,
            &user.display_name,
            exclude,
        )?)
    }

    /// Finishes a registration. `name` is validated against `existing`; `created` is the RFC 3339
    /// UTC time to record. The caller stores the result ([`PasskeyStore::add_passkey`]).
    pub fn finish_registration(
        &self,
        response: &RegisterPublicKeyCredential,
        state: &PasskeyRegistration,
        name: &str,
        existing: &[StoredPasskey],
        created: &str,
    ) -> Result<StoredPasskey, CeremonyError> {
        let name = validate_name(name, existing).ok_or(CeremonyError::InvalidName)?;
        let credential = self.webauthn.finish_passkey_registration(response, state)?;
        let stored = StoredPasskey {
            id: credential_id_string(credential.cred_id().as_ref()),
            name,
            created: created.to_owned(),
            last_used: None,
            credential,
        };
        // Only keys a host can verify are accepted, so every passkey can approve overrides.
        stored.pinned_key().ok_or(CeremonyError::UnsupportedKey)?;
        Ok(stored)
    }

    /// Starts an assertion against the person's passkeys, with a random challenge.
    pub fn start_authentication(
        &self,
        passkeys: &[StoredPasskey],
    ) -> Result<(RequestChallengeResponse, PasskeyAuthentication), CeremonyError> {
        if passkeys.is_empty() {
            return Err(CeremonyError::UnknownCredential);
        }
        let creds = passkeys
            .iter()
            .map(|p| p.credential.clone())
            .collect::<Vec<_>>();
        Ok(self.webauthn.start_passkey_authentication(&creds)?)
    }

    /// Finishes an assertion: webauthn-rs checks origin, challenge, user verification, signature
    /// and counter. The matching passkey's counter and `last_used` are then saved in `store`.
    pub async fn finish_authentication<S: PasskeyStore>(
        &self,
        store: &S,
        user: &str,
        response: &PublicKeyCredential,
        state: &PasskeyAuthentication,
        now: &str,
    ) -> Result<AuthenticationResult, CeremonyError<S::Error>> {
        let result = self
            .webauthn
            .finish_passkey_authentication(response, state)
            .map_err(CeremonyError::Webauthn)?;
        let mut passkeys = store.passkeys(user).await.map_err(CeremonyError::Store)?;
        let stored = passkeys
            .iter_mut()
            .find(|p| p.credential.cred_id() == result.cred_id())
            .ok_or(CeremonyError::UnknownCredential)?;
        stored.credential.update_credential(&result);
        stored.last_used = Some(now.to_owned());
        store
            .update_passkey(user, stored)
            .await
            .map_err(CeremonyError::Store)?;
        Ok(result)
    }

    /// Request options for an assertion over `challenge` (for instance Keel's 32-byte override
    /// challenge), allowing the given passkeys, with user verification required.
    pub fn bound_request(
        &self,
        passkeys: &[StoredPasskey],
        challenge: &[u8],
    ) -> Result<RequestChallengeResponse, CeremonyError> {
        let (mut request, _random_state) = self.start_authentication(passkeys)?;
        request.public_key.challenge = Base64UrlSafeData::from(challenge);
        Ok(request)
    }

    /// Verifies an assertion over `challenge` made with one of `passkeys`, and returns that
    /// passkey. Uses [`verify_assertion`]: type, challenge, origin, RP id, UP and UV, signature.
    pub fn verify_bound_assertion<'a>(
        &self,
        passkeys: &'a [StoredPasskey],
        response: &PublicKeyCredential,
        challenge: &[u8],
    ) -> Result<&'a StoredPasskey, CeremonyError> {
        let id = credential_id_string(response.raw_id.as_ref());
        let stored = passkeys
            .iter()
            .find(|p| p.id == id)
            .ok_or(CeremonyError::UnknownCredential)?;
        let (key, alg) = stored.pinned_key().ok_or(CeremonyError::UnsupportedKey)?;
        verify_assertion(
            &key,
            alg,
            response.response.authenticator_data.as_ref(),
            response.response.client_data_json.as_ref(),
            response.response.signature.as_ref(),
            challenge,
            &self.origin,
            &self.rp_id,
        )?;
        Ok(stored)
    }
}
