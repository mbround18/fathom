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

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, PoisonError};

use keel_protocol::{UpdateCheckState, UpdateOutcome};
use oci_registry::Digest;
use time::OffsetDateTime;
use tokio::sync::watch;
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
            image: config.image.to_string(),
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

/// The update notice's shared state, one per process.
#[derive(Debug)]
pub struct Updates {
    /// The running build, fixed for the process.
    pub build: BuildInfo,
    pub config: UpdateConfig,
    /// The current status; subscribers (owner sockets) see each change.
    pub status: watch::Sender<UpdateStatus>,
    pub cache: Mutex<ImageCache>,
    /// The last accepted `CheckForUpdates`, for the FR-007 rate limit.
    pub last_manual: Mutex<Option<OffsetDateTime>>,
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
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse_watched_image;
    use oci_registry::{Credentials, Platform};
    use std::time::Duration;

    fn config(check: bool) -> UpdateConfig {
        UpdateConfig {
            check,
            interval: Duration::from_secs(6 * 3600),
            image: parse_watched_image("mbround18/keel").unwrap(),
            credentials: Credentials::Anonymous,
        }
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
        }
    }

    #[test]
    fn status_starts_not_checked() {
        let u = Updates::new(build(None), config(true));
        let s = u.status();
        assert_eq!(s.state, UpdateCheckState::NotChecked);
        assert!(s.enabled);
        assert_eq!(s.image, "registry-1.docker.io/mbround18/keel:latest");
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
