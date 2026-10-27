//! COSE_Key encoding for the two supported public keys (RFC 9052 §7, RFC 9053).
//!
//! Approver keys are pinned on hosts as COSE_Key bytes (`approvers.json`, `public_key_cose`), so
//! a host never needs webauthn-rs to read them. Only the shapes WebAuthn uses are accepted: an
//! EC2 P-256 key with `alg` ES256, and an OKP Ed25519 key with `alg` EdDSA. Decoding is strict:
//! a definite-length map with integer labels, no duplicate labels, no trailing bytes.

use webauthn_rs::prelude::{COSEAlgorithm, COSEKey, COSEKeyType, ECDSACurve, EDDSACurve};

use crate::verify::{CoseAlg, VerifyError};

/// A pinned credential public key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoseKey {
    /// EC2 key on P-256, `alg` ES256.
    Es256 {
        /// The affine x coordinate.
        x: [u8; 32],
        /// The affine y coordinate.
        y: [u8; 32],
    },
    /// OKP key on Ed25519, `alg` EdDSA.
    EdDsa {
        /// The public key.
        x: [u8; 32],
    },
}

/// Why COSE_Key bytes were refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CoseError {
    /// Not the CBOR this module reads.
    #[error("malformed CBOR")]
    Malformed,
    /// Valid CBOR, but not a supported key.
    #[error("unsupported COSE key")]
    Unsupported,
}

// COSE labels and values.
const KTY: i64 = 1;
const ALG: i64 = 3;
const CRV: i64 = -1;
const X: i64 = -2;
const Y: i64 = -3;
const KTY_OKP: i64 = 1;
const KTY_EC2: i64 = 2;
const CRV_P256: i64 = 1;
const CRV_ED25519: i64 = 6;

#[derive(Debug, PartialEq, Eq)]
enum Value {
    Int(i64),
    Bytes(Vec<u8>),
}

impl CoseKey {
    /// The key's algorithm.
    pub fn alg(&self) -> CoseAlg {
        match self {
            CoseKey::Es256 { .. } => CoseAlg::Es256,
            CoseKey::EdDsa { .. } => CoseAlg::EdDsa,
        }
    }

    /// Reads COSE_Key bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, CoseError> {
        let map = decode_map(bytes)?;
        let get = |label: i64| map.iter().find(|(l, _)| *l == label).map(|(_, v)| v);
        let int = |label: i64| match get(label) {
            Some(Value::Int(i)) => Ok(*i),
            Some(_) => Err(CoseError::Unsupported),
            None => Err(CoseError::Unsupported),
        };
        let coord = |label: i64| -> Result<[u8; 32], CoseError> {
            match get(label) {
                Some(Value::Bytes(b)) => {
                    b.as_slice().try_into().map_err(|_| CoseError::Unsupported)
                }
                _ => Err(CoseError::Unsupported),
            }
        };
        match (int(KTY)?, int(ALG)?, int(CRV)?) {
            (KTY_EC2, -7, CRV_P256) => {
                if map.len() != 5 {
                    return Err(CoseError::Unsupported);
                }
                Ok(CoseKey::Es256 {
                    x: coord(X)?,
                    y: coord(Y)?,
                })
            }
            (KTY_OKP, -8, CRV_ED25519) => {
                if map.len() != 4 {
                    return Err(CoseError::Unsupported);
                }
                Ok(CoseKey::EdDsa { x: coord(X)? })
            }
            _ => Err(CoseError::Unsupported),
        }
    }

    /// Writes the key as CTAP2-canonical COSE_Key bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(77);
        match self {
            CoseKey::Es256 { x, y } => {
                out.push(0xa5); // map(5)
                put_int(&mut out, KTY);
                put_int(&mut out, KTY_EC2);
                put_int(&mut out, ALG);
                put_int(&mut out, -7);
                put_int(&mut out, CRV);
                put_int(&mut out, CRV_P256);
                put_int(&mut out, X);
                put_bytes(&mut out, x);
                put_int(&mut out, Y);
                put_bytes(&mut out, y);
            }
            CoseKey::EdDsa { x } => {
                out.push(0xa4); // map(4)
                put_int(&mut out, KTY);
                put_int(&mut out, KTY_OKP);
                put_int(&mut out, ALG);
                put_int(&mut out, -8);
                put_int(&mut out, CRV);
                put_int(&mut out, CRV_ED25519);
                put_int(&mut out, X);
                put_bytes(&mut out, x);
            }
        }
        out
    }

    /// The key webauthn-rs recorded for a registered credential, if it is a supported one.
    pub fn from_webauthn(key: &COSEKey) -> Result<Self, CoseError> {
        match (&key.type_, &key.key) {
            (COSEAlgorithm::ES256, COSEKeyType::EC_EC2(ec))
                if ec.curve == ECDSACurve::SECP256R1 =>
            {
                Ok(CoseKey::Es256 {
                    x: ec
                        .x
                        .as_slice()
                        .try_into()
                        .map_err(|_| CoseError::Unsupported)?,
                    y: ec
                        .y
                        .as_slice()
                        .try_into()
                        .map_err(|_| CoseError::Unsupported)?,
                })
            }
            (COSEAlgorithm::EDDSA, COSEKeyType::EC_OKP(okp))
                if okp.curve == EDDSACurve::ED25519 =>
            {
                Ok(CoseKey::EdDsa {
                    x: okp
                        .x
                        .as_slice()
                        .try_into()
                        .map_err(|_| CoseError::Unsupported)?,
                })
            }
            _ => Err(CoseError::Unsupported),
        }
    }

    /// Verifies `signature` over `message` (ES256: DER ECDSA with SHA-256; EdDSA: strict Ed25519).
    pub(crate) fn verify(&self, message: &[u8], signature: &[u8]) -> Result<(), VerifyError> {
        match self {
            CoseKey::Es256 { x, y } => {
                use p256::ecdsa::signature::Verifier as _;
                let point = p256::EncodedPoint::from_affine_coordinates(x.into(), y.into(), false);
                let key = p256::ecdsa::VerifyingKey::from_encoded_point(&point)
                    .map_err(|_| VerifyError::BadPublicKey)?;
                let sig = p256::ecdsa::Signature::from_der(signature)
                    .map_err(|_| VerifyError::BadSignature)?;
                key.verify(message, &sig)
                    .map_err(|_| VerifyError::BadSignature)
            }
            CoseKey::EdDsa { x } => {
                let key = ed25519_dalek::VerifyingKey::from_bytes(x)
                    .map_err(|_| VerifyError::BadPublicKey)?;
                let sig = ed25519_dalek::Signature::from_slice(signature)
                    .map_err(|_| VerifyError::BadSignature)?;
                key.verify_strict(message, &sig)
                    .map_err(|_| VerifyError::BadSignature)
            }
        }
    }
}

fn put_head(out: &mut Vec<u8>, major: u8, n: u64) {
    let m = major << 5;
    if n < 24 {
        out.push(m | n as u8);
    } else if n <= 0xff {
        out.extend_from_slice(&[m | 24, n as u8]);
    } else if n <= 0xffff {
        out.push(m | 25);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else if n <= 0xffff_ffff {
        out.push(m | 26);
        out.extend_from_slice(&(n as u32).to_be_bytes());
    } else {
        out.push(m | 27);
        out.extend_from_slice(&n.to_be_bytes());
    }
}

fn put_int(out: &mut Vec<u8>, i: i64) {
    if i >= 0 {
        put_head(out, 0, i as u64);
    } else {
        put_head(out, 1, (-1 - i) as u64);
    }
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    put_head(out, 2, b.len() as u64);
    out.extend_from_slice(b);
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, CoseError> {
        let b = *self.buf.get(self.pos).ok_or(CoseError::Malformed)?;
        self.pos += 1;
        Ok(b)
    }

    fn take(&mut self, n: usize) -> Result<&[u8], CoseError> {
        let end = self.pos.checked_add(n).ok_or(CoseError::Malformed)?;
        let s = self.buf.get(self.pos..end).ok_or(CoseError::Malformed)?;
        self.pos = end;
        Ok(s)
    }

    /// Reads a head: (major type, argument). Indefinite lengths are refused.
    fn head(&mut self) -> Result<(u8, u64), CoseError> {
        let b = self.byte()?;
        let major = b >> 5;
        let info = b & 0x1f;
        let n = match info {
            0..=23 => u64::from(info),
            24 => u64::from(self.byte()?),
            25 => u64::from(u16::from_be_bytes(
                self.take(2)?.try_into().map_err(|_| CoseError::Malformed)?,
            )),
            26 => u64::from(u32::from_be_bytes(
                self.take(4)?.try_into().map_err(|_| CoseError::Malformed)?,
            )),
            27 => u64::from_be_bytes(self.take(8)?.try_into().map_err(|_| CoseError::Malformed)?),
            _ => return Err(CoseError::Malformed),
        };
        Ok((major, n))
    }

    fn int(&mut self) -> Result<i64, CoseError> {
        match self.head()? {
            (0, n) => i64::try_from(n).map_err(|_| CoseError::Malformed),
            (1, n) => i64::try_from(n)
                .map(|n| -1 - n)
                .map_err(|_| CoseError::Malformed),
            _ => Err(CoseError::Malformed),
        }
    }

    fn value(&mut self) -> Result<Value, CoseError> {
        let save = self.pos;
        match self.head()? {
            (0 | 1, _) => {
                self.pos = save;
                Ok(Value::Int(self.int()?))
            }
            (2, n) => {
                let n = usize::try_from(n).map_err(|_| CoseError::Malformed)?;
                Ok(Value::Bytes(self.take(n)?.to_vec()))
            }
            _ => Err(CoseError::Unsupported),
        }
    }
}

fn decode_map(bytes: &[u8]) -> Result<Vec<(i64, Value)>, CoseError> {
    let mut r = Reader { buf: bytes, pos: 0 };
    let (major, n) = r.head()?;
    if major != 5 || n > 16 {
        return Err(CoseError::Malformed);
    }
    let mut out: Vec<(i64, Value)> = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let label = r.int()?;
        let value = r.value()?;
        if out.iter().any(|(l, _)| *l == label) {
            return Err(CoseError::Malformed);
        }
        out.push((label, value));
    }
    if r.pos != bytes.len() {
        return Err(CoseError::Malformed);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_both_shapes() {
        for key in [
            CoseKey::Es256 {
                x: [1; 32],
                y: [2; 32],
            },
            CoseKey::EdDsa { x: [3; 32] },
        ] {
            assert_eq!(CoseKey::decode(&key.encode()), Ok(key));
        }
    }

    #[test]
    fn refuses_trailing_bytes_duplicates_and_other_keys() {
        let mut bytes = CoseKey::EdDsa { x: [3; 32] }.encode();
        bytes.push(0);
        assert_eq!(CoseKey::decode(&bytes), Err(CoseError::Malformed));

        // map(2) {1: 1, 1: 1}
        assert_eq!(
            CoseKey::decode(&[0xa2, 0x01, 0x01, 0x01, 0x01]),
            Err(CoseError::Malformed)
        );

        // An EC2 key that claims EdDSA.
        let mut es = CoseKey::Es256 {
            x: [1; 32],
            y: [2; 32],
        }
        .encode();
        es[4] = 0x27; // alg -8
        assert_eq!(CoseKey::decode(&es), Err(CoseError::Unsupported));

        // Indefinite-length map.
        assert_eq!(CoseKey::decode(&[0xbf, 0xff]), Err(CoseError::Malformed));
    }
}
