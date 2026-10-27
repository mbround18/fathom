//! Approval policy resolution (spec 055 T060/T066, data-model `approval_policies`).
//!
//! Every workspace has a catch-all at the lowest precedence (priority [`CATCH_ALL_PRIORITY`])
//! that requires one approval and cannot be removed. Other policies match a run by job name
//! glob, inventory name and limit glob; among the policies that match, the lowest `priority`
//! wins (ties broken by id, so the answer never depends on row order). [`resolve`] also explains
//! itself: every policy it looked at, whether it matched, and why.
//!
//! Resolution fails closed: a workspace whose catch-all is somehow missing resolves to a
//! synthetic catch-all, so a run never skips approval because a row was lost.

use globset::Glob;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The catch-all's priority: the lowest precedence there is.
pub const CATCH_ALL_PRIORITY: i32 = i32::MAX;

/// The longest approval TTL a policy may ask for: 30 days.
pub const MAX_APPROVAL_TTL_SECS: u32 = 30 * 24 * 3600;

/// The catch-all's TTL: 24 hours.
pub const CATCH_ALL_TTL_SECS: u32 = 24 * 3600;

/// What a policy applies to. An absent field matches anything.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyMatch {
    /// A glob over the job name.
    #[serde(default)]
    pub job: Option<String>,
    /// An inventory name, exactly.
    #[serde(default)]
    pub inventory: Option<String>,
    /// A glob over the run's `limit`. A run with no limit is matched as the empty string, so
    /// `*` matches it and `web*` does not.
    #[serde(default)]
    pub limit: Option<String>,
}

/// One approval policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub id: Uuid,
    pub priority: i32,
    pub matcher: PolicyMatch,
    pub requires_approval: bool,
    /// At least 1.
    pub approvals_needed: u16,
    /// The requester cannot approve their own run.
    pub four_eyes: bool,
    pub approval_ttl_secs: u32,
}

impl Policy {
    /// The workspace catch-all: everything needs one approval.
    #[must_use]
    pub fn catch_all(id: Uuid) -> Self {
        Self {
            id,
            priority: CATCH_ALL_PRIORITY,
            matcher: PolicyMatch::default(),
            requires_approval: true,
            approvals_needed: 1,
            four_eyes: false,
            approval_ttl_secs: CATCH_ALL_TTL_SECS,
        }
    }

    #[must_use]
    pub const fn is_catch_all(&self) -> bool {
        self.priority == CATCH_ALL_PRIORITY
    }
}

/// Why a policy write is refused. Each is an `OpError` code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error("the catch-all policy cannot be removed")]
    CatchAllPermanent,
    #[error("priority {0} is reserved for the catch-all")]
    PriorityReserved(i32),
    #[error("approvals_needed must be at least 1")]
    ApprovalsNeeded,
    #[error("the approval TTL must be between 1 second and 30 days")]
    Ttl,
    #[error("invalid {field} glob")]
    BadGlob { field: &'static str },
    #[error("the catch-all must require approval and match everything")]
    CatchAllWeakened,
}

impl PolicyError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::CatchAllPermanent => "catch_all_permanent",
            Self::PriorityReserved(_) => "priority_reserved",
            Self::ApprovalsNeeded => "approvals_needed",
            Self::Ttl => "approval_ttl",
            Self::BadGlob { .. } => "bad_glob",
            Self::CatchAllWeakened => "catch_all_weakened",
        }
    }
}

/// Checks a policy before it is written. `existing` is the stored version when updating.
///
/// # Errors
///
/// The policy is malformed, takes the catch-all's priority, or weakens the catch-all.
pub fn validate(p: &Policy, existing: Option<&Policy>) -> Result<(), PolicyError> {
    if p.approvals_needed == 0 {
        return Err(PolicyError::ApprovalsNeeded);
    }
    if p.approval_ttl_secs == 0 || p.approval_ttl_secs > MAX_APPROVAL_TTL_SECS {
        return Err(PolicyError::Ttl);
    }
    for (field, glob) in [("job", &p.matcher.job), ("limit", &p.matcher.limit)] {
        if let Some(g) = glob
            && (g.is_empty() || g.len() > 256 || Glob::new(g).is_err())
        {
            return Err(PolicyError::BadGlob { field });
        }
    }
    let was_catch_all = existing.is_some_and(Policy::is_catch_all);
    if was_catch_all {
        // The catch-all may change its quorum, four-eyes and TTL, never what it covers.
        if !p.is_catch_all() || !p.requires_approval || p.matcher != PolicyMatch::default() {
            return Err(PolicyError::CatchAllWeakened);
        }
    } else if p.is_catch_all() {
        return Err(PolicyError::PriorityReserved(p.priority));
    }
    Ok(())
}

/// Checks that `p` may be removed.
///
/// # Errors
///
/// `p` is the catch-all.
pub fn check_remove(p: &Policy) -> Result<(), PolicyError> {
    if p.is_catch_all() {
        Err(PolicyError::CatchAllPermanent)
    } else {
        Ok(())
    }
}

/// The run a policy is resolved for.
#[derive(Debug, Clone, Copy)]
pub struct Subject<'a> {
    pub job: &'a str,
    pub inventory: &'a str,
    pub limit: Option<&'a str>,
}

/// One policy, as [`resolve`] judged it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub policy: Uuid,
    pub priority: i32,
    pub matched: bool,
    /// Why it matched or not, for people.
    pub why: String,
}

/// Which policy applies, and how that was decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explanation {
    pub winner: Policy,
    /// Every policy considered, in precedence order.
    pub checks: Vec<Check>,
    /// Why the winner won.
    pub reason: String,
}

fn glob_matches(glob: &str, value: &str) -> bool {
    // A glob that does not compile matches nothing: it cannot loosen approval.
    Glob::new(glob).is_ok_and(|g| g.compile_matcher().is_match(value))
}

fn judge(p: &Policy, s: &Subject<'_>) -> (bool, String) {
    let m = &p.matcher;
    if let Some(g) = &m.job
        && !glob_matches(g, s.job)
    {
        return (false, format!("job `{}` does not match `{g}`", s.job));
    }
    if let Some(i) = &m.inventory
        && i != s.inventory
    {
        return (false, format!("inventory `{}` is not `{i}`", s.inventory));
    }
    let limit = s.limit.unwrap_or("");
    if let Some(g) = &m.limit
        && !glob_matches(g, limit)
    {
        return (
            false,
            match s.limit {
                Some(l) => format!("limit `{l}` does not match `{g}`"),
                None => format!("the run has no limit, and `{g}` needs one"),
            },
        );
    }
    let mut parts = Vec::new();
    if let Some(g) = &m.job {
        parts.push(format!("job matches `{g}`"));
    }
    if let Some(i) = &m.inventory {
        parts.push(format!("inventory is `{i}`"));
    }
    if let Some(g) = &m.limit {
        parts.push(format!("limit matches `{g}`"));
    }
    if parts.is_empty() {
        (true, "matches every run".to_owned())
    } else {
        (true, parts.join(", "))
    }
}

/// Resolves which policy applies to `s`: the matching policy with the lowest priority.
#[must_use]
pub fn resolve(policies: &[Policy], s: &Subject<'_>) -> Explanation {
    let mut ordered: Vec<&Policy> = policies.iter().collect();
    ordered.sort_by_key(|p| (p.priority, p.id));
    let mut checks = Vec::with_capacity(ordered.len());
    let mut winner: Option<&Policy> = None;
    for p in ordered {
        let (matched, why) = judge(p, s);
        if matched && winner.is_none() {
            winner = Some(p);
        }
        checks.push(Check {
            policy: p.id,
            priority: p.priority,
            matched,
            why,
        });
    }
    match winner {
        Some(w) => {
            let reason = if w.is_catch_all() {
                "no other policy matches, so the workspace catch-all applies".to_owned()
            } else {
                format!(
                    "the lowest-priority policy that matches (priority {})",
                    w.priority
                )
            };
            Explanation {
                winner: w.clone(),
                checks,
                reason,
            }
        }
        None => Explanation {
            winner: Policy::catch_all(Uuid::nil()),
            checks,
            reason: "no policy matches and the catch-all is missing: approval is required"
                .to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn policy(n: u128, priority: i32, matcher: PolicyMatch, requires: bool) -> Policy {
        Policy {
            id: id(n),
            priority,
            matcher,
            requires_approval: requires,
            approvals_needed: 1,
            four_eyes: false,
            approval_ttl_secs: 3600,
        }
    }

    fn m(job: Option<&str>, inventory: Option<&str>, limit: Option<&str>) -> PolicyMatch {
        PolicyMatch {
            job: job.map(str::to_owned),
            inventory: inventory.map(str::to_owned),
            limit: limit.map(str::to_owned),
        }
    }

    fn subject<'a>(job: &'a str, inventory: &'a str, limit: Option<&'a str>) -> Subject<'a> {
        Subject {
            job,
            inventory,
            limit,
        }
    }

    #[test]
    fn lowest_priority_among_matches_wins() {
        let ps = vec![
            Policy::catch_all(id(1)),
            policy(2, 20, m(Some("web*"), None, None), true),
            policy(3, 10, m(Some("web"), None, None), false),
            policy(4, 5, m(Some("db*"), None, None), false),
        ];
        let e = resolve(&ps, &subject("web", "prod", None));
        assert_eq!(e.winner.id, id(3));
        assert!(!e.winner.requires_approval);
        let e = resolve(&ps, &subject("webapp", "prod", None));
        assert_eq!(e.winner.id, id(2));
        // Order of the input does not matter.
        let mut rev = ps.clone();
        rev.reverse();
        assert_eq!(
            resolve(&rev, &subject("web", "prod", None)).winner.id,
            id(3)
        );
    }

    #[test]
    fn ties_break_by_id() {
        let ps = vec![
            policy(9, 10, m(None, None, None), true),
            policy(8, 10, m(None, None, None), false),
            Policy::catch_all(id(1)),
        ];
        assert_eq!(resolve(&ps, &subject("x", "y", None)).winner.id, id(8));
    }

    #[test]
    fn match_by_job_glob_inventory_and_limit_glob() {
        let ps = vec![
            Policy::catch_all(id(1)),
            policy(2, 10, m(None, Some("staging"), None), false),
            policy(3, 20, m(None, None, Some("canary*")), false),
            policy(
                4,
                30,
                m(Some("deploy-*"), Some("prod"), Some("web*")),
                false,
            ),
        ];
        assert_eq!(
            resolve(&ps, &subject("any", "staging", None)).winner.id,
            id(2)
        );
        assert_eq!(
            resolve(&ps, &subject("any", "prod", Some("canary1")))
                .winner
                .id,
            id(3)
        );
        assert_eq!(
            resolve(&ps, &subject("deploy-web", "prod", Some("web1")))
                .winner
                .id,
            id(4)
        );
        // Each field must match.
        for s in [
            subject("deploy-web", "dev", Some("web1")),
            subject("other", "prod", Some("web1")),
            subject("deploy-web", "prod", Some("db1")),
            subject("deploy-web", "prod", None),
        ] {
            assert!(resolve(&ps, &s).winner.is_catch_all(), "{s:?}");
        }
    }

    #[test]
    fn no_limit_matches_star_but_not_a_prefix() {
        let ps = vec![
            Policy::catch_all(id(1)),
            policy(2, 10, m(None, None, Some("*")), false),
        ];
        assert_eq!(resolve(&ps, &subject("j", "i", None)).winner.id, id(2));
        let ps = vec![
            Policy::catch_all(id(1)),
            policy(2, 10, m(None, None, Some("web*")), false),
        ];
        assert!(resolve(&ps, &subject("j", "i", None)).winner.is_catch_all());
    }

    #[test]
    fn catch_all_requires_approval_and_applies_when_nothing_else_matches() {
        let c = Policy::catch_all(id(1));
        assert!(c.requires_approval);
        assert_eq!(c.priority, i32::MAX);
        assert!(c.approvals_needed >= 1);
        let e = resolve(std::slice::from_ref(&c), &subject("j", "i", Some("x")));
        assert_eq!(e.winner, c);
        assert!(e.reason.contains("catch-all"), "{}", e.reason);
    }

    #[test]
    fn a_missing_catch_all_fails_closed() {
        let ps = vec![policy(2, 10, m(Some("web"), None, None), false)];
        let e = resolve(&ps, &subject("db", "prod", None));
        assert!(e.winner.requires_approval);
        assert!(e.winner.is_catch_all());
        assert!(
            resolve(&[], &subject("db", "prod", None))
                .winner
                .requires_approval
        );
    }

    #[test]
    fn the_catch_all_cannot_be_removed_or_weakened() {
        let c = Policy::catch_all(id(1));
        assert_eq!(check_remove(&c), Err(PolicyError::CatchAllPermanent));
        assert_eq!(
            check_remove(&policy(2, 1, m(None, None, None), false)),
            Ok(())
        );
        let mut weaker = c.clone();
        weaker.requires_approval = false;
        assert_eq!(
            validate(&weaker, Some(&c)),
            Err(PolicyError::CatchAllWeakened)
        );
        let mut narrowed = c.clone();
        narrowed.matcher.job = Some("web".into());
        assert_eq!(
            validate(&narrowed, Some(&c)),
            Err(PolicyError::CatchAllWeakened)
        );
        let mut moved = c.clone();
        moved.priority = 5;
        assert_eq!(
            validate(&moved, Some(&c)),
            Err(PolicyError::CatchAllWeakened)
        );
        // Its quorum and TTL may change.
        let mut stricter = c.clone();
        stricter.approvals_needed = 2;
        stricter.four_eyes = true;
        assert_eq!(validate(&stricter, Some(&c)), Ok(()));
    }

    #[test]
    fn user_policies_are_checked() {
        let ok = policy(2, 10, m(Some("web*"), None, None), true);
        assert_eq!(validate(&ok, None), Ok(()));
        let mut p = ok.clone();
        p.priority = i32::MAX;
        assert_eq!(
            validate(&p, None),
            Err(PolicyError::PriorityReserved(i32::MAX))
        );
        let mut p = ok.clone();
        p.approvals_needed = 0;
        assert_eq!(validate(&p, None), Err(PolicyError::ApprovalsNeeded));
        let mut p = ok.clone();
        p.approval_ttl_secs = 0;
        assert_eq!(validate(&p, None), Err(PolicyError::Ttl));
        let mut p = ok.clone();
        p.matcher.job = Some("[".into());
        assert_eq!(
            validate(&p, None),
            Err(PolicyError::BadGlob { field: "job" })
        );
        let mut p = ok;
        p.matcher.limit = Some(String::new());
        assert_eq!(
            validate(&p, None),
            Err(PolicyError::BadGlob { field: "limit" })
        );
    }

    #[test]
    fn explain_says_which_policy_wins_and_why() {
        let ps = vec![
            Policy::catch_all(id(1)),
            policy(2, 10, m(Some("db*"), None, None), false),
            policy(3, 20, m(None, Some("prod"), None), false),
        ];
        let e = resolve(&ps, &subject("web", "prod", None));
        assert_eq!(e.winner.id, id(3));
        assert!(e.reason.contains("priority 20"), "{}", e.reason);
        assert_eq!(
            e.checks
                .iter()
                .map(|c| (c.policy, c.matched))
                .collect::<Vec<_>>(),
            vec![(id(2), false), (id(3), true), (id(1), true)]
        );
        assert!(
            e.checks[0].why.contains("does not match `db*`"),
            "{}",
            e.checks[0].why
        );
        assert!(
            e.checks[1].why.contains("inventory is `prod`"),
            "{}",
            e.checks[1].why
        );
    }
}
