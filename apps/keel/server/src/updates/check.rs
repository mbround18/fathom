//! One update check (research R1, contracts/registry.md "Requests"):
//!
//! 1. HEAD the watched tag. Never a GET: a missing `Docker-Content-Digest` is `no_digest_header`.
//! 2. The running digest: `up_to_date`, at zero pulls (SC-005).
//! 3. A digest already resolved: served from [`super::ImageCache`], HEAD only.
//! 4. Otherwise `resolve` it by digest for Keel's platform (one pull), read the labels into
//!    [`ImageInfo`] and cache it.
//! 5. Classify ([`super::classify`]).
//!
//! The whole check is bounded at [`BOUND`]; the crate bounds each request at 10 s. Every check is a
//! `keel.update_check` span and feeds the `keel.update.*` instruments (R9). Nothing here records
//! the token, the password or an `Authorization` header: the crate never hands them out.

use std::time::{Duration, Instant};

use keel_protocol::UpdateOutcome;
use oci_registry::{Client, Digest, RegistryError, RequestCounts, Resolved};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::classify::{self, Classified};
use super::{ImageInfo, Updates};
use crate::build_info::BuildInfo;

/// The bound on one whole check (R1 step 6).
pub const BOUND: Duration = Duration::from_secs(30);

const LABEL_VERSION: &str = "org.opencontainers.image.version";
const LABEL_REVISION: &str = "org.opencontainers.image.revision";
const LABEL_CREATED: &str = "org.opencontainers.image.created";
const LABEL_SOURCE: &str = "org.opencontainers.image.source";

/// What started a check, as the span's `keel.update.trigger`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Scheduled,
    Manual,
}

impl Trigger {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Manual => "manual",
        }
    }
}

/// A successful check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    pub classified: Classified,
    /// The watched tag's image; `None` only for `up_to_date`.
    pub available: Option<ImageInfo>,
}

/// One check's result and the requests it made, failures included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The error is `RegistryError::code()`, or `timeout` for the whole-check bound.
    pub result: Result<Checked, &'static str>,
    pub requests: RequestCounts,
}

impl Report {
    /// 1 when the check GET a manifest (a Docker Hub pull), else 0 (R1 step 7).
    #[must_use]
    pub fn pulls(&self) -> u32 {
        u32::from(self.requests.manifest_get > 0)
    }
}

/// Runs one check in its `keel.update_check` span, records its metrics, and returns its report.
pub async fn run(updates: &Updates, client: &Client, trigger: Trigger) -> Report {
    let span = crate::telemetry::update_check_span(trigger.as_str(), &updates.config.image_name);
    let started = Instant::now();
    let report = {
        let _entered = span.enter();
        tracing::debug!("update check started");
        drop(_entered);
        run_bounded(updates, client, BOUND).await
    };
    let outcome = match &report.result {
        Ok(c) => outcome_str(c.classified.outcome),
        Err(_) => "failed",
    };
    span.record("keel.update.outcome", outcome);
    span.record("keel.update.requests.head", report.requests.head);
    span.record("keel.update.requests.get", report.requests.get);
    span.record("keel.update.pulls", report.pulls());
    if let Some(status) = status_code(&report.result) {
        span.record("http.response.status_code", status);
    }
    match &report.result {
        Ok(c) => {
            let _entered = span.enter();
            tracing::info!(
                outcome,
                reason = c.classified.cant_tell_reason.unwrap_or(""),
                "update check done"
            );
        }
        Err(code) => {
            span.record("error.type", *code);
            let _entered = span.enter();
            tracing::warn!(error = *code, "update check failed");
        }
    }
    crate::telemetry::record_update_check(
        outcome,
        started.elapsed(),
        report.requests.head,
        report.requests.get,
        report.pulls(),
    );
    report
}

/// One check bounded at `bound`. Requests made before the bound hit are still counted.
pub async fn run_bounded(updates: &Updates, client: &Client, bound: Duration) -> Report {
    let mut requests = RequestCounts::default();
    let result = match tokio::time::timeout(bound, check(updates, client, &mut requests)).await {
        Ok(Ok(checked)) => Ok(checked),
        Ok(Err(e)) => Err(e.code()),
        Err(_) => Err(RegistryError::Timeout.code()),
    };
    Report { result, requests }
}

fn add(total: &mut RequestCounts, more: RequestCounts) {
    total.head += more.head;
    total.get += more.get;
    total.manifest_get += more.manifest_get;
}

async fn check(
    updates: &Updates,
    client: &Client,
    requests: &mut RequestCounts,
) -> Result<Checked, RegistryError> {
    let config = &updates.config;
    let head = client.head_digest(&config.image, &config.credentials).await;
    add(requests, head.requests);
    let digest = head.result?;

    let build = &updates.build;
    if build.digest.as_deref() == Some(digest.as_str()) {
        return Ok(Checked {
            classified: classify::classify(build, &bare(digest)),
            available: None,
        });
    }

    let cached = updates.cache().get(&digest).cloned();
    let info = if let Some(info) = cached {
        info
    } else {
        let resolved = client
            .resolve(
                &config.image.with_digest(digest.clone()),
                &build.platform,
                &config.credentials,
            )
            .await;
        add(requests, resolved.requests);
        match resolved.result {
            Ok(r) => {
                let info = image_info(build, digest, &r);
                updates.cache().insert(info.clone());
                info
            }
            Err(RegistryError::NoImageForPlatform) => {
                return Ok(Checked {
                    classified: Classified::cant_tell(classify::NO_IMAGE_FOR_PLATFORM),
                    available: Some(bare(digest)),
                });
            }
            Err(e) => return Err(e),
        }
    };
    let classified = classify::classify(build, &info);
    let available = (classified.outcome != UpdateOutcome::UpToDate).then_some(info);
    Ok(Checked {
        classified,
        available,
    })
}

/// An image known only by its digest.
fn bare(digest: Digest) -> ImageInfo {
    ImageInfo {
        digest,
        version: None,
        revision: None,
        built_at: None,
        release_notes: None,
        release_notes_note: None,
    }
}

/// [`ImageInfo`] from the image's labels (data-model.md): `version`, `revision`, and `created`
/// with the config's `created` as the fallback date; the release-notes link per R7. Empty or
/// unparsable values are `None`.
#[must_use]
pub fn image_info(build: &BuildInfo, digest: Digest, r: &Resolved) -> ImageInfo {
    let label = |k: &str| {
        r.labels
            .get(k)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    let date = |s: Option<String>| s.and_then(|s| OffsetDateTime::parse(s.trim(), &Rfc3339).ok());
    let version = label(LABEL_VERSION);
    let built_at = date(label(LABEL_CREATED)).or_else(|| date(r.created.clone()));
    let (release_notes, release_notes_note) = classify::release_notes(
        build.source.as_ref(),
        version.as_deref(),
        label(LABEL_SOURCE).as_deref(),
    );
    ImageInfo {
        digest,
        version,
        revision: label(LABEL_REVISION),
        built_at,
        release_notes,
        release_notes_note,
    }
}

/// The outcome's wire name, for spans and metrics.
#[must_use]
pub const fn outcome_str(o: UpdateOutcome) -> &'static str {
    match o {
        UpdateOutcome::UpToDate => "up_to_date",
        UpdateOutcome::UpdateAvailable => "update_available",
        UpdateOutcome::OlderBuild => "older_build",
        UpdateOutcome::Rebuilt => "rebuilt",
        UpdateOutcome::CantTell => "cant_tell",
    }
}

/// `http.response.status_code` where the result implies it. The crate reports error codes, not
/// statuses, so only the unambiguous ones are recorded.
fn status_code(result: &Result<Checked, &'static str>) -> Option<u16> {
    match result {
        Ok(_) => Some(200),
        Err("rate_limited") => Some(429),
        Err("not_found") => Some(404),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::UpdateConfig;
    use oci_registry::Platform;
    use oci_registry::testing::{FakeRegistry, ImageSpec, LoggedRequest};
    use url::Url;

    const REPO: &str = "mbround18/keel";
    const SOURCE: &str = "https://github.com/mbround18/fathom";
    const OLD: &str = "2026-09-01T12:00:00Z";
    const NEW: &str = "2026-10-01T12:00:00Z";

    fn amd64() -> Platform {
        Platform {
            os: "linux".into(),
            architecture: "amd64".into(),
            variant: None,
        }
    }

    fn build(digest: Option<&Digest>, revision: &str, date: &str) -> BuildInfo {
        BuildInfo {
            version: Some("0.2.0".into()),
            revision: Some(revision.into()),
            built_at: Some(OffsetDateTime::parse(date, &Rfc3339).unwrap()),
            source: Some(Url::parse(SOURCE).unwrap()),
            digest: digest.map(|d| d.as_str().to_owned()),
            platform: amd64(),
        }
    }

    fn updates(fake: &FakeRegistry, build: BuildInfo) -> (Updates, Client) {
        let host = fake.url();
        let image = format!(
            "{}:{}/{REPO}:latest",
            host.host_str().unwrap(),
            host.port().unwrap()
        );
        let config = UpdateConfig::watching(true, Duration::from_secs(6 * 3600), &image).unwrap();
        let client = Client::with_base_url("keel/test (update-check)", fake.url());
        (Updates::new(build, config), client)
    }

    fn labels(version: &str, revision: &str, created: &str) -> Vec<(String, String)> {
        vec![
            (LABEL_VERSION.into(), version.into()),
            (LABEL_REVISION.into(), revision.into()),
            (LABEL_CREATED.into(), created.into()),
            (LABEL_SOURCE.into(), SOURCE.into()),
        ]
    }

    fn spec(labels: &[(String, String)]) -> ImageSpec {
        labels
            .iter()
            .fold(ImageSpec::new("linux", "amd64"), |s, (k, v)| s.label(k, v))
    }

    fn manifest_gets(log: &[LoggedRequest]) -> usize {
        log.iter()
            .filter(|r| r.method == "GET" && r.path.contains("/manifests/"))
            .count()
    }

    fn only_heads_and_tokens(log: &[LoggedRequest]) -> bool {
        log.iter()
            .all(|r| r.method == "HEAD" || (r.method == "GET" && r.path.starts_with("/token")))
    }

    #[tokio::test]
    async fn the_running_digest_is_up_to_date_with_heads_only() {
        let fake = FakeRegistry::start().await;
        let running = fake.image(REPO, "latest", &spec(&labels("0.2.0", "abc1234", OLD)));
        let (u, client) = updates(&fake, build(Some(&running.manifest), "abc1234", OLD));

        let report = run_bounded(&u, &client, BOUND).await;
        let checked = report.result.clone().unwrap();
        assert_eq!(checked.classified.outcome, UpdateOutcome::UpToDate);
        assert_eq!(checked.available, None);
        assert_eq!(report.requests.manifest_get, 0);
        assert_eq!(report.pulls(), 0);
        let log = fake.requests();
        assert!(only_heads_and_tokens(&log), "{log:?}");
        assert_eq!(manifest_gets(&log), 0);
        assert!(u.cache().is_empty());
    }

    #[tokio::test]
    async fn a_new_digest_resolves_once_then_comes_from_the_cache() {
        let fake = FakeRegistry::start().await;
        let running = digest_of_char('a');
        let new = fake.image(REPO, "latest", &spec(&labels("0.3.0", "def5678", NEW)));
        let (u, client) = updates(&fake, build(Some(&running), "abc1234", OLD));

        let report = run_bounded(&u, &client, BOUND).await;
        assert_eq!(report.pulls(), 1);
        assert_eq!(report.requests.manifest_get, 1);
        let checked = report.result.clone().unwrap();
        assert_eq!(checked.classified.outcome, UpdateOutcome::UpdateAvailable);
        let info = checked.available.unwrap();
        assert_eq!(info.digest, new.manifest);
        assert_eq!(info.version.as_deref(), Some("0.3.0"));
        assert_eq!(info.revision.as_deref(), Some("def5678"));
        assert_eq!(
            info.built_at,
            Some(OffsetDateTime::parse(NEW, &Rfc3339).unwrap())
        );
        assert_eq!(
            info.release_notes.as_ref().map(Url::as_str),
            Some("https://github.com/mbround18/fathom/releases/tag/keel-v0.3.0")
        );
        assert_eq!(info.release_notes_note, None);
        assert_eq!(manifest_gets(&fake.requests()), 1);
        assert_eq!(u.cache().len(), 1);

        // The same digest again: HEAD only.
        fake.clear_requests();
        let report = run_bounded(&u, &client, BOUND).await;
        assert_eq!(report.pulls(), 0);
        assert_eq!(
            report.result.unwrap().classified.outcome,
            UpdateOutcome::UpdateAvailable
        );
        let log = fake.requests();
        assert!(only_heads_and_tokens(&log), "{log:?}");
        assert_eq!(report.requests.get, 0, "{log:?}");
    }

    #[tokio::test]
    async fn a_multi_arch_index_picks_keels_platform() {
        let fake = FakeRegistry::start().await;
        let pairs = labels("0.3.0", "def5678", NEW);
        let refs: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let index = fake.multi_arch(REPO, "latest", &refs);
        let (u, client) = updates(&fake, build(Some(&digest_of_char('a')), "abc1234", OLD));

        let report = run_bounded(&u, &client, BOUND).await;
        assert_eq!(report.requests.manifest_get, 2);
        assert_eq!(report.pulls(), 1);
        let checked = report.result.clone().unwrap();
        assert_eq!(checked.classified.outcome, UpdateOutcome::UpdateAvailable);
        // The tag's digest is the index's: what the role pins (R3).
        assert_eq!(checked.available.unwrap().digest, index.index);
    }

    #[tokio::test]
    async fn the_cache_holds_sixteen_images_and_evicts_the_oldest() {
        let fake = FakeRegistry::start().await;
        let (u, client) = updates(&fake, build(Some(&digest_of_char('a')), "abc1234", OLD));
        let mut seen = Vec::new();
        for n in 0..=super::super::ImageCache::CAP {
            let image = fake.image(
                REPO,
                "latest",
                &spec(&labels("0.3.0", "def5678", NEW)).label("n", &n.to_string()),
            );
            let report = run_bounded(&u, &client, BOUND).await;
            assert_eq!(report.pulls(), 1, "image {n}");
            seen.push(image.manifest);
        }
        assert_eq!(u.cache().len(), super::super::ImageCache::CAP);
        assert!(u.cache().get(&seen[0]).is_none());
        assert!(u.cache().get(&seen[1]).is_some());

        // The evicted first image costs a pull again; the latest doesn't.
        let first = seen[0].clone();
        fake.tag(REPO, "latest", &first);
        assert_eq!(run_bounded(&u, &client, BOUND).await.pulls(), 1);
        fake.tag(REPO, "latest", &seen[seen.len() - 1]);
        assert_eq!(run_bounded(&u, &client, BOUND).await.pulls(), 0);
    }

    #[tokio::test]
    async fn an_unknown_running_digest_compares_by_revision_and_date() {
        let fake = FakeRegistry::start().await;
        fake.image(REPO, "latest", &spec(&labels("0.2.0", "abc1234", OLD)));
        let (u, client) = updates(&fake, build(None, "abc1234", OLD));
        assert!(!u.status().digest_compare);

        let report = run_bounded(&u, &client, BOUND).await;
        // No digest to compare, so the image is resolved and classified by its labels.
        assert_eq!(report.pulls(), 1);
        let checked = report.result.clone().unwrap();
        assert_eq!(checked.classified.outcome, UpdateOutcome::UpToDate);
        assert_eq!(checked.available, None);

        let newer = fake.image(REPO, "latest", &spec(&labels("0.3.0", "def5678", NEW)));
        let checked = run_bounded(&u, &client, BOUND).await.result.unwrap();
        assert_eq!(checked.classified.outcome, UpdateOutcome::UpdateAvailable);
        assert_eq!(checked.available.unwrap().digest, newer.manifest);
    }

    #[tokio::test]
    async fn older_rebuilt_and_missing_labels() {
        let fake = FakeRegistry::start().await;
        let (u, client) = updates(&fake, build(Some(&digest_of_char('a')), "abc1234", NEW));

        fake.image(REPO, "latest", &spec(&labels("0.1.0", "0000aaa", OLD)));
        let c = run_bounded(&u, &client, BOUND).await.result.unwrap();
        assert_eq!(c.classified.outcome, UpdateOutcome::OlderBuild);

        fake.image(REPO, "latest", &spec(&labels("0.2.0", "abc1234", OLD)));
        let c = run_bounded(&u, &client, BOUND).await.result.unwrap();
        assert_eq!(c.classified.outcome, UpdateOutcome::Rebuilt);
        assert!(c.available.is_some());

        fake.image(REPO, "latest", &ImageSpec::new("linux", "amd64"));
        let c = run_bounded(&u, &client, BOUND).await.result.unwrap();
        assert_eq!(
            c.classified,
            Classified::cant_tell(classify::MISSING_LABELS)
        );
        let info = c.available.unwrap();
        assert_eq!(info.release_notes, None);
        assert_eq!(
            info.release_notes_note.as_deref(),
            Some(classify::NOTE_UNTAGGED)
        );
    }

    #[tokio::test]
    async fn the_config_date_is_the_fallback_build_date() {
        let fake = FakeRegistry::start().await;
        let (u, client) = updates(&fake, build(Some(&digest_of_char('a')), "abc1234", OLD));
        fake.image(
            REPO,
            "latest",
            &ImageSpec::new("linux", "amd64")
                .label(LABEL_REVISION, "def5678")
                .created(NEW),
        );
        let c = run_bounded(&u, &client, BOUND).await.result.unwrap();
        assert_eq!(c.classified.outcome, UpdateOutcome::UpdateAvailable);
        assert_eq!(
            c.available.unwrap().built_at,
            Some(OffsetDateTime::parse(NEW, &Rfc3339).unwrap())
        );
    }

    #[tokio::test]
    async fn an_index_without_keels_platform_cant_tell() {
        let fake = FakeRegistry::start().await;
        let index = fake.index_without_amd64(REPO, "latest");
        let (u, client) = updates(&fake, build(Some(&digest_of_char('a')), "abc1234", OLD));
        let report = run_bounded(&u, &client, BOUND).await;
        let c = report.result.clone().unwrap();
        assert_eq!(
            c.classified,
            Classified::cant_tell(classify::NO_IMAGE_FOR_PLATFORM)
        );
        assert_eq!(c.available.unwrap().digest, index);
    }

    #[tokio::test]
    async fn rate_limited_is_an_error_not_an_update() {
        let fake = FakeRegistry::start().await;
        fake.image(REPO, "latest", &spec(&labels("0.3.0", "def5678", NEW)));
        fake.set_status_with_headers(
            &format!("/v2/{REPO}/manifests/latest"),
            429,
            &[("Retry-After", "120")],
        );
        let (u, client) = updates(&fake, build(Some(&digest_of_char('a')), "abc1234", OLD));
        let report = run_bounded(&u, &client, BOUND).await;
        assert_eq!(report.result, Err("rate_limited"));
        assert!(report.requests.head >= 1);
        assert_eq!(report.pulls(), 0);
    }

    #[tokio::test]
    async fn a_head_without_the_digest_header_never_gets() {
        let fake = FakeRegistry::start().await;
        fake.image(REPO, "latest", &spec(&labels("0.3.0", "def5678", NEW)));
        fake.omit_digest_header(true);
        let (u, client) = updates(&fake, build(Some(&digest_of_char('a')), "abc1234", OLD));
        let report = run_bounded(&u, &client, BOUND).await;
        assert_eq!(report.result, Err("no_digest_header"));
        assert_eq!(report.requests.manifest_get, 0);
        assert_eq!(manifest_gets(&fake.requests()), 0);
    }

    #[tokio::test]
    async fn the_whole_check_is_bounded() {
        assert_eq!(BOUND, Duration::from_secs(30));
        // A registry that accepts and never answers.
        let silent = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = silent.local_addr().unwrap();
        let hold = tokio::spawn(async move {
            let mut held = Vec::new();
            loop {
                if let Ok((s, _)) = silent.accept().await {
                    held.push(s);
                }
            }
        });
        let base = Url::parse(&format!("http://{addr}/")).unwrap();
        let config = UpdateConfig::watching(
            true,
            Duration::from_secs(3600),
            &format!("127.0.0.1:{}/{REPO}", addr.port()),
        )
        .unwrap();
        let u = Updates::new(build(None, "abc1234", OLD), config);
        let client = Client::with_base_url("keel/test", base);
        let started = Instant::now();
        let report = run_bounded(&u, &client, Duration::from_millis(300)).await;
        assert_eq!(report.result, Err("timeout"));
        assert!(started.elapsed() < Duration::from_secs(5));
        hold.abort();
    }

    #[tokio::test]
    async fn run_records_and_returns_the_report() {
        let fake = FakeRegistry::start().await;
        let running = fake.image(REPO, "latest", &spec(&labels("0.2.0", "abc1234", OLD)));
        let (u, client) = updates(&fake, build(Some(&running.manifest), "abc1234", OLD));
        let report = run(&u, &client, Trigger::Manual).await;
        assert_eq!(
            report.result.unwrap().classified.outcome,
            UpdateOutcome::UpToDate
        );
    }

    fn digest_of_char(c: char) -> Digest {
        Digest::parse(&format!("sha256:{}", c.to_string().repeat(64))).unwrap()
    }
}
