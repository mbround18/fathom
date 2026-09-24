//! RBAC capability types, spec `033-rauthy-oidc-teams-users` (R7). Derived, never persisted.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One "can the current identity do X" question. Cluster-scoped when `namespace` is `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CapabilityQuery {
    pub context: String,
    #[serde(default)]
    pub namespace: Option<String>,
    pub verb: String,
    /// Plural resource name (e.g. `pods`); use `api_group` for non-core groups.
    pub resource: String,
    #[serde(default)]
    pub api_group: Option<String>,
    #[serde(default)]
    pub subresource: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum CapabilitySource {
    /// Answered from a cached `SelfSubjectRulesReview`.
    Rules,
    /// Answered by a `SelfSubjectAccessReview`.
    Review,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CapabilityResult {
    pub allowed: bool,
    /// Verbatim reason from the API server when denied.
    #[serde(default)]
    pub reason: Option<String>,
    pub source: CapabilitySource,
    /// False when the check could not be completed: the UI shows "could not verify" and the
    /// result is never treated as allowed (fail closed).
    pub verified: bool,
}

/// One resource rule from a `SelfSubjectRulesReview` (mirrors `ResourceRule`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
pub struct CapabilityRule {
    #[serde(default)]
    pub verbs: Vec<String>,
    #[serde(default)]
    pub api_groups: Vec<String>,
    #[serde(default)]
    pub resources: Vec<String>,
    #[serde(default)]
    pub resource_names: Vec<String>,
}

/// Resolved rules for one (context, identity, namespace).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CapabilitySet {
    pub context: String,
    /// `None` = cluster-scoped bucket.
    #[serde(default)]
    pub namespace: Option<String>,
    /// Identity `sub` the set was resolved for (empty for kubeconfig identities).
    pub identity_sub: String,
    #[serde(default)]
    pub rules: Vec<CapabilityRule>,
    /// True when the review was incomplete or failed: rules are then partial/empty and every
    /// unlisted verb must fall back to a per-check review or be denied. Never allow-by-default.
    pub incomplete: bool,
    /// Unix seconds.
    pub fetched_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(v: T) {
        let json = serde_json::to_string(&v).unwrap();
        let back: T = serde_json::from_str(&json).unwrap();
        assert_eq!(back, v, "{json}");
    }

    pub(crate) fn query() -> CapabilityQuery {
        CapabilityQuery {
            context: "prod".into(),
            namespace: Some("default".into()),
            verb: "delete".into(),
            resource: "pods".into(),
            api_group: None,
            subresource: Some("exec".into()),
        }
    }

    #[test]
    fn query_result_roundtrip() {
        rt(query());
        rt(CapabilityQuery {
            namespace: None,
            subresource: None,
            api_group: Some("rbac.authorization.k8s.io".into()),
            ..query()
        });
        for source in [CapabilitySource::Rules, CapabilitySource::Review] {
            rt(CapabilityResult {
                allowed: false,
                reason: Some("forbidden".into()),
                source,
                verified: true,
            });
        }
        rt(CapabilityResult {
            allowed: false,
            reason: None,
            source: CapabilitySource::Review,
            verified: false,
        });
    }

    #[test]
    fn set_roundtrip() {
        rt(CapabilitySet {
            context: "prod".into(),
            namespace: None,
            identity_sub: "u1".into(),
            rules: vec![CapabilityRule {
                verbs: vec!["get".into(), "list".into()],
                api_groups: vec![String::new()],
                resources: vec!["pods".into()],
                resource_names: vec![],
            }],
            incomplete: false,
            fetched_at: 1,
        });
        rt(CapabilitySet {
            context: "prod".into(),
            namespace: Some("ns".into()),
            identity_sub: String::new(),
            rules: vec![],
            incomplete: true,
            fetched_at: 2,
        });
    }
}
