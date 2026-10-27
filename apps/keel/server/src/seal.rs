//! Sealing secrets at rest (spec 055 T024, research "Credentials"): XChaCha20-Poly1305 under a
//! fresh per-item data key, which is itself wrapped by the master key.
//!
//! The master key is 32 random bytes in `--master-key-file`, host-owned and mode `0400`. Keel
//! refuses to start when the file is missing, unreadable, the wrong length, or readable by group
//! or others. [`keygen`] writes a new file and never overwrites one.
//!
//! A sealed blob is `"KS1" ‖ wrap_nonce[24] ‖ wrapped_data_key[48] ‖ nonce[24] ‖ ciphertext`. The
//! caller's `aad` (for example `credential:<id>`) is bound into both layers, so a blob copied onto
//! another row does not open.
//!
//! Errors never carry key material, plaintext or the path's contents.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use rand::TryRng;
use rand::rngs::SysRng;
use secrecy::{ExposeSecret, SecretBox, SecretSlice};

const MAGIC: &[u8; 3] = b"KS1";
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
const WRAPPED_LEN: usize = KEY_LEN + TAG_LEN;
const HEADER_LEN: usize = MAGIC.len() + NONCE_LEN + WRAPPED_LEN + NONCE_LEN;

/// Why sealing, opening or loading failed. Carries no secret material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SealError {
    /// The master key file is missing.
    KeyMissing(String),
    /// The master key file cannot be used (unreadable, wrong length, too open).
    KeyUnusable(String),
    /// The key file already exists (`keygen`).
    KeyExists(String),
    /// The OS random number generator failed.
    Random,
    /// The blob is not a sealed blob.
    Malformed,
    /// Authentication failed: wrong key, wrong `aad` or a modified blob.
    Open,
}

impl fmt::Display for SealError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SealError::KeyMissing(p) => write!(
                f,
                "the master key file {p} does not exist; create it with `keel keygen` (KEEL_MASTER_KEY_FILE)"
            ),
            SealError::KeyUnusable(why) => write!(f, "the master key file cannot be used: {why}"),
            SealError::KeyExists(p) => {
                write!(f, "{p} already exists; keygen never overwrites a key")
            }
            SealError::Random => f.write_str("the OS random number generator is unavailable"),
            SealError::Malformed => f.write_str("the sealed value is malformed"),
            SealError::Open => f.write_str("the sealed value does not open under this master key"),
        }
    }
}

impl std::error::Error for SealError {}

/// The master key. Its Debug output is redacted and the bytes are zeroed on drop.
pub struct MasterKey(SecretBox<[u8; KEY_LEN]>);

impl fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MasterKey([redacted])")
    }
}

impl MasterKey {
    /// Wraps raw key bytes (tests, and [`MasterKey::load`]).
    #[must_use]
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(SecretBox::new(Box::new(bytes)))
    }

    /// Loads the key from `path`, refusing anything but a 32-byte file closed to group and others.
    pub fn load(path: &Path) -> Result<Self, SealError> {
        let shown = path.display().to_string();
        let meta = match fs::metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(SealError::KeyMissing(shown));
            }
            Err(e) => return Err(SealError::KeyUnusable(format!("{shown}: {}", e.kind()))),
        };
        if !meta.is_file() {
            return Err(SealError::KeyUnusable(format!(
                "{shown} is not a regular file"
            )));
        }
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(SealError::KeyUnusable(format!(
                "{shown} has mode {mode:04o}; it must not be readable by group or others (use 0400)"
            )));
        }
        let bytes =
            fs::read(path).map_err(|e| SealError::KeyUnusable(format!("{shown}: {}", e.kind())))?;
        let key: [u8; KEY_LEN] = bytes.as_slice().try_into().map_err(|_| {
            SealError::KeyUnusable(format!("{shown} must hold exactly {KEY_LEN} bytes"))
        })?;
        drop(SecretSlice::from(bytes));
        Ok(Self::from_bytes(key))
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(&(*self.0.expose_secret()).into())
    }

    /// Seals `plaintext` under a fresh data key, bound to `aad`.
    pub fn seal(&self, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, SealError> {
        let data_key = SecretBox::new(Box::new(random::<KEY_LEN>()?));
        let wrap_nonce = random::<NONCE_LEN>()?;
        let nonce = random::<NONCE_LEN>()?;

        let wrapped = self
            .cipher()
            .encrypt(
                &XNonce::from(wrap_nonce),
                Payload {
                    msg: data_key.expose_secret(),
                    aad: &wrap_aad(aad),
                },
            )
            .map_err(|_| SealError::Open)?;
        let body = XChaCha20Poly1305::new(&(*data_key.expose_secret()).into())
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| SealError::Open)?;

        let mut out = Vec::with_capacity(HEADER_LEN + body.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&wrap_nonce);
        out.extend_from_slice(&wrapped);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&body);
        Ok(out)
    }

    /// Opens a blob from [`MasterKey::seal`] with the same `aad`.
    pub fn open(&self, aad: &[u8], sealed: &[u8]) -> Result<SecretSlice<u8>, SealError> {
        if sealed.len() < HEADER_LEN + TAG_LEN || &sealed[..MAGIC.len()] != MAGIC {
            return Err(SealError::Malformed);
        }
        let (wrap_nonce, rest) = sealed[MAGIC.len()..].split_at(NONCE_LEN);
        let (wrapped, rest) = rest.split_at(WRAPPED_LEN);
        let (nonce, body) = rest.split_at(NONCE_LEN);
        let wrap_nonce: [u8; NONCE_LEN] =
            wrap_nonce.try_into().map_err(|_| SealError::Malformed)?;
        let nonce: [u8; NONCE_LEN] = nonce.try_into().map_err(|_| SealError::Malformed)?;

        let data_key = SecretSlice::from(
            self.cipher()
                .decrypt(
                    &XNonce::from(wrap_nonce),
                    Payload {
                        msg: wrapped,
                        aad: &wrap_aad(aad),
                    },
                )
                .map_err(|_| SealError::Open)?,
        );
        let cipher = XChaCha20Poly1305::new_from_slice(data_key.expose_secret())
            .map_err(|_| SealError::Malformed)?;
        let plain = cipher
            .decrypt(&XNonce::from(nonce), Payload { msg: body, aad })
            .map_err(|_| SealError::Open)?;
        Ok(SecretSlice::from(plain))
    }
}

/// The data-key layer's AAD: domain-separated from the body's.
fn wrap_aad(aad: &[u8]) -> Vec<u8> {
    let mut v = b"keel-seal-v1/data-key/".to_vec();
    v.extend_from_slice(aad);
    v
}

fn random<const N: usize>() -> Result<[u8; N], SealError> {
    let mut b = [0u8; N];
    SysRng
        .try_fill_bytes(&mut b)
        .map_err(|_| SealError::Random)?;
    Ok(b)
}

/// `keel keygen`: writes 32 random bytes to a new file at mode `0400`. Refuses an existing path.
pub fn keygen(path: &Path) -> Result<(), SealError> {
    let shown = path.display().to_string();
    let key = SecretBox::new(Box::new(random::<KEY_LEN>()?));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o400)
        .open(path)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::AlreadyExists => SealError::KeyExists(shown.clone()),
            kind => SealError::KeyUnusable(format!("cannot create {shown}: {kind}")),
        })?;
    file.write_all(key.expose_secret())
        .and_then(|()| file.sync_all())
        .map_err(|e| SealError::KeyUnusable(format!("cannot write {shown}: {}", e.kind())))?;
    // The umask can only remove bits; set the mode explicitly in case the file system ignored it.
    fs::set_permissions(path, fs::Permissions::from_mode(0o400))
        .map_err(|e| SealError::KeyUnusable(format!("cannot chmod {shown}: {}", e.kind())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(b: u8) -> MasterKey {
        MasterKey::from_bytes([b; KEY_LEN])
    }

    #[test]
    fn round_trip() {
        let k = key(7);
        let sealed = k.seal(b"credential:1", b"hunter2").unwrap();
        assert!(!sealed.windows(7).any(|w| w == b"hunter2"));
        let plain = k.open(b"credential:1", &sealed).unwrap();
        assert_eq!(plain.expose_secret(), b"hunter2");
        // Fresh data key and nonces each time.
        assert_ne!(sealed, k.seal(b"credential:1", b"hunter2").unwrap());
    }

    #[test]
    fn empty_plaintext_round_trips() {
        let k = key(1);
        let sealed = k.seal(b"x", b"").unwrap();
        assert!(k.open(b"x", &sealed).unwrap().expose_secret().is_empty());
    }

    #[test]
    fn wrong_key_wrong_aad_and_tampering_fail() {
        let sealed = key(7).seal(b"credential:1", b"hunter2").unwrap();
        assert_eq!(
            key(8).open(b"credential:1", &sealed).unwrap_err(),
            SealError::Open
        );
        assert_eq!(
            key(7).open(b"credential:2", &sealed).unwrap_err(),
            SealError::Open
        );
        for i in [3, HEADER_LEN - 1, sealed.len() - 1] {
            let mut bad = sealed.clone();
            bad[i] ^= 1;
            assert_eq!(
                key(7).open(b"credential:1", &bad).unwrap_err(),
                SealError::Open,
                "byte {i}"
            );
        }
        assert_eq!(
            key(7).open(b"credential:1", b"KS1").unwrap_err(),
            SealError::Malformed
        );
        let mut other_magic = sealed;
        other_magic[0] = b'X';
        assert_eq!(
            key(7).open(b"credential:1", &other_magic).unwrap_err(),
            SealError::Malformed
        );
    }

    #[test]
    fn debug_is_redacted() {
        assert_eq!(format!("{:?}", key(0x41)), "MasterKey([redacted])");
    }

    #[test]
    fn keygen_writes_32_bytes_at_0400_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("master.key");
        keygen(&path).unwrap();
        let meta = fs::metadata(&path).unwrap();
        assert_eq!(meta.len(), 32);
        assert_eq!(meta.permissions().mode() & 0o777, 0o400);
        let before = fs::read(&path).unwrap();
        assert!(matches!(keygen(&path), Err(SealError::KeyExists(_))));
        assert_eq!(fs::read(&path).unwrap(), before);

        let k = MasterKey::load(&path).unwrap();
        let sealed = k.seal(b"a", b"b").unwrap();
        assert_eq!(
            MasterKey::load(&path)
                .unwrap()
                .open(b"a", &sealed)
                .unwrap()
                .expose_secret(),
            b"b"
        );
    }

    #[test]
    fn load_refuses_missing_unreadable_open_and_wrong_length() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("none");
        assert!(matches!(
            MasterKey::load(&missing),
            Err(SealError::KeyMissing(_))
        ));

        let write = |name: &str, bytes: &[u8], mode: u32| {
            let p = dir.path().join(name);
            fs::write(&p, bytes).unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(mode)).unwrap();
            p
        };
        let open = write("open", &[1; 32], 0o644);
        assert!(
            matches!(MasterKey::load(&open), Err(SealError::KeyUnusable(m)) if m.contains("0644"))
        );
        let short = write("short", &[1; 31], 0o400);
        assert!(
            matches!(MasterKey::load(&short), Err(SealError::KeyUnusable(m)) if m.contains("32 bytes"))
        );
        assert!(matches!(
            MasterKey::load(dir.path()),
            Err(SealError::KeyUnusable(_))
        ));

        // Mode 000 is unreadable for anyone but root.
        let unreadable = write("unreadable", &[1; 32], 0o000);
        if fs::read(&unreadable).is_err() {
            assert!(matches!(
                MasterKey::load(&unreadable),
                Err(SealError::KeyUnusable(_))
            ));
        }

        let ok = write("ok", &[1; 32], 0o600);
        assert!(MasterKey::load(&ok).is_ok());
    }

    #[test]
    fn errors_never_show_key_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("k");
        fs::write(&p, b"SECRETSECRETSECRETSECRETSECRET!").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o400)).unwrap();
        let err = MasterKey::load(&p).unwrap_err().to_string();
        assert!(!err.contains("SECRET"), "{err}");
    }
}
