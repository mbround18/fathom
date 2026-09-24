//! Role assignment rendering and orphaned-subject detection (033 US4).
//!
//! Everything here is pure: `preview` renders YAML only (the write is the existing
//! `ApplyResourceYaml`), and `find_orphans` is a read-only join of binding subjects against the
//! IdP directory. Nothing is ever mutated, and nothing is auto-deleted.

use crate::protocol::v1::{
    IdpDirectorySnapshot, OidcIssuerConfig, OrphanedSubject, RoleAssignmentDraft,
    RoleAssignmentSubject, RoleRefKind,
};

/// Result of rendering a draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolePreview {
    pub yaml: String,
    pub rendered_subject_name: String,
    pub warnings: Vec<String>,
}

fn prefix_of(issuer: &OidcIssuerConfig, group: bool) -> Option<&str> {
    let p = if group {
        issuer.groups_prefix.as_deref()
    } else {
        issuer.username_prefix.as_deref()
    };
    p.filter(|p| !p.is_empty())
}

/// `prefix + name`, verbatim (`:` and `/` are legal in subject names; YAML quoting happens at
/// render time, so the name is displayed unescaped). A missing prefix yields a warning.
pub fn render_subject_name(
    issuer: &OidcIssuerConfig,
    subject: &RoleAssignmentSubject,
) -> (String, Option<String>) {
    let (name, group) = match subject {
        RoleAssignmentSubject::User { username } => (username, false),
        RoleAssignmentSubject::Group { name } => (name, true),
    };
    match prefix_of(issuer, group) {
        Some(p) => (format!("{p}{name}"), None),
        None => {
            let claim = if group { "groups" } else { "username" };
            (
                name.clone(),
                Some(format!(
                    "No {claim} prefix is configured for issuer '{}'; the subject is rendered without one. \
                     Set it to match the API server's --oidc-{claim}-prefix or the binding may not match.",
                    issuer.id
                )),
            )
        }
    }
}

fn default_binding_name(draft: &RoleAssignmentDraft, rendered: &str) -> String {
    let slug: String = rendered
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    let mut name = format!("{}-{}", slug, draft.role_ref.name.to_ascii_lowercase());
    name.truncate(63);
    name.trim_end_matches(|c: char| !c.is_ascii_alphanumeric())
        .to_owned()
}

/// Renders the RoleBinding/ClusterRoleBinding YAML. No cluster access.
pub fn preview(issuer: &OidcIssuerConfig, draft: &RoleAssignmentDraft) -> RolePreview {
    let (rendered, warning) = render_subject_name(issuer, &draft.subject);
    let name = draft
        .binding_name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| default_binding_name(draft, &rendered));
    let namespaced = draft.namespace.is_some();
    let mut metadata = serde_json::json!({ "name": name });
    if let Some(ns) = &draft.namespace {
        metadata["namespace"] = serde_json::json!(ns);
    }
    let subject_kind = match draft.subject {
        RoleAssignmentSubject::User { .. } => "User",
        RoleAssignmentSubject::Group { .. } => "Group",
    };
    let role_kind = match draft.role_ref.kind {
        RoleRefKind::Role => "Role",
        RoleRefKind::ClusterRole => "ClusterRole",
    };
    let mut warnings: Vec<String> = warning.into_iter().collect();
    if !namespaced && draft.role_ref.kind == RoleRefKind::Role {
        warnings.push("A ClusterRoleBinding cannot reference a namespaced Role.".into());
    }
    let doc = serde_json::json!({
        "apiVersion": "rbac.authorization.k8s.io/v1",
        "kind": if namespaced { "RoleBinding" } else { "ClusterRoleBinding" },
        "metadata": metadata,
        "roleRef": {
            "apiGroup": "rbac.authorization.k8s.io",
            "kind": role_kind,
            "name": draft.role_ref.name,
        },
        "subjects": [{
            "apiGroup": "rbac.authorization.k8s.io",
            "kind": subject_kind,
            "name": rendered,
        }],
    });
    RolePreview {
        yaml: serde_yaml::to_string(&doc).unwrap_or_default(),
        rendered_subject_name: rendered,
        warnings,
    }
}

/// One subject of an existing binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingSubject {
    /// `"RoleBinding/ns/name"` or `"ClusterRoleBinding/name"`.
    pub binding: String,
    pub kind: String,
    pub name: String,
}

/// Subjects carrying the issuer's prefix whose user/group is absent from the directory. A
/// never-fetched directory yields nothing: absence is not evidence then. Read-only.
pub fn find_orphans(
    issuer: &OidcIssuerConfig,
    directory: &IdpDirectorySnapshot,
    subjects: &[BindingSubject],
) -> Vec<OrphanedSubject> {
    if directory.fetched_at == 0 && directory.users.is_empty() && directory.groups.is_empty() {
        return Vec::new();
    }
    subjects
        .iter()
        .filter_map(|s| {
            let group = match s.kind.as_str() {
                "Group" => true,
                "User" => false,
                _ => return None,
            };
            let bare = s.name.strip_prefix(prefix_of(issuer, group)?)?;
            let present = if group {
                directory.groups.iter().any(|g| g.name == bare)
            } else {
                directory.users.iter().any(|u| u.username == bare)
            };
            (!present).then(|| OrphanedSubject {
                binding: s.binding.clone(),
                subject_name: s.name.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::v1::{IdpGroup, IdpUser, RoleAssignmentRoleRef};

    fn issuer(prefix: Option<&str>) -> OidcIssuerConfig {
        let mut i: OidcIssuerConfig = serde_json::from_value(serde_json::json!({
            "id": "rauthy", "issuer_url": "https://x", "client_id": "c"
        }))
        .unwrap();
        i.username_prefix = prefix.map(Into::into);
        i.groups_prefix = prefix.map(Into::into);
        i
    }

    fn draft(subject: RoleAssignmentSubject, ns: Option<&str>) -> RoleAssignmentDraft {
        RoleAssignmentDraft {
            issuer_id: "rauthy".into(),
            subject,
            role_ref: RoleAssignmentRoleRef {
                kind: RoleRefKind::ClusterRole,
                name: "edit".into(),
            },
            namespace: ns.map(Into::into),
            binding_name: None,
        }
    }

    fn group(n: &str) -> RoleAssignmentSubject {
        RoleAssignmentSubject::Group { name: n.into() }
    }

    #[test]
    fn prefixes_group_and_user() {
        let i = issuer(Some("oidc:"));
        assert_eq!(render_subject_name(&i, &group("dev")).0, "oidc:dev");
        let u = RoleAssignmentSubject::User {
            username: "alice".into(),
        };
        assert_eq!(
            render_subject_name(&i, &u),
            ("oidc:alice".to_string(), None)
        );
    }

    #[test]
    fn missing_prefix_warns_not_guesses() {
        let p = preview(&issuer(None), &draft(group("dev"), Some("ns")));
        assert_eq!(p.rendered_subject_name, "dev");
        assert!(p.warnings.iter().any(|w| w.contains("groups prefix")));
    }

    #[test]
    fn colon_and_slash_survive_yaml_round_trip_and_display_unescaped() {
        let i = issuer(Some("oidc:"));
        let p = preview(&i, &draft(group("team:a/b"), Some("ns")));
        assert_eq!(p.rendered_subject_name, "oidc:team:a/b");
        let v: serde_yaml::Value = serde_yaml::from_str(&p.yaml).unwrap();
        assert_eq!(v["subjects"][0]["name"], "oidc:team:a/b");
        assert_eq!(v["kind"], "RoleBinding");
        assert_eq!(v["metadata"]["namespace"], "ns");
        let name = v["metadata"]["name"].as_str().unwrap();
        assert!(
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        );
    }

    #[test]
    fn cluster_scope_renders_clusterrolebinding_and_warns_on_role() {
        let i = issuer(Some("oidc:"));
        let p = preview(&i, &draft(group("dev"), None));
        let v: serde_yaml::Value = serde_yaml::from_str(&p.yaml).unwrap();
        assert_eq!(v["kind"], "ClusterRoleBinding");
        assert!(v["metadata"].get("namespace").is_none());
        assert!(p.warnings.is_empty());
        let mut d = draft(group("dev"), None);
        d.role_ref.kind = RoleRefKind::Role;
        assert_eq!(preview(&i, &d).warnings.len(), 1);
    }

    fn dir() -> IdpDirectorySnapshot {
        IdpDirectorySnapshot {
            issuer_id: "rauthy".into(),
            users: vec![IdpUser {
                sub: "1".into(),
                username: "alice".into(),
                email: None,
                groups: vec![],
                enabled: true,
            }],
            groups: vec![IdpGroup {
                name: "dev".into(),
                member_count: 1,
            }],
            fetched_at: 5,
            stale: false,
            error: None,
        }
    }

    fn subj(binding: &str, kind: &str, name: &str) -> BindingSubject {
        BindingSubject {
            binding: binding.into(),
            kind: kind.into(),
            name: name.into(),
        }
    }

    #[test]
    fn orphans_only_for_prefixed_missing_subjects() {
        let i = issuer(Some("oidc:"));
        let out = find_orphans(
            &i,
            &dir(),
            &[
                subj("ClusterRoleBinding/a", "Group", "oidc:dev"),
                subj("ClusterRoleBinding/b", "Group", "oidc:gone"),
                subj("ClusterRoleBinding/c", "User", "oidc:bob"),
                subj("ClusterRoleBinding/d", "User", "oidc:alice"),
                subj("ClusterRoleBinding/e", "Group", "system:masters"),
                subj("ClusterRoleBinding/f", "ServiceAccount", "oidc:x"),
            ],
        );
        let ids: Vec<_> = out.iter().map(|o| o.binding.as_str()).collect();
        assert_eq!(ids, ["ClusterRoleBinding/b", "ClusterRoleBinding/c"]);
    }

    #[test]
    fn never_fetched_directory_yields_no_orphans() {
        let mut d = dir();
        d.users.clear();
        d.groups.clear();
        d.fetched_at = 0;
        let i = issuer(Some("oidc:"));
        assert!(find_orphans(&i, &d, &[subj("x", "Group", "oidc:dev")]).is_empty());
    }
}
