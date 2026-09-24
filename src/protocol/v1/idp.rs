//! IdP directory, team-link and role-assignment types, spec `033-rauthy-oidc-teams-users`.
//! Read-only against the IdP: nothing here writes users, groups or memberships (R5).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct IdpUser {
    pub sub: String,
    pub username: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub groups: Vec<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct IdpGroup {
    pub name: String,
    pub member_count: u32,
}

/// Cached read of the IdP directory. On failure the last good data is kept and `stale` is set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct IdpDirectorySnapshot {
    pub issuer_id: String,
    #[serde(default)]
    pub users: Vec<IdpUser>,
    #[serde(default)]
    pub groups: Vec<IdpGroup>,
    /// Unix seconds of the last successful fetch (0 = never).
    pub fetched_at: u64,
    pub stale: bool,
    /// Verbatim failure (401/403/unreachable) when the latest refresh failed.
    #[serde(default)]
    pub error: Option<String>,
}

/// Advisory group -> shared-profile link (R8), stored additively in the 014 team store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TeamLink {
    pub group: String,
    pub profile_id: String,
}

/// Who a role assignment targets. The rendered subject name applies the issuer's prefix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum RoleAssignmentSubject {
    User { username: String },
    Group { name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum RoleRefKind {
    Role,
    ClusterRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct RoleAssignmentRoleRef {
    pub kind: RoleRefKind,
    pub name: String,
}

/// Input for `PreviewRoleAssignment`. Rendering only; the write is `ApplyResourceYaml`.
/// `namespace` `Some` renders a `RoleBinding`, `None` a `ClusterRoleBinding`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct RoleAssignmentDraft {
    /// Issuer whose claim/prefix mapping renders the subject name.
    pub issuer_id: String,
    pub subject: RoleAssignmentSubject,
    pub role_ref: RoleAssignmentRoleRef,
    #[serde(default)]
    pub namespace: Option<String>,
    /// Defaults to a generated name when absent.
    #[serde(default)]
    pub binding_name: Option<String>,
}

/// A binding subject that names a user/group absent from the IdP directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct OrphanedSubject {
    /// `"RoleBinding/ns/name"` or `"ClusterRoleBinding/name"`.
    pub binding: String,
    pub subject_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(v: T) {
        let json = serde_json::to_string(&v).unwrap();
        let back: T = serde_json::from_str(&json).unwrap();
        assert_eq!(back, v, "{json}");
    }

    #[test]
    fn directory_roundtrip() {
        rt(IdpDirectorySnapshot {
            issuer_id: "rauthy".into(),
            users: vec![IdpUser {
                sub: "u1".into(),
                username: "alice".into(),
                email: None,
                groups: vec!["fathom-viewers".into()],
                enabled: true,
            }],
            groups: vec![IdpGroup {
                name: "fathom-viewers".into(),
                member_count: 1,
            }],
            fetched_at: 10,
            stale: true,
            error: Some("HTTP 403".into()),
        });
        rt(TeamLink {
            group: "platform".into(),
            profile_id: "p1".into(),
        });
    }

    #[test]
    fn role_assignment_roundtrip() {
        for subject in [
            RoleAssignmentSubject::User {
                username: "a:b/c".into(),
            },
            RoleAssignmentSubject::Group { name: "dev".into() },
        ] {
            for kind in [RoleRefKind::Role, RoleRefKind::ClusterRole] {
                rt(RoleAssignmentDraft {
                    issuer_id: "rauthy".into(),
                    subject: subject.clone(),
                    role_ref: RoleAssignmentRoleRef {
                        kind,
                        name: "view".into(),
                    },
                    namespace: Some("default".into()),
                    binding_name: None,
                });
            }
        }
        rt(OrphanedSubject {
            binding: "ClusterRoleBinding/x".into(),
            subject_name: "oidc:gone".into(),
        });
    }
}
