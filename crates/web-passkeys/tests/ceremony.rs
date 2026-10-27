//! Registration and authentication with a software passkey, and assertions over an application
//! challenge (Keel's override challenge) checked by `verify_assertion`.

mod common;

use common::MemStore;
use web_passkeys::ceremony::validate_name;
use web_passkeys::verify::FLAG_UV;
use web_passkeys::{
    Ceremonies, CeremonyError, CoseAlg, CoseKey, PasskeyStore, PasskeyUser, StoredPasskey,
    VerifyError, verify_assertion,
};
use webauthn_authenticator_rs::WebauthnAuthenticator;
use webauthn_authenticator_rs::softpasskey::SoftPasskey;
use webauthn_rs::prelude::{PublicKeyCredential, RequestChallengeResponse, Url, Uuid};

const RP_ID: &str = "keel.example.test";
const ORIGIN: &str = "https://keel.example.test";
const USER: &str = "alice";

fn rp() -> Ceremonies {
    Ceremonies::new(RP_ID, ORIGIN, "Keel").expect("relying party")
}

fn origin() -> Url {
    Url::parse(ORIGIN).unwrap()
}

fn user() -> PasskeyUser {
    PasskeyUser {
        handle: Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef),
        name: USER.into(),
        display_name: "Alice".into(),
    }
}

/// Registers a software passkey for alice and stores it.
async fn enrol(
    rp: &Ceremonies,
    store: &MemStore,
    auth: &mut WebauthnAuthenticator<SoftPasskey>,
    name: &str,
) -> StoredPasskey {
    let existing = store.passkeys(USER).await.unwrap();
    let (ccr, state) = rp.start_registration(&user(), &existing).unwrap();
    let reg = auth
        .do_registration(origin(), ccr)
        .expect("softpasskey registers");
    let stored = rp
        .finish_registration(&reg, &state, name, &existing, "2026-10-27T12:00:00Z")
        .expect("registration verifies");
    store.add_passkey(USER, stored.clone()).await.unwrap();
    stored
}

fn authenticator() -> WebauthnAuthenticator<SoftPasskey> {
    // `true` lets the soft token claim user verification, which passkeys require.
    WebauthnAuthenticator::new(SoftPasskey::new(true))
}

/// The request with `userVerification` lowered to "discouraged", so the soft token signs with
/// the UV flag clear. Done through serde because webauthn-rs does not export the policy enum.
fn without_uv(request: RequestChallengeResponse) -> RequestChallengeResponse {
    let mut v = serde_json::to_value(&request).unwrap();
    v["publicKey"]["userVerification"] = "discouraged".into();
    serde_json::from_value(v).unwrap()
}

fn check(
    stored: &StoredPasskey,
    cred: &PublicKeyCredential,
    challenge: &[u8],
    origin: &str,
    rp_id: &str,
) -> Result<(), VerifyError> {
    let (key, alg) = stored.pinned_key().unwrap();
    verify_assertion(
        &key,
        alg,
        cred.response.authenticator_data.as_ref(),
        cred.response.client_data_json.as_ref(),
        cred.response.signature.as_ref(),
        challenge,
        origin,
        rp_id,
    )
}

#[tokio::test]
async fn registers_and_authenticates_and_saves_the_counter() {
    let rp = rp();
    let store = MemStore::default();
    let mut auth = authenticator();
    let stored = enrol(&rp, &store, &mut auth, "  laptop  ").await;
    assert_eq!(stored.name, "laptop");
    assert_eq!(stored.last_used, None);
    let (key, alg) = stored.pinned_key().expect("pinnable key");
    assert_eq!(alg, CoseAlg::Es256);
    assert_eq!(CoseKey::decode(&key).unwrap().alg(), CoseAlg::Es256);

    let passkeys = store.passkeys(USER).await.unwrap();
    let (rcr, state) = rp.start_authentication(&passkeys).unwrap();
    let cred = auth
        .do_authentication(origin(), rcr)
        .expect("softpasskey asserts");
    let result = rp
        .finish_authentication(&store, USER, &cred, &state, "2026-10-27T12:05:00Z")
        .await
        .expect("assertion verifies");
    assert!(result.user_verified());
    let after = store.passkeys(USER).await.unwrap();
    assert_eq!(after[0].last_used.as_deref(), Some("2026-10-27T12:05:00Z"));

    // An assertion for one ceremony does not finish another (fresh random challenge).
    let (_, other_state) = rp.start_authentication(&after).unwrap();
    let crossed = rp
        .finish_authentication(&store, USER, &cred, &other_state, "2026-10-27T12:06:00Z")
        .await;
    assert!(matches!(crossed, Err(CeremonyError::Webauthn(_))));
}

#[tokio::test]
async fn a_second_passkey_name_must_be_unique_and_short() {
    let rp = rp();
    let store = MemStore::default();
    let mut auth = authenticator();
    enrol(&rp, &store, &mut auth, "laptop").await;
    let existing = store.passkeys(USER).await.unwrap();

    assert!(validate_name("laptop", &existing).is_none());
    assert!(validate_name("   ", &existing).is_none());
    assert!(validate_name(&"x".repeat(65), &existing).is_none());
    assert_eq!(
        validate_name(&"x".repeat(64), &existing).as_deref(),
        Some(&*"x".repeat(64))
    );

    let (ccr, state) = rp.start_registration(&user(), &existing).unwrap();
    let mut other = authenticator();
    let reg = other.do_registration(origin(), ccr).unwrap();
    let dup = rp.finish_registration(&reg, &state, "laptop", &existing, "2026-10-27T12:00:00Z");
    assert!(matches!(dup, Err(CeremonyError::InvalidName)));
}

#[tokio::test]
async fn bound_assertion_over_an_application_challenge_verifies() {
    let rp = rp();
    let store = MemStore::default();
    let mut auth = authenticator();
    enrol(&rp, &store, &mut auth, "laptop").await;
    let passkeys = store.passkeys(USER).await.unwrap();

    let challenge = [0x5a_u8; 32];
    let request = rp.bound_request(&passkeys, &challenge).unwrap();
    let cred = auth.do_authentication(origin(), request).unwrap();

    let who = rp
        .verify_bound_assertion(&passkeys, &cred, &challenge)
        .unwrap();
    assert_eq!(who.id, passkeys[0].id);
    check(&passkeys[0], &cred, &challenge, ORIGIN, RP_ID).expect("pure verify agrees");
}

#[tokio::test]
async fn refuses_wrong_challenge_wrong_origin_missing_uv_and_bad_signature() {
    let rp = rp();
    let store = MemStore::default();
    let mut auth = authenticator();
    enrol(&rp, &store, &mut auth, "laptop").await;
    let passkeys = store.passkeys(USER).await.unwrap();
    let stored = &passkeys[0];

    let challenge = [7_u8; 32];
    let cred = auth
        .do_authentication(origin(), rp.bound_request(&passkeys, &challenge).unwrap())
        .unwrap();

    // Replay: the assertion was made for another challenge.
    let other = [8_u8; 32];
    assert_eq!(
        check(stored, &cred, &other, ORIGIN, RP_ID),
        Err(VerifyError::WrongChallenge)
    );
    assert!(matches!(
        rp.verify_bound_assertion(&passkeys, &cred, &other),
        Err(CeremonyError::Assertion(VerifyError::WrongChallenge))
    ));
    // A shorter or longer challenge is not a prefix match.
    assert_eq!(
        check(stored, &cred, &challenge[..31], ORIGIN, RP_ID),
        Err(VerifyError::WrongChallenge)
    );

    // Wrong origin, and wrong RP id.
    assert_eq!(
        check(
            stored,
            &cred,
            &challenge,
            "https://evil.example.test",
            RP_ID
        ),
        Err(VerifyError::WrongOrigin)
    );
    assert_eq!(
        check(stored, &cred, &challenge, ORIGIN, "evil.example.test"),
        Err(VerifyError::WrongRpId)
    );

    // An assertion actually made at a subdomain: the RP id still matches, the origin does not.
    let sub = Url::parse("https://sub.keel.example.test").unwrap();
    let elsewhere = auth
        .do_authentication(sub, rp.bound_request(&passkeys, &challenge).unwrap())
        .expect("the soft token signs for a subdomain of the RP id");
    assert_eq!(
        check(stored, &elsewhere, &challenge, ORIGIN, RP_ID),
        Err(VerifyError::WrongOrigin)
    );

    // UV missing: a genuinely signed assertion with the UV flag clear.
    let no_uv = auth
        .do_authentication(
            origin(),
            without_uv(rp.bound_request(&passkeys, &challenge).unwrap()),
        )
        .unwrap();
    assert_eq!(no_uv.response.authenticator_data.as_ref()[32] & FLAG_UV, 0);
    assert_eq!(
        check(stored, &no_uv, &challenge, ORIGIN, RP_ID),
        Err(VerifyError::UserNotVerified)
    );

    // Bad signature: one bit flipped in the signature, then in the signed authenticator data.
    let mut bad = cred.clone();
    let mut sig = bad.response.signature.as_ref().to_vec();
    let last = sig.len() - 1;
    sig[last] ^= 1;
    bad.response.signature = sig.into();
    assert_eq!(
        check(stored, &bad, &challenge, ORIGIN, RP_ID),
        Err(VerifyError::BadSignature)
    );

    let mut bad = cred.clone();
    let mut ad = bad.response.authenticator_data.as_ref().to_vec();
    ad[33] ^= 1; // counter byte: still well-formed, no longer what was signed
    bad.response.authenticator_data = ad.into();
    assert_eq!(
        check(stored, &bad, &challenge, ORIGIN, RP_ID),
        Err(VerifyError::BadSignature)
    );

    // A key that is not the signer's.
    let mut auth2 = authenticator();
    let store2 = MemStore::default();
    let other_key = enrol(&rp, &store2, &mut auth2, "phone").await;
    assert_eq!(
        check(&other_key, &cred, &challenge, ORIGIN, RP_ID),
        Err(VerifyError::BadSignature)
    );
    // The wrong pinned algorithm.
    let (key, _) = stored.pinned_key().unwrap();
    assert_eq!(
        verify_assertion(
            &key,
            CoseAlg::EdDsa,
            cred.response.authenticator_data.as_ref(),
            cred.response.client_data_json.as_ref(),
            cred.response.signature.as_ref(),
            &challenge,
            ORIGIN,
            RP_ID,
        ),
        Err(VerifyError::BadPublicKey)
    );
}

#[tokio::test]
async fn an_unknown_credential_is_refused() {
    let rp = rp();
    let store = MemStore::default();
    let mut auth = authenticator();
    enrol(&rp, &store, &mut auth, "laptop").await;
    let passkeys = store.passkeys(USER).await.unwrap();
    let challenge = [1_u8; 32];
    let cred = auth
        .do_authentication(origin(), rp.bound_request(&passkeys, &challenge).unwrap())
        .unwrap();

    let store2 = MemStore::default();
    let mut auth2 = authenticator();
    enrol(&rp, &store2, &mut auth2, "phone").await;
    let theirs = store2.passkeys(USER).await.unwrap();
    assert!(matches!(
        rp.verify_bound_assertion(&theirs, &cred, &challenge),
        Err(CeremonyError::UnknownCredential)
    ));
    assert!(matches!(
        rp.start_authentication(&[]),
        Err(CeremonyError::UnknownCredential)
    ));
}
