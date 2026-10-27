//! `verify_assertion` against fixed vectors: assertions built byte by byte from fixed ES256 and
//! Ed25519 keys (both signature schemes are deterministic), so every field can be varied alone.

use p256::ecdsa::signature::Signer as _;
use sha2::{Digest, Sha256};
use web_passkeys::verify::{FLAG_AT, FLAG_ED, FLAG_UP, FLAG_UV};
use web_passkeys::{CoseAlg, CoseKey, VerifyError, verify_assertion};

const RP_ID: &str = "keel.example.test";
const ORIGIN: &str = "https://keel.example.test";
const CHALLENGE: [u8; 32] = [0xc4; 32];
/// base64url (no padding) of `CHALLENGE`.
const CHALLENGE_B64: &str = "xMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMQ";

enum Signer {
    Es256(p256::ecdsa::SigningKey),
    EdDsa(ed25519_dalek::SigningKey),
}

impl Signer {
    fn es256() -> Self {
        Signer::Es256(p256::ecdsa::SigningKey::from_slice(&[0x11; 32]).unwrap())
    }

    fn eddsa() -> Self {
        Signer::EdDsa(ed25519_dalek::SigningKey::from_bytes(&[0x22; 32]))
    }

    fn cose(&self) -> (Vec<u8>, CoseAlg) {
        let key = match self {
            Signer::Es256(k) => {
                let point = k.verifying_key().to_encoded_point(false);
                CoseKey::Es256 {
                    x: (*point.x().unwrap()).into(),
                    y: (*point.y().unwrap()).into(),
                }
            }
            Signer::EdDsa(k) => CoseKey::EdDsa {
                x: k.verifying_key().to_bytes(),
            },
        };
        (key.encode(), key.alg())
    }

    fn sign(&self, msg: &[u8]) -> Vec<u8> {
        match self {
            Signer::Es256(k) => {
                let sig: p256::ecdsa::Signature = k.sign(msg);
                sig.to_der().as_bytes().to_vec()
            }
            Signer::EdDsa(k) => {
                use ed25519_dalek::Signer as _;
                k.sign(msg).to_bytes().to_vec()
            }
        }
    }
}

struct Assertion {
    auth_data: Vec<u8>,
    client_data: Vec<u8>,
    signature: Vec<u8>,
}

fn auth_data(rp_id: &str, flags: u8, counter: u32) -> Vec<u8> {
    let mut v = Sha256::digest(rp_id.as_bytes()).to_vec();
    v.push(flags);
    v.extend_from_slice(&counter.to_be_bytes());
    v
}

fn client_data(type_: &str, challenge: &str, origin: &str, extra: &str) -> Vec<u8> {
    format!(r#"{{"type":"{type_}","challenge":"{challenge}","origin":"{origin}"{extra}}}"#)
        .into_bytes()
}

fn signed(signer: &Signer, auth_data: Vec<u8>, client_data: Vec<u8>) -> Assertion {
    let mut msg = auth_data.clone();
    msg.extend_from_slice(&Sha256::digest(&client_data));
    let signature = signer.sign(&msg);
    Assertion {
        auth_data,
        client_data,
        signature,
    }
}

fn good(signer: &Signer) -> Assertion {
    signed(
        signer,
        auth_data(RP_ID, FLAG_UP | FLAG_UV, 7),
        client_data(
            "webauthn.get",
            CHALLENGE_B64,
            ORIGIN,
            r#","crossOrigin":false"#,
        ),
    )
}

fn verify(signer: &Signer, a: &Assertion) -> Result<(), VerifyError> {
    let (key, alg) = signer.cose();
    verify_assertion(
        &key,
        alg,
        &a.auth_data,
        &a.client_data,
        &a.signature,
        &CHALLENGE,
        ORIGIN,
        RP_ID,
    )
}

fn both() -> [Signer; 2] {
    [Signer::es256(), Signer::eddsa()]
}

#[test]
fn the_challenge_constant_is_its_base64url() {
    use base64::Engine as _;
    assert_eq!(
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(CHALLENGE),
        CHALLENGE_B64
    );
}

#[test]
fn fixed_vectors_verify() {
    for s in both() {
        assert_eq!(verify(&s, &good(&s)), Ok(()));
    }
}

#[test]
fn fixed_ed25519_vector_is_stable() {
    // The Ed25519 signature is deterministic: pin it so the encoding of the signed message
    // (authenticator data ‖ SHA-256(client data)) cannot drift unnoticed.
    let s = Signer::eddsa();
    let a = good(&s);
    assert_eq!(a.signature.len(), 64);
    assert_eq!(
        hex(&Sha256::digest(&a.signature)),
        "ce3b6da94737e40f975052f74bd6ff16a8239f2ac202aa53d5d2c0607ae8f867"
    );
    // The trailing-slash origin form is accepted for the same origin.
    let slash = signed(
        &s,
        auth_data(RP_ID, FLAG_UP | FLAG_UV, 7),
        client_data(
            "webauthn.get",
            CHALLENGE_B64,
            "https://keel.example.test/",
            "",
        ),
    );
    assert_eq!(verify(&s, &slash), Ok(()));
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn wrong_challenge_is_a_replay() {
    for s in both() {
        // Same bytes, one changed.
        let mut other = CHALLENGE_B64.to_owned();
        other.replace_range(0..1, "y");
        let a = signed(
            &s,
            auth_data(RP_ID, FLAG_UP | FLAG_UV, 7),
            client_data("webauthn.get", &other, ORIGIN, ""),
        );
        assert_eq!(verify(&s, &a), Err(VerifyError::WrongChallenge));
        // Padded base64 is not the same encoding.
        let padded = format!("{CHALLENGE_B64}=");
        let a = signed(
            &s,
            auth_data(RP_ID, FLAG_UP | FLAG_UV, 7),
            client_data("webauthn.get", &padded, ORIGIN, ""),
        );
        assert_eq!(verify(&s, &a), Err(VerifyError::WrongChallenge));
    }
}

#[test]
fn wrong_origin_type_rp_and_cross_origin_are_refused() {
    for s in both() {
        let flags = FLAG_UP | FLAG_UV;
        let cases = [
            (
                client_data(
                    "webauthn.get",
                    CHALLENGE_B64,
                    "https://keel.example.test:8443",
                    "",
                ),
                RP_ID,
                VerifyError::WrongOrigin,
            ),
            (
                client_data(
                    "webauthn.get",
                    CHALLENGE_B64,
                    "http://keel.example.test",
                    "",
                ),
                RP_ID,
                VerifyError::WrongOrigin,
            ),
            (
                client_data(
                    "webauthn.get",
                    CHALLENGE_B64,
                    "https://keel.example.test//",
                    "",
                ),
                RP_ID,
                VerifyError::WrongOrigin,
            ),
            (
                client_data(
                    "webauthn.get",
                    CHALLENGE_B64,
                    "https://keel.example.test/x",
                    "",
                ),
                RP_ID,
                VerifyError::WrongOrigin,
            ),
            (
                client_data(
                    "webauthn.get",
                    CHALLENGE_B64,
                    "https://keel.example.test.evil",
                    "",
                ),
                RP_ID,
                VerifyError::WrongOrigin,
            ),
            (
                client_data("webauthn.create", CHALLENGE_B64, ORIGIN, ""),
                RP_ID,
                VerifyError::WrongType,
            ),
            (
                client_data(
                    "webauthn.get",
                    CHALLENGE_B64,
                    ORIGIN,
                    r#","crossOrigin":true"#,
                ),
                RP_ID,
                VerifyError::CrossOrigin,
            ),
            (
                client_data("webauthn.get", CHALLENGE_B64, ORIGIN, ""),
                "other.example.test",
                VerifyError::WrongRpId,
            ),
        ];
        for (cd, rp, want) in cases {
            let a = signed(&s, auth_data(rp, flags, 1), cd);
            assert_eq!(verify(&s, &a), Err(want));
        }
        let a = signed(&s, auth_data(RP_ID, flags, 1), b"not json".to_vec());
        assert_eq!(verify(&s, &a), Err(VerifyError::ClientDataMalformed));
    }
}

#[test]
fn missing_uv_or_up_is_refused_even_when_signed() {
    for s in both() {
        let cd = || client_data("webauthn.get", CHALLENGE_B64, ORIGIN, "");
        let a = signed(&s, auth_data(RP_ID, FLAG_UP, 1), cd());
        assert_eq!(verify(&s, &a), Err(VerifyError::UserNotVerified));
        let a = signed(&s, auth_data(RP_ID, FLAG_UV, 1), cd());
        assert_eq!(verify(&s, &a), Err(VerifyError::UserNotPresent));

        // Flipping UV on in a UV-less assertion breaks the signature.
        let mut a = signed(&s, auth_data(RP_ID, FLAG_UP, 1), cd());
        a.auth_data[32] |= FLAG_UV;
        assert_eq!(verify(&s, &a), Err(VerifyError::BadSignature));
    }
}

#[test]
fn malformed_authenticator_data_is_refused() {
    for s in both() {
        let cd = || client_data("webauthn.get", CHALLENGE_B64, ORIGIN, "");
        // Too short.
        let mut short = auth_data(RP_ID, FLAG_UP | FLAG_UV, 1);
        short.pop();
        let a = signed(&s, short, cd());
        assert_eq!(verify(&s, &a), Err(VerifyError::AuthenticatorDataMalformed));
        // Trailing bytes without the extension flag.
        let mut trailing = auth_data(RP_ID, FLAG_UP | FLAG_UV, 1);
        trailing.push(0);
        let a = signed(&s, trailing, cd());
        assert_eq!(verify(&s, &a), Err(VerifyError::AuthenticatorDataMalformed));
        // Attested credential data has no place in an assertion.
        let a = signed(&s, auth_data(RP_ID, FLAG_UP | FLAG_UV | FLAG_AT, 1), cd());
        assert_eq!(verify(&s, &a), Err(VerifyError::AuthenticatorDataMalformed));
        // Extension data is allowed when flagged.
        let mut ext = auth_data(RP_ID, FLAG_UP | FLAG_UV | FLAG_ED, 1);
        ext.extend_from_slice(&[0xa0]); // empty CBOR map
        let a = signed(&s, ext, cd());
        assert_eq!(verify(&s, &a), Ok(()));
    }
}

#[test]
fn bad_signatures_and_keys_are_refused() {
    for s in both() {
        let good = good(&s);

        let mut a = good.signature.clone();
        let n = a.len() - 1;
        a[n] ^= 0x01;
        let bad = Assertion {
            signature: a,
            ..good_clone(&good)
        };
        assert_eq!(verify(&s, &bad), Err(VerifyError::BadSignature));

        let bad = Assertion {
            signature: Vec::new(),
            ..good_clone(&good)
        };
        assert_eq!(verify(&s, &bad), Err(VerifyError::BadSignature));

        // Client data changed after signing (same challenge, extra field).
        let bad = Assertion {
            client_data: client_data("webauthn.get", CHALLENGE_B64, ORIGIN, r#","x":1"#),
            ..good_clone(&good)
        };
        assert_eq!(verify(&s, &bad), Err(VerifyError::BadSignature));
    }

    // The other scheme's key, and a key/alg mismatch.
    let es = Signer::es256();
    let ed = Signer::eddsa();
    let a = good(&es);
    let (ed_key, _) = ed.cose();
    let (es_key, _) = es.cose();
    let run = |key: &[u8], alg| {
        verify_assertion(
            key,
            alg,
            &a.auth_data,
            &a.client_data,
            &a.signature,
            &CHALLENGE,
            ORIGIN,
            RP_ID,
        )
    };
    assert_eq!(run(&ed_key, CoseAlg::EdDsa), Err(VerifyError::BadSignature));
    assert_eq!(run(&es_key, CoseAlg::EdDsa), Err(VerifyError::BadPublicKey));
    assert_eq!(run(b"\xa0", CoseAlg::Es256), Err(VerifyError::BadPublicKey));
    // A P-256 point that is not on the curve.
    let off_curve = CoseKey::Es256 {
        x: [1; 32],
        y: [1; 32],
    }
    .encode();
    assert_eq!(
        run(&off_curve, CoseAlg::Es256),
        Err(VerifyError::BadPublicKey)
    );
    // Another ES256 key.
    let other = Signer::Es256(p256::ecdsa::SigningKey::from_slice(&[0x33; 32]).unwrap());
    let (other_key, _) = other.cose();
    assert_eq!(
        run(&other_key, CoseAlg::Es256),
        Err(VerifyError::BadSignature)
    );
}

fn good_clone(a: &Assertion) -> Assertion {
    Assertion {
        auth_data: a.auth_data.clone(),
        client_data: a.client_data.clone(),
        signature: a.signature.clone(),
    }
}
