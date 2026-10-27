//! The override challenge: what an approver's passkey signs (contract signed-mode.md).
//!
//! ```text
//! challenge = SHA-256( JCS({ "v": "keel-override-v1", "kind", "workspace", "binding", "run",
//!                            "job", "base", "target", "hosts", "expires_at" }) )
//! ```
//!
//! The server builds it to request an approval; the host rebuilds it from its own ledger base, the
//! target it verified and the run it was assigned, so an approval only fits the run, the commits
//! and the hosts it was given for.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::object::CommitId;

/// The `v` field of every challenge this crate builds.
pub const CHALLENGE_VERSION: &str = "keel-override-v1";

/// What an approval authorizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalKind {
    /// Commits that are unsigned, or a target that is not a descendant of the base.
    Override,
    /// A first run on a host, which has no base yet.
    Baseline,
}

/// Everything the challenge commits to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverrideScope {
    /// `override` or `baseline`; must be `baseline` exactly when `base` is `None`.
    pub kind: ApprovalKind,
    /// Workspace id (lowercase hyphenated UUID).
    pub workspace: String,
    /// Binding id (lowercase hyphenated UUID).
    pub binding: String,
    /// Run id (lowercase hyphenated UUID).
    pub run: String,
    /// Job name.
    pub job: String,
    /// The host's base for this job, `None` on a first run.
    pub base: Option<CommitId>,
    /// The commit to run.
    pub target: CommitId,
    /// Inventory names the approval covers: non-empty, sorted, no duplicates.
    pub hosts: Vec<String>,
    /// `YYYY-MM-DDTHH:MM:SSZ`.
    pub expires_at: String,
}

/// The scope cannot be turned into a challenge. Never "allow": the caller refuses.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScopeError {
    /// `workspace`, `binding` or `run` is not a lowercase hyphenated UUID.
    #[error("{0} is not a lowercase UUID")]
    NotUuid(&'static str),
    /// The job name is empty or has control characters.
    #[error("invalid job name")]
    Job,
    /// `kind` is `baseline` with a base, or `override` without one.
    #[error("kind does not match the base")]
    KindMismatch,
    /// `hosts` is empty, unsorted, has duplicates or empty names.
    #[error("hosts must be non-empty, sorted and unique")]
    Hosts,
    /// `expires_at` is not `YYYY-MM-DDTHH:MM:SSZ`.
    #[error("expires_at is not RFC 3339 UTC with seconds")]
    ExpiresAt,
}

#[derive(Serialize)]
struct Canonical<'a> {
    v: &'static str,
    kind: ApprovalKind,
    workspace: &'a str,
    binding: &'a str,
    run: &'a str,
    job: &'a str,
    base: Option<&'a str>,
    target: &'a str,
    hosts: &'a [String],
    expires_at: &'a str,
}

impl OverrideScope {
    /// Checks the scope: UUIDs, job, kind against base, hosts, timestamp.
    pub fn validate(&self) -> Result<(), ScopeError> {
        for (name, v) in [
            ("workspace", &self.workspace),
            ("binding", &self.binding),
            ("run", &self.run),
        ] {
            if !is_lower_uuid(v) {
                return Err(ScopeError::NotUuid(name));
            }
        }
        if self.job.is_empty() || self.job.chars().any(char::is_control) {
            return Err(ScopeError::Job);
        }
        if (self.kind == ApprovalKind::Baseline) != self.base.is_none() {
            return Err(ScopeError::KindMismatch);
        }
        if self.hosts.is_empty()
            || self.hosts.iter().any(String::is_empty)
            || self.hosts.windows(2).any(|w| w[0] >= w[1])
        {
            return Err(ScopeError::Hosts);
        }
        parse_utc(&self.expires_at).ok_or(ScopeError::ExpiresAt)?;
        Ok(())
    }

    /// The RFC 8785 canonical JSON the challenge hashes.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ScopeError> {
        self.validate()?;
        let value = Canonical {
            v: CHALLENGE_VERSION,
            kind: self.kind,
            workspace: &self.workspace,
            binding: &self.binding,
            run: &self.run,
            job: &self.job,
            base: self.base.as_ref().map(CommitId::as_str),
            target: self.target.as_str(),
            hosts: &self.hosts,
            expires_at: &self.expires_at,
        };
        // Serializing plain strings and an enum cannot fail.
        Ok(serde_jcs::to_vec(&value).expect("canonical JSON of strings"))
    }

    /// The 32-byte WebAuthn challenge.
    pub fn challenge(&self) -> Result<[u8; 32], ScopeError> {
        Ok(Sha256::digest(self.canonical_bytes()?).into())
    }
}

/// Sorts and deduplicates inventory names, as the challenge requires.
pub fn normalize_hosts<I, S>(hosts: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut v: Vec<String> = hosts.into_iter().map(Into::into).collect();
    v.sort();
    v.dedup();
    v
}

fn is_lower_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_digit() || (b'a'..=b'f').contains(c),
        })
}

/// Parses `YYYY-MM-DDTHH:MM:SSZ` (exactly that shape) into Unix seconds.
pub fn parse_utc(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
    {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let part = &s[r];
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())
            .flatten()
    };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, se) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || d < 1 || d > days_in_month(y, mo) || h > 23 || mi > 59 || se > 59
    {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se)
}

/// Formats Unix seconds as `YYYY-MM-DDTHH:MM:SSZ`. `None` outside years 0000 to 9999.
pub fn format_utc(secs: i64) -> Option<String> {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    if !(0..=9999).contains(&y) {
        return None;
    }
    Some(format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    ))
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

// Howard Hinnant's algorithms.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        if m <= 2 {
            yoe + era * 400 + 1
        } else {
            yoe + era * 400
        },
        m,
        d,
    )
}

#[cfg(test)]
mod tests {
    //! T090: the challenge is fixed bytes and a fixed hash for a fixed scope.
    use super::*;

    fn scope() -> OverrideScope {
        OverrideScope {
            kind: ApprovalKind::Override,
            workspace: "0b7f1c2e-3d4a-4b5c-8d6e-7f8091a2b3c4".into(),
            binding: "1c2d3e4f-5a6b-4c7d-8e9f-a0b1c2d3e4f5".into(),
            run: "2d3e4f50-6a7b-4c8d-9eaf-b0c1d2e3f405".into(),
            job: "web-deploy".into(),
            base: Some(CommitId::parse("1111111111111111111111111111111111111111").unwrap()),
            target: CommitId::parse("2222222222222222222222222222222222222222").unwrap(),
            hosts: normalize_hosts(["web-2", "web-1", "web-2"]),
            expires_at: "2026-01-01T12:00:00Z".into(),
        }
    }

    #[test]
    fn canonical_json_is_fixed() {
        let bytes = scope().canonical_bytes().unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            r#"{"base":"1111111111111111111111111111111111111111","binding":"1c2d3e4f-5a6b-4c7d-8e9f-a0b1c2d3e4f5","expires_at":"2026-01-01T12:00:00Z","hosts":["web-1","web-2"],"job":"web-deploy","kind":"override","run":"2d3e4f50-6a7b-4c8d-9eaf-b0c1d2e3f405","target":"2222222222222222222222222222222222222222","v":"keel-override-v1","workspace":"0b7f1c2e-3d4a-4b5c-8d6e-7f8091a2b3c4"}"#
        );
    }

    #[test]
    fn challenge_is_fixed() {
        let hex: String = scope()
            .challenge()
            .unwrap()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            hex,
            "7a07a5c015b504fffa849304a8df2e4e80f649092ddabf5ad0b644f714024cc2"
        );
    }

    #[test]
    fn baseline_has_null_base() {
        let mut s = scope();
        s.kind = ApprovalKind::Baseline;
        s.base = None;
        let text = String::from_utf8(s.canonical_bytes().unwrap()).unwrap();
        assert!(text.starts_with(r#"{"base":null,"#), "{text}");
        assert!(text.contains(r#""kind":"baseline""#));
    }

    #[test]
    fn every_field_changes_the_challenge() {
        let base = scope().challenge().unwrap();
        let mut variants = Vec::new();
        let mut s = scope();
        s.workspace = "0b7f1c2e-3d4a-4b5c-8d6e-7f8091a2b3c5".into();
        variants.push(s);
        let mut s = scope();
        s.binding = "1c2d3e4f-5a6b-4c7d-8e9f-a0b1c2d3e4f6".into();
        variants.push(s);
        let mut s = scope();
        s.run = "2d3e4f50-6a7b-4c8d-9eaf-b0c1d2e3f406".into();
        variants.push(s);
        let mut s = scope();
        s.job = "web-deploy2".into();
        variants.push(s);
        let mut s = scope();
        s.base = Some(CommitId::parse("3333333333333333333333333333333333333333").unwrap());
        variants.push(s);
        let mut s = scope();
        s.target = CommitId::parse("3333333333333333333333333333333333333333").unwrap();
        variants.push(s);
        let mut s = scope();
        s.hosts = vec!["web-1".into()];
        variants.push(s);
        let mut s = scope();
        s.expires_at = "2026-01-01T12:00:01Z".into();
        variants.push(s);
        for v in variants {
            assert_ne!(v.challenge().unwrap(), base, "{v:?}");
        }
    }

    #[test]
    fn invalid_scopes_are_refused() {
        let mut s = scope();
        s.run = s.run.to_uppercase();
        assert_eq!(s.challenge(), Err(ScopeError::NotUuid("run")));
        let mut s = scope();
        s.base = None;
        assert_eq!(s.challenge(), Err(ScopeError::KindMismatch));
        let mut s = scope();
        s.kind = ApprovalKind::Baseline;
        assert_eq!(s.challenge(), Err(ScopeError::KindMismatch));
        let mut s = scope();
        s.hosts = vec!["web-2".into(), "web-1".into()];
        assert_eq!(s.challenge(), Err(ScopeError::Hosts));
        let mut s = scope();
        s.hosts = vec![];
        assert_eq!(s.challenge(), Err(ScopeError::Hosts));
        let mut s = scope();
        s.job = String::new();
        assert_eq!(s.challenge(), Err(ScopeError::Job));
        for bad in [
            "2026-01-01T12:00:00+00:00",
            "2026-01-01T12:00:00.5Z",
            "2026-02-30T00:00:00Z",
            "2026-01-01 12:00:00Z",
        ] {
            let mut s = scope();
            s.expires_at = bad.into();
            assert_eq!(s.challenge(), Err(ScopeError::ExpiresAt), "{bad}");
        }
    }

    #[test]
    fn utc_round_trips() {
        assert_eq!(parse_utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_utc("2026-01-01T00:00:00Z"), Some(1_767_225_600));
        assert_eq!(parse_utc("2024-02-29T23:59:59Z"), Some(1_709_251_199));
        for t in [0, 1_767_225_600, 1_709_251_199, 951_782_400] {
            assert_eq!(parse_utc(&format_utc(t).unwrap()), Some(t));
        }
    }
}
