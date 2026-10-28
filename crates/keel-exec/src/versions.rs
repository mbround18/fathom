//! Version floors the runner checks on a host (spec 055 T087, research R3).
//!
//! `MIN_ANSIBLE_CORE` must match the floor in `apps/keel/runner/ansible/requirements.in`, whose
//! hash-pinned `requirements.txt` builds `/opt/keel-runner/venv`. A host whose venv reports an
//! older ansible-core is refused with `version_too_old`.

/// The oldest ansible-core the executor will run.
pub const MIN_ANSIBLE_CORE: &str = "2.21.0";

/// Whether `version` (as ansible-core reports it, e.g. `2.21.5` or `2.22.0rc1`) is at least
/// [`MIN_ANSIBLE_CORE`]. Fails closed: anything that does not parse as
/// `MAJOR.MINOR[.PATCH][suffix]` is `false`. A pre-release (`a`, `b`, `rc`, `.dev`) sorts below
/// the release it precedes, so `2.21.0rc1` does not meet a `2.21.0` floor.
pub fn meets_min_ansible_core(version: &str) -> bool {
    match (parse(version), parse(MIN_ANSIBLE_CORE)) {
        (Some(have), Some(min)) => have >= min,
        _ => false,
    }
}

/// `(major, minor, patch, is_final)`.
fn parse(version: &str) -> Option<(u64, u64, u64, bool)> {
    let (major, rest) = number(version.trim())?;
    let rest = rest.strip_prefix('.')?;
    let (minor, mut rest) = number(rest)?;
    let mut patch = 0;
    if let Some(after_dot) = rest.strip_prefix('.')
        && after_dot.starts_with(|c: char| c.is_ascii_digit())
    {
        (patch, rest) = number(after_dot)?;
    }
    let is_final = if rest.is_empty() || rest.starts_with(".post") || rest.starts_with('+') {
        true
    } else if ["a", "b", "rc", ".dev"].iter().any(|p| rest.starts_with(p)) {
        false
    } else {
        return None;
    };
    Some((major, minor, patch, is_final))
}

/// A leading run of ASCII digits, and whatever follows it.
fn number(s: &str) -> Option<(u64, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    Some((s[..end].parse().ok()?, &s[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_floor_itself_and_newer_releases_pass() {
        assert!(meets_min_ansible_core(MIN_ANSIBLE_CORE));
        assert!(meets_min_ansible_core("2.21.5"));
        assert!(meets_min_ansible_core("2.22.0"));
        assert!(meets_min_ansible_core("3.0"));
        assert!(meets_min_ansible_core("2.21.5.post1"));
    }

    #[test]
    fn older_releases_and_prereleases_of_the_floor_fail() {
        assert!(!meets_min_ansible_core("2.20.9"));
        assert!(!meets_min_ansible_core("2.19.14"));
        assert!(!meets_min_ansible_core("1.99.0"));
        assert!(!meets_min_ansible_core("2.21.0rc1"));
        assert!(!meets_min_ansible_core("2.21.0b2"));
    }

    #[test]
    fn unparseable_versions_fail_closed() {
        for v in [
            "",
            "unknown",
            "2",
            "two.21.0",
            "2.21.x",
            "2..0",
            "2.21.0-weird",
            " ",
        ] {
            assert!(!meets_min_ansible_core(v), "{v:?} must not pass");
        }
    }

    #[test]
    fn the_floor_matches_requirements_in() {
        let req = include_str!("../../../apps/keel/runner/ansible/requirements.in");
        let floor = req
            .lines()
            .find_map(|l| l.trim().strip_prefix("ansible-core>="))
            .expect("requirements.in pins an ansible-core floor");
        assert!(
            MIN_ANSIBLE_CORE.starts_with(floor.trim()),
            "{floor} vs {MIN_ANSIBLE_CORE}"
        );
    }
}
