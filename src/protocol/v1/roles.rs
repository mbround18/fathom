use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RoleSummary {
    pub name: String,
    pub namespace: String,
    pub rules_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PolicyRuleSummary {
    pub api_groups: Vec<String>,
    pub resources: Vec<String>,
    pub verbs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RoleDetails {
    pub name: String,
    pub namespace: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub rules: Vec<PolicyRuleSummary>,
    pub raw_json: String,
}

/// One resource name an API group serves, for the Role/ClusterRole rules editor's pickers
/// (042-role-create). `name` is the RBAC resource string: the plural (`deployments`) or a
/// subresource (`pods/log`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ApiResourceName {
    pub name: String,
    pub namespaced: bool,
    /// Verbs the API server advertises for this resource (discovery's `verbs`).
    pub verbs: Vec<String>,
}

/// Every resource one API group serves at its preferred version. `group` is `""` for core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ApiGroupResources {
    pub group: String,
    pub resources: Vec<ApiResourceName>,
}

/// Reply for `RefreshApiResources` (042-role-create): the cluster's API discovery, flattened
/// for the rules editor. Read-only. A group whose discovery failed (an unavailable aggregated
/// API, say) is named in `failed_groups` rather than failing the whole catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ApiResourceCatalog {
    pub groups: Vec<ApiGroupResources>,
    pub failed_groups: Vec<String>,
}
