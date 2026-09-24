//! Read-only RBAC helpers for 011-rbac-oidc-administration: self-permission gating
//! (FR-007), namespace-access resolution (Story 4), and best-effort kubeadm-cluster
//! detection (Story 5). None of these ever write to the cluster.

use std::collections::{BTreeMap, BTreeSet};

use crossbeam_channel::Sender;
use k8s_openapi::api::authorization::v1::{
    ResourceAttributes, SelfSubjectAccessReview, SelfSubjectAccessReviewSpec,
};
use k8s_openapi::api::core::v1::ConfigMap;
use k8s_openapi::api::rbac::v1::{ClusterRole, ClusterRoleBinding, Role, RoleBinding};
use kube::{
    ResourceExt,
    api::{Api, ListParams, PostParams},
};

use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, KubeadmDetectionResult, NamespaceAccessEntry,
    NamespaceAccessResolved, RbacAccessCheck, RbacAccessResult, ResourceReadEvent, Route,
    section_keys,
};

use super::{events, state::BackendState};

/// Handle `RefreshRbacSelfAccess`: run one `SelfSubjectAccessReview` per requested check
/// against the live API server and report the results. Never fails the whole batch over one
/// check erroring — an erroring check is reported as `allowed: false` with the verbatim error
/// as its reason, per FR-006/FR-007.
pub(super) async fn handle_refresh_rbac_self_access(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    checks: Vec<RbacAccessCheck>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<SelfSubjectAccessReview> = Api::all(client);

    let mut results = Vec::with_capacity(checks.len());
    for check in checks {
        let result = review_access(&api, check).await;
        results.push(result);
    }

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(
            section_keys::access_rolebindings(),
            state.cluster.current_context.clone(),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::RbacSelfAccessUpdated { results }),
    );
    Ok(())
}

/// Run one `SelfSubjectAccessReview` for `check`. An erroring review is reported as
/// `allowed: false` with the verbatim error as its reason.
async fn review_access(
    api: &Api<SelfSubjectAccessReview>,
    check: RbacAccessCheck,
) -> RbacAccessResult {
    let review = SelfSubjectAccessReview {
        metadata: Default::default(),
        spec: SelfSubjectAccessReviewSpec {
            resource_attributes: Some(ResourceAttributes {
                group: Some(check.group.clone().unwrap_or_default()),
                resource: Some(check.resource.clone()),
                verb: Some(check.verb.clone()),
                namespace: check.namespace.clone(),
                ..Default::default()
            }),
            non_resource_attributes: None,
        },
        status: None,
    };
    match api.create(&PostParams::default(), &review).await {
        Ok(reviewed) => {
            let status = reviewed.status.unwrap_or_default();
            RbacAccessResult {
                check,
                allowed: status.allowed,
                reason: status.reason.or(status.evaluation_error),
            }
        }
        Err(err) => RbacAccessResult {
            check,
            allowed: false,
            reason: Some(err.to_string()),
        },
    }
}

/// Handle `CheckResourceMutationAccess` (018): resolve the real group/plural of
/// `api_version`/`kind` via discovery, then review it. A discovery failure is reported as
/// `allowed: false` with the reason, never as a command error.
pub(super) async fn handle_check_resource_mutation_access(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    api_version: String,
    kind: String,
    namespace: Option<String>,
    verb: String,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<SelfSubjectAccessReview> = Api::all(client.clone());
    let result = match super::apply::resolve_api_resource(&client, &api_version, &kind).await {
        Ok(ar) => {
            review_access(
                &api,
                RbacAccessCheck {
                    verb,
                    resource: ar.plural,
                    namespace,
                    group: Some(ar.group),
                },
            )
            .await
        }
        Err(err) => RbacAccessResult {
            check: RbacAccessCheck {
                verb,
                resource: kind.clone(),
                namespace,
                group: None,
            },
            allowed: false,
            reason: Some(err.to_string()),
        },
    };
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(
            section_keys::access_rolebindings(),
            state.cluster.current_context.clone(),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::ResourceMutationAccessChecked {
            api_version,
            kind,
            result,
        }),
    );
    Ok(())
}

/// Handle `ResolveNamespaceAccess`: list the 4 RBAC kinds this feature already reads
/// elsewhere and resolve a subject's effective access from them (Story 4). Read-only.
pub(super) async fn handle_resolve_namespace_access(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    subject_kind: String,
    subject_name: String,
    subject_namespace: Option<String>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;

    let mut degraded_scopes = Vec::new();

    let roles: Vec<Role> = match Api::<Role>::all(client.clone())
        .list(&ListParams::default())
        .await
    {
        Ok(list) => list.items,
        Err(err) => {
            degraded_scopes.push(format!("Roles (all namespaces): {err}"));
            Vec::new()
        }
    };
    let cluster_roles: Vec<ClusterRole> = match Api::<ClusterRole>::all(client.clone())
        .list(&ListParams::default())
        .await
    {
        Ok(list) => list.items,
        Err(err) => {
            degraded_scopes.push(format!("ClusterRoles: {err}"));
            Vec::new()
        }
    };
    let role_bindings: Vec<RoleBinding> = match Api::<RoleBinding>::all(client.clone())
        .list(&ListParams::default())
        .await
    {
        Ok(list) => list.items,
        Err(err) => {
            degraded_scopes.push(format!("RoleBindings (all namespaces): {err}"));
            Vec::new()
        }
    };
    let cluster_role_bindings: Vec<ClusterRoleBinding> =
        match Api::<ClusterRoleBinding>::all(client.clone())
            .list(&ListParams::default())
            .await
        {
            Ok(list) => list.items,
            Err(err) => {
                degraded_scopes.push(format!("ClusterRoleBindings: {err}"));
                Vec::new()
            }
        };

    let mut resolved = resolve_namespace_access(
        &roles,
        &cluster_roles,
        &role_bindings,
        &cluster_role_bindings,
        &subject_kind,
        &subject_name,
        subject_namespace.as_deref(),
    );
    resolved.degraded_scopes.extend(degraded_scopes);

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(
            section_keys::access_namespace_access(),
            state.cluster.current_context.clone(),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::NamespaceAccessResolved(resolved)),
    );
    Ok(())
}

/// Pure resolution algorithm (unit-testable with fixture data, no cluster needed).
///
/// Joins bindings whose `subjects` match `(subject_kind, subject_name, subject_namespace)`
/// against their `roleRef`'s resolved rules. `RoleBinding` matches contribute to
/// `per_namespace`; `ClusterRoleBinding` matches contribute to `cluster_wide` (FR-012 — never
/// enumerated per namespace). A binding whose `roleRef` does not resolve to a known
/// Role/ClusterRole is recorded in `dangling_bindings` and otherwise skipped, without
/// affecting resolution of any other binding (spec.md edge case).
/// Accumulated verbs/resources/contributing-binding-names for one namespace bucket (or the
/// single cluster-wide bucket). Exists solely to keep `resolve_namespace_access` under
/// clippy's line-count/type-complexity limits.
#[derive(Default)]
struct AccessAccumulator {
    verbs: BTreeSet<String>,
    resources: BTreeSet<String>,
    via: BTreeSet<String>,
}

impl AccessAccumulator {
    fn fold(&mut self, verbs: BTreeSet<String>, resources: BTreeSet<String>, binding_ref: String) {
        self.verbs.extend(verbs);
        self.resources.extend(resources);
        self.via.insert(binding_ref);
    }

    fn into_entry(self, namespace: String) -> NamespaceAccessEntry {
        NamespaceAccessEntry {
            namespace,
            verbs: self.verbs.into_iter().collect(),
            resources: self.resources.into_iter().collect(),
            via: self.via.into_iter().collect(),
        }
    }
}

type RoleIndex<'a> = BTreeMap<(String, String), &'a Role>;
type ClusterRoleIndex<'a> = BTreeMap<String, &'a ClusterRole>;

/// The subject a caller is resolving access for. Bundled into one struct purely to keep the
/// fold functions below under clippy's argument-count limit.
struct SubjectQuery<'a> {
    kind: &'a str,
    name: &'a str,
    namespace: Option<&'a str>,
}

fn subject_matches(
    subjects: &Option<Vec<k8s_openapi::api::rbac::v1::Subject>>,
    query: &SubjectQuery<'_>,
) -> bool {
    subjects
        .as_ref()
        .map(|subjects| {
            subjects.iter().any(|s| {
                s.kind == query.kind
                    && s.name == query.name
                    && s.namespace.as_deref() == query.namespace
            })
        })
        .unwrap_or(false)
}

fn fold_role_bindings(
    role_bindings: &[RoleBinding],
    role_index: &RoleIndex<'_>,
    cluster_role_index: &ClusterRoleIndex<'_>,
    query: &SubjectQuery<'_>,
    per_namespace: &mut BTreeMap<String, AccessAccumulator>,
    dangling_bindings: &mut Vec<String>,
) {
    for binding in role_bindings {
        if !subject_matches(&binding.subjects, query) {
            continue;
        }
        let namespace = binding.namespace().unwrap_or_default();
        let binding_ref = format!("RoleBinding/{namespace}/{}", binding.name_any());

        let (verbs, resources) = match binding.role_ref.kind.as_str() {
            "Role" => match role_index.get(&(namespace.clone(), binding.role_ref.name.clone())) {
                Some(role) => rule_sets(role.rules.as_deref()),
                None => {
                    dangling_bindings.push(binding_ref);
                    continue;
                }
            },
            "ClusterRole" => match cluster_role_index.get(&binding.role_ref.name) {
                Some(cr) => rule_sets(cr.rules.as_deref()),
                None => {
                    dangling_bindings.push(binding_ref);
                    continue;
                }
            },
            _ => {
                dangling_bindings.push(binding_ref);
                continue;
            }
        };

        per_namespace
            .entry(namespace)
            .or_default()
            .fold(verbs, resources, binding_ref);
    }
}

fn fold_cluster_role_bindings(
    cluster_role_bindings: &[ClusterRoleBinding],
    cluster_role_index: &ClusterRoleIndex<'_>,
    query: &SubjectQuery<'_>,
    cluster_wide: &mut AccessAccumulator,
    dangling_bindings: &mut Vec<String>,
) {
    for binding in cluster_role_bindings {
        if !subject_matches(&binding.subjects, query) {
            continue;
        }
        let binding_ref = format!("ClusterRoleBinding/{}", binding.name_any());

        let (verbs, resources) = match cluster_role_index.get(&binding.role_ref.name) {
            Some(cr) => rule_sets(cr.rules.as_deref()),
            None => {
                dangling_bindings.push(binding_ref);
                continue;
            }
        };
        cluster_wide.fold(verbs, resources, binding_ref);
    }
}

/// Pure resolution algorithm (unit-testable with fixture data, no cluster needed).
///
/// Joins bindings whose `subjects` match `(subject_kind, subject_name, subject_namespace)`
/// against their `roleRef`'s resolved rules. `RoleBinding` matches contribute to
/// `per_namespace`; `ClusterRoleBinding` matches contribute to `cluster_wide` (FR-012 — never
/// enumerated per namespace). A binding whose `roleRef` does not resolve to a known
/// Role/ClusterRole is recorded in `dangling_bindings` and otherwise skipped, without
/// affecting resolution of any other binding (spec.md edge case).
pub(super) fn resolve_namespace_access(
    roles: &[Role],
    cluster_roles: &[ClusterRole],
    role_bindings: &[RoleBinding],
    cluster_role_bindings: &[ClusterRoleBinding],
    subject_kind: &str,
    subject_name: &str,
    subject_namespace: Option<&str>,
) -> NamespaceAccessResolved {
    let role_index: RoleIndex<'_> = roles
        .iter()
        .map(|r| ((r.namespace().unwrap_or_default(), r.name_any()), r))
        .collect();
    let cluster_role_index: ClusterRoleIndex<'_> =
        cluster_roles.iter().map(|r| (r.name_any(), r)).collect();

    let query = SubjectQuery {
        kind: subject_kind,
        name: subject_name,
        namespace: subject_namespace,
    };

    let mut dangling_bindings = Vec::new();
    let mut per_namespace: BTreeMap<String, AccessAccumulator> = BTreeMap::new();
    let mut cluster_wide_acc = AccessAccumulator::default();

    fold_role_bindings(
        role_bindings,
        &role_index,
        &cluster_role_index,
        &query,
        &mut per_namespace,
        &mut dangling_bindings,
    );
    fold_cluster_role_bindings(
        cluster_role_bindings,
        &cluster_role_index,
        &query,
        &mut cluster_wide_acc,
        &mut dangling_bindings,
    );

    let per_namespace: Vec<NamespaceAccessEntry> = per_namespace
        .into_iter()
        .map(|(namespace, acc)| acc.into_entry(namespace))
        .collect();

    let cluster_wide = if cluster_wide_acc.via.is_empty() {
        Vec::new()
    } else {
        vec![cluster_wide_acc.into_entry(String::new())]
    };

    let found_any = !per_namespace.is_empty() || !cluster_wide.is_empty();

    NamespaceAccessResolved {
        subject_kind: subject_kind.to_owned(),
        subject_name: subject_name.to_owned(),
        subject_namespace: subject_namespace.map(str::to_owned),
        cluster_wide,
        per_namespace,
        dangling_bindings,
        degraded_scopes: Vec::new(),
        found_any,
    }
}

fn rule_sets(
    rules: Option<&[k8s_openapi::api::rbac::v1::PolicyRule]>,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut verbs = BTreeSet::new();
    let mut resources = BTreeSet::new();
    for rule in rules.unwrap_or_default() {
        for verb in &rule.verbs {
            verbs.insert(verb.clone());
        }
        let groups = rule.api_groups.clone().unwrap_or_default();
        let group_labels: Vec<String> = if groups.is_empty() {
            vec![String::new()]
        } else {
            groups
        };
        for res in rule.resources.clone().unwrap_or_default() {
            for group in &group_labels {
                let label = if group.is_empty() {
                    res.clone()
                } else {
                    format!("{group}/{res}")
                };
                resources.insert(label);
            }
        }
    }
    (verbs, resources)
}

/// Handle `RefreshKubeadmDetection`: best-effort read of `kube-system/kubeadm-config`.
/// Never surfaces an error notification — any failure (forbidden, not found, or otherwise)
/// simply yields `checked: false`, since this is advisory framing only (Story 5).
pub(super) async fn handle_refresh_kubeadm_detection(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<ConfigMap> = Api::namespaced(client, "kube-system");

    let result = match api.get("kubeadm-config").await {
        Ok(_) => KubeadmDetectionResult {
            detected: true,
            checked: true,
        },
        Err(kube::Error::Api(resp)) if resp.code == 404 => KubeadmDetectionResult {
            detected: false,
            checked: true,
        },
        Err(_) => KubeadmDetectionResult {
            detected: false,
            checked: false,
        },
    };

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(
            section_keys::access_oidc_config(),
            state.cluster.current_context.clone(),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::KubeadmDetectionUpdated(result)),
    );
    Ok(())
}

/// The fixed self-access probe a hosted backend runs after connecting (spec 031, T033): can
/// this user read any of the core resources the shell shows? Reuses the spec 018
/// `RefreshRbacSelfAccess` review path, so it is a read-only `SelfSubjectAccessReview` batch.
#[must_use]
pub fn hosted_probe_checks() -> Vec<RbacAccessCheck> {
    let check = |group: &str, resource: &str| RbacAccessCheck {
        verb: "list".to_owned(),
        resource: resource.to_owned(),
        namespace: None,
        group: Some(group.to_owned()),
    };
    vec![
        check("", "namespaces"),
        check("", "pods"),
        check("apps", "deployments"),
        check("", "nodes"),
    ]
}

/// Interpret an `RbacSelfAccessUpdated` batch. `None` when it is not the hosted probe's answer
/// (a UI-issued gating batch); `Some(true)` when every probe check was denied, meaning the user
/// has no usable permissions (the "no access" state, SC-008).
#[must_use]
pub fn hosted_probe_verdict(results: &[RbacAccessResult]) -> Option<bool> {
    let probe = hosted_probe_checks();
    let is_probe = results.len() == probe.len()
        && results.iter().zip(&probe).all(|(r, p)| {
            r.check.verb == p.verb
                && r.check.resource == p.resource
                && r.check.namespace == p.namespace
                && r.check.group == p.group
        });
    is_probe.then(|| results.iter().all(|r| !r.allowed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::rbac::v1::{PolicyRule, RoleRef, Subject};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    fn role(namespace: &str, name: &str, verbs: &[&str], resources: &[&str]) -> Role {
        Role {
            metadata: ObjectMeta {
                name: Some(name.to_owned()),
                namespace: Some(namespace.to_owned()),
                ..Default::default()
            },
            rules: Some(vec![PolicyRule {
                api_groups: Some(vec![String::new()]),
                resources: Some(resources.iter().map(|s| s.to_string()).collect()),
                verbs: verbs.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            }]),
        }
    }

    fn cluster_role(name: &str, verbs: &[&str], resources: &[&str]) -> ClusterRole {
        ClusterRole {
            metadata: ObjectMeta {
                name: Some(name.to_owned()),
                ..Default::default()
            },
            rules: Some(vec![PolicyRule {
                api_groups: Some(vec![String::new()]),
                resources: Some(resources.iter().map(|s| s.to_string()).collect()),
                verbs: verbs.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            }]),
            aggregation_rule: None,
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
        role_ref_kind: &str,
        role_ref_name: &str,
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
                kind: role_ref_kind.to_owned(),
                name: role_ref_name.to_owned(),
            },
            subjects: Some(subjects),
        }
    }

    fn cluster_role_binding(
        name: &str,
        role_ref_name: &str,
        subjects: Vec<Subject>,
    ) -> ClusterRoleBinding {
        ClusterRoleBinding {
            metadata: ObjectMeta {
                name: Some(name.to_owned()),
                ..Default::default()
            },
            role_ref: RoleRef {
                api_group: "rbac.authorization.k8s.io".to_owned(),
                kind: "ClusterRole".to_owned(),
                name: role_ref_name.to_owned(),
            },
            subjects: Some(subjects),
        }
    }

    #[test]
    fn namespaced_rolebinding_access_resolves_per_namespace() {
        let roles = vec![role("team-a", "pod-reader", &["get", "list"], &["pods"])];
        let bindings = vec![role_binding(
            "team-a",
            "read-pods",
            "Role",
            "pod-reader",
            vec![group_subject("oidc:readers")],
        )];

        let resolved =
            resolve_namespace_access(&roles, &[], &bindings, &[], "Group", "oidc:readers", None);

        assert!(resolved.found_any);
        assert!(resolved.cluster_wide.is_empty());
        assert_eq!(resolved.per_namespace.len(), 1);
        let entry = &resolved.per_namespace[0];
        assert_eq!(entry.namespace, "team-a");
        assert_eq!(entry.verbs, vec!["get".to_owned(), "list".to_owned()]);
        assert_eq!(entry.resources, vec!["pods".to_owned()]);
        assert!(resolved.dangling_bindings.is_empty());
    }

    #[test]
    fn clusterrolebinding_access_is_marked_cluster_wide_not_per_namespace() {
        let cluster_roles = vec![cluster_role("view", &["get", "list"], &["*"])];
        let crbs = vec![cluster_role_binding(
            "org-viewers",
            "view",
            vec![group_subject("oidc:viewers")],
        )];

        let resolved = resolve_namespace_access(
            &[],
            &cluster_roles,
            &[],
            &crbs,
            "Group",
            "oidc:viewers",
            None,
        );

        assert!(resolved.found_any);
        assert!(resolved.per_namespace.is_empty());
        assert_eq!(resolved.cluster_wide.len(), 1);
        assert_eq!(
            resolved.cluster_wide[0].via,
            vec!["ClusterRoleBinding/org-viewers".to_owned()]
        );
    }

    #[test]
    fn subject_with_no_bindings_yields_found_any_false() {
        let resolved = resolve_namespace_access(&[], &[], &[], &[], "Group", "nobody", None);
        assert!(!resolved.found_any);
        assert!(resolved.per_namespace.is_empty());
        assert!(resolved.cluster_wide.is_empty());
    }

    #[test]
    fn dangling_roleref_is_reported_without_breaking_other_bindings() {
        let roles = vec![role("team-a", "pod-reader", &["get"], &["pods"])];
        let bindings = vec![
            role_binding(
                "team-a",
                "broken",
                "Role",
                "does-not-exist",
                vec![group_subject("oidc:readers")],
            ),
            role_binding(
                "team-a",
                "ok",
                "Role",
                "pod-reader",
                vec![group_subject("oidc:readers")],
            ),
        ];

        let resolved =
            resolve_namespace_access(&roles, &[], &bindings, &[], "Group", "oidc:readers", None);

        assert_eq!(
            resolved.dangling_bindings,
            vec!["RoleBinding/team-a/broken".to_owned()]
        );
        assert_eq!(resolved.per_namespace.len(), 1);
        assert!(resolved.per_namespace[0].verbs.contains(&"get".to_owned()));
    }

    #[test]
    fn serviceaccount_subject_namespace_must_match() {
        let cluster_roles = vec![cluster_role("view", &["get"], &["pods"])];
        let mut sa_subject = group_subject("build-bot");
        sa_subject.kind = "ServiceAccount".to_owned();
        sa_subject.namespace = Some("ci".to_owned());
        let crbs = vec![cluster_role_binding("ci-view", "view", vec![sa_subject])];

        let wrong_ns = resolve_namespace_access(
            &[],
            &cluster_roles,
            &[],
            &crbs,
            "ServiceAccount",
            "build-bot",
            Some("other-ns"),
        );
        assert!(!wrong_ns.found_any);

        let right_ns = resolve_namespace_access(
            &[],
            &cluster_roles,
            &[],
            &crbs,
            "ServiceAccount",
            "build-bot",
            Some("ci"),
        );
        assert!(right_ns.found_any);
    }

    fn probe_results(allowed: &[bool]) -> Vec<RbacAccessResult> {
        hosted_probe_checks()
            .into_iter()
            .zip(allowed)
            .map(|(check, a)| RbacAccessResult {
                check,
                allowed: *a,
                reason: None,
            })
            .collect()
    }

    #[test]
    fn probe_verdict_all_denied_is_zero_permission() {
        let r = probe_results(&[false; 4]);
        assert_eq!(hosted_probe_verdict(&r), Some(true));
    }

    #[test]
    fn probe_verdict_any_allowed_is_access() {
        let r = probe_results(&[false, true, false, false]);
        assert_eq!(hosted_probe_verdict(&r), Some(false));
    }

    #[test]
    fn non_probe_batches_are_ignored() {
        let mut r = probe_results(&[false; 4]);
        r[0].check.resource = "secrets".to_owned();
        assert_eq!(hosted_probe_verdict(&r), None);
        assert_eq!(hosted_probe_verdict(&[]), None);
    }
}
