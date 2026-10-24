//! The persisted network registry, `networks.json` (spec 052 data-model.md).
//!
//! `{ "version": 1, "people": { "<user id>": [KnownNetwork, ...] } }`, written like
//! `users.json`: a 0600 temp file in the same directory, fsync, rename over, fsync the directory.
//! A file that does not parse refuses start, naming its path; a missing file is empty.
//!
//! Every change happens under one lock and is persisted before it is visible, so two
//! simultaneous first sign-ins cannot both become `first`: the first persisted wins (R3). The
//! rule itself is [`depths_core::network`]; this module only stores its inputs.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::net::IpAddr;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use chrono::{DateTime, NaiveDate, Utc};
use depths_core::network::{
    self, Answer, AnswerError, KnownNetwork, NetworkKey, Recorded, Removed, TrustDecision,
    TrustPolicy,
};
use serde::{Deserialize, Serialize};

use crate::users::RegistryError;

const VERSION: u32 = 1;

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    version: u32,
    #[serde(default)]
    people: BTreeMap<String, Vec<KnownNetwork>>,
}

struct State {
    people: BTreeMap<String, Vec<KnownNetwork>>,
    /// The UTC day of the last prune; pruning runs at most daily.
    pruned: Option<NaiveDate>,
}

/// Why a change was not made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkError {
    /// The person has not used that network (answer), or there is no such row (remove).
    NotKnown,
    /// `none` is not an answer.
    NoAnswer,
    /// Writing `networks.json` failed; nothing changed.
    Storage(String),
}

impl From<RegistryError> for NetworkError {
    fn from(e: RegistryError) -> Self {
        Self::Storage(e.0)
    }
}

/// The registry. Cheap to query; every change writes the whole (small) file.
pub struct Networks {
    path: PathBuf,
    policy: TrustPolicy,
    state: Mutex<State>,
}

impl Networks {
    /// Loads `path` (missing = empty) and prunes it. Refuses a file that does not parse.
    pub fn open(
        path: PathBuf,
        policy: TrustPolicy,
        now: DateTime<Utc>,
    ) -> Result<Self, RegistryError> {
        let people = match std::fs::read(&path) {
            Ok(bytes) => {
                let f: File = serde_json::from_slice(&bytes).map_err(|e| {
                    RegistryError(format!(
                        "{} is not a valid network registry: {e}; fix or move it aside, Depths \
                         will not start with an unreadable registry",
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
                f.people
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(RegistryError(format!("{}: {e}", path.display()))),
        };
        let reg = Self {
            path,
            policy,
            state: Mutex::new(State {
                people,
                pruned: None,
            }),
        };
        reg.prune(now)?;
        Ok(reg)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn policy(&self) -> &TrustPolicy {
        &self.policy
    }

    pub fn key_of(&self, ip: IpAddr) -> NetworkKey {
        self.policy.key_of(ip)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Changes one person's rows: on a copy, persisted, then kept. A failed write changes nothing.
    fn change<T>(
        &self,
        user: &str,
        f: impl FnOnce(&mut Vec<KnownNetwork>) -> Result<T, NetworkError>,
    ) -> Result<T, NetworkError> {
        let mut st = self.lock();
        let mut rows = st.people.get(user).cloned().unwrap_or_default();
        let out = f(&mut rows)?;
        let mut people = st.people.clone();
        if rows.is_empty() {
            people.remove(user);
        } else {
            people.insert(user.to_owned(), rows);
        }
        persist(&self.path, &people)?;
        st.people = people;
        Ok(out)
    }

    /// The person's rows, whitelist rows included.
    pub fn list(&self, user: &str) -> Vec<KnownNetwork> {
        self.lock().people.get(user).cloned().unwrap_or_default()
    }

    /// The gate's decision for `user` at `ip`.
    pub fn trust(&self, user: &str, ip: IpAddr, now: DateTime<Utc>) -> TrustDecision {
        let st = self.lock();
        let rows = st.people.get(user).map_or(&[][..], Vec::as_slice);
        network::trust(&self.policy, ip, rows, now)
    }

    /// Whether `key` (any network, not just a used one) is trusted for `user`: the decision for
    /// its first address. Used for rows in the sessions and networks lists.
    pub fn trust_key(&self, user: &str, key: &NetworkKey, now: DateTime<Utc>) -> TrustDecision {
        self.trust(user, key.net(), now)
    }

    /// Records a use of `ip` by `user`. The person's first network becomes `first`.
    pub fn record(
        &self,
        user: &str,
        ip: IpAddr,
        now: DateTime<Utc>,
    ) -> Result<(Recorded, NetworkKey), NetworkError> {
        let key = self.key_of(ip);
        self.change(user, |rows| Ok((network::record_use(rows, key, now), key)))
    }

    /// The person's answer for a network they used. Never grants trust.
    pub fn answer(&self, user: &str, key: &NetworkKey, answer: Answer) -> Result<(), NetworkError> {
        self.change(user, |rows| {
            network::answer(rows, key, answer).map_err(|e| match e {
                AnswerError::NotKnown => NetworkError::NotKnown,
                AnswerError::NoAnswer => NetworkError::NoAnswer,
            })
        })
    }

    /// An admin whitelists `cidr` for `user`.
    pub fn whitelist(
        &self,
        user: &str,
        cidr: NetworkKey,
        by: &str,
        label: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<(), NetworkError> {
        self.change(user, |rows| {
            network::whitelist(rows, cidr, by, label, now);
            Ok(())
        })
    }

    /// Removes a whitelist row, or forgets a used network.
    pub fn remove(&self, user: &str, key: &NetworkKey) -> Result<Removed, NetworkError> {
        self.change(user, |rows| {
            network::remove(rows, key).ok_or(NetworkError::NotKnown)
        })
    }

    /// Forgets non-whitelisted networks unused for 180 days. Runs at most once per UTC day;
    /// returns how many rows went.
    pub fn prune(&self, now: DateTime<Utc>) -> Result<usize, RegistryError> {
        let mut st = self.lock();
        let today = now.date_naive();
        if st.pruned == Some(today) {
            return Ok(0);
        }
        let mut people = st.people.clone();
        let mut gone = 0;
        for rows in people.values_mut() {
            gone += network::prune(rows, now);
        }
        people.retain(|_, rows| !rows.is_empty());
        if gone > 0 {
            persist(&self.path, &people)?;
            st.people = people;
        }
        st.pruned = Some(today);
        Ok(gone)
    }
}

/// Temp file in the same directory (0600), fsync, rename over, fsync the directory.
fn persist(path: &Path, people: &BTreeMap<String, Vec<KnownNetwork>>) -> Result<(), RegistryError> {
    let err = |e: std::io::Error| RegistryError(format!("{}: {e}", path.display()));
    let dir = path
        .parent()
        .ok_or_else(|| RegistryError(format!("{} has no parent", path.display())))?;
    let body = serde_json::to_vec_pretty(&File {
        version: VERSION,
        people: people.clone(),
    })
    .map_err(|e| RegistryError(e.to_string()))?;
    let tmp = dir.join(format!(
        ".networks.json.{}.tmp",
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
    use chrono::{Duration, TimeZone as _};
    use depths_core::network::{Source, TrustReason};
    use std::os::unix::fs::PermissionsExt as _;

    fn t(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, day, 12, 0, 0).unwrap()
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn open(dir: &Path, now: DateTime<Utc>) -> Networks {
        Networks::open(dir.join("networks.json"), TrustPolicy::default(), now).unwrap()
    }

    #[test]
    fn first_network_persists_privately_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let reg = open(dir.path(), t(1));
        let (rec, key) = reg.record("discord:1", ip("203.0.113.7"), t(1)).unwrap();
        assert_eq!(rec, Recorded::First);
        assert_eq!(key.to_string(), "203.0.113.7/32");
        assert_eq!(
            reg.record("discord:1", ip("198.51.100.1"), t(1)).unwrap().0,
            Recorded::New
        );
        let mode = std::fs::metadata(reg.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let again = open(dir.path(), t(2));
        assert_eq!(again.list("discord:1").len(), 2);
        assert_eq!(
            again.trust("discord:1", ip("203.0.113.7"), t(2)).reason,
            TrustReason::First
        );
        assert!(!again.trust("discord:1", ip("198.51.100.1"), t(2)).trusted);
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(reg.path()).unwrap()).unwrap();
        assert_eq!(raw["version"], 1);
        assert_eq!(raw["people"]["discord:1"][0]["source"], "first");
    }

    #[test]
    fn a_corrupt_file_refuses_naming_its_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("networks.json"), b"{nope").unwrap();
        let e = Networks::open(
            dir.path().join("networks.json"),
            TrustPolicy::default(),
            t(1),
        )
        .err()
        .unwrap();
        assert!(e.0.contains("networks.json"), "{e}");
    }

    #[test]
    fn answers_whitelist_and_removal() {
        let dir = tempfile::tempdir().unwrap();
        let reg = open(dir.path(), t(1));
        reg.record("u", ip("203.0.113.7"), t(1)).unwrap();
        let (_, cafe) = reg.record("u", ip("198.51.100.1"), t(1)).unwrap();
        reg.answer("u", &cafe, Answer::Public).unwrap();
        assert_eq!(
            reg.trust("u", ip("198.51.100.1"), t(30)).reason,
            TrustReason::Public
        );
        assert_eq!(
            reg.answer("u", &NetworkKey::parse("192.0.2.1").unwrap(), Answer::Home),
            Err(NetworkError::NotKnown)
        );
        let office = NetworkKey::parse("198.51.100.0/24").unwrap();
        reg.whitelist("u", office, "discord:9", Some("office"), t(2))
            .unwrap();
        assert_eq!(
            reg.trust("u", ip("198.51.100.1"), t(2)).reason,
            TrustReason::Whitelist
        );
        assert_eq!(reg.remove("u", &office), Ok(Removed::Whitelist));
        assert!(!reg.trust("u", ip("198.51.100.1"), t(2)).trusted);
        assert_eq!(reg.remove("u", &cafe), Ok(Removed::Forgotten));
        assert_eq!(reg.remove("u", &cafe), Err(NetworkError::NotKnown));
        assert!(reg.list("u").iter().all(|n| n.source == Source::First));
    }

    #[test]
    fn prune_runs_at_load_and_daily() {
        let dir = tempfile::tempdir().unwrap();
        let reg = open(dir.path(), t(1));
        reg.record("u", ip("203.0.113.7"), t(1)).unwrap();
        reg.whitelist(
            "u",
            NetworkKey::parse("10.0.0.0/8").unwrap(),
            "a",
            None,
            t(1),
        )
        .unwrap();
        let later = t(1) + Duration::days(200);
        let again = open(dir.path(), later);
        assert_eq!(again.list("u").len(), 1);
        assert_eq!(again.prune(later).unwrap(), 0, "once a day");
    }

    #[test]
    fn a_failed_write_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let reg = open(dir.path(), t(1));
        reg.record("u", ip("203.0.113.7"), t(1)).unwrap();
        // Make the directory unwritable.
        let ro = std::fs::Permissions::from_mode(0o500);
        std::fs::set_permissions(dir.path(), ro).unwrap();
        let r = reg.record("u", ip("198.51.100.1"), t(1));
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        if r.is_ok() {
            // Running as root: permissions do not apply; nothing to check.
            return;
        }
        assert!(matches!(r, Err(NetworkError::Storage(_))));
        assert_eq!(reg.list("u").len(), 1);
    }
}
