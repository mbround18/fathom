//! The update notice (spec 061): what this Keel runs, what the watched tag points at, and the
//! in-memory status owners see (data-model.md "Update status").
//!
//! This is the skeleton (T020). [`Updates`] lives in [`crate::serve::AppState::updates`] and holds
//! everything the later pieces share:
//!
//! - the checker and its schedule (`check.rs`, contracts/config.md "Schedule") write
//!   [`Updates::status`] and fill [`Updates::cache`];
//! - the command handlers (`commands.rs`: `GetKeelBuild`, `GetUpdateStatus`, `CheckForUpdates`,
//!   `DismissUpdateDot`, `PreviewKeelUpdate`) read the status, shape it by role into
//!   `keel_protocol::UpdateStatus` / `UpdateStatusLine`, and use [`Updates::last_manual`] for the
//!   Check now rate limit (FR-007).
//!
//! Nothing here touches the registry. With `KEEL_UPDATE_CHECK=false` nothing ever will (FR-004).

pub mod check;
pub mod classify;
pub mod commands;
pub mod manual;
pub mod scheduler;

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use keel_core::ports::{Clock, SystemClock};
use keel_protocol::{UpdateCheckState, UpdateOutcome};
use oci_registry::{Client, Digest};
use time::OffsetDateTime;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use url::Url;

use crate::build_info::BuildInfo;
use crate::config::UpdateConfig;

/// The image the watched tag points at, built from `oci_registry::Resolved` (data-model.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageInfo {
    /// The tag's HEAD digest.
    pub digest: Digest,
    /// From the `org.opencontainers.image.*` labels.
    pub version: Option<String>,
    pub revision: Option<String>,
    /// The label, else `Resolved.created`.
    pub built_at: Option<OffsetDateTime>,
    pub release_notes: Option<Url>,
    /// Why there's no release-notes link (R7). Set exactly when `release_notes` isn't.
    pub release_notes_note: Option<String>,
}

/// The last failed check since the last success. Cleared on success (FR-010).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckError {
    /// `oci_registry::RegistryError::code()`.
    pub code: &'static str,
    pub at: OffsetDateTime,
}

/// The update status, in memory only (data-model.md "Update status"). Owners see all of it;
/// admins a derived `UpdateStatusLine` (SC-006).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateStatus {
    /// `KEEL_UPDATE_CHECK`.
    pub enabled: bool,
    /// The watched reference, without credentials.
    pub image: String,
    /// `NotChecked` after every restart (FR-008).
    pub state: UpdateCheckState,
    /// From the last successful check.
    pub outcome: Option<UpdateOutcome>,
    /// For `CantTell`: `no_image_for_platform`, `missing_labels`, `running_unknown`.
    pub cant_tell_reason: Option<String>,
    /// The last successful check.
    pub checked_at: Option<OffsetDateTime>,
    /// Set unless `UpToDate`.
    pub available: Option<ImageInfo>,
    pub last_error: Option<CheckError>,
    pub next_check_at: Option<OffsetDateTime>,
    /// When Check now is allowed again (FR-007).
    pub manual_after: Option<OffsetDateTime>,
    /// `false` when the running digest is unknown (R3).
    pub digest_compare: bool,
}

impl UpdateStatus {
    /// The status at start: nothing checked yet.
    #[must_use]
    pub fn initial(build: &BuildInfo, config: &UpdateConfig) -> Self {
        Self {
            enabled: config.check,
            image: config.image_name.clone(),
            state: UpdateCheckState::NotChecked,
            outcome: None,
            cant_tell_reason: None,
            checked_at: None,
            available: None,
            last_error: None,
            next_check_at: None,
            manual_after: None,
            digest_compare: build.digest.is_some(),
        }
    }
}

/// Remote images already resolved, by digest: a hit costs only the HEAD (data-model.md). Holds at
/// most [`ImageCache::CAP`] entries and evicts the oldest.
#[derive(Debug, Default)]
pub struct ImageCache {
    entries: HashMap<Digest, ImageInfo>,
    order: VecDeque<Digest>,
}

impl ImageCache {
    pub const CAP: usize = 16;

    #[must_use]
    pub fn get(&self, digest: &Digest) -> Option<&ImageInfo> {
        self.entries.get(digest)
    }

    /// Adds `info` under its digest, evicting the oldest entry past [`Self::CAP`].
    pub fn insert(&mut self, info: ImageInfo) {
        let digest = info.digest.clone();
        if self.entries.insert(digest.clone(), info).is_none() {
            self.order.push_back(digest);
        }
        while self.order.len() > Self::CAP {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Check now is allowed once a minute per instance (FR-007).
pub const MANUAL_EVERY: time::Duration = time::Duration::minutes(1);

/// The update notice's shared state, one per process.
pub struct Updates {
    /// The running build, fixed for the process.
    pub build: BuildInfo,
    pub config: UpdateConfig,
    /// The current status; subscribers (owner sockets) see each change.
    pub status: watch::Sender<UpdateStatus>,
    pub cache: Mutex<ImageCache>,
    /// The last accepted `CheckForUpdates`, for the FR-007 rate limit.
    pub last_manual: Mutex<Option<OffsetDateTime>>,
    clock: Arc<dyn Clock>,
    /// Built on the first check, so `KEEL_UPDATE_CHECK=false` never builds one (FR-004). The error
    /// is `RegistryError::code()`.
    client: OnceLock<Result<Client, &'static str>>,
    /// One check at a time: a manual check during a scheduled one waits for it.
    running: tokio::sync::Mutex<()>,
}

impl fmt::Debug for Updates {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Updates")
            .field("build", &self.build)
            .field("config", &self.config)
            .field("status", &*self.status.borrow())
            .field("client_built", &self.client_built())
            .finish_non_exhaustive()
    }
}

impl Updates {
    #[must_use]
    pub fn new(build: BuildInfo, config: UpdateConfig) -> Self {
        let (status, _) = watch::channel(UpdateStatus::initial(&build, &config));
        Self {
            build,
            config,
            status,
            cache: Mutex::new(ImageCache::default()),
            last_manual: Mutex::new(None),
            clock: Arc::new(SystemClock),
            client: OnceLock::new(),
            running: tokio::sync::Mutex::new(()),
        }
    }

    /// Uses `clock` for every timestamp (tests pass a `TestClock`).
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Uses `client` instead of building one (tests point it at a `FakeRegistry`).
    #[must_use]
    pub fn with_client(self, client: Client) -> Self {
        let _ = self.client.set(Ok(client));
        self
    }

    /// Whether a registry client exists. Always `false` with checks off (FR-004).
    #[must_use]
    pub fn client_built(&self) -> bool {
        self.client.get().is_some()
    }

    /// A copy of the current status.
    #[must_use]
    pub fn status(&self) -> UpdateStatus {
        self.status.borrow().clone()
    }

    /// The remote-image cache. A poisoned lock still holds a usable cache.
    pub fn cache(&self) -> std::sync::MutexGuard<'_, ImageCache> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn last_manual(&self) -> std::sync::MutexGuard<'_, Option<OffsetDateTime>> {
        self.last_manual
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// `Some(when)` while Check now is refused (FR-007): `CheckForUpdates` replies
    /// `update_check_too_soon` with this time. `None` when a manual check would run.
    #[must_use]
    pub fn manual_retry_at(&self) -> Option<OffsetDateTime> {
        let now = self.clock.now();
        self.last_manual()
            .map(|last| last + MANUAL_EVERY)
            .filter(|at| *at > now)
    }

    /// Runs one check now and returns the status it leaves.
    ///
    /// - With checks off, returns the (off) status without building a client or touching the
    ///   network (FR-004).
    /// - `manual`: within a minute of the last accepted manual check, returns the current status
    ///   unchanged (its `manual_after` says when); otherwise records this one. A manual check
    ///   doesn't move the schedule.
    /// - Checks are serialized. The status goes to `checking`, then to the result; each change is
    ///   pushed on [`Self::status`]. A failure keeps the last good result and sets `last_error`.
    pub async fn check_now(&self, manual: bool) -> UpdateStatus {
        if !self.config.check {
            return self.status();
        }
        if manual {
            let now = self.clock.now();
            let mut last = self.last_manual();
            if last.is_some_and(|l| now < l + MANUAL_EVERY) {
                return self.status();
            }
            *last = Some(now);
            drop(last);
            self.status
                .send_modify(|s| s.manual_after = Some(now + MANUAL_EVERY));
        }
        let _one = self.running.lock().await;
        self.status
            .send_modify(|s| s.state = UpdateCheckState::Checking);
        let trigger = if manual {
            check::Trigger::Manual
        } else {
            check::Trigger::Scheduled
        };
        let result = match self.client() {
            Ok(client) => check::run(self, client, trigger).await.result,
            Err(code) => Err(code),
        };
        self.apply(result);
        self.status()
    }

    fn client(&self) -> Result<&Client, &'static str> {
        self.client
            .get_or_init(|| {
                let id = self
                    .build
                    .version
                    .as_deref()
                    .or(self.build.revision.as_deref())
                    .unwrap_or("dev");
                Client::new(&format!("keel/{id} (update-check)")).map_err(|e| e.code())
            })
            .as_ref()
            .map_err(|e| *e)
    }

    fn apply(&self, result: Result<check::Checked, &'static str>) {
        let now = self.clock.now();
        self.status.send_modify(|s| match result {
            Ok(c) => {
                s.state = UpdateCheckState::Checked;
                s.outcome = Some(c.classified.outcome);
                s.cant_tell_reason = c.classified.cant_tell_reason.map(str::to_owned);
                s.available = c.available;
                s.checked_at = Some(now);
                s.last_error = None;
            }
            Err(code) => {
                s.state = if s.checked_at.is_some() {
                    UpdateCheckState::Checked
                } else {
                    UpdateCheckState::NotChecked
                };
                s.last_error = Some(CheckError { code, at: now });
            }
        });
    }

    /// Starts the scheduled checks (contracts/config.md "Schedule"), or `None` with checks off:
    /// then no task, no client, no request ever (FR-004).
    #[must_use]
    pub fn spawn_checker(self: &Arc<Self>) -> Option<JoinHandle<()>> {
        if !self.config.check {
            return None;
        }
        Some(self.spawn_checker_with(scheduler::os_random))
    }

    fn spawn_checker_with(
        self: &Arc<Self>,
        random: impl FnMut() -> f64 + Send + 'static,
    ) -> JoinHandle<()> {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            let next = {
                let this = Arc::clone(&this);
                move |delay: Duration| {
                    let at = this.clock.now() + delay;
                    this.status.send_modify(|s| s.next_check_at = Some(at));
                }
            };
            let check = {
                let this = Arc::clone(&this);
                move || {
                    let this = Arc::clone(&this);
                    async move { this.check_now(false).await.last_error.is_none() }
                }
            };
            scheduler::run(this.config.interval, random, check, next).await;
        })
    }
}

/// Records the build Keel started as in `keel_build_seen` (FR-019, data-model.md). A start on a
/// different digest or revision than the stored row writes one `keel_updated`, actor System; the
/// first start after migration 0011 only fills the row, as there is nothing to compare against.
/// Returns whether `keel_updated` was written.
///
/// # Errors
///
/// A database error.
pub async fn record_build_seen(db: &sqlx::PgPool, build: &BuildInfo) -> Result<bool, sqlx::Error> {
    use crate::audit::{self, Actor, Decision, Entry};
    let mut tx = db.begin().await?;
    let seen: Option<(Option<String>, Option<String>, Option<String>)> =
        sqlx::query_as("select digest, revision, version from keel_build_seen where id for update")
            .fetch_optional(&mut *tx)
            .await?;
    let changed = match &seen {
        None => false,
        Some((digest, revision, _)) => *digest != build.digest || *revision != build.revision,
    };
    if seen.is_some() && !changed {
        return Ok(false);
    }
    sqlx::query(
        "insert into keel_build_seen (id, digest, revision, version, seen_at) \
         values (true, $1, $2, $3, now()) \
         on conflict (id) do update set digest = excluded.digest, revision = excluded.revision, \
         version = excluded.version, seen_at = excluded.seen_at",
    )
    .bind(&build.digest)
    .bind(&build.revision)
    .bind(&build.version)
    .execute(&mut *tx)
    .await?;
    if let Some((old_digest, old_revision, old_version)) = seen {
        audit::record(
            &mut tx,
            Entry {
                actor: &Actor::System,
                action: "keel_updated",
                workspace: None,
                target: serde_json::json!({}),
                decision: Decision::Allowed,
                reason: None,
                context: serde_json::json!({
                    "old_digest": old_digest,
                    "new_digest": build.digest,
                    "old_revision": old_revision,
                    "new_revision": build.revision,
                    "old_version": old_version,
                    "new_version": build.version,
                }),
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(changed)
}


#[cfg(test)]
mod tests {
    use super::*;
    use oci_registry::Platform;
    use std::time::Duration;

    fn config(check: bool) -> UpdateConfig {
        UpdateConfig::watching(check, Duration::from_secs(6 * 3600), "mbround18/keel").unwrap()
    }

    fn build(digest: Option<&str>) -> BuildInfo {
        BuildInfo {
            version: None,
            revision: None,
            built_at: None,
            source: None,
            digest: digest.map(str::to_owned),
            platform: Platform::current(),
        }
    }

    fn digest(n: usize) -> Digest {
        Digest::parse(&format!("sha256:{n:064x}")).unwrap()
    }

    fn info(n: usize) -> ImageInfo {
        ImageInfo {
            digest: digest(n),
            version: None,
            revision: None,
            built_at: None,
            release_notes: None,
            release_notes_note: None,
        }
    }

    #[test]
    fn status_starts_not_checked() {
        let u = Updates::new(build(None), config(true));
        let s = u.status();
        assert_eq!(s.state, UpdateCheckState::NotChecked);
        assert!(s.enabled);
        assert_eq!(s.image, "mbround18/keel");
        assert_eq!(s.outcome, None);
        assert_eq!(s.checked_at, None);
        assert!(!s.digest_compare);
        assert_eq!(*u.last_manual.lock().unwrap(), None);
        assert!(u.cache().is_empty());

        let d = format!("sha256:{}", "a".repeat(64));
        let s = Updates::new(build(Some(&d)), config(false)).status();
        assert!(!s.enabled);
        assert!(s.digest_compare);
    }

    #[test]
    fn status_changes_reach_subscribers() {
        let u = Updates::new(build(None), config(true));
        let mut rx = u.status.subscribe();
        u.status
            .send_modify(|s| s.state = UpdateCheckState::Checking);
        assert!(rx.has_changed().unwrap());
        assert_eq!(rx.borrow_and_update().state, UpdateCheckState::Checking);
    }

    mod checking {
        use super::super::*;
        use keel_core::ports::TestClock;
        use oci_registry::Platform;
        use oci_registry::testing::{FakeRegistry, ImageSpec};
        use time::format_description::well_known::Rfc3339;

        const REPO: &str = "mbround18/keel";
        const OLD: &str = "2026-09-01T12:00:00Z";
        const NEW: &str = "2026-10-01T12:00:00Z";

        fn at(s: &str) -> OffsetDateTime {
            OffsetDateTime::parse(s, &Rfc3339).unwrap()
        }

        fn running() -> BuildInfo {
            BuildInfo {
                version: Some("0.2.0".into()),
                revision: Some("abc1234".into()),
                built_at: Some(at(OLD)),
                source: None,
                digest: Some(format!("sha256:{}", "a".repeat(64))),
                platform: Platform {
                    os: "linux".into(),
                    architecture: "amd64".into(),
                    variant: None,
                },
            }
        }

        fn watching(fake: &FakeRegistry, check: bool) -> UpdateConfig {
            let u = fake.url();
            let image = format!("{}:{}/{REPO}", u.host_str().unwrap(), u.port().unwrap());
            UpdateConfig::watching(check, Duration::from_secs(6 * 3600), &image).unwrap()
        }

        fn newer_image(fake: &FakeRegistry) -> Digest {
            fake.image(
                REPO,
                "latest",
                &ImageSpec::new("linux", "amd64")
                    .label("org.opencontainers.image.revision", "def5678")
                    .label("org.opencontainers.image.created", NEW),
            )
            .manifest
        }

        fn checker(fake: &FakeRegistry, clock: &Arc<TestClock>) -> Updates {
            Updates::new(running(), watching(fake, true))
                .with_clock(clock.clone())
                .with_client(Client::with_base_url("keel/test", fake.url()))
        }

        // T040: with checks off there is no task, no client and no request, ever (FR-004).
        #[tokio::test]
        async fn checks_off_never_touch_the_network() {
            let fake = FakeRegistry::start().await;
            newer_image(&fake);
            let u = Arc::new(Updates::new(running(), watching(&fake, false)));
            assert!(u.spawn_checker().is_none());

            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(3 * 24 * 3600)).await;
            tokio::time::resume();

            for manual in [true, false] {
                let s = u.check_now(manual).await;
                assert!(!s.enabled);
                assert_eq!(s.state, UpdateCheckState::NotChecked);
                assert_eq!(s.next_check_at, None);
            }
            assert_eq!(u.manual_retry_at(), None);
            assert!(!u.client_built());
            assert!(fake.requests().is_empty(), "{:?}", fake.requests());
        }

        #[tokio::test]
        async fn a_check_publishes_checking_then_the_result() {
            let fake = FakeRegistry::start().await;
            let new = newer_image(&fake);
            let clock = Arc::new(TestClock::new(at("2026-10-28T10:00:00Z")));
            let u = checker(&fake, &clock);
            let mut rx = u.status.subscribe();

            let s = u.check_now(false).await;
            assert_eq!(s.state, UpdateCheckState::Checked);
            assert_eq!(s.outcome, Some(UpdateOutcome::UpdateAvailable));
            assert_eq!(s.available.as_ref().map(|a| &a.digest), Some(&new));
            assert_eq!(s.checked_at, Some(clock.now()));
            assert_eq!(s.last_error, None);
            // A scheduled check isn't rate limited and doesn't set manual_after.
            assert_eq!(s.manual_after, None);
            assert_eq!(u.manual_retry_at(), None);
            assert!(rx.has_changed().unwrap());
            assert_eq!(*rx.borrow_and_update(), s);
        }

        #[tokio::test]
        async fn check_now_is_limited_to_once_a_minute() {
            let fake = FakeRegistry::start().await;
            newer_image(&fake);
            let t0 = at("2026-10-28T10:00:00Z");
            let clock = Arc::new(TestClock::new(t0));
            let u = checker(&fake, &clock);

            let s = u.check_now(true).await;
            assert_eq!(s.checked_at, Some(t0));
            assert_eq!(s.manual_after, Some(t0 + MANUAL_EVERY));
            assert_eq!(*u.last_manual.lock().unwrap(), Some(t0));

            // Too soon: nothing runs, the status is as it was.
            fake.clear_requests();
            clock.advance(time::Duration::seconds(30));
            assert_eq!(u.manual_retry_at(), Some(t0 + MANUAL_EVERY));
            let again = u.check_now(true).await;
            assert_eq!(again, s);
            assert!(fake.requests().is_empty());

            // A scheduled check in between still runs and doesn't reset the limit.
            let scheduled = u.check_now(false).await;
            assert_eq!(scheduled.checked_at, Some(t0 + time::Duration::seconds(30)));
            assert_eq!(u.manual_retry_at(), Some(t0 + MANUAL_EVERY));

            clock.advance(time::Duration::seconds(31));
            assert_eq!(u.manual_retry_at(), None);
            let later = u.check_now(true).await;
            assert_eq!(later.checked_at, Some(clock.now()));
            assert_eq!(later.manual_after, Some(clock.now() + MANUAL_EVERY));
        }

        #[tokio::test]
        async fn a_failure_keeps_the_last_result_and_success_clears_the_error() {
            let fake = FakeRegistry::start().await;
            let new = newer_image(&fake);
            let t0 = at("2026-10-28T10:00:00Z");
            let clock = Arc::new(TestClock::new(t0));
            let u = checker(&fake, &clock);

            let good = u.check_now(false).await;
            assert_eq!(good.outcome, Some(UpdateOutcome::UpdateAvailable));

            fake.set_status(&format!("/v2/{REPO}/manifests/latest"), 429);
            clock.advance(time::Duration::minutes(5));
            let failed = u.check_now(false).await;
            assert_eq!(failed.state, UpdateCheckState::Checked);
            assert_eq!(failed.outcome, good.outcome);
            assert_eq!(failed.available.as_ref().map(|a| &a.digest), Some(&new));
            assert_eq!(failed.checked_at, Some(t0));
            assert_eq!(
                failed.last_error,
                Some(CheckError {
                    code: "rate_limited",
                    at: clock.now()
                })
            );
        }

        #[tokio::test]
        async fn a_first_failure_stays_not_checked() {
            let fake = FakeRegistry::start().await;
            newer_image(&fake);
            fake.omit_digest_header(true);
            let clock = Arc::new(TestClock::new(at("2026-10-28T10:00:00Z")));
            let u = checker(&fake, &clock);
            let s = u.check_now(false).await;
            assert_eq!(s.state, UpdateCheckState::NotChecked);
            assert_eq!(s.outcome, None);
            assert_eq!(s.last_error.map(|e| e.code), Some("no_digest_header"));
        }

        #[tokio::test]
        async fn the_checker_announces_the_first_check_in_60_to_120_s() {
            let fake = FakeRegistry::start().await;
            let t0 = at("2026-10-28T10:00:00Z");
            let clock = Arc::new(TestClock::new(t0));
            let u = Arc::new(checker(&fake, &clock));
            let mut rx = u.status.subscribe();
            let task = u.spawn_checker().expect("checks are on");
            rx.changed().await.unwrap();
            let next = rx.borrow().next_check_at.expect("next check announced");
            task.abort();
            assert!(next >= t0 + time::Duration::seconds(60), "{next}");
            assert!(next <= t0 + time::Duration::seconds(120), "{next}");
            // The first check is a minute away: nothing has been asked yet.
            assert!(fake.requests().is_empty());
        }
    }

    #[test]
    fn cache_holds_sixteen_and_evicts_the_oldest() {
        let mut c = ImageCache::default();
        for n in 0..ImageCache::CAP {
            c.insert(info(n));
        }
        // Re-inserting a present digest doesn't make it newer or grow the cache.
        c.insert(info(0));
        assert_eq!(c.len(), ImageCache::CAP);
        c.insert(info(ImageCache::CAP));
        assert_eq!(c.len(), ImageCache::CAP);
        assert!(c.get(&digest(0)).is_none());
        assert!(c.get(&digest(1)).is_some());
        assert_eq!(
            c.get(&digest(ImageCache::CAP)),
            Some(&info(ImageCache::CAP))
        );
    }
}
