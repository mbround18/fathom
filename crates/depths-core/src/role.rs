//! Who may do what (R6, data-model.md "User and permissions").
//!
//! Two roles hold access: `user` (their assigned stacks only) and `admin` (everything). Everyone
//! else is `pending`, which permits nothing. The registry is read on every command and turned
//! into an [`Access`] there; this module only answers the question, with no I/O.
//!
//! The runtime feature flags (`DEPTHS_FEATURE_SHELL`, `DEPTHS_FEATURE_EDIT`) are not part of a
//! role. The server checks them beside [`permits`]; a flag turned off refuses everyone.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::name::StackName;

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Signed in, no access.
    #[default]
    Pending,
    /// Assigned stacks only.
    User,
    /// Everything.
    Admin,
}

/// One person's access, as the registry says at the time of the command.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Access {
    pub role: Role,
    /// Used for `user`; ignored for `admin`.
    pub stacks: BTreeSet<StackName>,
}

/// Every action the permission table names, one row each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// See the stack, its files (secret-looking values masked), status, logs and stats.
    View,
    /// Reveal masked `.env` values.
    RevealEnv,
    /// up, stop, restart, plain down, pull, update, and the per-service forms.
    Lifecycle,
    /// An interactive shell in one of the stack's containers.
    Shell,
    /// Edit and save the stack's files.
    Edit,
    /// `down` with volumes or images (FR-016).
    DownDestructive,
    CreateStack,
    DeleteStack,
    /// Foreign projects and standalone containers (FR-008).
    ViewForeign,
    ReadAudit,
    /// `GetUsers`, `PutUser`, `RemoveUser`.
    ManageUsers,
}

impl Action {
    /// Every action, for tables and tests.
    pub const ALL: [Self; 11] = [
        Self::View,
        Self::RevealEnv,
        Self::Lifecycle,
        Self::Shell,
        Self::Edit,
        Self::DownDestructive,
        Self::CreateStack,
        Self::DeleteStack,
        Self::ViewForeign,
        Self::ReadAudit,
        Self::ManageUsers,
    ];

    /// Whether a `user` may take this action on a stack assigned to them. The rest are admin-only.
    pub fn open_to_users(self) -> bool {
        matches!(
            self,
            Self::View | Self::RevealEnv | Self::Lifecycle | Self::Shell | Self::Edit
        )
    }
}

/// Whether `access` permits `action` on `stack` (`None` for actions that name no stack).
///
/// A `user` is refused every admin-only action, and every action on a stack outside their list,
/// including one that names no stack at all.
pub fn permits(access: &Access, stack: Option<&StackName>, action: Action) -> bool {
    match access.role {
        Role::Pending => false,
        Role::Admin => true,
        Role::User => {
            action.open_to_users() && stack.is_some_and(|s| access.stacks.contains(s))
        }
    }
}

/// Whether `access` lets its holder see anything at all; `false` drives the no-access page
/// (FR-031).
pub fn has_any_access(access: &Access) -> bool {
    match access.role {
        Role::Pending => false,
        Role::User => !access.stacks.is_empty(),
        Role::Admin => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(s: &str) -> StackName {
        StackName::parse(s).unwrap()
    }

    fn user(stacks: &[&str]) -> Access {
        Access {
            role: Role::User,
            stacks: stacks.iter().map(|s| name(s)).collect(),
        }
    }

    fn admin() -> Access {
        Access {
            role: Role::Admin,
            stacks: BTreeSet::new(),
        }
    }

    /// data-model.md's table: (action, user on an assigned stack, admin).
    const TABLE: [(Action, bool, bool); 11] = [
        (Action::View, true, true),
        (Action::RevealEnv, true, true),
        (Action::Lifecycle, true, true),
        (Action::Shell, true, true),
        (Action::Edit, true, true),
        (Action::DownDestructive, false, true),
        (Action::CreateStack, false, true),
        (Action::DeleteStack, false, true),
        (Action::ViewForeign, false, true),
        (Action::ReadAudit, false, true),
        (Action::ManageUsers, false, true),
    ];

    #[test]
    fn the_table_covers_every_action() {
        let rows: Vec<Action> = TABLE.iter().map(|(a, _, _)| *a).collect();
        assert_eq!(rows, Action::ALL.to_vec());
    }

    #[test]
    fn pending_permits_nothing() {
        let pending = Access {
            role: Role::Pending,
            stacks: [name("web")].into(),
        };
        for action in Action::ALL {
            assert!(!permits(&pending, Some(&name("web")), action), "{action:?}");
            assert!(!permits(&pending, None, action), "{action:?}");
        }
        assert!(!has_any_access(&pending));
    }

    #[test]
    fn a_user_on_an_assigned_stack_follows_the_table() {
        let u = user(&["web", "db"]);
        for (action, for_user, _) in TABLE {
            assert_eq!(
                permits(&u, Some(&name("web")), action),
                for_user,
                "user on an assigned stack, {action:?}"
            );
        }
    }

    #[test]
    fn a_user_on_an_unassigned_stack_is_permitted_nothing() {
        let u = user(&["web"]);
        for action in Action::ALL {
            assert!(!permits(&u, Some(&name("other")), action), "{action:?}");
        }
    }

    #[test]
    fn admin_follows_the_table_on_any_stack() {
        for (action, _, for_admin) in TABLE {
            assert_eq!(permits(&admin(), Some(&name("any")), action), for_admin);
            assert_eq!(permits(&admin(), None, action), for_admin);
        }
    }

    #[test]
    fn admin_only_actions_are_refused_to_every_user() {
        let users = [user(&[]), user(&["web"]), user(&["web", "db", "media"])];
        for u in &users {
            for action in Action::ALL.into_iter().filter(|a| !a.open_to_users()) {
                assert!(!permits(u, Some(&name("web")), action), "{action:?}");
                assert!(!permits(u, None, action), "{action:?}");
            }
        }
    }

    #[test]
    fn a_stack_scoped_action_with_no_stack_is_refused_to_a_user() {
        assert!(!permits(&user(&["web"]), None, Action::View));
    }

    #[test]
    fn a_user_with_no_stacks_has_no_access() {
        assert!(!has_any_access(&user(&[])));
        assert!(has_any_access(&user(&["web"])));
        assert!(has_any_access(&admin()));
    }

    #[test]
    fn roles_serialise_as_the_registry_writes_them() {
        assert_eq!(serde_json::to_string(&Role::Pending).unwrap(), r#""pending""#);
        let a: Access = serde_json::from_str(r#"{"role":"user","stacks":["web"]}"#).unwrap();
        assert_eq!(a, user(&["web"]));
        assert!(serde_json::from_str::<Access>(r#"{"role":"user","stacks":["../x"]}"#).is_err());
    }
}
