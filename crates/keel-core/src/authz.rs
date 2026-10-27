//! Authorization (spec 055, T016/T018). Deny by default; access is the union of a user's grants.
//!
//! A grant gives a team role on one workspace. A grant with `profiles` is scoped: it covers a
//! host only if the host has at least one of those profiles. An action on several hosts needs
//! every host covered. Instance `owner` and `admin` have every right everywhere.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::domain::{InstanceRole, TeamRole, WorkspaceId};

/// One grant, as the user holds it through a team.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub workspace: WorkspaceId,
    pub role: TeamRole,
    /// Empty means the whole workspace; otherwise only hosts with one of these profiles.
    pub profiles: Vec<String>,
}

impl Grant {
    fn covers(&self, workspace: WorkspaceId, host_profiles: Option<&[String]>) -> bool {
        if self.workspace != workspace {
            return false;
        }
        if self.profiles.is_empty() {
            return true;
        }
        match host_profiles {
            Some(hp) => self.profiles.iter().any(|p| hp.contains(p)),
            None => false,
        }
    }
}

/// What a role allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// See the workspace, its runs, inventory and audit.
    View,
    /// Start and cancel runs.
    Operate,
    /// Approve or deny runs.
    Approve,
    /// Administer the workspace: bindings, policies, credentials, receivers, grants.
    Administer,
}

impl Capability {
    /// The capabilities a team role grants.
    #[must_use]
    pub fn of(role: TeamRole) -> &'static [Capability] {
        use Capability::*;
        match role {
            TeamRole::Viewer => &[View],
            TeamRole::Operator => &[View, Operate],
            TeamRole::Approver => &[View, Approve],
            TeamRole::WsAdmin => &[View, Operate, Approve, Administer],
        }
    }
}

/// The capabilities a user has on one target (a workspace, or one host in it).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Access(BTreeSet<Capability>);

impl Access {
    #[must_use]
    pub fn allows(&self, cap: Capability) -> bool {
        self.0.contains(&cap)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = Capability> + '_ {
        self.0.iter().copied()
    }
}

/// The union of what `grants` give on `workspace`.
///
/// With `host_profiles = Some(..)` the target is one host with those profiles: scoped grants count
/// when they share a profile with it. With `None` the target is the workspace itself: only
/// unscoped grants count, except that any grant on the workspace lets its holder see it.
#[must_use]
pub fn effective(
    grants: &[Grant],
    workspace: WorkspaceId,
    host_profiles: Option<&[String]>,
) -> Access {
    let mut caps = BTreeSet::new();
    for g in grants.iter().filter(|g| g.workspace == workspace) {
        if g.covers(workspace, host_profiles) {
            caps.extend(Capability::of(g.role).iter().copied());
        } else if host_profiles.is_none() {
            // A scoped grant still shows its holder the workspace it lives in.
            caps.insert(Capability::View);
        }
    }
    Access(caps)
}

/// Who is asking.
#[derive(Clone, Debug)]
pub struct Subject<'a> {
    pub instance_role: InstanceRole,
    pub grants: &'a [Grant],
}

impl Subject<'_> {
    #[must_use]
    pub fn is_instance_admin(&self) -> bool {
        matches!(
            self.instance_role,
            InstanceRole::Owner | InstanceRole::Admin
        )
    }
}

/// Why an action was refused. The `code` is stable and goes into `Denied` and the audit log.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("no grant allows this")]
    NoGrant,
    #[error("a host in the target is not covered by any grant")]
    HostNotCovered,
    #[error("instance administrators only")]
    InstanceAdminOnly,
    #[error("the last owner cannot be removed or demoted")]
    LastOwner,
}

impl Refusal {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Refusal::NoGrant => "no_grant",
            Refusal::HostNotCovered => "host_not_covered",
            Refusal::InstanceAdminOnly => "instance_admin_only",
            Refusal::LastOwner => "last_owner",
        }
    }
}

/// Requires `cap` on `workspace`, and on every host in `hosts` (each given by its profiles).
/// With no hosts the workspace itself is the target.
pub fn require(
    subject: &Subject<'_>,
    cap: Capability,
    workspace: WorkspaceId,
    hosts: &[Vec<String>],
) -> Result<(), Refusal> {
    if subject.is_instance_admin() {
        return Ok(());
    }
    if hosts.is_empty() {
        return if effective(subject.grants, workspace, None).allows(cap) {
            Ok(())
        } else {
            Err(Refusal::NoGrant)
        };
    }
    let mut any = false;
    let mut all = true;
    for host in hosts {
        let ok = effective(subject.grants, workspace, Some(host)).allows(cap);
        any |= ok;
        all &= ok;
    }
    match (all, any) {
        (true, _) => Ok(()),
        (false, true) => Err(Refusal::HostNotCovered),
        (false, false) => Err(Refusal::NoGrant),
    }
}

/// Requires an instance owner or admin.
pub fn require_instance_admin(subject: &Subject<'_>) -> Result<(), Refusal> {
    if subject.is_instance_admin() {
        Ok(())
    } else {
        Err(Refusal::InstanceAdminOnly)
    }
}

/// Checks a change to a user's instance role. `to = None` removes (or disables) the user.
/// `owners` is the number of active owners now, including the target if it is one.
pub fn check_instance_role_change(
    from: InstanceRole,
    to: Option<InstanceRole>,
    owners: usize,
) -> Result<(), Refusal> {
    let loses_owner = from == InstanceRole::Owner && to != Some(InstanceRole::Owner);
    if loses_owner && owners <= 1 {
        return Err(Refusal::LastOwner);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn ws(n: u128) -> WorkspaceId {
        WorkspaceId(Uuid::from_u128(n))
    }

    fn grant(w: WorkspaceId, role: TeamRole, profiles: &[&str]) -> Grant {
        Grant {
            workspace: w,
            role,
            profiles: profiles.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    fn host(profiles: &[&str]) -> Vec<String> {
        profiles.iter().map(|s| (*s).to_owned()).collect()
    }

    fn user(grants: &[Grant]) -> Subject<'_> {
        Subject {
            instance_role: InstanceRole::User,
            grants,
        }
    }

    #[test]
    fn deny_by_default() {
        let s = user(&[]);
        for cap in [
            Capability::View,
            Capability::Operate,
            Capability::Approve,
            Capability::Administer,
        ] {
            assert_eq!(require(&s, cap, ws(1), &[]), Err(Refusal::NoGrant));
            assert_eq!(
                require(&s, cap, ws(1), &[host(&["web"])]),
                Err(Refusal::NoGrant)
            );
        }
        assert!(effective(&[], ws(1), None).is_empty());
        assert_eq!(require_instance_admin(&s), Err(Refusal::InstanceAdminOnly));
    }

    #[test]
    fn grants_on_another_workspace_do_not_count() {
        let g = [grant(ws(2), TeamRole::WsAdmin, &[])];
        assert_eq!(
            require(&user(&g), Capability::View, ws(1), &[]),
            Err(Refusal::NoGrant)
        );
    }

    #[test]
    fn roles_map_to_capabilities() {
        let w = ws(1);
        let cases = [
            (TeamRole::Viewer, [true, false, false, false]),
            (TeamRole::Operator, [true, true, false, false]),
            (TeamRole::Approver, [true, false, true, false]),
            (TeamRole::WsAdmin, [true, true, true, true]),
        ];
        let caps = [
            Capability::View,
            Capability::Operate,
            Capability::Approve,
            Capability::Administer,
        ];
        for (role, expect) in cases {
            let g = [grant(w, role, &[])];
            for (cap, want) in caps.iter().zip(expect) {
                assert_eq!(
                    require(&user(&g), *cap, w, &[]).is_ok(),
                    want,
                    "{role} {cap:?}"
                );
            }
        }
    }

    #[test]
    fn union_of_grants() {
        let w = ws(1);
        let g = [
            grant(w, TeamRole::Operator, &[]),
            grant(w, TeamRole::Approver, &[]),
        ];
        let s = user(&g);
        assert!(require(&s, Capability::Operate, w, &[]).is_ok());
        assert!(require(&s, Capability::Approve, w, &[]).is_ok());
        assert_eq!(
            require(&s, Capability::Administer, w, &[]),
            Err(Refusal::NoGrant)
        );
    }

    #[test]
    fn scoped_grant_covers_only_hosts_with_its_profiles() {
        let w = ws(1);
        let g = [grant(w, TeamRole::Operator, &["web", "edge"])];
        let s = user(&g);
        assert!(require(&s, Capability::Operate, w, &[host(&["db", "edge"])]).is_ok());
        assert_eq!(
            require(&s, Capability::Operate, w, &[host(&["db"])]),
            Err(Refusal::NoGrant)
        );
        assert_eq!(
            require(&s, Capability::Operate, w, &[host(&[])]),
            Err(Refusal::NoGrant)
        );
        // A scoped grant is not a workspace-wide right, but its holder can see the workspace.
        assert_eq!(
            require(&s, Capability::Operate, w, &[]),
            Err(Refusal::NoGrant)
        );
        assert!(require(&s, Capability::View, w, &[]).is_ok());
    }

    #[test]
    fn several_hosts_need_coverage_of_all() {
        let w = ws(1);
        let g = [
            grant(w, TeamRole::Operator, &["web"]),
            grant(w, TeamRole::Viewer, &[]),
        ];
        let s = user(&g);
        let web = host(&["web"]);
        let db = host(&["db"]);
        assert!(require(&s, Capability::Operate, w, &[web.clone(), web.clone()]).is_ok());
        assert_eq!(
            require(&s, Capability::Operate, w, &[web.clone(), db.clone()]),
            Err(Refusal::HostNotCovered)
        );
        assert!(require(&s, Capability::View, w, &[web, db]).is_ok());
    }

    #[test]
    fn coverage_can_come_from_different_grants() {
        let w = ws(1);
        let g = [
            grant(w, TeamRole::Operator, &["web"]),
            grant(w, TeamRole::Operator, &["db"]),
        ];
        assert!(
            require(
                &user(&g),
                Capability::Operate,
                w,
                &[host(&["web"]), host(&["db"])]
            )
            .is_ok()
        );
    }

    #[test]
    fn instance_admins_have_every_right() {
        for role in [InstanceRole::Owner, InstanceRole::Admin] {
            let s = Subject {
                instance_role: role,
                grants: &[],
            };
            assert!(require(&s, Capability::Administer, ws(9), &[host(&["x"])]).is_ok());
            assert!(require_instance_admin(&s).is_ok());
        }
    }

    #[test]
    fn last_owner_cannot_be_removed_or_demoted() {
        use InstanceRole::*;
        assert_eq!(
            check_instance_role_change(Owner, None, 1),
            Err(Refusal::LastOwner)
        );
        assert_eq!(
            check_instance_role_change(Owner, Some(Admin), 1),
            Err(Refusal::LastOwner)
        );
        assert_eq!(
            check_instance_role_change(Owner, Some(User), 1),
            Err(Refusal::LastOwner)
        );
        assert_eq!(
            check_instance_role_change(Owner, Some(User), 0),
            Err(Refusal::LastOwner)
        );
        assert!(check_instance_role_change(Owner, Some(Owner), 1).is_ok());
        assert!(check_instance_role_change(Owner, None, 2).is_ok());
        assert!(check_instance_role_change(Admin, None, 1).is_ok());
        assert!(check_instance_role_change(User, Some(Owner), 1).is_ok());
    }

    #[test]
    fn refusal_codes_are_stable() {
        assert_eq!(Refusal::NoGrant.code(), "no_grant");
        assert_eq!(Refusal::HostNotCovered.code(), "host_not_covered");
        assert_eq!(Refusal::LastOwner.code(), "last_owner");
    }
}
