//! The two image-update files in the data directory (spec 062 data-model.md, research R5).
//!
//! - `updates.json`: what administrators decided (settings, policies, rejected digests, Watchtower
//!   acknowledgements). Versioned, unknown fields refused. **Unreadable:** loaded as
//!   [`Decided::Unreadable`] and never rewritten: checks are off, Auto is paused, and the page
//!   shows the path and the error.
//! - `update-state.json`: what Depths observed (per-service results, the resolution cache,
//!   backoff, the run journal, keep tags). **Unreadable:** moved aside to
//!   `update-state.json.unreadable-<ts>` and rebuilt; it holds nothing a person typed.
//!
//! Both are written as `users.rs` writes `users.json`: a 0600 temp file in the same directory,
//! fsync, rename over, fsync the directory.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use depths_core::updates::window::Clock;
use depths_protocol::{
    Actor, MaintenanceWindow, Provider, RunId, ServiceUpdate, StackName, StackUpdatePolicy,
    TimeOfDay, UpdatePolicy, UpdateRun, UpdateScope, UpdateSettings, UpdatesAvailability,
    WatchtowerAnswer, Weekday,
};
use serde::{Deserialize, Serialize};

pub const UPDATES_FILE: &str = "updates.json";
pub const STATE_FILE: &str = "update-state.json";
const VERSION: u32 = 1;

/// FR-002: never more often than hourly.
pub const MIN_INTERVAL_SECS: u32 = 3600;
pub const DEFAULT_INTERVAL_SECS: u32 = 21_600;
/// FR-014.
pub const HEALTH_TIMEOUT_SECS: std::ops::RangeInclusive<u32> = 30..=1800;
pub const DEFAULT_HEALTH_TIMEOUT_SECS: u32 = 120;
/// "Bounded to 2,000 entries, least recently used evicted."
pub const MAX_RESOLUTIONS: usize = 2000;
/// "The open runs and the last 50 finished runs."
pub const MAX_FINISHED_RUNS: usize = 50;

/// Why a store operation changed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// `updates.json` could not be read at start; it is left alone until a person fixes it.
    Unreadable {
        path: String,
        error: String,
    },
    /// The change would make the file invalid.
    Invalid(String),
    Io(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable { path, error } => write!(f, "{path} is unreadable: {error}"),
            Self::Invalid(m) | Self::Io(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for StoreError {}

// ---------------------------------------------------------------------------
// updates.json
// ---------------------------------------------------------------------------

/// An actor as `updates.json` stores it: the audit shape, with unknown fields refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredActor {
    pub provider: Provider,
    pub subject: String,
    pub username: String,
}

impl From<Actor> for StoredActor {
    fn from(a: Actor) -> Self {
        Self {
            provider: a.provider,
            subject: a.subject,
            username: a.username,
        }
    }
}

impl From<StoredActor> for Actor {
    fn from(a: StoredActor) -> Self {
        Self {
            provider: a.provider,
            subject: a.subject,
            username: a.username,
        }
    }
}

/// The host settings, with who last changed them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredSettings {
    pub checks_enabled: bool,
    pub interval_secs: u32,
    pub window: MaintenanceWindow,
    pub scope: Option<UpdateScope>,
    pub label_only: bool,
    pub cleanup: bool,
    pub health_timeout_secs: u32,
    #[serde(default)]
    pub changed_by: Option<StoredActor>,
    #[serde(default)]
    pub changed_at: Option<String>,
}

impl Default for StoredSettings {
    fn default() -> Self {
        Self {
            checks_enabled: false,
            interval_secs: DEFAULT_INTERVAL_SECS,
            window: default_window(),
            scope: None,
            label_only: false,
            cleanup: false,
            health_timeout_secs: DEFAULT_HEALTH_TIMEOUT_SECS,
            changed_by: None,
            changed_at: None,
        }
    }
}

impl StoredSettings {
    pub fn settings(&self) -> UpdateSettings {
        UpdateSettings {
            checks_enabled: self.checks_enabled,
            interval_secs: self.interval_secs,
            window: self.window.clone(),
            scope: self.scope.clone(),
            label_only: self.label_only,
            cleanup: self.cleanup,
            health_timeout_secs: self.health_timeout_secs,
        }
    }

    /// Takes an administrator's new settings, naming them as the author.
    pub fn set(&mut self, s: UpdateSettings, by: Actor, at: String) {
        self.checks_enabled = s.checks_enabled;
        self.interval_secs = s.interval_secs;
        self.window = s.window;
        self.scope = s.scope;
        self.label_only = s.label_only;
        self.cleanup = s.cleanup;
        self.health_timeout_secs = s.health_timeout_secs;
        self.changed_by = Some(by.into());
        self.changed_at = Some(at);
    }
}

/// Every day, 03:00 to 05:00.
pub fn default_window() -> MaintenanceWindow {
    use Weekday as W;
    MaintenanceWindow {
        days: vec![W::Mon, W::Tue, W::Wed, W::Thu, W::Fri, W::Sat, W::Sun],
        start: TimeOfDay::new("03:00").expect("valid"),
        end: TimeOfDay::new("05:00").expect("valid"),
    }
}

/// A stack's stored policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredPolicy {
    pub policy: UpdatePolicy,
    #[serde(default)]
    pub window: Option<MaintenanceWindow>,
    pub set_by: StoredActor,
    pub set_at: String,
}

impl StoredPolicy {
    pub fn to_wire(&self) -> StackUpdatePolicy {
        StackUpdatePolicy {
            policy: self.policy,
            window: self.window.clone(),
            set_by: self.set_by.clone().into(),
            set_at: self.set_at.clone(),
        }
    }
}

/// A digest that failed its health check and was rolled back (FR-015).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rejected {
    pub stack: String,
    pub service: String,
    pub digest: String,
    pub at: String,
    pub reason: String,
    #[serde(default)]
    pub run_id: Option<RunId>,
}

/// An administrator's answer about one running Watchtower container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatchtowerAck {
    pub container_id: String,
    pub image_id: String,
    pub name: String,
    pub answer: WatchtowerAnswer,
    pub by: StoredActor,
    pub at: String,
}

/// `updates.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decisions {
    pub version: u32,
    #[serde(default)]
    pub settings: StoredSettings,
    #[serde(default)]
    pub policies: BTreeMap<String, StoredPolicy>,
    #[serde(default)]
    pub rejected: Vec<Rejected>,
    #[serde(default)]
    pub watchtower_acks: Vec<WatchtowerAck>,
}

impl Default for Decisions {
    fn default() -> Self {
        Self {
            version: VERSION,
            settings: StoredSettings::default(),
            policies: BTreeMap::new(),
            rejected: Vec::new(),
            watchtower_acks: Vec::new(),
        }
    }
}

/// data-model.md's rules for `updates.json`, beyond what the types already hold (`HH:MM`, the
/// scope's charset, at most seven days, unknown fields).
pub fn validate(d: &Decisions) -> Result<(), String> {
    if d.version != VERSION {
        return Err(format!("unsupported version {}", d.version));
    }
    let s = &d.settings;
    if s.interval_secs < MIN_INTERVAL_SECS {
        return Err(format!(
            "settings.interval_secs is {}; the minimum is {MIN_INTERVAL_SECS} (one hour)",
            s.interval_secs
        ));
    }
    if !HEALTH_TIMEOUT_SECS.contains(&s.health_timeout_secs) {
        return Err(format!(
            "settings.health_timeout_secs is {}; it must be 30 to 1800",
            s.health_timeout_secs
        ));
    }
    validate_window("settings.window", &s.window)?;
    for (stack, p) in &d.policies {
        if StackName::new(stack.as_str()).is_err() {
            return Err(format!("policies: {stack:?} is not a stack name"));
        }
        if let Some(w) = &p.window {
            validate_window(&format!("policies.{stack}.window"), w)?;
        }
    }
    Ok(())
}

/// Days non-empty and distinct; `end == start` refused (empty or 24 h is ambiguous).
pub fn validate_window(what: &str, w: &MaintenanceWindow) -> Result<(), String> {
    if w.days.is_empty() {
        return Err(format!("{what}.days is empty"));
    }
    let distinct: BTreeSet<String> = w.days.iter().map(|d| format!("{d:?}")).collect();
    if distinct.len() != w.days.len() {
        return Err(format!("{what}.days names a day twice"));
    }
    if w.start == w.end {
        return Err(format!(
            "{what}: end equals start; a full day is 00:00 to 23:59"
        ));
    }
    Ok(())
}

/// What `updates.json` held at start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decided {
    Ok(Box<Decisions>),
    /// Never rewritten (Principle VI).
    Unreadable {
        path: String,
        error: String,
    },
}

// ---------------------------------------------------------------------------
// update-state.json
// ---------------------------------------------------------------------------

/// One resolved digest (content addressed: never refreshed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredResolution {
    /// `linux/amd64`, `linux/arm64/v8`.
    pub platform: String,
    pub platform_digest: String,
    pub config_digest: String,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub created: Option<String>,
    pub resolved_at: String,
    /// Recency for eviction: a counter, not a clock, so tests are deterministic.
    #[serde(default)]
    pub used: u64,
}

/// The resolution cache, bounded to [`MAX_RESOLUTIONS`], least recently used evicted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Resolutions(BTreeMap<String, StoredResolution>);

impl Resolutions {
    fn next_use(&self) -> u64 {
        self.0.values().map(|r| r.used).max().map_or(0, |m| m + 1)
    }

    /// A hit counts as a use.
    pub fn get(&mut self, key: &str) -> Option<StoredResolution> {
        let next = self.next_use();
        let r = self.0.get_mut(key)?;
        r.used = next;
        Some(r.clone())
    }

    pub fn put(&mut self, key: String, mut r: StoredResolution) {
        r.used = self.next_use();
        self.0.insert(key, r);
        while self.0.len() > MAX_RESOLUTIONS {
            let oldest = self
                .0
                .iter()
                .min_by_key(|(_, r)| r.used)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(k) => self.0.remove(&k),
                None => break,
            };
        }
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn contains(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }
}

/// Per-registry backoff after a 429 or 5xx.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backoff {
    pub until: String,
    pub failures: u32,
    pub last_error: String,
}

/// A `depths-keep/…` tag Depths set, so cleanup only touches what it recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeepRecord {
    pub stack: String,
    pub service: String,
    pub tag: String,
    pub image_id: String,
    pub at: String,
}

/// `update-state.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observed {
    pub version: u32,
    /// By `<stack>/<service>`.
    #[serde(default)]
    pub services: BTreeMap<String, ServiceUpdate>,
    #[serde(default)]
    pub resolutions: Resolutions,
    /// By registry host.
    #[serde(default)]
    pub backoff: BTreeMap<String, Backoff>,
    /// Open runs and the last [`MAX_FINISHED_RUNS`] finished ones, oldest first.
    #[serde(default)]
    pub journal: Vec<UpdateRun>,
    #[serde(default)]
    pub keep: Vec<KeepRecord>,
}

impl Default for Observed {
    fn default() -> Self {
        Self {
            version: VERSION,
            services: BTreeMap::new(),
            resolutions: Resolutions::default(),
            backoff: BTreeMap::new(),
            journal: Vec::new(),
            keep: Vec::new(),
        }
    }
}

impl Observed {
    pub fn service_key(stack: &str, service: &str) -> String {
        format!("{stack}/{service}")
    }

    /// Adds or replaces a run by id, then drops the oldest finished runs past the bound.
    pub fn upsert_run(&mut self, run: UpdateRun) {
        match self.journal.iter_mut().find(|r| r.run_id == run.run_id) {
            Some(r) => *r = run,
            None => self.journal.push(run),
        }
        let finished = self
            .journal
            .iter()
            .filter(|r| r.finished_at.is_some())
            .count();
        let mut drop = finished.saturating_sub(MAX_FINISHED_RUNS);
        self.journal.retain(|r| {
            if drop > 0 && r.finished_at.is_some() {
                drop -= 1;
                false
            } else {
                true
            }
        });
    }

    pub fn open_runs(&self) -> impl Iterator<Item = &UpdateRun> {
        self.journal.iter().filter(|r| r.finished_at.is_none())
    }
}

// ---------------------------------------------------------------------------
// The store
// ---------------------------------------------------------------------------

pub struct Store {
    dir: PathBuf,
    decided: Mutex<Decided>,
    observed: Mutex<Observed>,
}

impl Store {
    /// Reads both files. A missing file is the defaults and is not written until something
    /// changes. Only an unusable directory is an error.
    pub fn open(dir: impl Into<PathBuf>, clock: Arc<dyn Clock>) -> Result<Self, StoreError> {
        let dir = dir.into();
        let decided = load_decisions(&dir.join(UPDATES_FILE))?;
        let observed = load_observed(&dir.join(STATE_FILE), clock.as_ref())?;
        Ok(Self {
            dir,
            decided: Mutex::new(decided),
            observed: Mutex::new(observed),
        })
    }

    fn decided(&self) -> MutexGuard<'_, Decided> {
        self.decided
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn observed_lock(&self) -> MutexGuard<'_, Observed> {
        self.observed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Off, on, or unreadable (checks off, Auto paused).
    pub fn availability(&self) -> UpdatesAvailability {
        match &*self.decided() {
            Decided::Ok(d) if d.settings.checks_enabled => UpdatesAvailability::On,
            Decided::Ok(_) => UpdatesAvailability::Off,
            Decided::Unreadable { path, error } => UpdatesAvailability::Unreadable {
                path: path.clone(),
                error: error.clone(),
            },
        }
    }

    /// The decisions, or why they could not be read.
    pub fn decisions(&self) -> Result<Decisions, StoreError> {
        match &*self.decided() {
            Decided::Ok(d) => Ok((**d).clone()),
            Decided::Unreadable { path, error } => Err(StoreError::Unreadable {
                path: path.clone(),
                error: error.clone(),
            }),
        }
    }

    /// The settings in force: the defaults (checks off) while the file is unreadable.
    pub fn settings(&self) -> UpdateSettings {
        match &*self.decided() {
            Decided::Ok(d) => d.settings.settings(),
            Decided::Unreadable { .. } => StoredSettings::default().settings(),
        }
    }

    /// Applies `change` to a copy, validates it, writes it, and only then keeps it. Refused while
    /// the file is unreadable: a person fixes it first.
    pub fn update_decisions(
        &self,
        change: impl FnOnce(&mut Decisions),
    ) -> Result<Decisions, StoreError> {
        let mut guard = self.decided();
        let Decided::Ok(current) = &*guard else {
            let Decided::Unreadable { path, error } = &*guard else {
                unreachable!()
            };
            return Err(StoreError::Unreadable {
                path: path.clone(),
                error: error.clone(),
            });
        };
        let mut next = (**current).clone();
        change(&mut next);
        validate(&next).map_err(StoreError::Invalid)?;
        persist(&self.dir.join(UPDATES_FILE), &next)?;
        *guard = Decided::Ok(Box::new(next.clone()));
        Ok(next)
    }

    pub fn observed(&self) -> Observed {
        self.observed_lock().clone()
    }

    /// Changes the observed state and writes it (fsync) before returning: the run journal relies
    /// on the phase being on disk before each mutating step.
    pub fn update_observed<T>(
        &self,
        change: impl FnOnce(&mut Observed) -> T,
    ) -> Result<T, StoreError> {
        let mut guard = self.observed_lock();
        let mut next = guard.clone();
        let out = change(&mut next);
        persist(&self.dir.join(STATE_FILE), &next)?;
        *guard = next;
        Ok(out)
    }

    /// A cached resolution; a hit counts as a use, kept in memory until the next write.
    pub fn resolution(&self, key: &str) -> Option<StoredResolution> {
        self.observed_lock().resolutions.get(key)
    }
}

fn load_decisions(path: &Path) -> Result<Decided, StoreError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Decided::Ok(Box::default()));
        }
        Err(e) => {
            return Ok(Decided::Unreadable {
                path: path.display().to_string(),
                error: e.to_string(),
            });
        }
    };
    let parsed = serde_json::from_slice::<Decisions>(&bytes)
        .map_err(|e| e.to_string())
        .and_then(|d| validate(&d).map(|()| d));
    Ok(match parsed {
        Ok(d) => Decided::Ok(Box::new(d)),
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %error,
                "updates.json is unreadable: checks are off and Auto is paused until it is fixed"
            );
            Decided::Unreadable {
                path: path.display().to_string(),
                error,
            }
        }
    })
}

fn load_observed(path: &Path, clock: &dyn Clock) -> Result<Observed, StoreError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Observed::default()),
        Err(e) => return Err(StoreError::Io(format!("{}: {e}", path.display()))),
    };
    let parsed = serde_json::from_slice::<Observed>(&bytes)
        .map_err(|e| e.to_string())
        .and_then(|o| {
            if o.version == VERSION {
                Ok(o)
            } else {
                Err(format!("unsupported version {}", o.version))
            }
        });
    match parsed {
        Ok(o) => Ok(o),
        Err(error) => {
            let aside = path.with_file_name(format!(
                "{STATE_FILE}.unreadable-{}",
                clock.now().format("%Y%m%dT%H%M%SZ")
            ));
            std::fs::rename(path, &aside)
                .map_err(|e| StoreError::Io(format!("{}: {e}", path.display())))?;
            tracing::warn!(
                path = %path.display(),
                moved_to = %aside.display(),
                %error,
                "update-state.json was unreadable: moved aside, rebuilt by the next check"
            );
            Ok(Observed::default())
        }
    }
}

/// Temp file in the same directory (0600), fsync, rename over, fsync the directory.
fn persist(path: &Path, value: &impl Serialize) -> Result<(), StoreError> {
    let err = |e: std::io::Error| StoreError::Io(format!("{}: {e}", path.display()));
    let dir = path
        .parent()
        .ok_or_else(|| StoreError::Io(format!("{} has no parent", path.display())))?;
    let mut body = serde_json::to_vec_pretty(value).map_err(|e| StoreError::Io(e.to_string()))?;
    body.push(b'\n');
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
        f.write_all(&body)?;
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

    use chrono::{DateTime, TimeZone as _, Utc};
    use depths_protocol::{RunTrigger, StackRun};
    use serde_json::{Value, json};

    struct Fixed(DateTime<Utc>);

    impl Clock for Fixed {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }

    fn clock() -> Arc<dyn Clock> {
        Arc::new(Fixed(Utc.with_ymd_and_hms(2026, 10, 28, 9, 30, 0).unwrap()))
    }

    fn open(dir: &Path) -> Store {
        Store::open(dir, clock()).unwrap()
    }

    fn ana() -> Actor {
        Actor {
            provider: Provider::Github,
            subject: "123".into(),
            username: "ana".into(),
        }
    }

    /// data-model.md's example `updates.json`, valid.
    fn good() -> Value {
        json!({
            "version": 1,
            "settings": {
                "checks_enabled": true,
                "interval_secs": 21600,
                "window": { "days": ["mon","tue","wed","thu","fri","sat","sun"], "start": "03:00", "end": "05:00" },
                "scope": null,
                "label_only": false,
                "cleanup": false,
                "health_timeout_secs": 120,
                "changed_by": { "provider": "github", "subject": "123", "username": "ana" },
                "changed_at": "2026-10-28T09:00:00Z"
            },
            "policies": {
                "media": {
                    "policy": "auto",
                    "window": null,
                    "set_by": { "provider": "github", "subject": "123", "username": "ana" },
                    "set_at": "2026-10-28T09:05:00Z"
                }
            },
            "rejected": [{ "stack": "media", "service": "app", "digest": "sha256:aa", "at": "2026-10-28T03:10:00Z",
                           "reason": "unhealthy after 120 s", "run_id": "6f1c1e9e-6b8a-4a51-9d3e-2b1f0c7a9e10" }],
            "watchtower_acks": [{ "container_id": "c", "image_id": "sha256:bb", "name": "watchtower",
                                  "answer": "monitor_only", "by": { "provider": "github", "subject": "123", "username": "ana" },
                                  "at": "2026-10-28T09:06:00Z" }]
        })
    }

    fn write(dir: &Path, v: &Value) -> Vec<u8> {
        let bytes = serde_json::to_vec_pretty(v).unwrap();
        std::fs::write(dir.join(UPDATES_FILE), &bytes).unwrap();
        bytes
    }

    /// Sets (or adds) the field `pointer` names.
    fn with(mut v: Value, pointer: &str, value: Value) -> Value {
        let (parent, key) = pointer.rsplit_once('/').expect("a pointer");
        let parent = v.pointer_mut(parent).unwrap_or_else(|| panic!("{pointer}"));
        parent
            .as_object_mut()
            .unwrap_or_else(|| panic!("{pointer}"))
            .insert(key.to_owned(), value);
        v
    }

    #[test]
    fn missing_files_are_the_defaults_and_nothing_is_written() {
        let d = tempfile::tempdir().unwrap();
        let s = open(d.path());
        assert_eq!(s.availability(), UpdatesAvailability::Off);
        let set = s.settings();
        assert!(!set.checks_enabled);
        assert_eq!(set.interval_secs, 21_600);
        assert_eq!(set.health_timeout_secs, 120);
        assert_eq!(set.window, default_window());
        assert_eq!(set.scope, None);
        assert_eq!(s.observed(), Observed::default());
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 0);
    }

    #[test]
    fn the_example_file_loads() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), &good());
        let s = open(d.path());
        assert_eq!(s.availability(), UpdatesAvailability::On);
        let dec = s.decisions().unwrap();
        assert_eq!(dec.policies["media"].policy, UpdatePolicy::Auto);
        assert_eq!(dec.policies["media"].to_wire().set_by, ana());
        assert_eq!(dec.rejected.len(), 1);
        assert_eq!(dec.watchtower_acks[0].answer, WatchtowerAnswer::MonitorOnly);
    }

    /// Every rule data-model.md quotes, and unknown fields, at each level.
    #[test]
    fn invalid_files_are_unreadable_and_never_rewritten() {
        let long = "a".repeat(65);
        let cases: Vec<(&str, Value)> = vec![
            (
                "interval below an hour",
                with(good(), "/settings/interval_secs", json!(3599)),
            ),
            (
                "health below 30",
                with(good(), "/settings/health_timeout_secs", json!(29)),
            ),
            (
                "health above 1800",
                with(good(), "/settings/health_timeout_secs", json!(1801)),
            ),
            ("no days", with(good(), "/settings/window/days", json!([]))),
            (
                "a repeated day",
                with(good(), "/settings/window/days", json!(["mon", "mon"])),
            ),
            (
                "eight days",
                with(good(), "/settings/window/days", json!(vec!["mon"; 8])),
            ),
            (
                "end equals start",
                with(good(), "/settings/window/end", json!("03:00")),
            ),
            (
                "24:00",
                with(good(), "/settings/window/end", json!("24:00")),
            ),
            (
                "not HH:MM",
                with(good(), "/settings/window/start", json!("3:00")),
            ),
            (
                "scope charset",
                with(good(), "/settings/scope", json!("a b")),
            ),
            ("scope empty", with(good(), "/settings/scope", json!(""))),
            (
                "scope too long",
                with(good(), "/settings/scope", json!(long)),
            ),
            ("version", with(good(), "/version", json!(2))),
            ("unknown top-level field", with(good(), "/extra", json!(1))),
            (
                "unknown settings field",
                with(good(), "/settings/extra", json!(1)),
            ),
            (
                "unknown policy field",
                with(good(), "/policies/media/extra", json!(1)),
            ),
            (
                "unknown actor field",
                with(good(), "/policies/media/set_by/email", json!("x")),
            ),
            (
                "unknown rejected field",
                with(good(), "/rejected/0/extra", json!(1)),
            ),
            (
                "unknown ack field",
                with(good(), "/watchtower_acks/0/extra", json!(1)),
            ),
            ("bad stack name", {
                let mut v = good();
                let p = v["policies"]["media"].clone();
                v["policies"]["../etc"] = p;
                v
            }),
            (
                "policy window end equals start",
                with(
                    good(),
                    "/policies/media/window",
                    json!({ "days": ["mon"], "start": "01:00", "end": "01:00" }),
                ),
            ),
            ("not json", json!("{")),
        ];
        for (what, v) in cases {
            let d = tempfile::tempdir().unwrap();
            let before = if what == "not json" {
                std::fs::write(d.path().join(UPDATES_FILE), b"{not json").unwrap();
                b"{not json".to_vec()
            } else {
                write(d.path(), &v)
            };
            let s = open(d.path());
            let UpdatesAvailability::Unreadable { path, error } = s.availability() else {
                panic!("{what}: loaded");
            };
            assert!(path.ends_with("updates.json"), "{what}: {path}");
            assert!(!error.is_empty(), "{what}");
            assert!(!s.settings().checks_enabled, "{what}: checks are off");
            let refused = s.update_decisions(|d| d.settings.cleanup = true);
            assert!(
                matches!(refused, Err(StoreError::Unreadable { .. })),
                "{what}"
            );
            assert_eq!(
                std::fs::read(d.path().join(UPDATES_FILE)).unwrap(),
                before,
                "{what}: never rewritten"
            );
        }
    }

    #[test]
    fn edge_values_are_accepted() {
        let cases: Vec<Value> = vec![
            with(good(), "/settings/interval_secs", json!(3600)),
            with(good(), "/settings/health_timeout_secs", json!(30)),
            with(good(), "/settings/health_timeout_secs", json!(1800)),
            with(good(), "/settings/scope", json!("none")),
            with(good(), "/settings/scope", json!("a".repeat(64))),
            with(good(), "/settings/scope", json!("media_1.x-y")),
            with(
                good(),
                "/settings/window",
                json!({ "days": ["sun"], "start": "00:00", "end": "23:59" }),
            ),
            // Crosses midnight.
            with(
                good(),
                "/settings/window",
                json!({ "days": ["fri", "sat"], "start": "23:00", "end": "01:00" }),
            ),
        ];
        for v in cases {
            let d = tempfile::tempdir().unwrap();
            write(d.path(), &v);
            assert!(open(d.path()).decisions().is_ok(), "{v}");
        }
    }

    #[test]
    fn a_change_is_validated_written_privately_and_reloaded() {
        let d = tempfile::tempdir().unwrap();
        let s = open(d.path());
        let mut next = s.settings();
        next.checks_enabled = true;
        next.interval_secs = 7200;
        s.update_decisions(|dec| {
            dec.settings
                .set(next.clone(), ana(), "2026-10-28T09:00:00Z".into());
            dec.policies.insert(
                "media".into(),
                StoredPolicy {
                    policy: UpdatePolicy::Auto,
                    window: None,
                    set_by: ana().into(),
                    set_at: "2026-10-28T09:05:00Z".into(),
                },
            );
        })
        .unwrap();
        let path = d.path().join(UPDATES_FILE);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let leftovers = std::fs::read_dir(d.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);

        let again = open(d.path());
        assert_eq!(again.availability(), UpdatesAvailability::On);
        assert_eq!(again.settings(), next);
        let dec = again.decisions().unwrap();
        assert_eq!(dec.settings.changed_by, Some(ana().into()));
        assert_eq!(dec.policies["media"].policy, UpdatePolicy::Auto);
    }

    #[test]
    fn an_invalid_change_is_refused_and_nothing_is_written() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), &good());
        let before = std::fs::read(d.path().join(UPDATES_FILE)).unwrap();
        let s = open(d.path());
        for change in [
            (|d: &mut Decisions| d.settings.interval_secs = 60) as fn(&mut Decisions),
            |d| d.settings.health_timeout_secs = 5,
            |d| d.settings.window.end = d.settings.window.start.clone(),
            |d| d.settings.window.days.clear(),
        ] {
            let e = s.update_decisions(change).unwrap_err();
            assert!(matches!(e, StoreError::Invalid(_)), "{e}");
        }
        assert_eq!(std::fs::read(d.path().join(UPDATES_FILE)).unwrap(), before);
        assert_eq!(s.settings().interval_secs, 21_600, "memory unchanged too");
    }

    #[test]
    fn an_unreadable_state_file_is_moved_aside_and_rebuilt() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(STATE_FILE), b"{garbage").unwrap();
        let s = open(d.path());
        assert_eq!(s.observed(), Observed::default());
        let aside = d
            .path()
            .join("update-state.json.unreadable-20261028T093000Z");
        assert_eq!(std::fs::read(&aside).unwrap(), b"{garbage");
        assert!(!d.path().join(STATE_FILE).exists());
        // The decisions file is untouched by any of this.
        assert!(!d.path().join(UPDATES_FILE).exists());

        // A version this server does not know is moved aside the same way.
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(STATE_FILE), br#"{"version":9}"#).unwrap();
        open(d.path());
        assert!(
            d.path()
                .join("update-state.json.unreadable-20261028T093000Z")
                .exists()
        );
    }

    fn run(n: u128, finished: bool) -> UpdateRun {
        UpdateRun {
            run_id: RunId(uuid::Uuid::from_u128(n)),
            trigger: RunTrigger::Manual { actor: ana() },
            stacks: vec![StackRun {
                stack: "media".into(),
                services: vec![],
                operation_id: None,
                phase: depths_protocol::RunPhase::Pending,
                outcome: None,
            }],
            started_at: "2026-10-28T03:00:00Z".into(),
            finished_at: finished.then(|| "2026-10-28T03:05:00Z".into()),
        }
    }

    #[test]
    fn the_state_persists_privately_and_reloads() {
        let d = tempfile::tempdir().unwrap();
        let s = open(d.path());
        s.update_observed(|o| {
            o.backoff.insert(
                "registry-1.docker.io".into(),
                Backoff {
                    until: "2026-10-28T09:31:00Z".into(),
                    failures: 1,
                    last_error: "429 Too Many Requests".into(),
                },
            );
            o.upsert_run(run(1, false));
            o.keep.push(KeepRecord {
                stack: "media".into(),
                service: "app".into(),
                tag: "depths-keep/media:app-previous".into(),
                image_id: "sha256:old".into(),
                at: "2026-10-28T03:01:00Z".into(),
            });
        })
        .unwrap();
        let mode = std::fs::metadata(d.path().join(STATE_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let again = open(d.path()).observed();
        assert_eq!(again, s.observed());
        assert_eq!(again.open_runs().count(), 1);
    }

    #[test]
    fn the_journal_keeps_open_runs_and_the_last_fifty_finished() {
        let mut o = Observed::default();
        o.upsert_run(run(0, false));
        for n in 1..=60 {
            o.upsert_run(run(n, true));
        }
        assert_eq!(o.journal.len(), 51);
        assert_eq!(o.open_runs().count(), 1);
        assert_eq!(
            o.journal[1].run_id,
            RunId(uuid::Uuid::from_u128(11)),
            "oldest finished dropped"
        );
        // Closing the open one replaces it, and the bound holds.
        o.upsert_run(run(0, true));
        assert_eq!(o.journal.len(), 50);
        assert_eq!(o.open_runs().count(), 0);
    }

    fn resolution(n: usize) -> StoredResolution {
        StoredResolution {
            platform: "linux/amd64".into(),
            platform_digest: format!("sha256:p{n}"),
            config_digest: format!("sha256:c{n}"),
            user: None,
            version: None,
            revision: None,
            created: None,
            resolved_at: "2026-10-28T03:00:00Z".into(),
            used: 0,
        }
    }

    #[test]
    fn the_resolution_cache_is_bounded_least_recently_used_first() {
        let mut r = Resolutions::default();
        for n in 0..MAX_RESOLUTIONS {
            r.put(format!("k{n}"), resolution(n));
        }
        assert_eq!(r.len(), MAX_RESOLUTIONS);
        // Using k0 makes k1 the least recently used.
        assert!(r.get("k0").is_some());
        r.put("new".into(), resolution(9999));
        assert_eq!(r.len(), MAX_RESOLUTIONS);
        assert!(r.contains("k0") && r.contains("new"));
        assert!(!r.contains("k1"));
        assert!(r.get("missing").is_none());
    }

    #[test]
    fn a_cache_hit_survives_a_reload_as_recent() {
        let d = tempfile::tempdir().unwrap();
        let s = open(d.path());
        s.update_observed(|o| {
            o.resolutions.put("a".into(), resolution(1));
            o.resolutions.put("b".into(), resolution(2));
        })
        .unwrap();
        assert!(s.resolution("a").is_some());
        s.update_observed(|_| ()).unwrap();
        let mut again = open(d.path()).observed().resolutions;
        let a = again.get("a").unwrap().used;
        let b = again.get("b").unwrap().used;
        assert!(a < b, "b was read last here; a was hit before the write");
    }
}
