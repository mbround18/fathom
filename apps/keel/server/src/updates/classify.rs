//! Classification of the watched image against the running build (research R1 step 5) and the
//! release-notes link (R7). Pure functions: no registry, no clock.

use keel_protocol::UpdateOutcome;
use url::Url;

use super::ImageInfo;
use crate::build_info::BuildInfo;

/// `cant_tell`: the running build lacks a revision or a build date.
pub const RUNNING_UNKNOWN: &str = "running_unknown";
/// `cant_tell`: the remote image lacks a revision or a build date label.
pub const MISSING_LABELS: &str = "missing_labels";
/// `cant_tell`: no image for Keel's platform in the remote index.
pub const NO_IMAGE_FOR_PLATFORM: &str = "no_image_for_platform";
/// `cant_tell`: different revisions built at the same instant, so neither date is decisive. Not
/// in R1's table; the conservative answer.
pub const SAME_BUILD_DATE: &str = "same_build_date";

/// R7: why there's no release-notes link.
pub const NOTE_UNTAGGED: &str = "No release notes for untagged builds";
pub const NOTE_SOURCE_DIFFERS: &str =
    "No release notes link: the image's source differs from this build's";
pub const NOTE_NOT_A_RELEASE: &str =
    "No release notes link: the image's version isn't a release version";

/// The outcome of one check, and the reason for `CantTell`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Classified {
    pub outcome: UpdateOutcome,
    pub cant_tell_reason: Option<&'static str>,
}

impl Classified {
    const fn is(outcome: UpdateOutcome) -> Self {
        Self {
            outcome,
            cant_tell_reason: None,
        }
    }

    #[must_use]
    pub const fn cant_tell(reason: &'static str) -> Self {
        Self {
            outcome: UpdateOutcome::CantTell,
            cant_tell_reason: Some(reason),
        }
    }
}

/// Classifies `remote` against the running build.
///
/// The same digest is always `up_to_date` (R1 step 2). Otherwise, by revision and build date:
///
/// | Remote vs running | Outcome |
/// |---|---|
/// | Same revision, same build date | `up_to_date` (another architecture was rebuilt) |
/// | Same revision, different build date | `rebuilt` |
/// | Remote build date older | `older_build` |
/// | Remote build date newer | `update_available` |
/// | Either side missing revision or date | `cant_tell` |
#[must_use]
pub fn classify(running: &BuildInfo, remote: &ImageInfo) -> Classified {
    if running.digest.as_deref() == Some(remote.digest.as_str()) {
        return Classified::is(UpdateOutcome::UpToDate);
    }
    let (Some(rev), Some(date)) = (running.revision.as_deref(), running.built_at) else {
        return Classified::cant_tell(RUNNING_UNKNOWN);
    };
    let (Some(remote_rev), Some(remote_date)) = (remote.revision.as_deref(), remote.built_at)
    else {
        return Classified::cant_tell(MISSING_LABELS);
    };
    if rev == remote_rev {
        if date == remote_date {
            Classified::is(UpdateOutcome::UpToDate)
        } else {
            Classified::is(UpdateOutcome::Rebuilt)
        }
    } else if remote_date < date {
        Classified::is(UpdateOutcome::OlderBuild)
    } else if remote_date > date {
        Classified::is(UpdateOutcome::UpdateAvailable)
    } else {
        Classified::cant_tell(SAME_BUILD_DATE)
    }
}

/// R7: `<source>/releases/tag/keel-v<version>` when the image has a version, its source label is
/// an `https` URL, and that source equals the running build's `KEEL_BUILD_SOURCE`. Otherwise no
/// link, and the note saying why. Exactly one of the two is `Some`.
#[must_use]
pub fn release_notes(
    running_source: Option<&Url>,
    version: Option<&str>,
    image_source: Option<&str>,
) -> (Option<Url>, Option<String>) {
    let note = |n: &str| (None, Some(n.to_owned()));
    let Some(version) = version.map(str::trim).filter(|v| !v.is_empty()) else {
        return note(NOTE_UNTAGGED);
    };
    let image_source = image_source.and_then(|s| Url::parse(s.trim()).ok());
    let (Some(running), Some(image)) = (running_source, image_source) else {
        return note(NOTE_SOURCE_DIFFERS);
    };
    if image.scheme() != "https" || !same_source(running, &image) {
        return note(NOTE_SOURCE_DIFFERS);
    }
    let release_like = version.len() <= 64
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    if !release_like {
        return note(NOTE_NOT_A_RELEASE);
    }
    let base = running.as_str().trim_end_matches('/');
    match Url::parse(&format!("{base}/releases/tag/keel-v{version}")) {
        Ok(url) => (Some(url), None),
        Err(_) => note(NOTE_NOT_A_RELEASE),
    }
}

/// Sources compare as URLs, ignoring one trailing slash.
fn same_source(a: &Url, b: &Url) -> bool {
    a.as_str().trim_end_matches('/') == b.as_str().trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    use super::*;
    use oci_registry::{Digest, Platform};
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;

    const SOURCE: &str = "https://github.com/mbround18/fathom";

    fn at(s: &str) -> OffsetDateTime {
        OffsetDateTime::parse(s, &Rfc3339).unwrap()
    }

    fn digest(c: char) -> String {
        format!("sha256:{}", c.to_string().repeat(64))
    }

    fn running(revision: Option<&str>, date: Option<&str>, digest: Option<String>) -> BuildInfo {
        BuildInfo {
            version: Some("0.2.0".into()),
            revision: revision.map(str::to_owned),
            built_at: date.map(at),
            source: Some(Url::parse(SOURCE).unwrap()),
            digest,
            platform: Platform {
                os: "linux".into(),
                architecture: "amd64".into(),
                variant: None,
            },
        }
    }

    fn remote(revision: Option<&str>, date: Option<&str>) -> ImageInfo {
        ImageInfo {
            digest: Digest::parse(&digest('b')).unwrap(),
            version: None,
            revision: revision.map(str::to_owned),
            built_at: date.map(at),
            release_notes: None,
            release_notes_note: None,
        }
    }

    const D1: &str = "2026-10-01T12:00:00Z";
    const D2: &str = "2026-10-20T12:00:00Z";

    #[test]
    fn the_same_digest_is_up_to_date_whatever_the_labels() {
        let r = running(None, None, Some(digest('b')));
        assert_eq!(
            classify(&r, &remote(None, None)),
            Classified::is(UpdateOutcome::UpToDate)
        );
    }

    #[test]
    fn every_row_of_r1_step_5() {
        let r = running(Some("abc1234"), Some(D1), Some(digest('a')));
        let cases = [
            // Same revision, same build date: another architecture was rebuilt.
            (
                remote(Some("abc1234"), Some(D1)),
                Classified::is(UpdateOutcome::UpToDate),
            ),
            // Same revision, different build date.
            (
                remote(Some("abc1234"), Some(D2)),
                Classified::is(UpdateOutcome::Rebuilt),
            ),
            (
                remote(Some("abc1234"), Some("2026-09-01T00:00:00Z")),
                Classified::is(UpdateOutcome::Rebuilt),
            ),
            // Remote build date older.
            (
                remote(Some("def5678"), Some("2026-09-01T00:00:00Z")),
                Classified::is(UpdateOutcome::OlderBuild),
            ),
            // Remote build date newer.
            (
                remote(Some("def5678"), Some(D2)),
                Classified::is(UpdateOutcome::UpdateAvailable),
            ),
            // Remote missing revision or date.
            (
                remote(None, Some(D2)),
                Classified::cant_tell(MISSING_LABELS),
            ),
            (
                remote(Some("def5678"), None),
                Classified::cant_tell(MISSING_LABELS),
            ),
            // Different revision, same instant.
            (
                remote(Some("def5678"), Some(D1)),
                Classified::cant_tell(SAME_BUILD_DATE),
            ),
        ];
        for (remote, want) in cases {
            assert_eq!(classify(&r, &remote), want, "{remote:?}");
        }
    }

    #[test]
    fn a_running_build_without_revision_or_date_cant_tell() {
        let newer = remote(Some("def5678"), Some(D2));
        for r in [
            running(None, Some(D1), Some(digest('a'))),
            running(Some("abc1234"), None, Some(digest('a'))),
            running(None, None, None),
        ] {
            assert_eq!(classify(&r, &newer), Classified::cant_tell(RUNNING_UNKNOWN));
        }
    }

    #[test]
    fn an_unknown_running_digest_compares_by_revision_and_date() {
        let r = running(Some("abc1234"), Some(D1), None);
        assert_eq!(
            classify(&r, &remote(Some("def5678"), Some(D2))),
            Classified::is(UpdateOutcome::UpdateAvailable)
        );
        assert_eq!(
            classify(&r, &remote(Some("abc1234"), Some(D1))),
            Classified::is(UpdateOutcome::UpToDate)
        );
    }

    #[test]
    fn release_notes_link_only_for_a_version_from_the_same_https_source() {
        let src = Url::parse(SOURCE).unwrap();
        let (url, note) = release_notes(Some(&src), Some("0.3.0"), Some(SOURCE));
        assert_eq!(
            url.unwrap().as_str(),
            "https://github.com/mbround18/fathom/releases/tag/keel-v0.3.0"
        );
        assert_eq!(note, None);
        // A trailing slash on either side is the same source.
        let (url, _) = release_notes(Some(&src), Some("0.3.0"), Some(&format!("{SOURCE}/")));
        assert!(url.is_some());
    }

    #[test]
    fn no_release_notes_without_a_version() {
        let src = Url::parse(SOURCE).unwrap();
        for v in [None, Some(""), Some("  ")] {
            let (url, note) = release_notes(Some(&src), v, Some(SOURCE));
            assert_eq!(url, None);
            assert_eq!(note.as_deref(), Some(NOTE_UNTAGGED));
        }
    }

    #[test]
    fn no_release_notes_when_the_source_differs_or_isnt_https() {
        let src = Url::parse(SOURCE).unwrap();
        for image_source in [
            None,
            Some("https://evil.example/mbround18/fathom"),
            Some("http://github.com/mbround18/fathom"),
            Some("not a url"),
            Some("https://github.com/mbround18/fathom/other"),
        ] {
            let (url, note) = release_notes(Some(&src), Some("0.3.0"), image_source);
            assert_eq!(url, None, "{image_source:?}");
            assert_eq!(
                note.as_deref(),
                Some(NOTE_SOURCE_DIFFERS),
                "{image_source:?}"
            );
        }
        // The running build has no source.
        let (url, note) = release_notes(None, Some("0.3.0"), Some(SOURCE));
        assert_eq!(url, None);
        assert_eq!(note.as_deref(), Some(NOTE_SOURCE_DIFFERS));
    }

    #[test]
    fn a_version_that_isnt_a_release_gets_no_link() {
        let src = Url::parse(SOURCE).unwrap();
        for v in ["0.3.0/../../x", "0.3.0?x=1", "0.3.0#x", "a b"] {
            let (url, note) = release_notes(Some(&src), Some(v), Some(SOURCE));
            assert_eq!(url, None, "{v}");
            assert_eq!(note.as_deref(), Some(NOTE_NOT_A_RELEASE), "{v}");
        }
    }
}
