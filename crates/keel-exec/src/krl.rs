//! OpenSSH key revocation lists (the subset Keel needs), per OpenSSH's `PROTOCOL.krl`.
//!
//! ssh-key does not implement KRLs, so this module encodes and parses the binary format itself:
//!
//! - **Written:** certificate sections (per CA: serial lists, serial ranges, key ids) and the
//!   explicit-key section.
//! - **Also read:** serial bitmaps and SHA-256 fingerprint sections, as `ssh-keygen -k` may
//!   write them.
//! - **Refused** (fail closed): signed KRLs, SHA-1 fingerprint sections and unknown sections.
//!
//! The output is accepted by `ssh-keygen -Q`; tests/ca.rs cross-checks against it when
//! `ssh-keygen` is installed.

use std::collections::{BTreeMap, BTreeSet};

use russh::keys::ssh_encoding::Encode;
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Certificate, HashAlg};

const MAGIC: u64 = 0x5353_484b_524c_0a00; // "SSHKRL\n\0"
const FORMAT_VERSION: u32 = 1;

const SECTION_CERTIFICATES: u8 = 1;
const SECTION_EXPLICIT_KEY: u8 = 2;
const SECTION_FINGERPRINT_SHA1: u8 = 3;
const SECTION_SIGNATURE: u8 = 4;
const SECTION_FINGERPRINT_SHA256: u8 = 5;

const CERT_SERIAL_LIST: u8 = 0x20;
const CERT_SERIAL_RANGE: u8 = 0x21;
const CERT_SERIAL_BITMAP: u8 = 0x22;
const CERT_KEY_ID: u8 = 0x23;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KrlError {
    #[error("not a KRL (bad magic)")]
    BadMagic,
    #[error("unsupported KRL format version {0}")]
    FormatVersion(u32),
    #[error("truncated KRL")]
    Truncated,
    #[error("unsupported KRL section {0:#04x}")]
    UnsupportedSection(u8),
    #[error("malformed KRL: {0}")]
    Malformed(&'static str),
}

/// Revocations scoped to one CA (or, with an empty key blob, to any CA).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CaRevocations {
    serials: BTreeSet<u64>,
    ranges: BTreeSet<(u64, u64)>,
    /// `(offset, big-endian bitmap)`: bit `i` (from the least significant end) revokes
    /// `offset + i`. Only ever read, never written.
    bitmaps: Vec<(u64, Vec<u8>)>,
    key_ids: BTreeSet<String>,
}

impl CaRevocations {
    fn serial_revoked(&self, serial: u64) -> bool {
        self.serials.contains(&serial)
            || self.ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&serial))
            || self.bitmaps.iter().any(|(offset, bits)| bitmap_has(*offset, bits, serial))
    }
}

fn bitmap_has(offset: u64, bits: &[u8], serial: u64) -> bool {
    let Some(i) = serial.checked_sub(offset) else { return false };
    let byte = i / 8;
    if byte >= bits.len() as u64 {
        return false;
    }
    let idx = bits.len() - 1 - byte as usize;
    bits[idx] & (1 << (i % 8)) != 0
}

/// A key revocation list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Krl {
    /// Monotonic version; the server bumps it on every change.
    pub version: u64,
    /// Unix seconds.
    pub generated_date: u64,
    pub comment: String,
    /// Keyed by the CA's public key blob; an empty blob means "any CA".
    certs: BTreeMap<Vec<u8>, CaRevocations>,
    /// Public key blobs revoked outright.
    keys: BTreeSet<Vec<u8>>,
    /// SHA-256 digests of revoked public key blobs.
    sha256: BTreeSet<Vec<u8>>,
}

fn blob(key: &KeyData) -> Vec<u8> {
    // Encoding a key that ssh-key parsed or generated cannot fail.
    key.encode_vec().expect("encode a public key blob")
}

impl Krl {
    pub fn new(version: u64, generated_date: u64) -> Self {
        Krl { version, generated_date, ..Krl::default() }
    }

    fn ca(&mut self, ca: &KeyData) -> &mut CaRevocations {
        self.certs.entry(blob(ca)).or_default()
    }

    /// Revoke the certificate with `serial` issued by `ca`.
    pub fn revoke_serial(&mut self, ca: &KeyData, serial: u64) {
        self.ca(ca).serials.insert(serial);
    }

    /// Revoke serials `min..=max` issued by `ca`.
    pub fn revoke_serial_range(&mut self, ca: &KeyData, min: u64, max: u64) {
        let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
        self.ca(ca).ranges.insert((lo, hi));
    }

    /// Revoke every certificate with `key_id` issued by `ca`.
    pub fn revoke_key_id(&mut self, ca: &KeyData, key_id: &str) {
        self.ca(ca).key_ids.insert(key_id.to_owned());
    }

    /// Revoke a public key outright (as a bare key, and any certificate for it).
    pub fn revoke_key(&mut self, key: &KeyData) {
        self.keys.insert(blob(key));
    }

    /// Whether a bare public key is revoked.
    pub fn is_key_revoked(&self, key: &KeyData) -> bool {
        if self.keys.contains(&blob(key)) {
            return true;
        }
        match key.fingerprint(HashAlg::Sha256).sha256() {
            Some(d) => self.sha256.contains(d.as_slice()),
            None => false,
        }
    }

    /// Whether a certificate is revoked: its key or its CA key is revoked, or its CA's section
    /// (or the any-CA section) lists its key id or serial. Serial 0 is never matched by serial,
    /// as in OpenSSH.
    pub fn is_certificate_revoked(&self, cert: &Certificate) -> bool {
        if self.is_key_revoked(cert.public_key()) || self.is_key_revoked(cert.signature_key()) {
            return true;
        }
        let ca_blob = blob(cert.signature_key());
        for scope in [Some(ca_blob.as_slice()), Some(&[][..])] {
            let Some(rev) = scope.and_then(|b| self.certs.get(b)) else { continue };
            if rev.key_ids.contains(cert.key_id()) {
                return true;
            }
            if cert.serial() != 0 && rev.serial_revoked(cert.serial()) {
                return true;
            }
        }
        false
    }

    /// The binary KRL, as `ssh-keygen -k` writes it (unsigned).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Vec::new();
        put_u64(&mut w, MAGIC);
        put_u32(&mut w, FORMAT_VERSION);
        put_u64(&mut w, self.version);
        put_u64(&mut w, self.generated_date);
        put_u64(&mut w, 0); // flags
        put_string(&mut w, b""); // reserved
        put_string(&mut w, self.comment.as_bytes());

        for (ca, rev) in &self.certs {
            let mut s = Vec::new();
            put_string(&mut s, ca);
            put_string(&mut s, b""); // reserved
            if !rev.serials.is_empty() {
                let mut list = Vec::new();
                for serial in &rev.serials {
                    put_u64(&mut list, *serial);
                }
                s.push(CERT_SERIAL_LIST);
                put_string(&mut s, &list);
            }
            for (lo, hi) in &rev.ranges {
                let mut range = Vec::new();
                put_u64(&mut range, *lo);
                put_u64(&mut range, *hi);
                s.push(CERT_SERIAL_RANGE);
                put_string(&mut s, &range);
            }
            if !rev.key_ids.is_empty() {
                let mut ids = Vec::new();
                for id in &rev.key_ids {
                    put_string(&mut ids, id.as_bytes());
                }
                s.push(CERT_KEY_ID);
                put_string(&mut s, &ids);
            }
            // Bitmaps are only read; they would be re-emitted as is.
            for (offset, bits) in &rev.bitmaps {
                let mut b = Vec::new();
                put_u64(&mut b, *offset);
                put_string(&mut b, bits);
                s.push(CERT_SERIAL_BITMAP);
                put_string(&mut s, &b);
            }
            w.push(SECTION_CERTIFICATES);
            put_string(&mut w, &s);
        }
        for (kind, set) in [(SECTION_EXPLICIT_KEY, &self.keys), (SECTION_FINGERPRINT_SHA256, &self.sha256)] {
            if set.is_empty() {
                continue;
            }
            let mut s = Vec::new();
            // OpenSSH requires these sorted (BTreeSet order matches its blob comparison).
            for b in set {
                put_string(&mut s, b);
            }
            w.push(kind);
            put_string(&mut w, &s);
        }
        w
    }

    /// Parse a binary KRL. Anything this module does not understand is an error.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, KrlError> {
        let mut r = Reader(bytes);
        if r.u64()? != MAGIC {
            return Err(KrlError::BadMagic);
        }
        let format = r.u32()?;
        if format != FORMAT_VERSION {
            return Err(KrlError::FormatVersion(format));
        }
        let mut krl = Krl::new(r.u64()?, r.u64()?);
        let _flags = r.u64()?;
        let _reserved = r.string()?;
        krl.comment = String::from_utf8(r.string()?.to_vec()).map_err(|_| KrlError::Malformed("comment"))?;

        while !r.0.is_empty() {
            let kind = r.u8()?;
            let mut s = Reader(r.string()?);
            match kind {
                SECTION_CERTIFICATES => {
                    let ca = s.string()?.to_vec();
                    let _reserved = s.string()?;
                    let rev = krl.certs.entry(ca).or_default();
                    while !s.0.is_empty() {
                        let sub = s.u8()?;
                        let mut d = Reader(s.string()?);
                        match sub {
                            CERT_SERIAL_LIST => {
                                while !d.0.is_empty() {
                                    rev.serials.insert(d.u64()?);
                                }
                            }
                            CERT_SERIAL_RANGE => {
                                let (lo, hi) = (d.u64()?, d.u64()?);
                                if lo > hi {
                                    return Err(KrlError::Malformed("serial range"));
                                }
                                rev.ranges.insert((lo, hi));
                                d.end()?;
                            }
                            CERT_SERIAL_BITMAP => {
                                let offset = d.u64()?;
                                let bits = d.string()?.to_vec();
                                rev.bitmaps.push((offset, bits));
                                d.end()?;
                            }
                            CERT_KEY_ID => {
                                while !d.0.is_empty() {
                                    let id = String::from_utf8(d.string()?.to_vec())
                                        .map_err(|_| KrlError::Malformed("key id"))?;
                                    rev.key_ids.insert(id);
                                }
                            }
                            other => return Err(KrlError::UnsupportedSection(other)),
                        }
                    }
                }
                SECTION_EXPLICIT_KEY | SECTION_FINGERPRINT_SHA256 => {
                    while !s.0.is_empty() {
                        let b = s.string()?.to_vec();
                        if kind == SECTION_EXPLICIT_KEY {
                            krl.keys.insert(b);
                        } else {
                            krl.sha256.insert(b);
                        }
                    }
                }
                SECTION_FINGERPRINT_SHA1 | SECTION_SIGNATURE => return Err(KrlError::UnsupportedSection(kind)),
                other => return Err(KrlError::UnsupportedSection(other)),
            }
        }
        Ok(krl)
    }
}

fn put_u32(w: &mut Vec<u8>, v: u32) {
    w.extend_from_slice(&v.to_be_bytes());
}

fn put_u64(w: &mut Vec<u8>, v: u64) {
    w.extend_from_slice(&v.to_be_bytes());
}

fn put_string(w: &mut Vec<u8>, s: &[u8]) {
    put_u32(w, u32::try_from(s.len()).expect("KRL field under 4 GiB"));
    w.extend_from_slice(s);
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], KrlError> {
        if self.0.len() < n {
            return Err(KrlError::Truncated);
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, KrlError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, KrlError> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().expect("4 bytes")))
    }

    fn u64(&mut self) -> Result<u64, KrlError> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().expect("8 bytes")))
    }

    fn string(&mut self) -> Result<&'a [u8], KrlError> {
        let n = self.u32()? as usize;
        self.take(n)
    }

    fn end(&self) -> Result<(), KrlError> {
        if self.0.is_empty() { Ok(()) } else { Err(KrlError::Malformed("trailing bytes")) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmap_bits_count_from_the_least_significant_end() {
        // 0b0000_0101 0b0000_0010 → offset+1, offset+8, offset+10 (big-endian bytes).
        let bits = [0b0000_0101u8, 0b0000_0010];
        let hits: Vec<u64> = (100..120).filter(|s| bitmap_has(100, &bits, *s)).collect();
        assert_eq!(hits, [101, 108, 110]);
    }

    #[test]
    fn empty_krl_round_trips_and_garbage_is_refused() {
        let mut krl = Krl::new(3, 1_700_000_000);
        krl.comment = "keel".into();
        assert_eq!(Krl::from_bytes(&krl.to_bytes()).unwrap(), krl);
        assert_eq!(Krl::from_bytes(b"nope"), Err(KrlError::Truncated));
        assert_eq!(Krl::from_bytes(&[0u8; 40]), Err(KrlError::BadMagic));
        let mut signed = krl.to_bytes();
        signed.push(SECTION_SIGNATURE);
        signed.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(Krl::from_bytes(&signed), Err(KrlError::UnsupportedSection(SECTION_SIGNATURE)));
        let mut cut = krl.to_bytes();
        cut.extend_from_slice(&[SECTION_EXPLICIT_KEY, 0, 0, 0, 9, 1]);
        assert_eq!(Krl::from_bytes(&cut), Err(KrlError::Truncated));
    }
}
