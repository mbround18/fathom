//! The relay's Ed25519 host key and the pinned `hub_keys` (contracts/relay.md).
//!
//! The host key is what Keel pins (pasted into Keel when the relay is added); `hub_keys` holds
//! Keel's link key(s), pasted onto the relay. Both fail closed: a key file readable by others, an
//! empty `hub_keys`, or any line that is not `ssh-ed25519` is an error, never a skip.

use std::io::{ErrorKind, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use russh::keys::ssh_key::LineEnding;
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, PrivateKey, PublicKey};

/// Where `serve` and `key` keep the host key without `--key`.
pub const DEFAULT_KEY_PATH: &str = "/var/lib/keel-relay/host_ed25519";

/// Largest `hub_keys` file read. A few lines in practice.
const MAX_HUB_KEYS_LEN: u64 = 64 * 1024;

/// A host key or `hub_keys` problem.
#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("{path}: {what}: {detail}")]
    File {
        path: PathBuf,
        what: &'static str,
        detail: String,
    },
    #[error("{path}: line {line}: not an ssh-ed25519 public key")]
    HubKeyLine { path: PathBuf, line: usize },
    #[error("{path}: no keys; the relay would refuse every link")]
    NoHubKeys { path: PathBuf },
}

fn file_err(path: &Path, what: &'static str, detail: impl ToString) -> KeyError {
    KeyError::File {
        path: path.to_owned(),
        what,
        detail: detail.to_string(),
    }
}

/// Loads the host key at `path`, generating it (mode 0600) when missing. An existing key must be
/// a regular file, unreadable by group and others, and an unencrypted Ed25519 key.
pub fn load_or_create_host_key(path: &Path) -> Result<PrivateKey, KeyError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if !meta.file_type().is_file() {
                return Err(file_err(path, "refused", "not a regular file"));
            }
            if meta.permissions().mode() & 0o077 != 0 {
                return Err(file_err(path, "refused", "readable by others; chmod 600"));
            }
            let text = std::fs::read_to_string(path).map_err(|e| file_err(path, "read", e))?;
            let key = PrivateKey::from_openssh(&text)
                .map_err(|_| file_err(path, "parse", "not an OpenSSH private key"))?;
            if key.is_encrypted() || key.algorithm() != Algorithm::Ed25519 {
                return Err(file_err(
                    path,
                    "refused",
                    "must be an unencrypted Ed25519 key",
                ));
            }
            Ok(key)
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
                .map_err(|e| file_err(path, "generate", e))?;
            let pem = key
                .to_openssh(LineEnding::LF)
                .map_err(|e| file_err(path, "encode", e))?;
            write_private_new(path, pem.as_bytes()).map_err(|e| file_err(path, "write", e))?;
            Ok(key)
        }
        Err(e) => Err(file_err(path, "stat", e)),
    }
}

/// Creates `path` with mode 0600, failing if it exists (a racing `key` keeps the first key).
fn write_private_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// The two lines `keel-relay key` prints: the OpenSSH public line, then `SHA256:<fingerprint>`.
pub fn public_lines(key: &PrivateKey) -> [String; 2] {
    let mut public = key.public_key().clone();
    public.set_comment("keel-relay");
    [
        public.to_openssh().expect("encode an Ed25519 public key"),
        keel_exec::ca::fingerprint(key.public_key().key_data()),
    ]
}

/// Parses a `hub_keys` file: one `ssh-ed25519 <base64> [comment]` per line; blank lines and
/// `#` comments are skipped. Any other line, or no key at all, is an error.
pub fn parse_hub_keys(text: &str, path: &Path) -> Result<Vec<KeyData>, KeyError> {
    let mut keys = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad = || KeyError::HubKeyLine {
            path: path.to_owned(),
            line: i + 1,
        };
        let mut fields = line.split_whitespace();
        if fields.next() != Some("ssh-ed25519") {
            return Err(bad());
        }
        let b64 = fields.next().ok_or_else(bad)?;
        let key = PublicKey::from_openssh(&format!("ssh-ed25519 {b64}")).map_err(|_| bad())?;
        if key.algorithm() != Algorithm::Ed25519 {
            return Err(bad());
        }
        keys.push(key.key_data().clone());
    }
    if keys.is_empty() {
        return Err(KeyError::NoHubKeys {
            path: path.to_owned(),
        });
    }
    Ok(keys)
}

/// Reads and parses the `hub_keys` file at `path`.
pub fn load_hub_keys(path: &Path) -> Result<Vec<KeyData>, KeyError> {
    let meta = std::fs::metadata(path).map_err(|e| file_err(path, "read", e))?;
    if meta.len() > MAX_HUB_KEYS_LEN {
        return Err(file_err(path, "refused", "larger than 64 KiB"));
    }
    let text = std::fs::read_to_string(path).map_err(|e| file_err(path, "read", e))?;
    parse_hub_keys(&text, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn some_key_line() -> (String, KeyData) {
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let line = key.public_key().to_openssh().unwrap();
        (line, key.public_key().key_data().clone())
    }

    #[test]
    fn a_missing_key_is_generated_0600_and_then_reused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host_ed25519");
        let first = load_or_create_host_key(&path).unwrap();
        assert_eq!(first.algorithm(), Algorithm::Ed25519);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let again = load_or_create_host_key(&path).unwrap();
        assert_eq!(again.public_key(), first.public_key());
    }

    #[test]
    fn a_key_readable_by_others_or_not_a_key_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host_ed25519");
        load_or_create_host_key(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(load_or_create_host_key(&path).is_err());

        let junk = dir.path().join("junk");
        write_private_new(&junk, b"not a key").unwrap();
        assert!(load_or_create_host_key(&junk).is_err());

        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(load_or_create_host_key(&link).is_err(), "symlinks refused");
    }

    #[test]
    fn public_lines_are_the_openssh_line_then_the_fingerprint() {
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let [line, fp] = public_lines(&key);
        assert!(line.starts_with("ssh-ed25519 "), "{line}");
        assert!(!line.contains('\n'));
        let parsed = PublicKey::from_openssh(&line).unwrap();
        assert_eq!(parsed.key_data(), key.public_key().key_data());
        assert!(fp.starts_with("SHA256:"), "{fp}");
        assert_eq!(fp, keel_exec::ca::fingerprint(key.public_key().key_data()));
    }

    #[test]
    fn hub_keys_parse_skipping_blanks_and_comments() {
        let (a, ka) = some_key_line();
        let (b, kb) = some_key_line();
        let text = format!("# Keel link key\n\n{a} keel-link\n  {b}\n");
        let keys = parse_hub_keys(&text, Path::new("hub_keys")).unwrap();
        assert_eq!(keys, vec![ka, kb]);
    }

    #[test]
    fn hub_keys_fail_closed() {
        let p = Path::new("hub_keys");
        assert!(matches!(
            parse_hub_keys("", p),
            Err(KeyError::NoHubKeys { .. })
        ));
        assert!(matches!(
            parse_hub_keys("# only a comment\n\n", p),
            Err(KeyError::NoHubKeys { .. })
        ));
        let (good, _) = some_key_line();
        for bad in [
            "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ",
            "@cert-authority * ssh-ed25519 AAAA",
            "ssh-ed25519",
            "ssh-ed25519 not-base64",
        ] {
            let text = format!("{good}\n{bad}\n");
            assert!(
                matches!(
                    parse_hub_keys(&text, p),
                    Err(KeyError::HubKeyLine { line: 2, .. })
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn load_hub_keys_reads_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hub_keys");
        assert!(load_hub_keys(&path).is_err());
        let (line, key) = some_key_line();
        std::fs::write(&path, format!("{line}\n")).unwrap();
        assert_eq!(load_hub_keys(&path).unwrap(), vec![key]);
    }
}
