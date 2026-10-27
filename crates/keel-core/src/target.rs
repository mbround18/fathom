//! Run targeting (spec 055 T069): which hosts of an inventory revision a run reaches, from the
//! job's `limit` and the run's own, in Ansible's `--limit` syntax, and which changed paths
//! trigger a job.
//!
//! Supported: terms separated by `,` or `:`; `all` and `*`; host or group names with `*` and `?`
//! wildcards; `&term` (intersection) and `!term` (exclusion). Regex (`~`) and slice (`[..]`)
//! terms are refused rather than guessed at: a limit that cannot be read must not widen a run.

use std::collections::BTreeSet;

use globset::{Glob, GlobMatcher};

/// A host as an inventory revision lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target<'a> {
    pub host: &'a str,
    pub groups: &'a [String],
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LimitError {
    #[error("empty limit")]
    Empty,
    #[error("unsupported limit term `{0}` (regex and slice patterns are not supported)")]
    Unsupported(String),
    #[error("invalid limit term `{0}`")]
    Invalid(String),
}

impl LimitError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        "limit_invalid"
    }
}

fn matcher(term: &str) -> Result<GlobMatcher, LimitError> {
    if term.starts_with('~') || term.contains('[') || term.contains(']') {
        return Err(LimitError::Unsupported(term.to_owned()));
    }
    if term.is_empty() || term.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(LimitError::Invalid(term.to_owned()));
    }
    Glob::new(term)
        .map(|g| g.compile_matcher())
        .map_err(|_| LimitError::Invalid(term.to_owned()))
}

fn hits<'a>(term: &str, hosts: &[Target<'a>]) -> Result<BTreeSet<&'a str>, LimitError> {
    if term == "all" || term == "*" {
        return Ok(hosts.iter().map(|h| h.host).collect());
    }
    let m = matcher(term)?;
    Ok(hosts
        .iter()
        .filter(|h| m.is_match(h.host) || h.groups.iter().any(|g| m.is_match(g)))
        .map(|h| h.host)
        .collect())
}

/// The hosts `limit` selects from `hosts`; `None` selects them all.
///
/// # Errors
///
/// The limit is empty or uses a term Keel does not support.
pub fn select<'a>(
    limit: Option<&str>,
    hosts: &[Target<'a>],
) -> Result<BTreeSet<&'a str>, LimitError> {
    let Some(limit) = limit else {
        return Ok(hosts.iter().map(|h| h.host).collect());
    };
    let terms: Vec<&str> = limit
        .split([',', ':'])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    if terms.is_empty() {
        return Err(LimitError::Empty);
    }
    let mut positive: Option<BTreeSet<&'a str>> = None;
    let mut and: Vec<BTreeSet<&'a str>> = Vec::new();
    let mut not: Vec<BTreeSet<&'a str>> = Vec::new();
    for t in terms {
        if let Some(rest) = t.strip_prefix('&') {
            and.push(hits(rest, hosts)?);
        } else if let Some(rest) = t.strip_prefix('!') {
            not.push(hits(rest, hosts)?);
        } else {
            positive
                .get_or_insert_with(BTreeSet::new)
                .extend(hits(t, hosts)?);
        }
    }
    // Only `&` or `!` terms: they narrow everything, as in Ansible.
    let mut out = positive.unwrap_or_else(|| hosts.iter().map(|h| h.host).collect());
    for a in and {
        out.retain(|h| a.contains(h));
    }
    for n in not {
        out.retain(|h| !n.contains(h));
    }
    Ok(out)
}

/// Whether any of `changed` matches any of the job's path globs. A glob that does not compile
/// matches nothing.
#[must_use]
pub fn paths_match<'a>(globs: &[String], changed: impl IntoIterator<Item = &'a String>) -> bool {
    let matchers: Vec<GlobMatcher> = globs
        .iter()
        .filter_map(|g| Glob::new(g).ok().map(|g| g.compile_matcher()))
        .collect();
    changed
        .into_iter()
        .any(|p| matchers.iter().any(|m| m.is_match(p)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inv() -> Vec<(String, Vec<String>)> {
        vec![
            ("web-1".into(), vec!["web".into(), "eu".into()]),
            ("web-2".into(), vec!["web".into(), "us".into()]),
            ("db-1".into(), vec!["db".into(), "eu".into()]),
        ]
    }

    fn sel(limit: Option<&str>) -> Result<Vec<String>, LimitError> {
        let owned = inv();
        let hosts: Vec<Target<'_>> = owned
            .iter()
            .map(|(h, g)| Target { host: h, groups: g })
            .collect();
        select(limit, &hosts).map(|s| s.into_iter().map(str::to_owned).collect())
    }

    #[test]
    fn no_limit_and_all_select_everything() {
        assert_eq!(sel(None).unwrap(), ["db-1", "web-1", "web-2"]);
        assert_eq!(sel(Some("all")).unwrap(), ["db-1", "web-1", "web-2"]);
        assert_eq!(sel(Some("*")).unwrap(), ["db-1", "web-1", "web-2"]);
    }

    #[test]
    fn names_groups_and_globs() {
        assert_eq!(sel(Some("web")).unwrap(), ["web-1", "web-2"]);
        assert_eq!(sel(Some("db-1,web-2")).unwrap(), ["db-1", "web-2"]);
        assert_eq!(sel(Some("web-*")).unwrap(), ["web-1", "web-2"]);
        assert_eq!(sel(Some("nothing")).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn intersection_and_exclusion() {
        assert_eq!(sel(Some("web:&eu")).unwrap(), ["web-1"]);
        assert_eq!(sel(Some("all:!web-2")).unwrap(), ["db-1", "web-1"]);
        assert_eq!(sel(Some("!db")).unwrap(), ["web-1", "web-2"]);
        assert_eq!(sel(Some("eu,!db-1")).unwrap(), ["web-1"]);
    }

    #[test]
    fn regex_slices_and_empty_limits_fail_closed() {
        assert!(matches!(
            sel(Some("~web.*")),
            Err(LimitError::Unsupported(_))
        ));
        assert!(matches!(
            sel(Some("web[0]")),
            Err(LimitError::Unsupported(_))
        ));
        assert_eq!(sel(Some(",")), Err(LimitError::Empty));
        assert!(matches!(sel(Some("we b")), Err(LimitError::Invalid(_))));
    }

    #[test]
    fn changed_paths_trigger_jobs() {
        let changed = ["playbooks/web.yaml".to_owned(), "README".to_owned()];
        assert!(paths_match(&["playbooks/**".into()], &changed));
        assert!(paths_match(&["README".into()], &changed));
        assert!(!paths_match(&["roles/**".into()], &changed));
        assert!(!paths_match(&[], &changed));
    }
}
