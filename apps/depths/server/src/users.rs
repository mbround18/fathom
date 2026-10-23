//! T036: the users registry, `<data-dir>/users.json` (contracts/config.md, data-model "User").
//!
//! - Owners (from `OWNER_USER_ID`) are always `admin`; nothing here demotes or removes them.
//! - Anyone else who signs in is recorded as `pending`, with no access, until an admin admits
//!   them.
//! - A `github:<login>` owner is pinned to the account's numeric id at its first sign-in and
//!   matched by id from then on.
//! - The file is written only here: temp file (0600), fsync, rename, fsync the directory. A file
//!   that does not parse refuses start with its path; it is never read as an empty registry.

use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use depths_protocol::{Provider, Role, User};
use serde::{Deserialize, Serialize};

use crate::config::Owner;

const VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    version: u32,
    users: Vec<User>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryError(pub String);

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RegistryError {}

/// Who signed in, as the registry needs it.
#[derive(Debug, Clone)]
pub struct SignIn {
    pub provider: Provider,
    pub issuer: String,
    pub subject: String,
    pub username: String,
    pub email: Option<String>,
}

impl SignIn {
    pub fn id(&self) -> String {
        format!("{}:{}", provider_str(self.provider), self.subject)
    }

    pub fn from_identity(i: &web_auth::session::Identity) -> Self {
        Self {
            provider: match i.provider {
                web_auth::session::Provider::Oidc => Provider::Oidc,
                web_auth::session::Provider::Github => Provider::Github,
                web_auth::session::Provider::Discord => Provider::Discord,
            },
            issuer: i.issuer.clone(),
            subject: i.sub.clone(),
            username: i.username.clone(),
            email: i.email.clone(),
        }
    }
}

pub fn provider_str(p: Provider) -> &'static str {
    match p {
        Provider::Oidc => "oidc",
        Provider::Github => "github",
        Provider::Discord => "discord",
    }
}

fn provider_of(id: &str) -> Option<Provider> {
    match id.split_once(':')?.0 {
        "oidc" => Some(Provider::Oidc),
        "github" => Some(Provider::Github),
        "discord" => Some(Provider::Discord),
        _ => None,
    }
}

pub struct Registry {
    path: PathBuf,
    owners: Vec<Owner>,
    /// The configured OIDC issuer: an `oidc:` entry from another issuer never matches.
    oidc_issuer: Option<String>,
    users: Mutex<Vec<User>>,
    now: fn() -> String,
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

impl Registry {
    /// Loads `path` (missing = empty) and applies the owners. Refuses a file that does not parse,
    /// and a registry with no admin when no owner is configured.
    pub fn open(
        path: PathBuf,
        owners: Vec<Owner>,
        oidc_issuer: Option<String>,
    ) -> Result<Self, RegistryError> {
        Self::open_at(path, owners, oidc_issuer, now_rfc3339)
    }

    fn open_at(
        path: PathBuf,
        owners: Vec<Owner>,
        oidc_issuer: Option<String>,
        now: fn() -> String,
    ) -> Result<Self, RegistryError> {
        let users = match std::fs::read(&path) {
            Ok(bytes) => {
                let f: File = serde_json::from_slice(&bytes).map_err(|e| {
                    RegistryError(format!(
                        "{} is not a valid users registry: {e}; fix or move it aside, Depths will \
                         not start with an unreadable registry",
                        path.display()
                    ))
                })?;
                if f.version != VERSION {
                    return Err(RegistryError(format!(
                        "{}: unsupported version {}",
                        path.display(),
                        f.version
                    )));
                }
                f.users
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                return Err(RegistryError(format!("{}: {e}", path.display())));
            }
        };
        let reg = Self {
            path,
            owners,
            oidc_issuer,
            users: Mutex::new(users),
            now,
        };
        {
            let mut users = reg.lock();
            reg.apply_owners(&mut users);
            if !users.iter().any(|u| u.role == Role::Admin) {
                return Err(RegistryError(
                    "no owner is configured (OWNER_USER_ID) and users.json holds no admin: set \
                     OWNER_USER_ID to your account id, like discord:140642000969007104"
                        .into(),
                ));
            }
        }
        Ok(reg)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<User>> {
        self.users
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Every owner has an `admin` entry; former owners keep their entry, not the flag.
    fn apply_owners(&self, users: &mut Vec<User>) {
        for u in users.iter_mut() {
            u.owner = false;
        }
        for o in &self.owners {
            let existing = match o {
                Owner::Id(id) => users.iter().position(|u| &u.id == id),
                Owner::GithubLogin(login) => {
                    let key = format!("github:{login}");
                    users
                        .iter()
                        .position(|u| u.pinned_from.as_deref() == Some(&key))
                        .or_else(|| users.iter().position(|u| u.id == key))
                }
            };
            match existing {
                Some(i) => {
                    users[i].owner = true;
                    users[i].role = Role::Admin;
                }
                None => {
                    // Not yet signed in. A login owner is held under its login until pinned.
                    let (id, provider, subject) = match o {
                        Owner::Id(id) => (
                            id.clone(),
                            provider_of(id).unwrap_or(Provider::Discord),
                            id.split_once(':').map_or("", |(_, s)| s).to_owned(),
                        ),
                        Owner::GithubLogin(l) => {
                            (format!("github:{l}"), Provider::Github, l.clone())
                        }
                    };
                    users.push(User {
                        issuer: (provider == Provider::Oidc)
                            .then(|| self.oidc_issuer.clone())
                            .flatten(),
                        id,
                        provider,
                        subject,
                        username: None,
                        email: None,
                        role: Role::Admin,
                        stacks: Vec::new(),
                        owner: true,
                        pinned_from: None,
                        first_seen: None,
                        last_seen: None,
                    });
                }
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<User> {
        self.lock().iter().find(|u| u.id == id).cloned()
    }

    pub fn all(&self) -> Vec<User> {
        self.lock().clone()
    }

    /// Records a sign-in: refreshes a known entry, pins a `github:<login>` owner, or adds a
    /// `pending` one. Persisted before it returns.
    pub fn sign_in(&self, s: &SignIn) -> Result<User, RegistryError> {
        let now = (self.now)();
        let id = s.id();
        let mut users = self.lock();
        // Pin a login owner: the first sign-in of that login names the numeric id from then on.
        if s.provider == Provider::Github && !users.iter().any(|u| u.id == id) {
            let login_id = format!("github:{}", s.username.to_ascii_lowercase());
            let pin = self
                .owners
                .iter()
                .any(|o| matches!(o, Owner::GithubLogin(l) if format!("github:{l}") == login_id));
            if pin
                && let Some(u) = users
                    .iter_mut()
                    .find(|u| u.id == login_id && u.first_seen.is_none())
            {
                u.id.clone_from(&id);
                u.subject.clone_from(&s.subject);
                u.pinned_from = Some(login_id);
            }
        }
        let user = match users.iter_mut().find(|u| u.id == id) {
            Some(u) => {
                if u.provider == Provider::Oidc && u.issuer.as_deref() != Some(&s.issuer) {
                    return Err(RegistryError("oidc subject from another issuer".into()));
                }
                u.username = Some(s.username.clone());
                u.email.clone_from(&s.email);
                if u.first_seen.is_none() {
                    u.first_seen = Some(now.clone());
                }
                u.last_seen = Some(now);
                u.clone()
            }
            None => {
                let u = User {
                    id,
                    provider: s.provider,
                    issuer: (s.provider == Provider::Oidc).then(|| s.issuer.clone()),
                    subject: s.subject.clone(),
                    username: Some(s.username.clone()),
                    email: s.email.clone(),
                    role: Role::Pending,
                    stacks: Vec::new(),
                    owner: false,
                    pinned_from: None,
                    first_seen: Some(now.clone()),
                    last_seen: Some(now),
                };
                users.push(u.clone());
                u
            }
        };
        persist(&self.path, &users)?;
        Ok(user)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Temp file in the same directory (0600), fsync, rename over, fsync the directory.
fn persist(path: &Path, users: &[User]) -> Result<(), RegistryError> {
    let err = |e: std::io::Error| RegistryError(format!("{}: {e}", path.display()));
    let dir = path
        .parent()
        .ok_or_else(|| RegistryError(format!("{} has no parent", path.display())))?;
    let body = serde_json::to_vec_pretty(&File {
        version: VERSION,
        users: users.to_vec(),
    })
    .map_err(|e| RegistryError(e.to_string()))?;
    let tmp = dir.join(format!(
        ".users.json.{}.tmp",
        web_auth::session::random_token()
    ));
    let write = || -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(&body)?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)?;
        std::fs::File::open(dir)?.sync_all()
    };
    write().map_err(|e| {
        drop(std::fs::remove_file(&tmp));
        err(e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    fn at() -> String {
        "2026-10-23T12:00:00Z".into()
    }

    fn discord(sub: &str, name: &str) -> SignIn {
        SignIn {
            provider: Provider::Discord,
            issuer: "https://discord.com".into(),
            subject: sub.into(),
            username: name.into(),
            email: None,
        }
    }

    fn owner() -> Vec<Owner> {
        vec![Owner::Id("discord:1".into())]
    }

    fn open(dir: &Path, owners: Vec<Owner>) -> Result<Registry, RegistryError> {
        Registry::open_at(dir.join("users.json"), owners, None, at)
    }

    #[test]
    fn the_owner_is_admin_from_first_sign_in() {
        let d = tempfile::tempdir().unwrap();
        let r = open(d.path(), owner()).unwrap();
        let u = r.sign_in(&discord("1", "mike")).unwrap();
        assert_eq!(u.role, Role::Admin);
        assert!(u.owner);
        assert_eq!(u.username.as_deref(), Some("mike"));
        assert_eq!(u.first_seen.as_deref(), Some("2026-10-23T12:00:00Z"));
    }

    #[test]
    fn everyone_else_is_pending() {
        let d = tempfile::tempdir().unwrap();
        let r = open(d.path(), owner()).unwrap();
        let u = r.sign_in(&discord("2", "stranger")).unwrap();
        assert_eq!(u.role, Role::Pending);
        assert!(!u.owner);
        assert!(u.stacks.is_empty());
        // A second sign-in keeps the entry, still pending.
        let again = r.sign_in(&discord("2", "renamed")).unwrap();
        assert_eq!(again.role, Role::Pending);
        assert_eq!(again.username.as_deref(), Some("renamed"));
        assert_eq!(r.all().len(), 2);
    }

    #[test]
    fn the_registry_persists_privately_and_reloads() {
        let d = tempfile::tempdir().unwrap();
        let r = open(d.path(), owner()).unwrap();
        r.sign_in(&discord("2", "stranger")).unwrap();
        let mode = std::fs::metadata(d.path().join("users.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let leftovers: Vec<_> = std::fs::read_dir(d.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        let r = open(d.path(), owner()).unwrap();
        assert_eq!(r.get("discord:2").unwrap().role, Role::Pending);
    }

    #[test]
    fn a_corrupt_file_refuses_naming_its_path() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("users.json"), b"{not json").unwrap();
        let e = open(d.path(), owner()).err().unwrap();
        assert!(e.0.contains("users.json"), "{e}");
    }

    #[test]
    fn no_owner_and_no_admin_refuses() {
        let d = tempfile::tempdir().unwrap();
        let e = open(d.path(), vec![]).err().unwrap();
        assert!(e.0.contains("OWNER_USER_ID"), "{e}");
        // An admin already in the file is enough.
        let r = open(d.path(), owner()).unwrap();
        r.sign_in(&discord("1", "mike")).unwrap();
        assert!(open(d.path(), vec![]).is_ok());
    }

    #[test]
    fn a_github_login_owner_is_pinned_to_the_numeric_id() {
        let d = tempfile::tempdir().unwrap();
        let owners = vec![Owner::GithubLogin("mbround18".into())];
        let r = open(d.path(), owners.clone()).unwrap();
        let gh = |id: &str, login: &str| SignIn {
            provider: Provider::Github,
            issuer: "https://github.com".into(),
            subject: id.into(),
            username: login.into(),
            email: None,
        };
        let u = r.sign_in(&gh("42", "MBRound18")).unwrap();
        assert_eq!(u.id, "github:42");
        assert_eq!(u.role, Role::Admin);
        assert_eq!(u.pinned_from.as_deref(), Some("github:mbround18"));
        // Someone who later takes the login does not inherit it.
        let other = r.sign_in(&gh("99", "mbround18")).unwrap();
        assert_eq!(other.role, Role::Pending);
        // After a restart the pin holds.
        let r = open(d.path(), owners).unwrap();
        assert_eq!(r.get("github:42").unwrap().role, Role::Admin);
        assert!(r.get("github:99").is_some_and(|u| u.role == Role::Pending));
    }

    #[test]
    fn a_former_owner_loses_the_flag_but_keeps_the_entry() {
        let d = tempfile::tempdir().unwrap();
        let r = open(
            d.path(),
            vec![Owner::Id("discord:1".into()), Owner::Id("discord:5".into())],
        )
        .unwrap();
        r.sign_in(&discord("5", "x")).unwrap();
        let r = open(d.path(), owner()).unwrap();
        let u = r.get("discord:5").unwrap();
        assert!(!u.owner);
    }
}
