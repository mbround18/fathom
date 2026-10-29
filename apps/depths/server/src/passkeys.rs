//! Passkeys (spec 054 US4): the store, the ceremonies and the recovery codes.
//!
//! - `<data_dir>/passkeys.json` `{ version: 1, users: { <user>: [StoredPasskey] } }` holds every
//!   person's passkeys, keyed by their users-registry id. It is written atomically (0600 temp,
//!   fsync, rename, fsync the directory) and loaded at start. A file that does not parse is moved
//!   aside, never deleted, and Depths starts with no passkeys (the caller audits it).
//! - `<data_dir>/passkey-recovery.json` `{ user, code_sha256, expires, used }` holds the one
//!   outstanding recovery code, written by `depths passkey-recovery` on the host. Only the
//!   SHA-256 is ever stored.
//! - `crates/web-passkeys` runs the ceremonies. Ceremony state stays in memory: single-use,
//!   bound to the session handle, and void after two minutes.
//! - Credentials, challenges and recovery codes are never logged or audited.
//!
//! Where a session stands (`verified_at`, `recovering`) lives in [`crate::session_meta`]; the
//! gates are in [`crate::step_up`].

use std::collections::{BTreeMap, HashMap};
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use depths_protocol::{
    AssertionCredential, AttestationCredential, CeremonyId, PasskeyAssertion, PasskeyCredential,
    PasskeyInfo, PasskeyRefusedReason, PasskeyRequest, PasskeyResponse, PreviewId,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;
use web_passkeys::recovery::{self, IssuedRecoveryCode, RecoveryError, RecoveryRecord};
use web_passkeys::{Ceremonies, CeremonyError, PasskeyStore, PasskeyUser, StoredPasskey};
use webauthn_rs::prelude::{
    PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential,
    WebauthnError,
};

use crate::config::Config;
use crate::trust::rfc3339;

/// The file format version.
pub const VERSION: u32 = 1;
/// How long a ceremony can be answered, in seconds.
pub const CEREMONY_TTL_SECS: i64 = 120;
/// The most open ceremonies one session may hold; beginning another drops its oldest.
pub const MAX_CEREMONIES_PER_SESSION: usize = 8;
/// How long a passkey-removal preview stays confirmable, in seconds.
pub const REMOVAL_TTL_SECS: i64 = 300;
/// The relying-party name browsers show.
const RP_NAME: &str = "Depths";

pub fn passkeys_path(data_dir: &Path) -> PathBuf {
    data_dir.join("passkeys.json")
}

pub fn recovery_path(data_dir: &Path) -> PathBuf {
    data_dir.join("passkey-recovery.json")
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct PasskeysFile {
    version: u32,
    #[serde(default)]
    users: BTreeMap<String, Vec<StoredPasskey>>,
}

/// Writes `body` to `path` atomically: a 0600 temp file in the same directory, fsync, rename,
/// fsync the directory.
pub fn write_atomic(path: &Path, body: &[u8]) -> std::io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other(format!("{} has no parent", path.display())))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.tmp", web_auth::session::random_token()));
    let write = || -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(body)?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)?;
        std::fs::File::open(dir)?.sync_all()
    };
    write().inspect_err(|_| drop(std::fs::remove_file(&tmp)))
}

/// Passkeys in `<data_dir>/passkeys.json`, recovery codes in `<data_dir>/passkey-recovery.json`.
pub struct FileStore {
    path: PathBuf,
    recovery_path: PathBuf,
    users: Mutex<BTreeMap<String, Vec<StoredPasskey>>>,
}

impl FileStore {
    /// Loads the store (missing = empty). A file that does not parse is renamed to
    /// `passkeys.json.corrupt-<unix seconds>` and the store starts empty; the note returned says
    /// so, for the audit log.
    pub fn open(data_dir: &Path) -> Result<(Self, Option<String>), String> {
        let path = passkeys_path(data_dir);
        let (users, note) = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<PasskeysFile>(&bytes) {
                Ok(f) if f.version == VERSION => (f.users, None),
                parsed => {
                    let why = match parsed {
                        Ok(f) => format!("unknown version {}", f.version),
                        Err(e) => e.to_string(),
                    };
                    let aside =
                        data_dir.join(format!("passkeys.json.corrupt-{}", Utc::now().timestamp()));
                    std::fs::rename(&path, &aside).map_err(|e| {
                        format!(
                            "{} is not a valid passkey store ({why}) and could not be moved \
                             aside: {e}",
                            path.display()
                        )
                    })?;
                    let note = format!(
                        "{} was not a valid passkey store ({why}); moved to {}, starting with \
                         no passkeys",
                        path.display(),
                        aside.display()
                    );
                    (BTreeMap::new(), Some(note))
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (BTreeMap::new(), None),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        Ok((
            Self {
                path,
                recovery_path: recovery_path(data_dir),
                users: Mutex::new(users),
            },
            note,
        ))
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, Vec<StoredPasskey>>> {
        self.users.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn persist(&self, users: &BTreeMap<String, Vec<StoredPasskey>>) -> std::io::Result<()> {
        let body = serde_json::to_vec_pretty(&PasskeysFile {
            version: VERSION,
            users: users.clone(),
        })
        .map_err(std::io::Error::other)?;
        write_atomic(&self.path, &body)
    }

    /// Applies `change` to a copy, writes it, and only then keeps it: a failed write changes
    /// nothing.
    fn change<T>(
        &self,
        change: impl FnOnce(&mut BTreeMap<String, Vec<StoredPasskey>>) -> T,
    ) -> std::io::Result<T> {
        let mut users = self.lock();
        let mut next = users.clone();
        let out = change(&mut next);
        next.retain(|_, v| !v.is_empty());
        if next.len() != users.len() || !same(&next, &users) {
            self.persist(&next)?;
            *users = next;
        }
        Ok(out)
    }

    /// `user`'s passkeys, oldest first.
    pub fn list(&self, user: &str) -> Vec<StoredPasskey> {
        self.lock().get(user).cloned().unwrap_or_default()
    }

    /// Removes one passkey, never the last one (that takes a recovery code).
    pub fn remove_one(&self, user: &str, id: &str) -> Result<StoredPasskey, RemoveError> {
        self.change(|users| {
            let list = users.get_mut(user).ok_or(RemoveError::Unknown)?;
            let at = list
                .iter()
                .position(|p| p.id == id)
                .ok_or(RemoveError::Unknown)?;
            if list.len() <= 1 {
                return Err(RemoveError::Last);
            }
            Ok(list.remove(at))
        })
        .map_err(RemoveError::Storage)?
    }

    fn read_recovery(&self) -> std::io::Result<Option<RecoveryRecord>> {
        match std::fs::read(&self.recovery_path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// Two maps hold the same passkeys (ids, names, use times and credentials).
fn same(
    a: &BTreeMap<String, Vec<StoredPasskey>>,
    b: &BTreeMap<String, Vec<StoredPasskey>>,
) -> bool {
    serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
}

/// Why a passkey was not removed.
#[derive(Debug)]
pub enum RemoveError {
    Unknown,
    Last,
    Storage(std::io::Error),
}

#[async_trait]
impl PasskeyStore for FileStore {
    type Error = std::io::Error;

    async fn passkeys(&self, user: &str) -> Result<Vec<StoredPasskey>, Self::Error> {
        Ok(self.list(user))
    }

    async fn add_passkey(&self, user: &str, passkey: StoredPasskey) -> Result<(), Self::Error> {
        self.change(|users| users.entry(user.to_owned()).or_default().push(passkey))
    }

    async fn update_passkey(&self, user: &str, passkey: &StoredPasskey) -> Result<(), Self::Error> {
        self.change(|users| {
            if let Some(p) = users
                .get_mut(user)
                .and_then(|l| l.iter_mut().find(|p| p.id == passkey.id))
            {
                *p = passkey.clone();
            }
        })
    }

    async fn remove_passkey(&self, user: &str, id: &str) -> Result<bool, Self::Error> {
        self.change(|users| {
            users.get_mut(user).is_some_and(|l| {
                let before = l.len();
                l.retain(|p| p.id != id);
                l.len() != before
            })
        })
    }

    async fn remove_all_passkeys(&self, user: &str) -> Result<usize, Self::Error> {
        self.change(|users| users.remove(user).map_or(0, |l| l.len()))
    }

    async fn put_recovery(&self, record: RecoveryRecord) -> Result<(), Self::Error> {
        let body = serde_json::to_vec_pretty(&record).map_err(std::io::Error::other)?;
        write_atomic(&self.recovery_path, &body)
    }

    async fn recovery(&self, user: &str) -> Result<Option<RecoveryRecord>, Self::Error> {
        Ok(self.read_recovery()?.filter(|r| r.user == user))
    }

    async fn consume_recovery(&self, user: &str, code_sha256: &str) -> Result<bool, Self::Error> {
        // One process redeems (the server); the CLI only replaces the record.
        let _held = self.lock();
        let Some(mut r) = self.read_recovery()? else {
            return Ok(false);
        };
        if r.user != user || r.code_sha256 != code_sha256 || r.used {
            return Ok(false);
        }
        r.used = true;
        let body = serde_json::to_vec_pretty(&r).map_err(std::io::Error::other)?;
        write_atomic(&self.recovery_path, &body)?;
        Ok(true)
    }
}

/// Issues a recovery code for `user` into `data_dir` (the `depths passkey-recovery` command).
/// Only the SHA-256 is written; the code comes back to be printed once.
pub async fn issue_recovery(
    data_dir: &Path,
    user: &str,
    now: DateTime<Utc>,
) -> Result<IssuedRecoveryCode, String> {
    let store = FileStore {
        path: passkeys_path(data_dir),
        recovery_path: recovery_path(data_dir),
        users: Mutex::new(BTreeMap::new()),
    };
    recovery::issue(&store, user, now.timestamp())
        .await
        .map_err(|e| e.to_string())
}

/// The `depths passkey-recovery --user <id>` command (FR-021): issues a code for `user` (a
/// users-registry id, `provider:subject`), audits `PasskeyRecoveryIssued` by the system actor,
/// and hands the code back to be printed once. Neither the code nor its hash is audited.
pub async fn recovery_command(
    data_dir: &Path,
    audit_rotate_bytes: u64,
    user: &str,
    now: DateTime<Utc>,
) -> Result<IssuedRecoveryCode, String> {
    let user = user.trim();
    if user.is_empty() || !user.contains(':') {
        return Err(format!(
            "--user wants a users-registry id such as `discord:1234`, not `{user}`"
        ));
    }
    let audit_dir = data_dir.join("audit");
    // Open the log first: a code nobody can see was issued is worse than no code.
    let audit = crate::audit::Audit::open(audit_dir.clone(), audit_rotate_bytes)
        .map_err(|e| format!("cannot open the audit log in {}: {e}", audit_dir.display()))?;
    let issued = issue_recovery(data_dir, user, now).await?;
    let entry = crate::audit::passkey(
        crate::audit::system_actor(),
        depths_protocol::AuditAction::PasskeyRecoveryIssued,
        std::net::IpAddr::from([127, 0, 0, 1]),
        depths_protocol::AuditOutcome::Succeeded,
        depths_protocol::AuditPasskey {
            person: Some(user.to_owned()),
            ..Default::default()
        },
    );
    audit
        .record(&entry)
        .map_err(|e| format!("the code was issued but could not be audited: {e}"))?;
    Ok(issued)
}

// ---------------------------------------------------------------------------
// Clock
// ---------------------------------------------------------------------------

/// The wall clock, plus an offset tests can move forward instead of sleeping.
#[derive(Debug, Default)]
pub struct Clock {
    offset_secs: AtomicI64,
}

impl Clock {
    pub fn now(&self) -> DateTime<Utc> {
        Utc::now() + chrono::Duration::seconds(self.offset_secs.load(Ordering::Relaxed))
    }

    /// Moves this clock `secs` seconds forward (tests).
    pub fn advance(&self, secs: i64) {
        self.offset_secs.fetch_add(secs, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Ceremonies
// ---------------------------------------------------------------------------

enum Kind {
    Register {
        name: String,
        state: Box<PasskeyRegistration>,
    },
    Verify {
        state: Box<PasskeyAuthentication>,
    },
}

struct Pending {
    session: String,
    user: String,
    created: DateTime<Utc>,
    kind: Kind,
}

struct Removal {
    session: String,
    user: String,
    id: String,
    created: DateTime<Utc>,
}

/// Why finishing a ceremony did not work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fail {
    Refused(PasskeyRefusedReason),
    /// The store or the relying party failed; logged, never shown.
    Internal,
}

impl From<PasskeyRefusedReason> for Fail {
    fn from(r: PasskeyRefusedReason) -> Self {
        Fail::Refused(r)
    }
}

impl Fail {
    /// The reason to reply with: an internal failure fails closed as `invalid`.
    pub fn reason(self) -> PasskeyRefusedReason {
        match self {
            Fail::Refused(r) => r,
            Fail::Internal => PasskeyRefusedReason::Invalid,
        }
    }
}

/// Why a ceremony could not begin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeginError {
    /// `--public-url` cannot be a relying party.
    Unavailable,
    /// The person has no passkey to verify with.
    NotEnrolled,
    /// A registration needs a recent check (or a redeemed recovery code) first.
    StepUpFirst,
    Internal,
}

/// What a finished ceremony did.
#[derive(Debug, Clone)]
pub enum Finished {
    Registered(PasskeyInfo),
    /// The name of the passkey that answered.
    Verified {
        name: String,
    },
}

/// The refusal a failed ceremony maps to, or `None` for a server-side failure.
fn reason_of<E: std::error::Error>(e: &CeremonyError<E>) -> Option<PasskeyRefusedReason> {
    Some(match e {
        CeremonyError::Webauthn(w) => match w {
            WebauthnError::InvalidRPOrigin
            | WebauthnError::InvalidRPIDHash
            | WebauthnError::CredentialCrossOrigin => PasskeyRefusedReason::WrongOrigin,
            WebauthnError::CredentialPossibleCompromise
            | WebauthnError::CredentialCounterUpdateFailure => PasskeyRefusedReason::Replayed,
            WebauthnError::CredentialNotFound => PasskeyRefusedReason::UnknownCredential,
            _ => PasskeyRefusedReason::Invalid,
        },
        CeremonyError::UnknownCredential => PasskeyRefusedReason::UnknownCredential,
        CeremonyError::InvalidName
        | CeremonyError::UnsupportedKey
        | CeremonyError::Assertion(_) => PasskeyRefusedReason::Invalid,
        CeremonyError::Config | CeremonyError::Store(_) => return None,
    })
}

fn fail_of<E: std::error::Error>(e: &CeremonyError<E>) -> Fail {
    if let Some(r) = reason_of(e) {
        return Fail::Refused(r);
    }
    tracing::error!(error = %e, "passkey ceremony failed");
    Fail::Internal
}

/// A browser credential as webauthn-rs takes it. Transport hints and extension results are
/// dropped first: nothing is decided on them.
fn to_webauthn<T: DeserializeOwned>(credential: &impl Serialize) -> Option<T> {
    let mut v = serde_json::to_value(credential).ok()?;
    let obj = v.as_object_mut()?;
    obj.remove("extensions");
    if let Some(r) = obj.get_mut("response").and_then(Value::as_object_mut) {
        r.remove("transports");
    }
    serde_json::from_value(v).ok()
}

/// The inner `publicKey` options of a challenge, as the browser API takes them.
fn public_key(challenge: &impl Serialize) -> Option<Value> {
    serde_json::to_value(challenge)
        .ok()?
        .get_mut("publicKey")
        .map(Value::take)
}

/// The opaque WebAuthn user handle for a users-registry id: the first 16 bytes of its SHA-256,
/// so it is stable and never the provider id itself.
pub fn user_handle(user: &str) -> Uuid {
    let digest = Sha256::digest(user.as_bytes());
    let mut b = [0u8; 16];
    b.copy_from_slice(&digest[..16]);
    Uuid::from_bytes(b)
}

pub fn info(p: &StoredPasskey) -> PasskeyInfo {
    PasskeyInfo {
        id: p.id.clone(),
        name: p.name.clone(),
        created: p.created.clone(),
        last_used: p.last_used.clone(),
    }
}

/// The relying party for `public_url`: its host is the RP id (`localhost` for a loopback
/// address), its origin the only one accepted.
fn relying_party(cfg: &Config) -> Option<Ceremonies> {
    let mut url = cfg.public_url.clone();
    let loopback = match url.host() {
        Some(url::Host::Ipv4(a)) => a.is_loopback(),
        Some(url::Host::Ipv6(a)) => a.is_loopback(),
        _ => false,
    };
    if loopback {
        url.set_host(Some("localhost")).ok()?;
    }
    let host = url.host_str()?.to_owned();
    let origin = url.origin().ascii_serialization();
    Ceremonies::new(&host, &origin, RP_NAME)
        .inspect_err(|e| tracing::warn!(error = %e, "passkeys unavailable"))
        .ok()
}

/// The passkey service: the store, the relying party, the open ceremonies and removals.
pub struct Passkeys {
    /// `None` when `--public-url` cannot be a relying party; every ceremony is then refused.
    ceremonies: Option<Ceremonies>,
    store: FileStore,
    pub clock: Clock,
    window: chrono::Duration,
    pending: Mutex<HashMap<CeremonyId, Pending>>,
    /// Ceremonies already answered, newest last, so a resent answer reads as `replayed`.
    spent: Mutex<std::collections::VecDeque<(CeremonyId, String)>>,
    removals: Mutex<HashMap<PreviewId, Removal>>,
}

/// How many answered ceremonies are remembered for telling a replay from an expiry.
const SPENT_REMEMBERED: usize = 256;

impl Passkeys {
    /// Opens the store in `data_dir` and the relying party for `public_url`. The note says the
    /// store was corrupt and moved aside.
    pub fn open(cfg: &Config) -> Result<(Self, Option<String>), String> {
        let (store, note) = FileStore::open(&cfg.data_dir)?;
        let window = chrono::Duration::from_std(cfg.step_up_window)
            .unwrap_or_else(|_| chrono::Duration::minutes(5));
        Ok((
            Self {
                ceremonies: relying_party(cfg),
                store,
                clock: Clock::default(),
                window,
                pending: Mutex::new(HashMap::new()),
                spent: Mutex::new(std::collections::VecDeque::new()),
                removals: Mutex::new(HashMap::new()),
            },
            note,
        ))
    }

    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }

    /// The step-up window.
    pub fn window(&self) -> chrono::Duration {
        self.window
    }

    /// `verified_at` is within the step-up window.
    pub fn fresh(&self, verified_at: Option<DateTime<Utc>>) -> bool {
        verified_at.is_some_and(|t| self.now() - t < self.window)
    }

    pub fn list(&self, user: &str) -> Vec<PasskeyInfo> {
        self.store.list(user).iter().map(info).collect()
    }

    pub fn enrolled(&self, user: &str) -> bool {
        !self.store.list(user).is_empty()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<CeremonyId, Pending>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn open_ceremony(&self, p: Pending) -> CeremonyId {
        let id = CeremonyId(Uuid::new_v4());
        let now = self.now();
        let ttl = chrono::Duration::seconds(CEREMONY_TTL_SECS);
        let mut map = self.lock();
        map.retain(|_, c| now - c.created < ttl);
        let mut mine: Vec<(CeremonyId, DateTime<Utc>)> = map
            .iter()
            .filter(|(_, c)| c.session == p.session)
            .map(|(k, c)| (*k, c.created))
            .collect();
        if mine.len() >= MAX_CEREMONIES_PER_SESSION {
            mine.sort_by_key(|(_, created)| *created);
            for (old, _) in mine
                .iter()
                .take(mine.len() + 1 - MAX_CEREMONIES_PER_SESSION)
            {
                map.remove(old);
            }
        }
        map.insert(id, p);
        id
    }

    /// Takes the ceremony: it must be this session's and live. Taking it uses it up; another
    /// session's ceremony is left alone, so it cannot be spoiled from outside.
    fn take(&self, session: &str, id: CeremonyId) -> Result<Pending, PasskeyRefusedReason> {
        let now = self.now();
        let mut map = self.lock();
        if map.get(&id).is_none_or(|p| p.session != session) {
            let spent = self.spent.lock().unwrap_or_else(PoisonError::into_inner);
            if spent.iter().any(|(s, by)| *s == id && by == session) {
                return Err(PasskeyRefusedReason::Replayed);
            }
            return Err(PasskeyRefusedReason::Expired);
        }
        let p = map.remove(&id).ok_or(PasskeyRefusedReason::Expired)?;
        drop(map);
        if now - p.created >= chrono::Duration::seconds(CEREMONY_TTL_SECS) {
            return Err(PasskeyRefusedReason::Expired);
        }
        let mut spent = self.spent.lock().unwrap_or_else(PoisonError::into_inner);
        if spent.len() >= SPENT_REMEMBERED {
            spent.pop_front();
        }
        spent.push_back((id, session.to_owned()));
        Ok(p)
    }

    /// Drops every ceremony and removal of `session` (it ended).
    pub fn forget_session(&self, session: &str) {
        self.lock().retain(|_, p| p.session != session);
        self.removals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|_, r| r.session != session);
    }

    /// Starts registering a passkey named `name`. `allowed` says this session may add one: the
    /// person has none, checked recently, or redeemed a recovery code.
    pub fn begin_register(
        &self,
        session: &str,
        user: &str,
        display: &str,
        name: &str,
        allowed: bool,
    ) -> Result<PasskeyRequest, BeginError> {
        let ceremonies = self.ceremonies.as_ref().ok_or(BeginError::Unavailable)?;
        let existing = self.store.list(user);
        if !(existing.is_empty() || allowed) {
            return Err(BeginError::StepUpFirst);
        }
        let person = PasskeyUser {
            handle: user_handle(user),
            name: display.to_owned(),
            display_name: display.to_owned(),
        };
        let (challenge, state) =
            ceremonies
                .start_registration(&person, &existing)
                .map_err(|e| {
                    tracing::error!(error = %e, "start passkey registration failed");
                    BeginError::Internal
                })?;
        let options = public_key(&challenge).ok_or(BeginError::Internal)?;
        let ceremony = self.open_ceremony(Pending {
            session: session.to_owned(),
            user: user.to_owned(),
            created: self.now(),
            kind: Kind::Register {
                name: name.to_owned(),
                state: Box::new(state),
            },
        });
        Ok(PasskeyRequest { ceremony, options })
    }

    /// Starts an assertion against `user`'s passkeys (a verify or a step-up).
    pub fn begin_verify(&self, session: &str, user: &str) -> Result<PasskeyRequest, BeginError> {
        let ceremonies = self.ceremonies.as_ref().ok_or(BeginError::Unavailable)?;
        let existing = self.store.list(user);
        if existing.is_empty() {
            return Err(BeginError::NotEnrolled);
        }
        let (challenge, state) = ceremonies.start_authentication(&existing).map_err(|e| {
            tracing::error!(error = %e, "start passkey authentication failed");
            BeginError::Internal
        })?;
        let options = public_key(&challenge).ok_or(BeginError::Internal)?;
        let ceremony = self.open_ceremony(Pending {
            session: session.to_owned(),
            user: user.to_owned(),
            created: self.now(),
            kind: Kind::Verify {
                state: Box::new(state),
            },
        });
        Ok(PasskeyRequest { ceremony, options })
    }

    /// Finishes a ceremony of `session`. A registration is re-checked against `allowed` (see
    /// [`Self::begin_register`]). An answer of the wrong kind spends the ceremony.
    pub async fn finish(
        &self,
        session: &str,
        user: &str,
        response: &PasskeyResponse,
        allowed: bool,
    ) -> Result<Finished, Fail> {
        let p = self.take(session, response.ceremony)?;
        if p.user != user {
            return Err(PasskeyRefusedReason::Expired.into());
        }
        match (&p.kind, &response.credential) {
            (Kind::Register { name, state }, PasskeyCredential::Attestation(c)) => self
                .register(user, name, state, c, allowed)
                .map(Finished::Registered),
            (Kind::Verify { state }, PasskeyCredential::Assertion(c)) => self
                .assert(user, state, c)
                .await
                .map(|name| Finished::Verified { name }),
            _ => Err(PasskeyRefusedReason::Expired.into()),
        }
    }

    /// Checks a step-up assertion (on `Confirm` or `OpenShell`) against a verify ceremony of
    /// `session`. Returns the name of the passkey that answered.
    pub async fn verify_assertion(
        &self,
        session: &str,
        user: &str,
        assertion: &PasskeyAssertion,
    ) -> Result<String, Fail> {
        let p = self.take(session, assertion.ceremony)?;
        let Kind::Verify { state } = &p.kind else {
            return Err(PasskeyRefusedReason::Expired.into());
        };
        if p.user != user {
            return Err(PasskeyRefusedReason::Expired.into());
        }
        self.assert(user, state, &assertion.credential).await
    }

    async fn assert(
        &self,
        user: &str,
        state: &PasskeyAuthentication,
        credential: &AssertionCredential,
    ) -> Result<String, Fail> {
        let ceremonies = self.ceremonies.as_ref().ok_or(Fail::Internal)?;
        let response: PublicKeyCredential =
            to_webauthn(credential).ok_or(PasskeyRefusedReason::Invalid)?;
        let result = ceremonies
            .finish_authentication(&self.store, user, &response, state, &rfc3339(self.now()))
            .await
            .map_err(|e| fail_of(&e))?;
        let id = web_passkeys::ceremony::credential_id_string(result.cred_id().as_ref());
        Ok(self
            .store
            .list(user)
            .into_iter()
            .find(|p| p.id == id)
            .map(|p| p.name)
            .unwrap_or_default())
    }

    fn register(
        &self,
        user: &str,
        name: &str,
        state: &PasskeyRegistration,
        credential: &AttestationCredential,
        allowed: bool,
    ) -> Result<PasskeyInfo, Fail> {
        let ceremonies = self.ceremonies.as_ref().ok_or(Fail::Internal)?;
        let response: RegisterPublicKeyCredential =
            to_webauthn(credential).ok_or(PasskeyRefusedReason::Invalid)?;
        // Re-checked under the store lock, so two registrations cannot both be "the first".
        let created = rfc3339(self.now());
        let mut out = Err(Fail::Internal);
        let changed = self.store.change(|users| {
            let existing = users.get(user).cloned().unwrap_or_default();
            if !(existing.is_empty() || allowed) {
                out = Err(PasskeyRefusedReason::Expired.into());
                return;
            }
            match ceremonies.finish_registration(&response, state, name, &existing, &created) {
                Ok(p) if existing.iter().any(|e| e.id == p.id) => {
                    out = Err(PasskeyRefusedReason::Invalid.into());
                }
                Ok(p) => {
                    out = Ok(info(&p));
                    users.entry(user.to_owned()).or_default().push(p);
                }
                Err(e) => out = Err(fail_of(&e)),
            }
        });
        if let Err(e) = changed {
            tracing::error!(error = %e, "passkey store write failed");
            return Err(Fail::Internal);
        }
        out
    }

    /// Opens a removal preview for one of `user`'s passkeys: never the last one.
    pub fn preview_remove(
        &self,
        session: &str,
        user: &str,
        id: &str,
    ) -> Result<(PreviewId, String), PasskeyRefusedReason> {
        let existing = self.store.list(user);
        let target = existing
            .iter()
            .find(|p| p.id == id)
            .ok_or(PasskeyRefusedReason::UnknownCredential)?;
        if existing.len() <= 1 {
            return Err(PasskeyRefusedReason::LastPasskey);
        }
        let preview = PreviewId(Uuid::new_v4());
        let now = self.now();
        let mut map = self.removals.lock().unwrap_or_else(PoisonError::into_inner);
        map.retain(|_, r| now - r.created < chrono::Duration::seconds(REMOVAL_TTL_SECS));
        map.insert(
            preview,
            Removal {
                session: session.to_owned(),
                user: user.to_owned(),
                id: id.to_owned(),
                created: now,
            },
        );
        Ok((preview, target.name.clone()))
    }

    /// `preview` is a live removal preview of `session`.
    pub fn has_removal(&self, session: &str, preview: PreviewId) -> bool {
        let now = self.now();
        self.removals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&preview)
            .is_some_and(|r| {
                r.session == session
                    && now - r.created < chrono::Duration::seconds(REMOVAL_TTL_SECS)
            })
    }

    /// Confirms a removal preview (after its step-up): removes the passkey, never the last one.
    pub fn confirm_remove(
        &self,
        session: &str,
        user: &str,
        preview: PreviewId,
    ) -> Result<StoredPasskey, Fail> {
        let r = {
            let mut map = self.removals.lock().unwrap_or_else(PoisonError::into_inner);
            if map
                .get(&preview)
                .is_none_or(|r| r.session != session || r.user != user)
            {
                return Err(PasskeyRefusedReason::Expired.into());
            }
            map.remove(&preview).ok_or(PasskeyRefusedReason::Expired)?
        };
        if self.now() - r.created >= chrono::Duration::seconds(REMOVAL_TTL_SECS) {
            return Err(PasskeyRefusedReason::Expired.into());
        }
        self.store.remove_one(user, &r.id).map_err(|e| match e {
            RemoveError::Unknown => PasskeyRefusedReason::UnknownCredential.into(),
            RemoveError::Last => PasskeyRefusedReason::LastPasskey.into(),
            RemoveError::Storage(e) => {
                tracing::error!(error = %e, "passkey store write failed");
                Fail::Internal
            }
        })
    }

    /// Redeems `user`'s recovery code and removes every passkey they have. Returns how many.
    pub async fn recover(&self, user: &str, code: &str) -> Result<usize, Fail> {
        match recovery::redeem(&self.store, user, code, self.now().timestamp()).await {
            Ok(()) => {}
            Err(RecoveryError::Invalid) => {
                return Err(PasskeyRefusedReason::RecoveryInvalid.into());
            }
            Err(e) => {
                tracing::error!(error = %e, "recovery failed");
                return Err(Fail::Internal);
            }
        }
        self.store.remove_all_passkeys(user).await.map_err(|e| {
            tracing::error!(error = %e, "passkey store write failed");
            Fail::Internal
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real credential from a soft authenticator; the store never looks inside it.
    fn passkey(id: &str, name: &str) -> StoredPasskey {
        use webauthn_authenticator_rs::WebauthnAuthenticator;
        use webauthn_authenticator_rs::softpasskey::SoftPasskey;
        let origin = "https://depths.example.test";
        let rp = Ceremonies::new("depths.example.test", origin, RP_NAME).unwrap();
        let person = PasskeyUser {
            handle: user_handle("u"),
            name: "u".into(),
            display_name: "u".into(),
        };
        let (ccr, state) = rp.start_registration(&person, &[]).unwrap();
        let reg = WebauthnAuthenticator::new(SoftPasskey::new(true))
            .do_registration(url::Url::parse(origin).unwrap(), ccr)
            .unwrap();
        let mut p = rp
            .finish_registration(&reg, &state, name, &[], "2026-10-28T00:00:00Z")
            .unwrap();
        p.id = id.to_owned();
        p
    }

    #[test]
    fn recovery_command_writes_only_the_hash_and_audits_without_the_code() {
        let dir = tempfile::tempdir().unwrap();
        assert!(run(recovery_command(dir.path(), 1 << 20, "nobody", Utc::now())).is_err());
        let issued = run(recovery_command(
            dir.path(),
            1 << 20,
            "discord:7",
            Utc::now(),
        ))
        .unwrap();
        let file = std::fs::read_to_string(recovery_path(dir.path())).unwrap();
        assert!(file.contains("discord:7"));
        assert!(!file.contains(&issued.code), "only the hash is stored");
        let audit = crate::audit::Audit::open(dir.path().join("audit"), 1 << 20).unwrap();
        let (entries, _) = audit.page(&Default::default(), None).unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert!(matches!(
            e.action,
            depths_protocol::AuditAction::PasskeyRecoveryIssued
        ));
        assert_eq!(
            e.passkey.as_ref().unwrap().person.as_deref(),
            Some("discord:7")
        );
        let json = serde_json::to_string(e).unwrap();
        assert!(!json.contains(&issued.code));
        assert!(!json.contains(&issued.record.code_sha256));
    }

    fn run<T>(f: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn writes_are_atomic_private_and_survive_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let (store, note) = FileStore::open(dir.path()).unwrap();
        assert!(note.is_none());
        run(store.add_passkey("github:1", passkey("a", "Laptop"))).unwrap();
        let path = passkeys_path(dir.path());
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        // No temp file is left behind.
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["passkeys.json".to_owned()], "{names:?}");
        let (again, _) = FileStore::open(dir.path()).unwrap();
        assert_eq!(again.list("github:1")[0].name, "Laptop");
        let v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(v["version"], 1);
    }

    #[test]
    fn a_failed_write_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = FileStore::open(dir.path()).unwrap();
        run(store.add_passkey("u", passkey("a", "One"))).unwrap();
        // A directory where the temp file's rename target is: the write fails.
        std::fs::remove_file(passkeys_path(dir.path())).unwrap();
        std::fs::create_dir(passkeys_path(dir.path())).unwrap();
        assert!(run(store.add_passkey("u", passkey("b", "Two"))).is_err());
        assert_eq!(store.list("u").len(), 1);
    }

    #[test]
    fn a_corrupt_file_is_moved_aside_and_the_store_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(passkeys_path(dir.path()), b"{not json").unwrap();
        let (store, note) = FileStore::open(dir.path()).unwrap();
        let note = note.expect("noted for the audit log");
        assert!(note.contains("moved to"), "{note}");
        assert!(store.list("u").is_empty());
        let aside: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("passkeys.json.corrupt-"))
            .collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(
            std::fs::read(dir.path().join(&aside[0])).unwrap(),
            b"{not json"
        );
    }

    #[test]
    fn names_are_trimmed_bounded_and_unique_per_user() {
        let existing = vec![passkey("a", "Laptop")];
        assert_eq!(
            web_passkeys::ceremony::validate_name("  Phone ", &existing).as_deref(),
            Some("Phone")
        );
        assert!(web_passkeys::ceremony::validate_name("Laptop", &existing).is_none());
        assert!(web_passkeys::ceremony::validate_name("   ", &existing).is_none());
        assert!(web_passkeys::ceremony::validate_name(&"x".repeat(65), &existing).is_none());
        assert!(web_passkeys::ceremony::validate_name(&"x".repeat(64), &existing).is_some());
        // Another person may use the same name.
        assert!(web_passkeys::ceremony::validate_name("Laptop", &[]).is_some());
    }

    #[test]
    fn the_last_passkey_cannot_be_removed_except_by_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = FileStore::open(dir.path()).unwrap();
        run(store.add_passkey("u", passkey("a", "One"))).unwrap();
        run(store.add_passkey("u", passkey("b", "Two"))).unwrap();
        assert!(matches!(
            store.remove_one("u", "x"),
            Err(RemoveError::Unknown)
        ));
        assert_eq!(store.remove_one("u", "a").unwrap().name, "One");
        assert!(matches!(store.remove_one("u", "b"), Err(RemoveError::Last)));
        // Recovery removes them all.
        let issued = recovery::issue_from_entropy("u", 1_000, [7; 10]);
        run(store.put_recovery(issued.record)).unwrap();
        run(recovery::redeem(&store, "u", &issued.code, 1_001)).unwrap();
        assert_eq!(run(store.remove_all_passkeys("u")).unwrap(), 1);
        assert!(store.list("u").is_empty());
        // Used once only.
        assert!(run(recovery::redeem(&store, "u", &issued.code, 1_002)).is_err());
    }

    #[test]
    fn user_handles_are_stable_and_not_the_id() {
        assert_eq!(user_handle("github:1"), user_handle("github:1"));
        assert_ne!(user_handle("github:1"), user_handle("github:2"));
    }

    #[test]
    fn the_clock_moves_forward_on_request() {
        let c = Clock::default();
        let before = c.now();
        c.advance(3600);
        assert!(c.now() - before >= chrono::Duration::seconds(3600));
    }
}
