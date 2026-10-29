//! Registry credentials from `$DOCKER_CONFIG/config.json` (contracts/registry.md "Credentials
//! file").
//!
//! Read once per check pass, never cached across passes, never written. Only plain `auths`
//! entries are used; credential helpers and identity tokens make the registry "can't check".
//! Nothing read here reaches a log, a span, the protocol or Depths' state, except the file's path
//! when it cannot be read.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use depths_core::updates::reference::DOCKER_HUB;
use depths_core::updates::registry::RegistryError;
use depths_protocol::RegistryAuth;
use oci_registry::Credentials;
use secrecy::SecretString;
use serde::Deserialize;

/// What a host's entry gave.
#[derive(Clone)]
enum Entry {
    Basic {
        username: String,
        password: SecretString,
    },
    IdentityToken,
    /// Present but neither form: treated as no entry.
    Empty,
}

/// One pass's view of `config.json`.
#[derive(Clone, Default)]
pub struct CredsSnapshot {
    auths: BTreeMap<String, Entry>,
    creds_store: Option<String>,
    cred_helpers: BTreeMap<String, String>,
    /// "docker config unreadable: <path>", for the registries panel.
    note: Option<String>,
}

impl std::fmt::Debug for CredsSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredsSnapshot")
            .field("entries", &self.auths.len())
            .field("note", &self.note)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct RawFile {
    #[serde(default)]
    auths: BTreeMap<String, RawEntry>,
    #[serde(default, rename = "credsStore")]
    creds_store: Option<String>,
    #[serde(default, rename = "credHelpers")]
    cred_helpers: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct RawEntry {
    #[serde(default)]
    auth: Option<String>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    identitytoken: Option<String>,
}

impl RawEntry {
    fn into_entry(self) -> Entry {
        if self.identitytoken.is_some_and(|t| !t.is_empty()) {
            return Entry::IdentityToken;
        }
        if let (Some(u), Some(p)) = (self.username, self.password)
            && !u.is_empty()
        {
            return Entry::Basic {
                username: u,
                password: SecretString::from(p),
            };
        }
        let decoded = self
            .auth
            .and_then(|a| {
                base64::engine::general_purpose::STANDARD
                    .decode(a.trim())
                    .ok()
            })
            .and_then(|b| String::from_utf8(b).ok());
        match decoded.as_deref().and_then(|s| s.split_once(':')) {
            Some((u, p)) if !u.is_empty() => Entry::Basic {
                username: u.to_owned(),
                password: SecretString::from(p.to_owned()),
            },
            _ => Entry::Empty,
        }
    }
}

const IDENTITY_TOKEN: &str =
    "identity tokens are not supported; log in with a username and password";

fn helper_unsupported(name: &str) -> String {
    format!("credential helper `{name}` is not supported in the Depths container")
}

/// The keys `config.json` may file `host` under, best first.
fn keys_for(host: &str) -> Vec<String> {
    let mut keys = vec![
        host.to_owned(),
        format!("https://{host}"),
        format!("https://{host}/v1/"),
        format!("http://{host}"),
    ];
    if host == DOCKER_HUB || host == "index.docker.io" || host == "registry-1.docker.io" {
        for k in [
            "https://index.docker.io/v1/",
            "index.docker.io",
            "docker.io",
            "registry-1.docker.io",
        ] {
            if !keys.iter().any(|x| x == k) {
                keys.push(k.to_owned());
            }
        }
    }
    keys
}

impl CredsSnapshot {
    /// Reads `<dir>/config.json`. Missing: anonymous. Unreadable: anonymous plus a note.
    pub fn load(dir: &Path) -> Self {
        let path: PathBuf = dir.join("config.json");
        let unreadable = || {
            tracing::warn!(path = %path.display(), "docker config unreadable; checking anonymously");
            Self {
                note: Some(format!("docker config unreadable: {}", path.display())),
                ..Self::default()
            }
        };
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(_) => return unreadable(),
        };
        match serde_json::from_slice::<RawFile>(&bytes) {
            Ok(raw) => Self {
                auths: raw
                    .auths
                    .into_iter()
                    .map(|(k, v)| (k, v.into_entry()))
                    .collect(),
                creds_store: raw.creds_store.filter(|s| !s.is_empty()),
                cred_helpers: raw.cred_helpers,
                note: None,
            },
            Err(_) => unreadable(),
        }
    }

    /// The panel note when the file could not be read.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// The credentials for a registry host as depths-core names it (`docker.io` for Hub).
    pub fn for_host(&self, host: &str) -> Result<Credentials, RegistryError> {
        let keys = keys_for(host);
        if let Some(name) = keys.iter().find_map(|k| self.cred_helpers.get(k)) {
            return Err(RegistryError::CredentialsUnsupported(helper_unsupported(
                name,
            )));
        }
        let entry = keys.iter().find_map(|k| self.auths.get(k));
        match entry {
            Some(Entry::Basic { username, password }) => Ok(Credentials::Basic {
                username: username.clone(),
                password: password.clone(),
            }),
            Some(Entry::IdentityToken) => Err(RegistryError::CredentialsUnsupported(
                IDENTITY_TOKEN.to_owned(),
            )),
            Some(Entry::Empty) | None => match &self.creds_store {
                Some(store) => Err(RegistryError::CredentialsUnsupported(helper_unsupported(
                    store,
                ))),
                None => Ok(Credentials::Anonymous),
            },
        }
    }

    /// How the registries panel describes a host's credentials.
    pub fn auth_for(&self, host: &str) -> RegistryAuth {
        match self.for_host(host) {
            Ok(Credentials::Anonymous) => RegistryAuth::Anonymous,
            Ok(Credentials::Basic { .. }) => RegistryAuth::Config,
            Err(e) => RegistryAuth::Unsupported {
                reason: e.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::sync::{Arc, Mutex};

    use secrecy::ExposeSecret as _;

    use super::*;

    fn snapshot(json: &str) -> CredsSnapshot {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("config.json"), json).unwrap();
        CredsSnapshot::load(d.path())
    }

    fn basic(c: Result<Credentials, RegistryError>) -> (String, String) {
        match c {
            Ok(Credentials::Basic { username, password }) => {
                (username, password.expose_secret().to_owned())
            }
            other => panic!("expected Basic, got {other:?}"),
        }
    }

    fn b64(s: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(s)
    }

    #[test]
    fn plain_auths_entries() {
        let s = snapshot(&format!(
            r#"{{"auths": {{
                "ghcr.io": {{"auth": "{}"}},
                "registry.example": {{"username": "bob", "password": "hunter2"}}
            }}}}"#,
            b64("alice:s3cret:with:colons")
        ));
        assert_eq!(
            basic(s.for_host("ghcr.io")),
            ("alice".into(), "s3cret:with:colons".into())
        );
        assert_eq!(
            basic(s.for_host("registry.example")),
            ("bob".into(), "hunter2".into())
        );
        assert!(matches!(s.for_host("quay.io"), Ok(Credentials::Anonymous)));
        assert_eq!(s.auth_for("ghcr.io"), RegistryAuth::Config);
        assert_eq!(s.auth_for("quay.io"), RegistryAuth::Anonymous);
    }

    #[test]
    fn key_matching() {
        for key in [
            "reg.example:5000",
            "https://reg.example:5000",
            "https://reg.example:5000/v1/",
            "http://reg.example:5000",
        ] {
            let s = snapshot(&format!(
                r#"{{"auths": {{"{key}": {{"auth": "{}"}}}}}}"#,
                b64("u:p")
            ));
            assert_eq!(basic(s.for_host("reg.example:5000")).0, "u", "{key}");
        }
        for key in [
            "https://index.docker.io/v1/",
            "index.docker.io",
            "docker.io",
            "registry-1.docker.io",
        ] {
            let s = snapshot(&format!(
                r#"{{"auths": {{"{key}": {{"auth": "{}"}}}}}}"#,
                b64("hub:p")
            ));
            assert_eq!(basic(s.for_host(DOCKER_HUB)).0, "hub", "{key}");
        }
    }

    #[test]
    fn helpers_and_identity_tokens_are_unsupported() {
        let s = snapshot(
            r#"{"auths": {"ghcr.io": {"identitytoken": "tok"}, "quay.io": {}},
                "credsStore": "desktop",
                "credHelpers": {"123.dkr.ecr.us-east-1.amazonaws.com": "ecr-login"}}"#,
        );
        assert_eq!(
            s.for_host("ghcr.io").unwrap_err(),
            RegistryError::CredentialsUnsupported(IDENTITY_TOKEN.into())
        );
        assert_eq!(
            s.for_host("123.dkr.ecr.us-east-1.amazonaws.com")
                .unwrap_err()
                .to_string(),
            "can't check: credential helper `ecr-login` is not supported in the Depths container"
        );
        // A credsStore with no auths entry for the host.
        assert_eq!(
            s.for_host("quay.io").unwrap_err().to_string(),
            "can't check: credential helper `desktop` is not supported in the Depths container"
        );
        assert!(matches!(
            s.auth_for("ghcr.io"),
            RegistryAuth::Unsupported { .. }
        ));
    }

    #[test]
    fn missing_file_is_anonymous_without_a_note() {
        let d = tempfile::tempdir().unwrap();
        let s = CredsSnapshot::load(d.path());
        assert!(matches!(s.for_host("ghcr.io"), Ok(Credentials::Anonymous)));
        assert_eq!(s.note(), None);
    }

    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().write(b)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buf {
        type Writer = Buf;
        fn make_writer(&'a self) -> Buf {
            self.clone()
        }
    }

    #[test]
    fn unreadable_file_is_anonymous_with_a_note_and_logs_no_secret() {
        let buf = Buf::default();
        let sub = tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_max_level(tracing::Level::TRACE)
            .finish();
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("config.json");
        let mut f = std::fs::File::create(&path).unwrap();
        // Not JSON, with a secret in it.
        write!(
            f,
            "{{\"auths\": {{\"ghcr.io\": {{\"auth\": \"{}\"",
            b64("u:topsecret")
        )
        .unwrap();
        drop(f);

        let s = tracing::subscriber::with_default(sub, || {
            let s = CredsSnapshot::load(d.path());
            let _ = s.for_host("ghcr.io");
            tracing::info!(snapshot = ?s, "loaded");
            s
        });
        assert!(matches!(s.for_host("ghcr.io"), Ok(Credentials::Anonymous)));
        assert_eq!(
            s.note(),
            Some(format!("docker config unreadable: {}", path.display()).as_str())
        );

        // A readable file with secrets: nothing of them in the log or the Debug output.
        let buf2 = buf.clone();
        let sub = tracing_subscriber::fmt()
            .with_writer(buf2)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        tracing::subscriber::with_default(sub, || {
            let s = snapshot(&format!(
                r#"{{"auths": {{"ghcr.io": {{"auth": "{}"}}}}}}"#,
                b64("u:topsecret")
            ));
            let c = s.for_host("ghcr.io");
            tracing::info!(snapshot = ?s, creds = ?c, "loaded");
        });
        let log = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(log.contains("unreadable"), "{log}");
        assert!(!log.contains("topsecret"), "{log}");
        assert!(!log.contains(&b64("u:topsecret")), "{log}");
    }
}
