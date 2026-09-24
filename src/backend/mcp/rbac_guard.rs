//! RBAC scoping for MCP tool calls (015-mcp-agent-exposure, T022, research.md R5).
//!
//! Every MCP tool call — read or mutate — passes through [`check`] before anything touches the
//! cluster. The access set it checks against is produced by
//! [`crate::backend::rbac_access::resolve_namespace_access`], the *same* pure resolver 011's
//! namespace-access view already uses; there is no second authorization model here.
//!
//! [`check`] is deliberately pure: it takes an already-resolved [`NamespaceAccessResolved`] and
//! answers yes/no. That is what makes "denied calls never reach the cluster" testable with
//! fixtures (see this module's tests) — the guard returns an [`McpToolDenial`] and the caller in
//! `tools_read`/`tools_mutate` returns before constructing any `kube::Api`.

use k8s_openapi::api::rbac::v1::{ClusterRole, ClusterRoleBinding, Role, RoleBinding};
use kube::api::{Api, ListParams, PostParams};

use crate::backend::capabilities;
use crate::protocol::{NamespaceAccessEntry, NamespaceAccessResolved};

/// Why an MCP tool call was refused. Mirrors data-model.md's `McpToolDenial`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct McpToolDenial {
    pub(super) tool: String,
    pub(super) reason: String,
    pub(super) category: McpDenialCategory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum McpDenialCategory {
    RbacDenied,
    MutationGuardRejected,
    NoClusterConnected,
    #[allow(dead_code)] // used by Phase 6's demo-chat handler (T044)
    ServerDisabled,
}

impl McpToolDenial {
    pub(super) fn no_cluster(tool: &str) -> Self {
        Self {
            tool: tool.to_owned(),
            reason: "Fathom is not connected to a cluster, so there is nothing to read.".to_owned(),
            category: McpDenialCategory::NoClusterConnected,
        }
    }

    /// The mutation guard refused: the call presented no resource version, a stale one, or a
    /// `uid` from a different object, or named a kind this server will not write. Carries the
    /// tool's own explanation, which always states that nothing was written.
    pub(super) fn mutation_guard(tool: &str, reason: impl Into<String>) -> Self {
        Self {
            tool: tool.to_owned(),
            reason: reason.into(),
            category: McpDenialCategory::MutationGuardRejected,
        }
    }

    /// Fathom could not work out its own permissions (the `SelfSubjectReview` failed, the API
    /// server is unreachable, …). Refused rather than assumed-allowed: an unknown access set is
    /// treated as no access, and nothing is read.
    pub(super) fn rbac_unavailable(tool: &str, detail: &str) -> Self {
        Self {
            tool: tool.to_owned(),
            reason: format!(
                "Fathom could not determine its own cluster permissions ({detail}), so the call \
                 was refused before anything was read."
            ),
            category: McpDenialCategory::RbacDenied,
        }
    }

    /// The single line an attached MCP client sees. Same "what was refused, and why"
    /// phrasing as 011's namespace-access-denial UX.
    pub(super) fn message(&self) -> String {
        format!("{} refused: {}", self.tool, self.reason)
    }
}

/// Gate one tool call on the resolved access of whichever identity this Fathom instance
/// connects to the cluster with.
///
/// `namespace` is `None` for a cluster-wide call (e.g. `list_resources` with no namespace),
/// which requires a cluster-wide grant. A namespaced call is allowed by either a cluster-wide
/// grant or a `RoleBinding`-derived grant in exactly that namespace.
pub(super) fn check(
    resolved: &NamespaceAccessResolved,
    tool: &str,
    namespace: Option<&str>,
    verb: &str,
    resource: &str,
) -> Result<(), McpToolDenial> {
    let allowed = capabilities::access_allows(resolved, namespace, verb, resource);

    if allowed {
        return Ok(());
    }

    let scope = match namespace {
        Some(ns) => format!("namespace '{ns}'"),
        None => "cluster scope".to_owned(),
    };
    Err(McpToolDenial {
        tool: tool.to_owned(),
        reason: format!(
            "Fathom's cluster identity ({} '{}') cannot '{verb}' {resource} in {scope}. \
             Nothing was read from the cluster.",
            resolved.subject_kind, resolved.subject_name
        ),
        category: McpDenialCategory::RbacDenied,
    })
}

/// Resolve the access set for the identity this `client` authenticates as, by asking the API
/// server who we are (`SelfSubjectReview`) and folding the cluster's RBAC objects with
/// [`crate::backend::rbac_access::resolve_namespace_access`] for the user and each of its
/// groups.
///
/// Read-only throughout: the one `create` is a `SelfSubjectReview`, the standard
/// "who am I" subresource, which persists nothing.
pub(super) async fn resolve_current_access(
    client: &kube::Client,
) -> anyhow::Result<NamespaceAccessResolved> {
    use k8s_openapi::api::authentication::v1::SelfSubjectReview;

    let review_api: Api<SelfSubjectReview> = Api::all(client.clone());
    let reviewed = review_api
        .create(&PostParams::default(), &SelfSubjectReview::default())
        .await?;
    let user = reviewed
        .status
        .and_then(|s| s.user_info)
        .unwrap_or_default();
    let username = user
        .username
        .unwrap_or_else(|| "system:anonymous".to_owned());
    let groups = user.groups.unwrap_or_default();

    let roles: Vec<Role> = Api::all(client.clone())
        .list(&ListParams::default())
        .await
        .map(|l| l.items)
        .unwrap_or_default();
    let cluster_roles: Vec<ClusterRole> = Api::all(client.clone())
        .list(&ListParams::default())
        .await
        .map(|l| l.items)
        .unwrap_or_default();
    let role_bindings: Vec<RoleBinding> = Api::all(client.clone())
        .list(&ListParams::default())
        .await
        .map(|l| l.items)
        .unwrap_or_default();
    let cluster_role_bindings: Vec<ClusterRoleBinding> = Api::all(client.clone())
        .list(&ListParams::default())
        .await
        .map(|l| l.items)
        .unwrap_or_default();

    let mut resolved = crate::backend::rbac_access::resolve_namespace_access(
        &roles,
        &cluster_roles,
        &role_bindings,
        &cluster_role_bindings,
        "User",
        &username,
        None,
    );
    for group in &groups {
        let group_access = crate::backend::rbac_access::resolve_namespace_access(
            &roles,
            &cluster_roles,
            &role_bindings,
            &cluster_role_bindings,
            "Group",
            group,
            None,
        );
        merge(&mut resolved, group_access);
    }
    Ok(resolved)
}

/// Union `extra`'s grants into `into`: a subject's effective access is the union of every
/// binding that matches it, whether as a user or via one of its groups.
fn merge(into: &mut NamespaceAccessResolved, extra: NamespaceAccessResolved) {
    for entry in extra.cluster_wide {
        merge_entry(&mut into.cluster_wide, entry);
    }
    for entry in extra.per_namespace {
        merge_entry(&mut into.per_namespace, entry);
    }
    into.found_any |= extra.found_any;
}

fn merge_entry(entries: &mut Vec<NamespaceAccessEntry>, entry: NamespaceAccessEntry) {
    if let Some(existing) = entries.iter_mut().find(|e| e.namespace == entry.namespace) {
        existing.verbs.extend(entry.verbs);
        existing.verbs.sort_unstable();
        existing.verbs.dedup();
        existing.resources.extend(entry.resources);
        existing.resources.sort_unstable();
        existing.resources.dedup();
        existing.via.extend(entry.via);
        existing.via.sort_unstable();
        existing.via.dedup();
    } else {
        entries.push(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::rbac::v1::{PolicyRule, RoleRef, Subject};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    /// Fixtures mirror `rbac_access.rs`'s own test module (`role`, `role_binding`,
    /// `group_subject`) so both suites describe the same cluster shapes.
    fn role(namespace: &str, name: &str, verbs: &[&str], resources: &[&str]) -> Role {
        Role {
            metadata: ObjectMeta {
                name: Some(name.to_owned()),
                namespace: Some(namespace.to_owned()),
                ..Default::default()
            },
            rules: Some(vec![PolicyRule {
                api_groups: Some(vec![String::new()]),
                resources: Some(resources.iter().map(|s| (*s).to_owned()).collect()),
                verbs: verbs.iter().map(|s| (*s).to_owned()).collect(),
                ..Default::default()
            }]),
        }
    }

    fn group_subject(name: &str) -> Subject {
        Subject {
            kind: "Group".to_owned(),
            name: name.to_owned(),
            namespace: None,
            api_group: None,
        }
    }

    fn role_binding(
        namespace: &str,
        name: &str,
        role: &str,
        subjects: Vec<Subject>,
    ) -> RoleBinding {
        RoleBinding {
            metadata: ObjectMeta {
                name: Some(name.to_owned()),
                namespace: Some(namespace.to_owned()),
                ..Default::default()
            },
            role_ref: RoleRef {
                api_group: "rbac.authorization.k8s.io".to_owned(),
                kind: "Role".to_owned(),
                name: role.to_owned(),
            },
            subjects: Some(subjects),
        }
    }

    /// The identity can read pods in `allowed` and has no grant at all in `denied`.
    fn fixture_access() -> NamespaceAccessResolved {
        let roles = vec![role("allowed", "pod-reader", &["get", "list"], &["pods"])];
        let bindings = vec![role_binding(
            "allowed",
            "read-pods",
            "pod-reader",
            vec![group_subject("oidc:readers")],
        )];
        crate::backend::rbac_access::resolve_namespace_access(
            &roles,
            &[],
            &bindings,
            &[],
            "Group",
            "oidc:readers",
            None,
        )
    }

    #[test]
    fn mcp_tool_calls_respect_namespace_rbac() {
        let access = fixture_access();

        // In scope: the namespace the fixture grants pod reads in.
        assert_eq!(
            check(&access, "get_resource", Some("allowed"), "get", "pods"),
            Ok(())
        );

        // Out of scope: same verb/resource, a namespace with no grant. The guard must deny
        // with a specific reason *before* any cluster read is attempted — the caller in
        // `tools_read` returns this denial without constructing a `kube::Api` at all.
        let denial = check(&access, "get_resource", Some("denied"), "get", "pods")
            .expect_err("a namespace with no RoleBinding must be refused");
        assert_eq!(denial.category, McpDenialCategory::RbacDenied);
        assert_eq!(denial.tool, "get_resource");
        assert!(
            denial.reason.contains("denied") && denial.reason.contains("pods"),
            "denial must name the refused namespace and resource: {}",
            denial.reason
        );
        assert!(
            denial.reason.contains("Nothing was read from the cluster"),
            "denial must state that no read happened: {}",
            denial.reason
        );

        // A verb the fixture does not grant is refused even inside the allowed namespace.
        let verb_denial = check(
            &access,
            "delete_resource",
            Some("allowed"),
            "delete",
            "pods",
        )
        .expect_err("'delete' is not in the fixture's verb set");
        assert_eq!(verb_denial.category, McpDenialCategory::RbacDenied);

        // A resource the fixture does not grant is refused too.
        assert!(check(&access, "get_resource", Some("allowed"), "get", "secrets").is_err());

        // A cluster-wide call needs a cluster-wide grant; a RoleBinding is not enough.
        assert!(check(&access, "list_resources", None, "list", "pods").is_err());
    }

    #[test]
    fn cluster_wide_grant_covers_every_namespace_and_the_cluster_scope() {
        let cluster_roles = vec![k8s_openapi::api::rbac::v1::ClusterRole {
            metadata: ObjectMeta {
                name: Some("view".to_owned()),
                ..Default::default()
            },
            rules: Some(vec![PolicyRule {
                api_groups: Some(vec![String::new()]),
                resources: Some(vec!["*".to_owned()]),
                verbs: vec!["get".to_owned(), "list".to_owned()],
                ..Default::default()
            }]),
            aggregation_rule: None,
        }];
        let crbs = vec![ClusterRoleBinding {
            metadata: ObjectMeta {
                name: Some("org-viewers".to_owned()),
                ..Default::default()
            },
            role_ref: RoleRef {
                api_group: "rbac.authorization.k8s.io".to_owned(),
                kind: "ClusterRole".to_owned(),
                name: "view".to_owned(),
            },
            subjects: Some(vec![group_subject("oidc:viewers")]),
        }];
        let access = crate::backend::rbac_access::resolve_namespace_access(
            &[],
            &cluster_roles,
            &[],
            &crbs,
            "Group",
            "oidc:viewers",
            None,
        );

        assert_eq!(
            check(&access, "get_resource", Some("anything"), "get", "pods"),
            Ok(())
        );
        assert_eq!(
            check(&access, "list_resources", None, "list", "pods"),
            Ok(())
        );
        // Still verb-scoped: a read-only ClusterRole does not authorize a write.
        assert!(
            check(
                &access,
                "delete_resource",
                Some("anything"),
                "delete",
                "pods"
            )
            .is_err()
        );
    }

    /// T033 / spec.md US2 acceptance 4: read access is not write access.
    ///
    /// The fixture identity can `get`/`list` pods and deployments in `readonly` — enough for
    /// every read tool — and the *same* namespace, kind and identity are refused the moment the
    /// verb becomes a mutating one. This is the check `tools_mutate`'s handlers make first, so a
    /// mutation call scoped outside what Fathom's own identity may do is refused before the
    /// resource-version guard re-reads anything and before any write is attempted.
    #[test]
    fn mcp_mutation_calls_are_denied_where_only_reads_are_granted() {
        let roles = vec![role(
            "readonly",
            "viewer",
            &["get", "list", "watch"],
            &["pods", "deployments"],
        )];
        let bindings = vec![role_binding(
            "readonly",
            "viewers",
            "viewer",
            vec![group_subject("oidc:readers")],
        )];
        let access = crate::backend::rbac_access::resolve_namespace_access(
            &roles,
            &[],
            &bindings,
            &[],
            "Group",
            "oidc:readers",
            None,
        );

        // The read tools are allowed here — this is genuinely a namespace the identity can see.
        assert_eq!(
            check(
                &access,
                "get_resource",
                Some("readonly"),
                "get",
                "deployments"
            ),
            Ok(())
        );

        // Every mutating verb the mutation tools use is refused in that same namespace.
        for (tool, verb, resource) in [
            ("scale_workload", "update", "deployments"),
            ("apply_resource_yaml", "update", "deployments"),
            ("delete_resource", "delete", "pods"),
        ] {
            let denial = check(&access, tool, Some("readonly"), verb, resource)
                .expect_err("a read-only grant must not authorize a mutation");
            assert_eq!(denial.category, McpDenialCategory::RbacDenied);
            assert_eq!(denial.tool, tool);
            assert!(
                denial
                    .reason
                    .contains(&format!("cannot '{verb}' {resource}")),
                "the refusal must name the verb and resource it refused: {}",
                denial.reason
            );
            assert!(
                denial.reason.contains("Nothing was read from the cluster"),
                "a refusal before the version guard means nothing was touched at all: {}",
                denial.reason
            );
        }

        // And a mutating verb in a namespace with no grant at all is refused too.
        assert!(check(&access, "delete_resource", Some("other"), "delete", "pods").is_err());
    }

    #[test]
    fn a_mutation_guard_denial_is_its_own_category_and_says_nothing_was_written() {
        let denial = McpToolDenial::mutation_guard(
            "scale_workload",
            "Deployment payments/api has changed since it was read. Nothing was written to the \
             cluster.",
        );
        assert_eq!(denial.category, McpDenialCategory::MutationGuardRejected);
        assert!(denial.message().starts_with("scale_workload refused:"));
        assert!(
            denial
                .message()
                .contains("Nothing was written to the cluster")
        );
    }

    #[test]
    fn a_subject_with_no_bindings_is_denied_everything() {
        let access = crate::backend::rbac_access::resolve_namespace_access(
            &[],
            &[],
            &[],
            &[],
            "User",
            "nobody",
            None,
        );
        assert!(!access.found_any);
        assert!(check(&access, "get_resource", Some("default"), "get", "pods").is_err());
        assert!(check(&access, "list_resources", None, "list", "pods").is_err());
    }
}
