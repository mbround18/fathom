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
    /// Reveal masked `.env` values. Admin-only: a user edits `.env` with its secrets masked, and
    /// the save keeps the value of every line still showing the mask.
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
    /// Host metrics (`WatchHost`): CPU, memory, disk and load of the machine Depths runs on.
    ViewHost,
}

impl Action {
    /// Every action, for tables and tests.
    pub const ALL: [Self; 12] = [
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
        Self::ViewHost,
    ];

    /// Whether a `user` may take this action on a stack assigned to them. The rest are admin-only.
    pub fn open_to_users(self) -> bool {
        matches!(
            self,
            Self::View | Self::Lifecycle | Self::Shell | Self::Edit
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
        Role::User => action.open_to_users() && stack.is_some_and(|s| access.stacks.contains(s)),
    }
}

/// What a command or route does, as finely as the network gate needs (spec 052 R4). Finer than
/// [`Action`]: `Lifecycle` covers both `restart` (allowed from an untrusted network) and `pull`
/// (not).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    /// Lists, stack detail and masked files.
    View,
    /// Log follows, stats and host watches, and closing them.
    Subscribe,
    /// `up`, and a service's `up`.
    Start,
    Stop,
    Restart,
    /// `down`, with or without volumes or images.
    Down,
    Pull,
    Update,
    Delete,
    RevealEnv,
    /// Saving a stack's files.
    Edit,
    /// Opening a shell, and its input and resizes.
    Shell,
    ReadAudit,
    ManageUsers,
    /// Whitelisting or forgetting someone's networks.
    ManageNetworks,
    /// Answering "home or public" for the network in use.
    AnswerNetwork,
    /// Ending one's own sessions, or (admin) someone else's.
    EndSession,
    /// Listing a stack's mounts and a folder in one (spec 053 `ListMounts`, `ListDir`).
    FilesList,
    /// Reading a volume file's contents: `ReadFile`, `FolderSize`, `ListHistory`, `ReadVersion`
    /// and the download route.
    FilesRead,
    /// Changing a volume: `PreviewFileOp` and its confirm, and the upload route.
    FilesWrite,
}

impl CommandKind {
    pub const ALL: [Self; 20] = [
        Self::View,
        Self::Subscribe,
        Self::Start,
        Self::Stop,
        Self::Restart,
        Self::Down,
        Self::Pull,
        Self::Update,
        Self::Delete,
        Self::RevealEnv,
        Self::Edit,
        Self::Shell,
        Self::ReadAudit,
        Self::ManageUsers,
        Self::ManageNetworks,
        Self::AnswerNetwork,
        Self::EndSession,
        Self::FilesList,
        Self::FilesRead,
        Self::FilesWrite,
    ];

    /// The [`Action`] the volume explorer's kinds need: [`Action::Edit`] for all three, so a
    /// viewer gets no Files tab and none of its commands (spec 053 FR-017). `None` for every
    /// other kind, whose action the server picks per command.
    pub fn files_action(self) -> Option<Action> {
        match self {
            Self::FilesList | Self::FilesRead | Self::FilesWrite => Some(Action::Edit),
            _ => None,
        }
    }
}

/// Whether `kind` is allowed from an untrusted network: the restricted view (spec 052 FR-010).
/// Viewing, start/stop/restart for maintenance, answering the question and ending sessions,
/// and listing a volume's folders (never its file contents, spec 053 FR-018); nothing else.
/// This is checked after [`permits`], never instead of it.
pub fn restricted_ok(kind: CommandKind) -> bool {
    use CommandKind as K;
    match kind {
        K::View
        | K::Subscribe
        | K::Start
        | K::Stop
        | K::Restart
        | K::AnswerNetwork
        | K::EndSession
        | K::FilesList => true,
        K::Down
        | K::Pull
        | K::Update
        | K::Delete
        | K::RevealEnv
        | K::Edit
        | K::Shell
        | K::ReadAudit
        | K::ManageUsers
        | K::ManageNetworks
        | K::FilesRead
        | K::FilesWrite => false,
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
    const TABLE: [(Action, bool, bool); 12] = [
        (Action::View, true, true),
        (Action::RevealEnv, false, true),
        (Action::Lifecycle, true, true),
        (Action::Shell, true, true),
        (Action::Edit, true, true),
        (Action::DownDestructive, false, true),
        (Action::CreateStack, false, true),
        (Action::DeleteStack, false, true),
        (Action::ViewForeign, false, true),
        (Action::ReadAudit, false, true),
        (Action::ManageUsers, false, true),
        (Action::ViewHost, false, true),
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

    /// spec 052 FR-010: (kind, allowed from an untrusted network).
    const RESTRICTED: [(CommandKind, bool); 20] = [
        (CommandKind::View, true),
        (CommandKind::Subscribe, true),
        (CommandKind::Start, true),
        (CommandKind::Stop, true),
        (CommandKind::Restart, true),
        (CommandKind::Down, false),
        (CommandKind::Pull, false),
        (CommandKind::Update, false),
        (CommandKind::Delete, false),
        (CommandKind::RevealEnv, false),
        (CommandKind::Edit, false),
        (CommandKind::Shell, false),
        (CommandKind::ReadAudit, false),
        (CommandKind::ManageUsers, false),
        (CommandKind::ManageNetworks, false),
        (CommandKind::AnswerNetwork, true),
        (CommandKind::EndSession, true),
        (CommandKind::FilesList, true),
        (CommandKind::FilesRead, false),
        (CommandKind::FilesWrite, false),
    ];

    const FILES: [CommandKind; 3] = [
        CommandKind::FilesList,
        CommandKind::FilesRead,
        CommandKind::FilesWrite,
    ];

    #[test]
    fn the_files_kinds_need_edit_and_nothing_else_names_an_action() {
        for kind in CommandKind::ALL {
            let want = FILES.contains(&kind).then_some(Action::Edit);
            assert_eq!(kind.files_action(), want, "{kind:?}");
        }
    }

    #[test]
    fn a_viewer_is_refused_every_files_kind() {
        let viewers = [
            Access {
                role: Role::Pending,
                stacks: [name("web")].into(),
            },
            user(&["db"]),
            user(&[]),
        ];
        for kind in FILES {
            let action = kind.files_action().unwrap();
            for v in &viewers {
                assert!(!permits(v, Some(&name("web")), action), "{kind:?} {v:?}");
            }
            assert!(permits(&user(&["web"]), Some(&name("web")), action));
            assert!(permits(&admin(), Some(&name("web")), action));
        }
    }

    #[test]
    fn only_listing_files_is_allowed_when_restricted() {
        let ok: Vec<CommandKind> = FILES.into_iter().filter(|k| restricted_ok(*k)).collect();
        assert_eq!(ok, vec![CommandKind::FilesList]);
    }

    #[test]
    fn the_restricted_table_covers_every_kind_and_matches() {
        let rows: Vec<CommandKind> = RESTRICTED.iter().map(|(k, _)| *k).collect();
        assert_eq!(rows, CommandKind::ALL.to_vec());
        for (kind, ok) in RESTRICTED {
            assert_eq!(restricted_ok(kind), ok, "{kind:?}");
        }
    }

    #[test]
    fn roles_serialise_as_the_registry_writes_them() {
        assert_eq!(
            serde_json::to_string(&Role::Pending).unwrap(),
            r#""pending""#
        );
        let a: Access = serde_json::from_str(r#"{"role":"user","stacks":["web"]}"#).unwrap();
        assert_eq!(a, user(&["web"]));
        assert!(serde_json::from_str::<Access>(r#"{"role":"user","stacks":["../x"]}"#).is_err());
    }
}
