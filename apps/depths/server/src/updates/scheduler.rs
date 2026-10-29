//! The check part of the update scheduler (research "The loop", T038).
//!
//! One task sleeps until the next stack's check slot (`hash(stack) mod interval`), so checks are
//! spread across the interval and never burst. A settings change or Check now wakes it through
//! [`Updates::wake`]. While checks are off it waits on that wake-up alone: no timer, no registry
//! request (FR-001, SC-001).
//!
//! A pass checks each service HEAD-first through depths-core's `Checker`, records what it saw in
//! `update-state.json`, and backs off per registry after a 429 or a network failure:
//! exponential from one minute up to the check interval, never sooner than the registry's
//! `Retry-After`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use depths_core::ports::ImageInfo;
use depths_core::updates::check::{self, Checker, ResolutionCache, Running, Target, Verdict};
use depths_core::updates::labels::LabelSettings;
use depths_core::updates::policy::{self, ServiceFacts, UpdatePolicy};
use depths_core::updates::reference::ImageRef;
use depths_core::updates::registry::{RegistryError, Resolution};
use depths_protocol::{AvailableImage, RunningImage, ServiceUpdate, UpdateState};

use super::Updates;
use super::inputs::{Inputs, ServiceInput};
use super::store::{Backoff, MAX_HISTORY, Observed, Store, StoredResolution};
use super::view;

const LABEL_VERSION: &str = "org.opencontainers.image.version";
const LABEL_REVISION: &str = "org.opencontainers.image.revision";

/// The first backoff after a refusal.
pub const BACKOFF_START: Duration = Duration::from_secs(60);

/// Check now and the scheduler's next wake-up, behind the service's lock.
#[derive(Debug, Default)]
pub struct CheckNow {
    /// When Check now last started a pass.
    last: Option<DateTime<Utc>>,
    /// What is waiting: `Some(None)` every stack, `Some(Some(..))` those stacks.
    pending: Option<Option<Vec<String>>>,
    /// When the next scheduled check runs; `None` while checks are off or nothing is scheduled.
    next_check: Option<DateTime<Utc>>,
}

/// Why Check now did not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckNowRefused {
    /// Checks are off: `Invalid`.
    Off,
    /// A Check now ran less than the gap ago: `Busy` with the seconds left.
    Busy { retry_after_secs: u32 },
}

impl Updates {
    /// Queues a Check now pass and wakes the scheduler. At most one per
    /// `DEPTHS_UPDATES_CHECK_GAP` host-wide (FR-002), and never while checks are off.
    pub fn request_check_now(&self, stacks: Option<Vec<String>>) -> Result<(), CheckNowRefused> {
        if !self.checks_enabled() {
            return Err(CheckNowRefused::Off);
        }
        let now = self.clock.now();
        let gap = chrono::Duration::from_std(self.config.check_min_gap)
            .unwrap_or_else(|_| chrono::Duration::seconds(60));
        {
            let mut c = self.check_now_lock();
            if let Some(last) = c.last
                && now < last + gap
            {
                let left = (last + gap - now).num_milliseconds();
                let secs = u32::try_from((left + 999) / 1000)
                    .unwrap_or(u32::MAX)
                    .max(1);
                return Err(CheckNowRefused::Busy {
                    retry_after_secs: secs,
                });
            }
            c.last = Some(now);
            c.pending = Some(stacks);
        }
        self.wake();
        Ok(())
    }

    /// Wakes the scheduler: Check now, or a settings change.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// Fires after every change to what `ListUpdates` shows.
    pub fn subscribe_changes(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.changed.subscribe()
    }

    /// Tells the subscribers that the view changed.
    pub fn notify_changed(&self) {
        let _ = self.changed.send(());
    }

    fn check_now_lock(&self) -> std::sync::MutexGuard<'_, CheckNow> {
        self.check_now
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn take_check_now(&self) -> Option<Option<Vec<String>>> {
        self.check_now_lock().pending.take()
    }

    fn set_next_check(&self, at: Option<DateTime<Utc>>) {
        self.check_now_lock().next_check = at;
    }
}

/// When the next scheduled check runs.
pub fn next_check(u: &Updates) -> Option<DateTime<Utc>> {
    u.check_now_lock().next_check
}

/// FNV-1a: stable across builds and runs, unlike `DefaultHasher`.
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// A stack's slot within the interval, in seconds: `hash(stack) mod interval`.
pub fn slot_offset(stack: &str, interval_secs: u32) -> i64 {
    let i = u64::from(interval_secs.max(1));
    i64::try_from(fnv1a(stack) % i).unwrap_or(0)
}

/// The latest slot at or before `now`.
pub fn prev_slot(stack: &str, interval_secs: u32, now: DateTime<Utc>) -> DateTime<Utc> {
    let i = i64::from(interval_secs.max(1));
    let off = slot_offset(stack, interval_secs);
    let k = (now.timestamp() - off).div_euclid(i);
    Utc.timestamp_opt(k * i + off, 0).single().unwrap_or(now)
}

/// The first slot after `now`.
pub fn next_slot(stack: &str, interval_secs: u32, now: DateTime<Utc>) -> DateTime<Utc> {
    prev_slot(stack, interval_secs, now)
        + chrono::Duration::seconds(i64::from(interval_secs.max(1)))
}

/// The wait after the `failures`th refusal in a row: one minute, doubling, at most the interval,
/// and never shorter than the registry's `Retry-After`.
pub fn backoff_delay(failures: u32, interval: Duration, retry_after: Option<Duration>) -> Duration {
    let cap = interval.max(BACKOFF_START);
    let shift = failures.saturating_sub(1).min(20);
    let delay = BACKOFF_START.saturating_mul(1u32 << shift).min(cap);
    delay.max(retry_after.unwrap_or_default())
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// The resolution cache over `update-state.json`: reads hit the store, new entries are written
/// with the pass's results.
struct StoreCache<'a> {
    store: &'a Store,
    platform: String,
    at: String,
    local: HashMap<String, Resolution>,
    new: Vec<(String, StoredResolution)>,
}

impl ResolutionCache for StoreCache<'_> {
    fn get(&mut self, key: &str) -> Option<Resolution> {
        if let Some(r) = self.local.get(key) {
            return Some(r.clone());
        }
        let r = from_stored(self.store.resolution(key)?);
        self.local.insert(key.to_owned(), r.clone());
        Some(r)
    }

    fn put(&mut self, key: String, r: Resolution) {
        self.new
            .push((key.clone(), to_stored(&r, &self.platform, &self.at)));
        self.local.insert(key, r);
    }
}

fn from_stored(s: StoredResolution) -> Resolution {
    let mut labels = BTreeMap::new();
    if let Some(v) = s.version {
        labels.insert(LABEL_VERSION.to_owned(), v);
    }
    if let Some(v) = s.revision {
        labels.insert(LABEL_REVISION.to_owned(), v);
    }
    Resolution {
        index_digest: None,
        manifest_digest: s.platform_digest,
        config_digest: s.config_digest,
        labels,
        user: s.user,
        created: s.created,
    }
}

fn to_stored(r: &Resolution, platform: &str, at: &str) -> StoredResolution {
    StoredResolution {
        platform: platform.to_owned(),
        platform_digest: r.manifest_digest.clone(),
        config_digest: r.config_digest.clone(),
        user: r.user.clone(),
        version: r.labels.get(LABEL_VERSION).cloned(),
        revision: r.labels.get(LABEL_REVISION).cloned(),
        created: r.created.clone(),
        resolved_at: at.to_owned(),
        used: 0,
    }
}

fn running_image(reference: Option<&ImageRef>, i: &ImageInfo) -> RunningImage {
    let repo_digest = i
        .repo_digests
        .iter()
        .find(|d| reference.is_none_or(|r| ImageRef::parse(d).is_ok_and(|p| p.name() == r.name())))
        .cloned();
    RunningImage {
        image_id: i.id.clone(),
        repo_digest,
        version: i.labels.get(LABEL_VERSION).cloned(),
        revision: i.labels.get(LABEL_REVISION).cloned(),
    }
}

fn available_image(digest: String, r: &Resolution) -> AvailableImage {
    AvailableImage {
        digest,
        platform_digest: r.manifest_digest.clone(),
        config_digest: r.config_digest.clone(),
        version: r.labels.get(LABEL_VERSION).cloned(),
        revision: r.labels.get(LABEL_REVISION).cloned(),
        created: r.created.clone(),
        user: r.user.clone(),
    }
}

fn failed_reason(e: &RegistryError) -> String {
    let s = e.to_string();
    if s.starts_with("can't check") {
        s
    } else {
        format!("can't check: {s}")
    }
}

/// What one pass learnt, written in one `update_observed`.
#[derive(Default)]
struct PassResult {
    services: BTreeMap<String, ServiceUpdate>,
    history: BTreeMap<String, String>,
    backoff: BTreeMap<String, Backoff>,
    cleared: BTreeSet<String>,
}

/// Checks `stacks` once. Returns early, without a request, while checks are off.
pub async fn pass(u: &Updates, inputs: Arc<dyn Inputs>, stacks: Vec<String>) {
    if !u.checks_enabled() {
        return;
    }
    let gathered = {
        let inputs = inputs.clone();
        tokio::task::spawn_blocking(move || {
            let all = inputs.stacks();
            let host = inputs.host();
            let wanted: Vec<String> = stacks.into_iter().filter(|s| all.contains(s)).collect();
            let services: Vec<(String, Vec<ServiceInput>)> = wanted
                .into_iter()
                .map(|s| {
                    let v = inputs.services(&s);
                    (s, v)
                })
                .collect();
            let watchtower = inputs.watchtower();
            (all, host, services, watchtower)
        })
        .await
    };
    let Ok((all, host, services, watchtower)) = gathered else {
        tracing::warn!("update check: gathering inputs panicked");
        return;
    };
    let Some(host) = host else {
        tracing::warn!("update check skipped: docker did not report the host platform");
        return;
    };
    // US4: what Auto pauses on. Read every pass; nothing here touches it (FR-021).
    u.set_watchtower(watchtower);

    let now = u.clock.now();
    let now_s = rfc3339(now);
    let settings = u.store.settings();
    let interval = Duration::from_secs(u64::from(settings.interval_secs.max(60)));
    let decisions = u.store.decisions().ok();
    let observed = u.store.observed();
    let platform = host.platform.clone();
    let mut cache = StoreCache {
        store: &u.store,
        platform: format!(
            "{}/{}{}",
            platform.os,
            platform.architecture,
            platform
                .variant
                .as_deref()
                .map(|v| format!("/{v}"))
                .unwrap_or_default()
        ),
        at: now_s.clone(),
        local: HashMap::new(),
        new: Vec::new(),
    };
    let registry = u.registry.clone();
    let mut out = PassResult::default();
    // Registries backed off before or during this pass: no request until `until`.
    let mut backed_off: BTreeMap<String, String> = observed
        .backoff
        .iter()
        .filter(|(_, b)| parse_time(&b.until).is_some_and(|t| t > now))
        .map(|(h, b)| (h.clone(), b.until.clone()))
        .collect();

    for (stack, inputs) in &services {
        let stack_policy = decisions
            .as_ref()
            .and_then(|d| d.policies.get(stack))
            .map(|p| view::policy_to_core(p.policy));
        for s in inputs {
            let key = Observed::service_key(stack, &s.service);
            let prior = observed.services.get(&key);
            let reference = s.image.as_deref().map(ImageRef::parse).and_then(Result::ok);
            // An image with no name left came from a registry once; only its digests are unknown.
            let local_image = !s.unnamed_image
                && s.running
                    .as_ref()
                    .is_some_and(|r| r.repo_digests.is_empty());
            let eff = policy::effective(
                ServiceFacts {
                    image: s.image.as_deref(),
                    has_build: s.has_build,
                    local_image,
                    is_depths: s.is_depths,
                    labels: &s.labels,
                },
                stack_policy,
                LabelSettings {
                    label_only: settings.label_only,
                    scope: settings.scope.as_ref().map(|s| s.as_str()),
                },
            );
            let mut su = ServiceUpdate {
                stack: stack.clone(),
                service: s.service.clone(),
                reference: s.image.clone().unwrap_or_default(),
                running: s
                    .running
                    .as_ref()
                    .map(|i| running_image(reference.as_ref(), i)),
                available: None,
                state: UpdateState::NotChecked,
                reason: None,
                effective: view::effective_to_wire(&eff),
                last_checked: prior.and_then(|p| p.last_checked.clone()),
                last_outcome: prior.and_then(|p| p.last_outcome.clone()),
                replicas: s.replicas,
            };
            if let Some(ex) = &eff.excluded {
                su.state = UpdateState::Excluded;
                su.reason = Some(view::exclusion_reason(ex));
                out.services.insert(key, su);
                continue;
            }
            if eff.policy == UpdatePolicy::Off {
                su.reason = Some("updates are off for this stack".into());
                out.services.insert(key, su);
                continue;
            }
            let Some(reference) = reference else {
                continue;
            };
            if let Some(until) = backed_off.get(&reference.registry) {
                su.state = UpdateState::CheckFailed;
                su.available = prior.and_then(|p| p.available.clone());
                su.reason = Some(format!(
                    "can't check: {} is rate limited until {until}",
                    reference.registry
                ));
                out.services.insert(key, su);
                continue;
            }
            let history = observed.history.get(&key).cloned().unwrap_or_default();
            let rejected: Vec<String> = decisions
                .as_ref()
                .map(|d| {
                    d.rejected
                        .iter()
                        .filter(|r| &r.stack == stack && r.service == s.service)
                        .map(|r| r.digest.clone())
                        .collect()
                })
                .unwrap_or_default();
            let running = s.running.as_ref().map(|i| Running {
                image_id: &i.id,
                repo_digests: &i.repo_digests,
            });
            let store = match running {
                Some(r) if !s.unnamed_image => check::detect_store(r.image_id, r.repo_digests),
                _ => host.store,
            };
            let verdict = Checker::new(registry.as_ref(), &platform, store, &mut cache)
                .check(&Target {
                    reference: &reference,
                    running,
                    ran_before: &history,
                    rejected: &rejected,
                })
                .await;
            su.last_checked = Some(now_s.clone());
            match verdict {
                Verdict::UpToDate { digest } => {
                    su.state = UpdateState::UpToDate;
                    out.history.insert(key.clone(), digest);
                    out.cleared.insert(reference.registry.clone());
                }
                Verdict::Available { digest, resolution } => {
                    su.state = UpdateState::Available;
                    su.available = Some(available_image(digest, &resolution));
                    out.cleared.insert(reference.registry.clone());
                }
                Verdict::MovedBack { .. } => {
                    su.state = UpdateState::MovedBack;
                    su.reason =
                        Some("the tag points back to an image this service ran before".into());
                    out.cleared.insert(reference.registry.clone());
                }
                Verdict::Rejected { .. } => {
                    su.state = UpdateState::Rejected;
                    su.reason = Some("this image was rolled back before".into());
                    out.cleared.insert(reference.registry.clone());
                }
                Verdict::NotRunning { .. } => {
                    su.reason = Some("no container runs for this service".into());
                    out.cleared.insert(reference.registry.clone());
                }
                Verdict::Failed(e) => {
                    su.state = UpdateState::CheckFailed;
                    su.available = prior.and_then(|p| p.available.clone());
                    if su.running.is_none() {
                        su.running = prior.and_then(|p| p.running.clone());
                    }
                    su.reason = Some(failed_reason(&e));
                    if e.starts_backoff() {
                        let host = reference.registry.clone();
                        let failures = observed.backoff.get(&host).map_or(0, |b| b.failures) + 1;
                        let delay = backoff_delay(failures, interval, registry.retry_after(&host));
                        let until = rfc3339(
                            now + chrono::Duration::from_std(delay)
                                .unwrap_or_else(|_| chrono::Duration::seconds(60)),
                        );
                        tracing::info!(registry = %host, failures, until = %until, "update check: registry backoff");
                        backed_off.insert(host.clone(), until.clone());
                        out.cleared.remove(&host);
                        out.backoff.insert(
                            host,
                            Backoff {
                                until,
                                failures,
                                last_error: e.code().to_owned(),
                            },
                        );
                    }
                }
            }
            out.services.insert(key, su);
        }
    }

    let checked: BTreeSet<&str> = services.iter().map(|(s, _)| s.as_str()).collect();
    let all: BTreeSet<&str> = all.iter().map(String::as_str).collect();
    let new_resolutions = std::mem::take(&mut cache.new);
    let written = u.store.update_observed(|o| {
        // Services that left a checked stack, and stacks that are gone.
        o.services.retain(|k, v| {
            all.contains(v.stack.as_str())
                && (!checked.contains(v.stack.as_str()) || out.services.contains_key(k))
        });
        o.history
            .retain(|k, _| k.split_once('/').is_some_and(|(s, _)| all.contains(s)));
        for (k, v) in &out.services {
            o.services.insert(k.clone(), v.clone());
        }
        for (k, d) in &out.history {
            let h = o.history.entry(k.clone()).or_default();
            if h.last() != Some(d) {
                h.retain(|x| x != d);
                h.push(d.clone());
            }
            if h.len() > MAX_HISTORY {
                let drop = h.len() - MAX_HISTORY;
                h.drain(..drop);
            }
        }
        for h in &out.cleared {
            o.backoff.remove(h);
        }
        for (h, b) in &out.backoff {
            o.backoff.insert(h.clone(), b.clone());
        }
        for (k, r) in new_resolutions {
            o.resolutions.put(k, r);
        }
    });
    if let Err(e) = written {
        tracing::warn!(error = %e, "update check: cannot write the update state");
        return;
    }
    u.notify_changed();
}

/// The scheduler task: checks each stack at its slot, Check now when asked, and nothing while
/// checks are off.
pub async fn run(u: Arc<Updates>, inputs: Arc<dyn Inputs>) {
    // When each stack was last checked by this task, seeded from the stored state so a restart
    // does not check everything at once. A stack never checked is checked at once.
    let mut last: HashMap<String, DateTime<Utc>> = HashMap::new();
    for s in u.store.observed().services.values() {
        if let Some(t) = s.last_checked.as_deref().and_then(parse_time) {
            let e = last.entry(s.stack.clone()).or_insert(t);
            *e = (*e).max(t);
        }
    }
    loop {
        if !u.checks_enabled() {
            u.set_next_check(None);
            u.wake.notified().await;
            continue;
        }
        let now = u.clock.now();
        let interval = u.store.settings().interval_secs.max(60);
        let all = {
            let inputs = inputs.clone();
            tokio::task::spawn_blocking(move || inputs.stacks())
                .await
                .unwrap_or_default()
        };
        if let Some(req) = u.take_check_now() {
            let stacks = req.unwrap_or_else(|| all.clone());
            pass(&u, inputs.clone(), stacks.clone()).await;
            for s in stacks {
                last.insert(s, now);
            }
            continue;
        }
        let due: Vec<String> = all
            .iter()
            .filter(|s| {
                last.get(*s)
                    .is_none_or(|t| *t < prev_slot(s, interval, now))
            })
            .cloned()
            .collect();
        if !due.is_empty() {
            pass(&u, inputs.clone(), due.clone()).await;
            for s in due {
                last.insert(s, now);
            }
            continue;
        }
        let next = all.iter().map(|s| next_slot(s, interval, now)).min();
        u.set_next_check(next);
        let wait = next.map_or(Duration::from_secs(u64::from(interval)), |t| {
            (t - now)
                .to_std()
                .unwrap_or_default()
                .max(Duration::from_secs(1))
        });
        tokio::select! {
            () = tokio::time::sleep(wait) => {}
            () = u.wake.notified() => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use depths_core::ports::{HostPlatform, ImageStore};
    use depths_core::updates::registry::{Platform, Registry, RegistryFuture};
    use depths_core::updates::window::Clock;
    use depths_protocol::{Actor, Provider, UpdateSettings};

    use super::*;

    const D_OLD: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    const D_NEW: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";
    const CFG_NEW: &str = "sha256:3333333333333333333333333333333333333333333333333333333333333333";

    /// Wall time that follows tokio's (paused) clock.
    struct TokioClock {
        base: DateTime<Utc>,
        start: tokio::time::Instant,
    }

    impl TokioClock {
        fn new() -> Self {
            Self {
                base: Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap(),
                start: tokio::time::Instant::now(),
            }
        }
    }

    impl Clock for TokioClock {
        fn now(&self) -> DateTime<Utc> {
            self.base + chrono::Duration::from_std(self.start.elapsed()).unwrap()
        }
    }

    /// Every tag points to `digest`; counts requests per call.
    struct FakeRegistry {
        digest: Mutex<Result<String, RegistryError>>,
        heads: AtomicUsize,
        resolves: AtomicUsize,
        heads_by_stack: Mutex<BTreeMap<String, Vec<DateTime<Utc>>>>,
        clock: Arc<dyn Clock>,
    }

    impl FakeRegistry {
        fn new(clock: Arc<dyn Clock>) -> Self {
            Self {
                digest: Mutex::new(Ok(D_OLD.into())),
                heads: AtomicUsize::new(0),
                resolves: AtomicUsize::new(0),
                heads_by_stack: Mutex::new(BTreeMap::new()),
                clock,
            }
        }
    }

    impl Registry for FakeRegistry {
        fn head_digest<'a>(&'a self, r: &'a ImageRef) -> RegistryFuture<'a, String> {
            self.heads.fetch_add(1, Ordering::SeqCst);
            // The fake images are named after their stack.
            let stack = r
                .repository
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_owned();
            self.heads_by_stack
                .lock()
                .unwrap()
                .entry(stack)
                .or_default()
                .push(self.clock.now());
            let out = self.digest.lock().unwrap().clone();
            Box::pin(async move { out })
        }

        fn resolve<'a>(
            &'a self,
            _r: &'a ImageRef,
            _digest: &'a str,
            _p: &'a Platform,
        ) -> RegistryFuture<'a, Resolution> {
            self.resolves.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Ok(Resolution {
                    index_digest: None,
                    manifest_digest: D_NEW.into(),
                    config_digest: CFG_NEW.into(),
                    labels: BTreeMap::from([(LABEL_VERSION.to_owned(), "1.28".to_owned())]),
                    user: None,
                    created: None,
                })
            })
        }
    }

    /// Stacks `s0..sN`, one service `app` running `registry.test/<stack>:1` at `D_OLD`.
    struct FakeInputs {
        stacks: Vec<String>,
    }

    impl Inputs for FakeInputs {
        fn stacks(&self) -> Vec<String> {
            self.stacks.clone()
        }

        fn host(&self) -> Option<HostPlatform> {
            Some(HostPlatform {
                platform: Platform {
                    os: "linux".into(),
                    architecture: "amd64".into(),
                    variant: None,
                },
                store: ImageStore::Classic,
            })
        }

        fn services(&self, stack: &str) -> Vec<ServiceInput> {
            vec![ServiceInput {
                service: "app".into(),
                image: Some(format!("registry.test/{stack}:1")),
                replicas: 1,
                running: Some(ImageInfo {
                    id: "sha256:9999999999999999999999999999999999999999999999999999999999999999"
                        .into(),
                    repo_digests: vec![format!("registry.test/{stack}@{D_OLD}")],
                    ..ImageInfo::default()
                }),
                ..ServiceInput::default()
            }]
        }
    }

    fn stacks(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("s{i}")).collect()
    }

    fn open(dir: &std::path::Path, clock: Arc<dyn Clock>, reg: Arc<FakeRegistry>) -> Arc<Updates> {
        let mut cfg = crate::updates::tests::config(dir, "on");
        std::fs::create_dir_all(&cfg.data_dir).unwrap();
        cfg.updates.check_min_gap = Duration::from_secs(60);
        cfg.updates.docker_config = dir.join("docker");
        Updates::open(&cfg, clock, move || reg as Arc<dyn Registry>)
            .unwrap()
            .unwrap()
    }

    fn turn_on(u: &Updates, interval_secs: u32) {
        let s = u.store.settings();
        let on = UpdateSettings {
            checks_enabled: true,
            interval_secs,
            ..s
        };
        u.store
            .update_decisions(|d| {
                d.settings.set(
                    on,
                    Actor {
                        provider: Provider::Depths,
                        subject: "admin".into(),
                        username: "admin".into(),
                    },
                    "2026-10-01T00:00:00Z".into(),
                );
            })
            .unwrap();
        u.wake();
    }

    #[test]
    fn slots_are_hash_mod_interval_and_spread() {
        let i = 21_600;
        assert_eq!(slot_offset("web", i), slot_offset("web", i));
        assert!(slot_offset("web", i) < i64::from(i));
        let offsets: BTreeSet<i64> = stacks(30).iter().map(|s| slot_offset(s, i)).collect();
        assert!(
            offsets.len() >= 28,
            "30 stacks share few slots: {offsets:?}"
        );
        let now = Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap();
        let p = prev_slot("web", i, now);
        let n = next_slot("web", i, now);
        assert!(p <= now && now < n);
        assert_eq!((n - p).num_seconds(), i64::from(i));
        assert_eq!(
            (p.timestamp() - slot_offset("web", i)).rem_euclid(i64::from(i)),
            0
        );
    }

    #[test]
    fn backoff_doubles_from_a_minute_to_the_interval_and_honours_retry_after() {
        let hour = Duration::from_secs(3600);
        let secs: Vec<u64> = (1..=8)
            .map(|f| backoff_delay(f, hour, None).as_secs())
            .collect();
        assert_eq!(secs, vec![60, 120, 240, 480, 960, 1920, 3600, 3600]);
        assert_eq!(
            backoff_delay(1, hour, Some(Duration::from_secs(300))),
            Duration::from_secs(300)
        );
        assert_eq!(backoff_delay(u32::MAX, hour, None), hour);
    }

    #[tokio::test(start_paused = true)]
    async fn no_registry_request_while_checks_are_off() {
        let dir = tempfile::tempdir().unwrap();
        let clock: Arc<dyn Clock> = Arc::new(TokioClock::new());
        let reg = Arc::new(FakeRegistry::new(clock.clone()));
        let u = open(dir.path(), clock, reg.clone());
        let inputs: Arc<dyn Inputs> = Arc::new(FakeInputs { stacks: stacks(5) });
        let task = tokio::spawn(run(u.clone(), inputs.clone()));
        tokio::time::sleep(Duration::from_secs(48 * 3600)).await;
        pass(&u, inputs, stacks(5)).await;
        assert_eq!(reg.heads.load(Ordering::SeqCst), 0);
        assert_eq!(u.request_check_now(None), Err(CheckNowRefused::Off));
        assert_eq!(next_check(&u), None);
        task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn each_stack_is_checked_once_per_interval_at_its_slot() {
        let dir = tempfile::tempdir().unwrap();
        let clock: Arc<dyn Clock> = Arc::new(TokioClock::new());
        let reg = Arc::new(FakeRegistry::new(clock.clone()));
        let u = open(dir.path(), clock.clone(), reg.clone());
        let inputs: Arc<dyn Inputs> = Arc::new(FakeInputs { stacks: stacks(30) });
        turn_on(&u, 3600);
        let task = tokio::spawn(run(u.clone(), inputs));
        // Never checked: the first pass checks every stack once.
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(reg.heads.load(Ordering::SeqCst), 30);
        assert!(next_check(&u).is_some());
        // Then each stack once per interval, at its own slot.
        tokio::time::sleep(Duration::from_secs(3 * 3600)).await;
        assert_eq!(reg.heads.load(Ordering::SeqCst), 30 + 3 * 30);
        let by_stack = reg.heads_by_stack.lock().unwrap().clone();
        for (stack, times) in &by_stack {
            for t in &times[1..] {
                let off = (t.timestamp() - slot_offset(stack, 3600)).rem_euclid(3600);
                assert!(off <= 1, "{stack} checked {off}s after its slot");
            }
        }
        // Up to date: one HEAD each, no resolve.
        assert_eq!(reg.resolves.load(Ordering::SeqCst), 0);
        let o = u.store.observed();
        assert_eq!(o.services.len(), 30);
        assert!(
            o.services
                .values()
                .all(|s| s.state == UpdateState::UpToDate)
        );
        task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn check_now_runs_at_most_once_a_minute() {
        let dir = tempfile::tempdir().unwrap();
        let clock: Arc<dyn Clock> = Arc::new(TokioClock::new());
        let reg = Arc::new(FakeRegistry::new(clock.clone()));
        let u = open(dir.path(), clock, reg.clone());
        let inputs: Arc<dyn Inputs> = Arc::new(FakeInputs { stacks: stacks(3) });
        turn_on(&u, 21_600);
        let task = tokio::spawn(run(u.clone(), inputs));
        tokio::time::sleep(Duration::from_secs(1)).await;
        let first = reg.heads.load(Ordering::SeqCst);
        assert_eq!(first, 3);

        assert_eq!(u.request_check_now(None), Ok(()));
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(reg.heads.load(Ordering::SeqCst), 6);
        match u.request_check_now(None) {
            Err(CheckNowRefused::Busy { retry_after_secs }) => {
                assert!((58..=60).contains(&retry_after_secs), "{retry_after_secs}");
            }
            other => panic!("{other:?}"),
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert_eq!(u.request_check_now(Some(vec!["s1".into()])), Ok(()));
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(reg.heads.load(Ordering::SeqCst), 7);
        task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn a_moved_tag_is_resolved_once_and_shown_available() {
        let dir = tempfile::tempdir().unwrap();
        let clock: Arc<dyn Clock> = Arc::new(TokioClock::new());
        let reg = Arc::new(FakeRegistry::new(clock.clone()));
        *reg.digest.lock().unwrap() = Ok(D_NEW.into());
        let u = open(dir.path(), clock, reg.clone());
        let inputs: Arc<dyn Inputs> = Arc::new(FakeInputs { stacks: stacks(2) });
        turn_on(&u, 3600);
        let mut changes = u.subscribe_changes();
        pass(&u, inputs.clone(), stacks(2)).await;
        assert!(changes.try_recv().is_ok());
        let o = u.store.observed();
        let s = &o.services["s0/app"];
        assert_eq!(s.state, UpdateState::Available);
        let a = s.available.as_ref().unwrap();
        assert_eq!(a.digest, D_NEW);
        assert_eq!(a.config_digest, CFG_NEW);
        assert_eq!(a.version.as_deref(), Some("1.28"));
        assert_eq!(reg.resolves.load(Ordering::SeqCst), 2);
        // The resolution is cached in the state file: the next pass resolves nothing.
        pass(&u, inputs, stacks(2)).await;
        assert_eq!(reg.resolves.load(Ordering::SeqCst), 2);
        assert_eq!(view::badge(&view::services(&u)), 2);
    }

    /// `s0`'s `app` runs an image someone replaced by pulling the tag by hand: the image has no
    /// name left and only its id is known.
    struct HandPulled;

    impl Inputs for HandPulled {
        fn stacks(&self) -> Vec<String> {
            stacks(1)
        }

        fn host(&self) -> Option<HostPlatform> {
            FakeInputs { stacks: stacks(1) }.host()
        }

        fn services(&self, stack: &str) -> Vec<ServiceInput> {
            vec![ServiceInput {
                service: "app".into(),
                image: Some(format!("registry.test/{stack}:1")),
                replicas: 1,
                running: Some(ImageInfo {
                    id: "sha256:9999999999999999999999999999999999999999999999999999999999999999"
                        .into(),
                    ..ImageInfo::default()
                }),
                unnamed_image: true,
                ..ServiceInput::default()
            }]
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_container_whose_image_lost_its_name_is_matched_by_image_id() {
        let dir = tempfile::tempdir().unwrap();
        let clock: Arc<dyn Clock> = Arc::new(TokioClock::new());
        let reg = Arc::new(FakeRegistry::new(clock.clone()));
        *reg.digest.lock().unwrap() = Ok(D_NEW.into());
        let u = open(dir.path(), clock, reg.clone());
        turn_on(&u, 3600);
        pass(&u, Arc::new(HandPulled), stacks(1)).await;
        let s = &u.store.observed().services["s0/app"];
        assert_eq!(
            s.state,
            UpdateState::Available,
            "not \"no container runs\", not a local image: {:?}",
            s.reason
        );
        assert_eq!(s.reason, None);
        let running = s.running.as_ref().unwrap();
        assert!(running.image_id.ends_with("9999"));
        assert_eq!(running.repo_digest, None);
        assert_eq!(s.available.as_ref().unwrap().digest, D_NEW);
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limits_back_off_per_registry_and_keep_the_state() {
        let dir = tempfile::tempdir().unwrap();
        let clock: Arc<dyn Clock> = Arc::new(TokioClock::new());
        let reg = Arc::new(FakeRegistry::new(clock.clone()));
        *reg.digest.lock().unwrap() = Ok(D_NEW.into());
        let u = open(dir.path(), clock, reg.clone());
        let inputs: Arc<dyn Inputs> = Arc::new(FakeInputs { stacks: stacks(3) });
        turn_on(&u, 3600);
        pass(&u, inputs.clone(), stacks(3)).await;
        assert_eq!(reg.heads.load(Ordering::SeqCst), 3);

        *reg.digest.lock().unwrap() = Err(RegistryError::RateLimited);
        pass(&u, inputs.clone(), stacks(3)).await;
        // The first refusal backs the registry off: the other two make no request.
        assert_eq!(reg.heads.load(Ordering::SeqCst), 4);
        let o = u.store.observed();
        let b = &o.backoff["registry.test"];
        assert_eq!(b.failures, 1);
        assert_eq!(b.last_error, "rate_limited");
        for s in o.services.values() {
            assert_eq!(s.state, UpdateState::CheckFailed);
            assert!(
                s.available.is_some(),
                "the last known available image is kept"
            );
            assert!(s.reason.as_deref().unwrap().starts_with("can't check"));
        }

        // Within the minute: no request.
        tokio::time::sleep(Duration::from_secs(30)).await;
        pass(&u, inputs.clone(), stacks(3)).await;
        assert_eq!(reg.heads.load(Ordering::SeqCst), 4);
        // After it: one more refusal doubles the wait.
        tokio::time::sleep(Duration::from_secs(31)).await;
        pass(&u, inputs.clone(), stacks(3)).await;
        assert_eq!(reg.heads.load(Ordering::SeqCst), 5);
        assert_eq!(u.store.observed().backoff["registry.test"].failures, 2);
        // Success clears it.
        tokio::time::sleep(Duration::from_secs(121)).await;
        *reg.digest.lock().unwrap() = Ok(D_NEW.into());
        pass(&u, inputs, stacks(3)).await;
        assert!(u.store.observed().backoff.is_empty());
        assert_eq!(view::badge(&view::services(&u)), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_check_never_reports_an_update() {
        let dir = tempfile::tempdir().unwrap();
        let clock: Arc<dyn Clock> = Arc::new(TokioClock::new());
        let reg = Arc::new(FakeRegistry::new(clock.clone()));
        *reg.digest.lock().unwrap() = Err(RegistryError::NoDigestHeader);
        let u = open(dir.path(), clock, reg.clone());
        let inputs: Arc<dyn Inputs> = Arc::new(FakeInputs { stacks: stacks(1) });
        turn_on(&u, 3600);
        pass(&u, inputs, stacks(1)).await;
        let o = u.store.observed();
        let s = &o.services["s0/app"];
        assert_eq!(s.state, UpdateState::CheckFailed);
        assert!(s.available.is_none());
        assert!(
            o.backoff.is_empty(),
            "a missing digest header does not back off"
        );
        assert_eq!(view::badge(&view::services(&u)), 0);
    }
}
