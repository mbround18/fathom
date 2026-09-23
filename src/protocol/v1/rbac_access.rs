//! Types for 011-rbac-oidc-administration's read-only self-access-review, namespace-access
//! resolution, and kubeadm-detection commands. All values here are derived/computed — none are
//! persisted, and none of the commands that produce them can mutate the cluster.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One "can I do X" question for [`crate::protocol::BackendCmdV1::RefreshRbacSelfAccess`].
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RbacAccessCheck {
    pub verb: String,
    pub resource: String,
    /// `Some(namespace)` for a namespaced check (e.g. creating a `RoleBinding`), `None` for a
    /// cluster-scoped check (e.g. creating a `ClusterRoleBinding`).
    pub namespace: Option<String>,
}

/// The answer to one [`RbacAccessCheck`], from a live `SelfSubjectAccessReview`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RbacAccessResult {
    pub check: RbacAccessCheck,
    pub allowed: bool,
    /// Verbatim reason string from the review's status, if the API server provided one.
    pub reason: Option<String>,
}

/// Resolved access for one namespace (or the cluster-wide bucket), derived from matching
/// `RoleBinding`/`ClusterRoleBinding` subjects joined against their `roleRef`'s rules.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct NamespaceAccessEntry {
    /// Empty for cluster-wide entries (see [`NamespaceAccessResolved::cluster_wide`]).
    pub namespace: String,
    pub verbs: Vec<String>,
    pub resources: Vec<String>,
    /// Binding names contributing this access, e.g. `"RoleBinding/foo"`.
    pub via: Vec<String>,
}

/// Result of resolving a subject's effective namespace access (Story 4). Never triggers a
/// cluster write; computed entirely from already-readable Role/ClusterRole/RoleBinding/
/// ClusterRoleBinding objects.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct NamespaceAccessResolved {
    pub subject_kind: String,
    pub subject_name: String,
    pub subject_namespace: Option<String>,
    pub cluster_wide: Vec<NamespaceAccessEntry>,
    pub per_namespace: Vec<NamespaceAccessEntry>,
    /// `"RoleBinding/ns/name"` or `"ClusterRoleBinding/name"` whose `roleRef` does not resolve.
    pub dangling_bindings: Vec<String>,
    /// Namespaces/scopes that could not be listed with the operator's current identity.
    pub degraded_scopes: Vec<String>,
    pub found_any: bool,
}

/// Best-effort detection of whether the connected cluster looks kubeadm-managed (Story 5,
/// FR-009 acceptance scenario 1). Never surfaced as an error: an inaccessible or absent
/// `kube-system/kubeadm-config` ConfigMap simply yields `checked: false`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct KubeadmDetectionResult {
    pub detected: bool,
    pub checked: bool,
}
