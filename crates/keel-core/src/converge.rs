//! Host convergence planning (spec 068): recipe expansion, `covers`, a host's plan and the
//! combined approval terms of a converge. Pure: the server plans with it and the runner
//! validates the same recipes through `manifest::validate`.
//!
//! - A profile's recipe is `includes` (applied first, in order, recursively) then `jobs` (in
//!   order). A profile without either keeps its old meaning: every job whose `profiles` names it,
//!   in manifest (name) order.
//! - [`expand`] walks a host's assigned profiles in name order, depth-first. Each job appears
//!   once, at its first position.
//! - [`coverage`] is the transitive closure of `jobs.<j>.covers`; [`host_plan`] drops a step
//!   another step of the same plan covers.
//! - [`combine_policies`] is the strictest of the steps' approval terms.

use std::collections::{BTreeMap, BTreeSet};

use crate::manifest::Manifest;
use crate::policy::{CATCH_ALL_TTL_SECS, Policy};

/// The most steps one host's plan may have.
pub const MAX_PLAN_STEPS: usize = 32;

/// The deepest chain of `includes`, counting the profile itself.
pub const MAX_INCLUDE_DEPTH: usize = 8;

/// Why a set of profiles cannot be expanded. Each maps to a manifest validation code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExpandError {
    /// The profiles of the cycle, starting and ending with the same name.
    #[error("profile cycle: {}", .0.join(" → "))]
    Cycle(Vec<String>),
    #[error("profile '{0}' is not declared under profiles")]
    UnknownProfile(String),
    #[error("profile '{profile}' lists job '{job}', which the manifest does not declare")]
    UnknownJob { profile: String, job: String },
    #[error("the plan has {0} steps; at most {MAX_PLAN_STEPS} are allowed")]
    TooManySteps(usize),
    /// The include chain that went too deep, outermost first.
    #[error("includes nest deeper than {MAX_INCLUDE_DEPTH}: {}", .0.join(" → "))]
    TooDeep(Vec<String>),
}

impl ExpandError {
    /// The manifest validation code (contracts/manifest.md).
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Cycle(_) => "profile_cycle",
            Self::UnknownProfile(_) => "unknown_include",
            Self::UnknownJob { .. } => "unknown_recipe_job",
            Self::TooManySteps(_) | Self::TooDeep(_) => "recipe_too_long",
        }
    }
}

/// Why `covers` cannot be closed. Each maps to a manifest validation code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoverError {
    #[error("job '{0}' covers itself")]
    SelfCover(String),
    #[error("job '{job}' covers '{cover}', which the manifest does not declare")]
    Unknown { job: String, cover: String },
    /// The jobs of the cycle, starting and ending with the same name.
    #[error("covers cycle: {}", .0.join(" → "))]
    Cycle(Vec<String>),
}

impl CoverError {
    /// The manifest validation code (contracts/manifest.md).
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::SelfCover(_) => "self_cover",
            Self::Unknown { .. } => "unknown_cover",
            Self::Cycle(_) => "cover_cycle",
        }
    }
}

/// Every job's transitive `covers`. A job that covers nothing has no entry.
pub type Coverage = BTreeMap<String, BTreeSet<String>>;

/// One step of a host's plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedStep {
    pub job: String,
    /// The profile path that put the step there, from the assigned profile to the one whose
    /// recipe (or fallback) names the job, e.g. `[keel-node, debian]`.
    pub via: Vec<String>,
}

/// A job a host's profiles reach that the plan leaves out (spec 068 T074): it has a required
/// param (a `choice` with no `default`), and a converge sends no params. Start it by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedStep {
    pub job: String,
    /// The profile path that reached it, as [`PlannedStep::via`].
    pub via: Vec<String>,
    /// The first required param.
    pub param: String,
}

/// One host's plan with what it left out: see [`host_steps`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostSteps {
    pub steps: Vec<PlannedStep>,
    /// Manual-only jobs the profiles reach, each once, in the order they were reached.
    pub skipped: Vec<SkippedStep>,
}

struct Walk<'m> {
    m: &'m Manifest,
    steps: Vec<PlannedStep>,
    skipped: Vec<SkippedStep>,
    placed: BTreeSet<String>,
    /// Profiles already expanded: their jobs are placed, so a second include adds nothing.
    done: BTreeSet<String>,
    stack: Vec<String>,
}

impl Walk<'_> {
    fn profile(&mut self, name: &str) -> Result<(), ExpandError> {
        if let Some(i) = self.stack.iter().position(|p| p == name) {
            let mut cycle = self.stack[i..].to_vec();
            cycle.push(name.to_owned());
            return Err(ExpandError::Cycle(cycle));
        }
        if self.done.contains(name) {
            return Ok(());
        }
        let Some(profile) = self.m.profiles.get(name) else {
            return Err(ExpandError::UnknownProfile(name.to_owned()));
        };
        self.stack.push(name.to_owned());
        if self.stack.len() > MAX_INCLUDE_DEPTH {
            return Err(ExpandError::TooDeep(self.stack.clone()));
        }
        if profile.has_recipe() {
            for inc in &profile.includes {
                self.profile(inc)?;
            }
            for job in &profile.jobs {
                if !self.m.jobs.contains_key(job) {
                    return Err(ExpandError::UnknownJob {
                        profile: name.to_owned(),
                        job: job.clone(),
                    });
                }
                self.place(job);
            }
        } else {
            let m = self.m;
            for (job, def) in &m.jobs {
                if def.definition.profiles.iter().any(|p| p == name) {
                    self.place(job);
                }
            }
        }
        self.stack.pop();
        self.done.insert(name.to_owned());
        Ok(())
    }

    /// Places `job` at its first position. A manual-only job never becomes a step: the implicit
    /// expansion reaches it (validation keeps it out of recipes), and it is reported rather than
    /// failing the plan (spec 068 T074).
    fn place(&mut self, job: &str) {
        if !self.placed.insert(job.to_owned()) {
            return;
        }
        let required = self
            .m
            .jobs
            .get(job)
            .and_then(|j| j.definition.required_params().next());
        if let Some(param) = required {
            self.skipped.push(SkippedStep {
                job: job.to_owned(),
                via: self.stack.clone(),
                param: param.to_owned(),
            });
        } else {
            self.steps.push(PlannedStep {
                job: job.to_owned(),
                via: self.stack.clone(),
            });
        }
    }
}

fn walk(profiles: &[String], m: &Manifest) -> Result<HostSteps, ExpandError> {
    let roots: BTreeSet<&String> = profiles.iter().collect();
    let mut w = Walk {
        m,
        steps: Vec::new(),
        skipped: Vec::new(),
        placed: BTreeSet::new(),
        done: BTreeSet::new(),
        stack: Vec::new(),
    };
    for p in roots {
        w.profile(p)?;
    }
    if w.steps.len() > MAX_PLAN_STEPS {
        return Err(ExpandError::TooManySteps(w.steps.len()));
    }
    Ok(HostSteps {
        steps: w.steps,
        skipped: w.skipped,
    })
}

/// The ordered jobs for `profiles`: the profiles in name order, each depth-first (includes,
/// then jobs). A job reached twice keeps its first position. Manual-only jobs are left out
/// ([`host_steps`] reports them).
///
/// # Errors
///
/// A cycle (named), an unknown profile or recipe job, more than [`MAX_PLAN_STEPS`] steps or
/// includes deeper than [`MAX_INCLUDE_DEPTH`].
pub fn expand(profiles: &[String], m: &Manifest) -> Result<Vec<String>, ExpandError> {
    Ok(walk(profiles, m)?
        .steps
        .into_iter()
        .map(|s| s.job)
        .collect())
}

/// The transitive closure of every job's `covers`.
///
/// # Errors
///
/// A job covers itself, an undeclared job, or a cycle of covers.
pub fn coverage(m: &Manifest) -> Result<Coverage, CoverError> {
    for (job, j) in &m.jobs {
        for c in &j.definition.covers {
            if c == job {
                return Err(CoverError::SelfCover(job.clone()));
            }
            if !m.jobs.contains_key(c) {
                return Err(CoverError::Unknown {
                    job: job.clone(),
                    cover: c.clone(),
                });
            }
        }
    }
    let mut closed: Coverage = BTreeMap::new();
    for job in m.jobs.keys() {
        let mut stack = Vec::new();
        close(job, m, &mut stack, &mut closed)?;
    }
    closed.retain(|_, v| !v.is_empty());
    Ok(closed)
}

fn close(
    job: &str,
    m: &Manifest,
    stack: &mut Vec<String>,
    closed: &mut Coverage,
) -> Result<(), CoverError> {
    if let Some(i) = stack.iter().position(|j| j == job) {
        let mut cycle = stack[i..].to_vec();
        cycle.push(job.to_owned());
        return Err(CoverError::Cycle(cycle));
    }
    if closed.contains_key(job) {
        return Ok(());
    }
    stack.push(job.to_owned());
    let mut all = BTreeSet::new();
    if let Some(j) = m.jobs.get(job) {
        for c in &j.definition.covers {
            close(c, m, stack, closed)?;
            all.insert(c.clone());
            if let Some(more) = closed.get(c) {
                all.extend(more.iter().cloned());
            }
        }
    }
    stack.pop();
    closed.insert(job.to_owned(), all);
    Ok(())
}

/// One host's plan: [`expand`] its assigned profiles, then drop every job another step of the
/// same plan covers (`k3s-master` + `k3s-audit` ⇒ `k3s-master`). The steps come from the
/// recipes; the host's ceiling, which the runner checks per step, is the guard (FR-017).
///
/// # Errors
///
/// As [`expand`].
pub fn host_plan(
    assigned: &[String],
    m: &Manifest,
    cov: &Coverage,
) -> Result<Vec<PlannedStep>, ExpandError> {
    Ok(host_steps(assigned, m, cov)?.steps)
}

/// [`host_plan`], with the manual-only jobs it skipped (spec 068 T074), for a preview to show.
///
/// # Errors
///
/// As [`expand`].
pub fn host_steps(
    assigned: &[String],
    m: &Manifest,
    cov: &Coverage,
) -> Result<HostSteps, ExpandError> {
    let HostSteps { steps, skipped } = walk(assigned, m)?;
    let covered: BTreeSet<&String> = steps
        .iter()
        .filter_map(|s| cov.get(&s.job))
        .flatten()
        .collect();
    let steps = steps
        .iter()
        .filter(|s| !covered.contains(&s.job))
        .cloned()
        .collect();
    Ok(HostSteps { steps, skipped })
}

/// Whether a pending converge with `remaining` jobs still to run applies `job`: it runs `job`
/// itself, or a job that covers it.
#[must_use]
pub fn covers(remaining: &[String], job: &str, cov: &Coverage) -> bool {
    remaining
        .iter()
        .any(|r| r == job || cov.get(r).is_some_and(|c| c.contains(job)))
}

/// The approval terms a converge is held with: the strictest across its steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalTerms {
    pub requires_approval: bool,
    pub approvals_needed: u16,
    pub four_eyes: bool,
    pub approval_ttl_secs: u32,
}

/// The strictest of the steps' resolved policies: approval if any step needs it, the largest
/// quorum, four-eyes if any, and the shortest TTL. Quorum, four-eyes and TTL only count from
/// policies that require approval. No steps fails closed to the catch-all's terms.
#[must_use]
pub fn combine_policies(steps: &[Policy]) -> ApprovalTerms {
    let catch_all = ApprovalTerms {
        requires_approval: true,
        approvals_needed: 1,
        four_eyes: false,
        approval_ttl_secs: CATCH_ALL_TTL_SECS,
    };
    if steps.is_empty() {
        return catch_all;
    }
    let requiring: Vec<&Policy> = steps.iter().filter(|p| p.requires_approval).collect();
    if requiring.is_empty() {
        return ApprovalTerms {
            requires_approval: false,
            approvals_needed: 1,
            four_eyes: false,
            approval_ttl_secs: steps
                .iter()
                .map(|p| p.approval_ttl_secs)
                .min()
                .unwrap_or(CATCH_ALL_TTL_SECS),
        };
    }
    ApprovalTerms {
        requires_approval: true,
        approvals_needed: requiring
            .iter()
            .map(|p| p.approvals_needed)
            .max()
            .unwrap_or(1)
            .max(1),
        four_eyes: requiring.iter().any(|p| p.four_eyes),
        approval_ttl_secs: requiring
            .iter()
            .map(|p| p.approval_ttl_secs)
            .min()
            .unwrap_or(CATCH_ALL_TTL_SECS),
    }
}

/// Whether a suggestion adds or removes a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SuggestionAction {
    Add,
    Remove,
}

/// One profile change the manifest's `groups:` map suggests for a host (FR-009). Never applied
/// on its own: an admin accepts it through the profile preview.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Suggestion {
    pub profile: String,
    pub action: SuggestionAction,
    /// For `Add`, the host's groups that map to the profile; for `Remove`, every group that
    /// could produce it (none of which the host is in). Sorted, so a stored suggestion can be
    /// compared with a fresh one: a dismissed suggestion comes back only when these change
    /// ([`still_dismissed`]).
    pub groups: Vec<String>,
}

/// The profile changes `m.groups` suggests for a host in `groups` with `assigned` profiles:
/// adds, then removes, each in profile name order.
///
/// - Add: a profile some group of the host maps to, not assigned yet.
/// - Remove: an assigned profile that some `groups:` entry maps to, when none of the host's
///   groups does. A profile no mapping could produce was assigned by hand and is never
///   suggested for removal.
///
/// Profiles the manifest does not define are ignored (validation refuses them anyway).
#[must_use]
pub fn suggest(groups: &[String], assigned: &[String], m: &Manifest) -> Vec<Suggestion> {
    let host_groups: BTreeSet<&str> = groups.iter().map(String::as_str).collect();
    let assigned: BTreeSet<&str> = assigned.iter().map(String::as_str).collect();
    // profile → (the host's groups mapping to it, every group mapping to it); `m.groups` is a
    // BTreeMap, so both lists come out sorted.
    let mut by_profile: BTreeMap<&str, (Vec<String>, Vec<String>)> = BTreeMap::new();
    for (group, profiles) in &m.groups {
        for p in profiles.iter().filter(|p| m.profiles.contains_key(*p)) {
            let entry = by_profile.entry(p.as_str()).or_default();
            if host_groups.contains(group.as_str()) && !entry.0.contains(group) {
                entry.0.push(group.clone());
            }
            if !entry.1.contains(group) {
                entry.1.push(group.clone());
            }
        }
    }
    let mut adds = Vec::new();
    let mut removes = Vec::new();
    for (profile, (mine, all)) in by_profile {
        let is_assigned = assigned.contains(profile);
        if !mine.is_empty() && !is_assigned {
            adds.push(Suggestion {
                profile: profile.to_owned(),
                action: SuggestionAction::Add,
                groups: mine,
            });
        } else if mine.is_empty() && is_assigned {
            removes.push(Suggestion {
                profile: profile.to_owned(),
                action: SuggestionAction::Remove,
                groups: all,
            });
        }
    }
    adds.extend(removes);
    adds
}

/// Whether a suggestion dismissed as `dismissed` stays dismissed now that a sync suggests
/// `now`: the same profile and action from the same groups. A change of groups brings it back.
#[must_use]
pub fn still_dismissed(dismissed: &Suggestion, now: &Suggestion) -> bool {
    dismissed == now
}

/// What a changed path under `group_vars/` or `host_vars/` names.
#[derive(Debug, PartialEq, Eq)]
enum VarsTarget<'a> {
    Group(&'a str),
    Host(&'a str),
}

/// The group or host a vars file is for, from the last `group_vars`/`host_vars` component of
/// `path`: `<dir>/<name>.yml`, `<dir>/<name>.yaml`, or anything under `<dir>/<name>/`. Any
/// other file there (`group_vars/web.json`, `group_vars/README`) is not recognised, so the
/// caller keeps the full target set.
fn vars_target(path: &str) -> Option<VarsTarget<'_>> {
    let parts: Vec<&str> = path.split('/').collect();
    let at = parts
        .iter()
        .rposition(|p| *p == "group_vars" || *p == "host_vars")?;
    let name = match &parts[at + 1..] {
        [file] => file
            .strip_suffix(".yml")
            .or_else(|| file.strip_suffix(".yaml"))?,
        [dir, rest @ ..] if !rest.is_empty() && rest.iter().all(|p| !p.is_empty()) => dir,
        _ => return None,
    };
    if name.is_empty() {
        return None;
    }
    Some(if parts[at] == "group_vars" {
        VarsTarget::Group(name)
    } else {
        VarsTarget::Host(name)
    })
}

/// FR-020: the hosts a path-triggered run narrows to when every changed path that matched the
/// job's `paths` globs is a group or host vars file. `None` keeps the full target set: when a
/// matched path is anything else, when one is `group_vars/all…`, or when nothing matched.
///
/// `inventory` is each host with its groups, transitively (as `inventory::HostGroups` holds
/// them at the pinned revision), so a group's hosts include its child groups' hosts. A group
/// or host the inventory does not know narrows to no hosts: no run.
#[must_use]
pub fn narrow_by_vars(
    changed: &[String],
    paths: &[String],
    inventory: &[crate::inventory::HostGroups],
) -> Option<BTreeSet<String>> {
    let matchers: Vec<globset::GlobMatcher> = paths
        .iter()
        .filter_map(|g| globset::Glob::new(g).ok().map(|g| g.compile_matcher()))
        .collect();
    let mut groups = BTreeSet::new();
    let mut hosts = BTreeSet::new();
    let mut any = false;
    for path in changed
        .iter()
        .filter(|p| matchers.iter().any(|m| m.is_match(p.as_str())))
    {
        any = true;
        match vars_target(path)? {
            VarsTarget::Group("all") => return None,
            VarsTarget::Group(g) => {
                groups.insert(g);
            }
            VarsTarget::Host(h) => {
                hosts.insert(h);
            }
        }
    }
    if !any {
        return None;
    }
    Some(
        inventory
            .iter()
            .filter(|h| {
                hosts.contains(h.host.as_str())
                    || h.groups.iter().any(|g| groups.contains(g.as_str()))
            })
            .map(|h| h.host.clone())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use uuid::Uuid;

    use super::*;
    use crate::manifest::{InventorySource, Job, JobDefinition, Profile};
    use crate::policy::PolicyMatch;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    fn job(profiles: &[&str], covers: &[&str]) -> Job {
        Job {
            definition: JobDefinition {
                playbook: "p.yaml".into(),
                profiles: s(profiles),
                inventory: "fleet".into(),
                inventory_source: InventorySource::Repo,
                limit: None,
                vars: BTreeMap::new(),
                credentials: Vec::new(),
                paths: None,
                timeout_secs: None,
                rollback: None,
                params: BTreeMap::new(),
                covers: s(covers),
                env: BTreeMap::new(),
            },
            missing_credentials: Vec::new(),
        }
    }

    fn recipe(includes: &[&str], jobs: &[&str]) -> Profile {
        Profile {
            description: None,
            includes: s(includes),
            jobs: s(jobs),
        }
    }

    #[derive(Default)]
    struct M(Manifest);

    impl M {
        fn profile(mut self, name: &str, p: Profile) -> Self {
            self.0.profiles.insert(name.into(), p);
            self
        }
        fn job(mut self, name: &str, j: Job) -> Self {
            self.0.jobs.insert(name.into(), j);
            self
        }
    }

    /// The operator's fleet, as contracts/manifest.md writes it.
    fn fleet() -> Manifest {
        M::default()
            .profile("debian", recipe(&[], &["debian"]))
            .profile("keel-node", recipe(&["debian"], &["keel-node"]))
            .profile("k3s-master", recipe(&["debian"], &["k3s-master"]))
            .profile("k3s-audit", Profile::default())
            .profile("prox-vm", Profile::default())
            .profile("gameserver", recipe(&["debian"], &["gameserver"]))
            .job("debian", job(&["debian"], &[]))
            .job("keel-node", job(&["keel-node"], &[]))
            .job("k3s-master", job(&["k3s-master"], &["k3s-audit"]))
            .job("k3s-audit", job(&["k3s-audit"], &[]))
            .job("prox-vm", job(&["prox-vm"], &[]))
            .job("gameserver", job(&["gameserver"], &["prox-vm"]))
            .0
    }

    #[test]
    fn expand_runs_includes_depth_first_then_jobs() {
        let m = fleet();
        assert_eq!(
            expand(&s(&["keel-node"]), &m).unwrap(),
            s(&["debian", "keel-node"])
        );
        let m = M(m)
            .profile("base", recipe(&[], &["a", "b"]))
            .profile("mid", recipe(&["base"], &["c"]))
            .profile("top", recipe(&["mid", "debian"], &["d"]))
            .job("a", job(&["base"], &[]))
            .job("b", job(&["base"], &[]))
            .job("c", job(&["mid"], &[]))
            .job("d", job(&["top"], &[]))
            .0;
        assert_eq!(
            expand(&s(&["top"]), &m).unwrap(),
            s(&["a", "b", "c", "debian", "d"])
        );
    }

    #[test]
    fn expand_walks_assigned_profiles_in_name_order() {
        let m = fleet();
        // keel-node sorts after k3s-master whatever order the caller passes.
        let want = s(&["debian", "k3s-master", "keel-node"]);
        assert_eq!(expand(&s(&["keel-node", "k3s-master"]), &m).unwrap(), want);
        assert_eq!(expand(&s(&["k3s-master", "keel-node"]), &m).unwrap(), want);
    }

    #[test]
    fn expand_first_position_wins() {
        let m = M(fleet())
            .profile("x", recipe(&["debian"], &["keel-node", "debian"]))
            .0;
        assert_eq!(
            expand(&s(&["keel-node", "x"]), &m).unwrap(),
            s(&["debian", "keel-node"])
        );
        let via = walk(&s(&["keel-node"]), &m).unwrap().steps;
        assert_eq!(via[0].via, s(&["keel-node", "debian"]));
        assert_eq!(via[1].via, s(&["keel-node"]));
    }

    #[test]
    fn a_profile_without_a_recipe_runs_the_jobs_naming_it() {
        let m = M::default()
            .profile("web", Profile::default())
            .job("z-web", job(&["web"], &[]))
            .job("a-web", job(&["web", "db"], &[]))
            .job("other", job(&["db"], &[]))
            .profile("db", Profile::default())
            .0;
        assert_eq!(expand(&s(&["web"]), &m).unwrap(), s(&["a-web", "z-web"]));
        // An included profile without a recipe falls back too.
        let m = M(m).profile("both", recipe(&["web"], &["other"])).0;
        assert_eq!(
            expand(&s(&["both"]), &m).unwrap(),
            s(&["a-web", "z-web", "other"])
        );
    }

    #[test]
    fn expand_names_the_cycle() {
        let m = M::default()
            .profile("a", recipe(&["b"], &[]))
            .profile("b", recipe(&["c"], &[]))
            .profile("c", recipe(&["a"], &[]))
            .0;
        let e = expand(&s(&["a"]), &m).unwrap_err();
        assert_eq!(e, ExpandError::Cycle(s(&["a", "b", "c", "a"])));
        assert_eq!(e.code(), "profile_cycle");
        assert_eq!(e.to_string(), "profile cycle: a → b → c → a");
        let self_loop = M::default().profile("a", recipe(&["a"], &[])).0;
        assert_eq!(
            expand(&s(&["a"]), &self_loop).unwrap_err(),
            ExpandError::Cycle(s(&["a", "a"]))
        );
    }

    #[test]
    fn expand_refuses_unknown_names() {
        let m = M::default().profile("a", recipe(&["ghost"], &[])).0;
        let e = expand(&s(&["a"]), &m).unwrap_err();
        assert_eq!(e, ExpandError::UnknownProfile("ghost".into()));
        assert_eq!(e.code(), "unknown_include");
        let m = M::default().profile("a", recipe(&[], &["ghost"])).0;
        assert_eq!(
            expand(&s(&["a"]), &m).unwrap_err().code(),
            "unknown_recipe_job"
        );
        assert_eq!(
            expand(&s(&["nope"]), &fleet()).unwrap_err(),
            ExpandError::UnknownProfile("nope".into())
        );
    }

    #[test]
    fn expand_refuses_more_than_32_steps() {
        let mut m = M::default();
        let names: Vec<String> = (0..33).map(|i| format!("j{i:02}")).collect();
        for n in &names {
            m = m.job(n, job(&["big"], &[]));
        }
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let m = m.profile("big", recipe(&[], &refs)).0;
        let e = expand(&s(&["big"]), &m).unwrap_err();
        assert_eq!(e, ExpandError::TooManySteps(33));
        assert_eq!(e.code(), "recipe_too_long");
        // 32 is fine.
        let mut m2 = m.clone();
        m2.profiles.get_mut("big").unwrap().jobs.pop();
        assert_eq!(expand(&s(&["big"]), &m2).unwrap().len(), 32);
    }

    #[test]
    fn expand_refuses_includes_deeper_than_8() {
        let mut m = M::default();
        for i in 0..9 {
            let inc = if i == 8 {
                vec![]
            } else {
                vec![format!("p{}", i + 1)]
            };
            let inc: Vec<&str> = inc.iter().map(String::as_str).collect();
            m = m.profile(&format!("p{i}"), recipe(&inc, &[]));
        }
        let e = expand(&s(&["p0"]), &m.0).unwrap_err();
        assert!(
            matches!(&e, ExpandError::TooDeep(chain) if chain.len() == 9),
            "{e:?}"
        );
        assert_eq!(e.code(), "recipe_too_long");
    }

    #[test]
    fn coverage_is_transitive() {
        let m = M::default()
            .job("a", job(&["x"], &["b"]))
            .job("b", job(&["x"], &["c"]))
            .job("c", job(&["x"], &[]))
            .job("d", job(&["x"], &["c"]))
            .0;
        let cov = coverage(&m).unwrap();
        assert_eq!(cov["a"], ["b", "c"].map(String::from).into());
        assert_eq!(cov["b"], ["c"].map(String::from).into());
        assert!(!cov.contains_key("c"));
        assert!(covers(&s(&["a"]), "c", &cov));
        assert!(covers(&s(&["d", "z"]), "c", &cov));
        assert!(covers(&s(&["c"]), "c", &cov), "a job covers its own run");
        assert!(!covers(&s(&["c"]), "a", &cov));
        assert!(!covers(&[], "c", &cov));
    }

    #[test]
    fn coverage_refuses_self_cover_unknown_and_cycles() {
        let m = M::default().job("a", job(&["x"], &["a"])).0;
        assert_eq!(coverage(&m).unwrap_err(), CoverError::SelfCover("a".into()));
        assert_eq!(coverage(&m).unwrap_err().code(), "self_cover");
        let m = M::default().job("a", job(&["x"], &["ghost"])).0;
        assert_eq!(coverage(&m).unwrap_err().code(), "unknown_cover");
        let m = M::default()
            .job("a", job(&["x"], &["b"]))
            .job("b", job(&["x"], &["c"]))
            .job("c", job(&["x"], &["a"]))
            .0;
        let e = coverage(&m).unwrap_err();
        assert_eq!(e, CoverError::Cycle(s(&["a", "b", "c", "a"])));
        assert_eq!(e.code(), "cover_cycle");
    }

    #[test]
    fn host_plan_drops_covered_steps() {
        let m = fleet();
        let cov = coverage(&m).unwrap();
        let plan = host_plan(&s(&["k3s-master", "k3s-audit"]), &m, &cov).unwrap();
        let jobs: Vec<&str> = plan.iter().map(|p| p.job.as_str()).collect();
        assert_eq!(jobs, ["debian", "k3s-master"]);
        // Order of the covering step does not matter.
        let m2 = M(m.clone())
            .profile("audit-first", recipe(&["k3s-audit"], &["k3s-master"]))
            .0;
        let plan = host_plan(&s(&["audit-first"]), &m2, &cov).unwrap();
        assert_eq!(
            plan.iter().map(|p| p.job.as_str()).collect::<Vec<_>>(),
            ["k3s-master"]
        );
        // Nothing covered: the plan is the expansion.
        let plan = host_plan(&s(&["keel-node"]), &m, &cov).unwrap();
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].via, s(&["keel-node", "debian"]));
        // gameserver covers prox-vm.
        let plan = host_plan(&s(&["gameserver", "prox-vm"]), &m, &cov).unwrap();
        assert_eq!(
            plan.iter().map(|p| p.job.as_str()).collect::<Vec<_>>(),
            ["debian", "gameserver"]
        );
    }

    fn pol(requires: bool, needed: u16, four_eyes: bool, ttl: u32) -> Policy {
        Policy {
            id: Uuid::nil(),
            priority: 1,
            matcher: PolicyMatch::default(),
            requires_approval: requires,
            approvals_needed: needed,
            four_eyes,
            approval_ttl_secs: ttl,
        }
    }

    #[test]
    fn combine_policies_takes_the_strictest() {
        let t = combine_policies(&[
            pol(false, 5, true, 60),
            pol(true, 1, false, 7200),
            pol(true, 2, false, 3600),
            pol(true, 1, true, 86_400),
        ]);
        assert_eq!(
            t,
            ApprovalTerms {
                requires_approval: true,
                approvals_needed: 2,
                four_eyes: true,
                approval_ttl_secs: 3600,
            }
        );
        let none = combine_policies(&[pol(false, 1, false, 100), pol(false, 3, true, 50)]);
        assert!(!none.requires_approval);
        assert!(!none.four_eyes);
        // No steps fails closed.
        let empty = combine_policies(&[]);
        assert!(empty.requires_approval);
        assert_eq!(empty.approvals_needed, 1);
    }

    /// A small deterministic generator (xorshift), so the property runs the same every time
    /// without a new dependency.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            usize::try_from(self.next() % (n as u64)).unwrap_or(0)
        }
    }

    /// Random acyclic manifests: profile `pK` may include only profiles with a larger index.
    fn random_manifest(r: &mut Rng) -> Manifest {
        let profiles = 2 + r.below(8);
        let jobs = 1 + r.below(10);
        let mut m = M::default();
        for j in 0..jobs {
            let p = format!("p{}", r.below(profiles));
            m = m.job(&format!("j{j}"), job(&[p.as_str()], &[]));
        }
        for p in 0..profiles {
            let mut prof = Profile::default();
            if r.below(3) > 0 {
                for _ in 0..r.below(3) {
                    if p + 1 < profiles {
                        let q = p + 1 + r.below(profiles - p - 1);
                        prof.includes.push(format!("p{q}"));
                    }
                }
                for _ in 0..r.below(4) {
                    prof.jobs.push(format!("j{}", r.below(jobs)));
                }
            }
            m = m.profile(&format!("p{p}"), prof);
        }
        m.0
    }

    /// Every job a profile can reach through its recipe (or fallback).
    fn reachable(p: &str, m: &Manifest, out: &mut BTreeSet<String>) {
        let prof = &m.profiles[p];
        if prof.has_recipe() {
            for i in &prof.includes {
                reachable(i, m, out);
            }
            out.extend(prof.jobs.iter().cloned());
        } else {
            for (j, def) in &m.jobs {
                if def.definition.profiles.iter().any(|x| x == p) {
                    out.insert(j.clone());
                }
            }
        }
    }

    #[test]
    fn property_every_step_is_reachable_and_appears_once() {
        let mut r = Rng(0x068C_0DE5_EED5_0001);
        for _ in 0..2000 {
            let m = random_manifest(&mut r);
            let names: Vec<String> = m.profiles.keys().cloned().collect();
            let take = 1 + r.below(names.len());
            let assigned: Vec<String> = (0..take)
                .map(|_| names[r.below(names.len())].clone())
                .collect();
            let Ok(HostSteps { steps, .. }) = walk(&assigned, &m) else {
                // Only the step bound can refuse an acyclic manifest; depth is ≤ 9 here.
                continue;
            };
            let mut want = BTreeSet::new();
            for a in &assigned {
                reachable(a, &m, &mut want);
            }
            let got: Vec<&String> = steps.iter().map(|s| &s.job).collect();
            let unique: BTreeSet<&String> = got.iter().copied().collect();
            assert_eq!(unique.len(), got.len(), "a job appears twice: {got:?}");
            assert_eq!(
                unique,
                want.iter().collect::<BTreeSet<_>>(),
                "steps are exactly the reachable jobs"
            );
            for st in &steps {
                assert!(
                    assigned.contains(&st.via[0]),
                    "via starts at an assigned profile"
                );
            }
        }
    }

    // --- T036: suggest ---

    fn with_groups(map: &[(&str, &[&str])]) -> Manifest {
        let mut m = fleet();
        for (g, ps) in map {
            m.groups.insert((*g).to_owned(), s(ps));
        }
        m
    }

    fn sug(profile: &str, action: SuggestionAction, groups: &[&str]) -> Suggestion {
        Suggestion {
            profile: profile.into(),
            action,
            groups: s(groups),
        }
    }

    #[test]
    fn suggest_adds_from_the_hosts_groups() {
        let m = with_groups(&[
            ("k3s_masters", &["k3s-master"]),
            ("keel_servers", &["keel-node", "debian"]),
            ("linux", &["debian"]),
        ]);
        let got = suggest(&s(&["k3s_masters", "linux"]), &s(&[]), &m);
        assert_eq!(
            got,
            vec![
                sug("debian", SuggestionAction::Add, &["linux"]),
                sug("k3s-master", SuggestionAction::Add, &["k3s_masters"]),
            ]
        );
        // Already assigned: nothing to add.
        let got = suggest(
            &s(&["k3s_masters", "linux"]),
            &s(&["debian", "k3s-master"]),
            &m,
        );
        assert!(got.is_empty(), "{got:?}");
        // Two of the host's groups map to one profile: one suggestion naming both.
        let got = suggest(&s(&["keel_servers", "linux"]), &s(&["keel-node"]), &m);
        assert_eq!(
            got,
            vec![sug(
                "debian",
                SuggestionAction::Add,
                &["keel_servers", "linux"]
            )]
        );
    }

    #[test]
    fn suggest_removes_only_what_a_mapping_could_produce() {
        let m = with_groups(&[("k3s_masters", &["k3s-master"]), ("linux", &["debian"])]);
        // The host left k3s_masters: k3s-master is suggested for removal. gameserver was
        // assigned by hand (no group maps to it) and is left alone.
        let got = suggest(
            &s(&["linux"]),
            &s(&["debian", "gameserver", "k3s-master"]),
            &m,
        );
        assert_eq!(
            got,
            vec![sug(
                "k3s-master",
                SuggestionAction::Remove,
                &["k3s_masters"]
            )]
        );
        // Adds come before removes.
        let got = suggest(&s(&["k3s_masters"]), &s(&["debian"]), &m);
        assert_eq!(
            got,
            vec![
                sug("k3s-master", SuggestionAction::Add, &["k3s_masters"]),
                sug("debian", SuggestionAction::Remove, &["linux"]),
            ]
        );
    }

    #[test]
    fn suggest_ignores_unknown_profiles_and_an_empty_map() {
        let m = with_groups(&[("linux", &["no-such-profile"])]);
        assert!(suggest(&s(&["linux"]), &s(&[]), &m).is_empty());
        assert!(suggest(&s(&["linux"]), &s(&["debian"]), &fleet()).is_empty());
    }

    #[test]
    fn a_dismissed_suggestion_returns_only_when_its_groups_change() {
        let m = with_groups(&[("keel_servers", &["debian"]), ("linux", &["debian"])]);
        let dismissed = suggest(&s(&["linux"]), &s(&[]), &m).remove(0);
        // The next sync suggests the same thing: still dismissed.
        let again = suggest(&s(&["linux", "other"]), &s(&[]), &m).remove(0);
        assert!(still_dismissed(&dismissed, &again));
        // The host joins another group mapping to it: the suggestion comes back.
        let changed = suggest(&s(&["keel_servers", "linux"]), &s(&[]), &m).remove(0);
        assert!(!still_dismissed(&dismissed, &changed));
    }

    // --- T072: narrow_by_vars (FR-020) ---

    fn inventory() -> Vec<crate::inventory::HostGroups> {
        let h = |host: &str, groups: &[&str]| crate::inventory::HostGroups {
            host: host.into(),
            groups: s(groups),
        };
        // `k3s` has the child groups `k3s_masters` and `k3s_agents`: membership is transitive.
        vec![
            h("db-1", &["databases"]),
            h("k3s-a1", &["k3s", "k3s_agents"]),
            h("k3s-m1", &["k3s", "k3s_masters"]),
            h("keel-1", &["keel_servers"]),
            h("lone", &["ungrouped"]),
        ]
    }

    fn narrow(changed: &[&str]) -> Option<Vec<String>> {
        narrow_by_vars(&s(changed), &s(&["inventory/**"]), &inventory())
            .map(|set| set.into_iter().collect())
    }

    #[test]
    fn a_group_vars_file_narrows_to_its_groups_hosts() {
        assert_eq!(
            narrow(&["inventory/group_vars/keel_servers.yml"]),
            Some(s(&["keel-1"]))
        );
        assert_eq!(
            narrow(&["inventory/group_vars/databases.yaml"]),
            Some(s(&["db-1"]))
        );
        // Directory form.
        assert_eq!(
            narrow(&["inventory/group_vars/databases/main.yml"]),
            Some(s(&["db-1"]))
        );
    }

    #[test]
    fn a_parent_group_includes_its_child_groups_hosts() {
        assert_eq!(
            narrow(&["inventory/group_vars/k3s.yml"]),
            Some(s(&["k3s-a1", "k3s-m1"]))
        );
        assert_eq!(
            narrow(&["inventory/group_vars/k3s_masters.yml"]),
            Some(s(&["k3s-m1"]))
        );
    }

    #[test]
    fn host_vars_and_several_files_union() {
        assert_eq!(
            narrow(&[
                "inventory/host_vars/lone.yml",
                "inventory/group_vars/databases.yml",
                "inventory/host_vars/keel-1/secrets.yaml",
            ]),
            Some(s(&["db-1", "keel-1", "lone"]))
        );
    }

    #[test]
    fn all_or_any_other_matched_path_keeps_every_host() {
        assert_eq!(narrow(&["inventory/group_vars/all.yml"]), None);
        assert_eq!(narrow(&["inventory/group_vars/all/vault.yml"]), None);
        assert_eq!(
            narrow(&["inventory/group_vars/databases.yml", "inventory/hosts.yml"]),
            None
        );
        // Not a vars file Keel recognises.
        assert_eq!(narrow(&["inventory/group_vars/databases.json"]), None);
        assert_eq!(narrow(&["inventory/group_vars/README"]), None);
        assert_eq!(narrow(&["inventory/group_vars/"]), None);
    }

    #[test]
    fn unmatched_changes_do_not_count() {
        // The playbook change does not match `inventory/**`, so only the vars file counts.
        assert_eq!(
            narrow(&["playbooks/site.yml", "inventory/group_vars/databases.yml"]),
            Some(s(&["db-1"]))
        );
        // Nothing matched: nothing to narrow (the caller does not run anyway).
        assert_eq!(narrow(&["playbooks/site.yml"]), None);
    }

    #[test]
    fn an_unknown_group_narrows_to_no_hosts() {
        assert_eq!(narrow(&["inventory/group_vars/retired.yml"]), Some(vec![]));
    }
}
