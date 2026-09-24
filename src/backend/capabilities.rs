//! Capability resolver (033-rauthy-oidc-teams-users, US5, research R7).
//!
//! Extends 011's `rbac_access.rs`: answers "can the current identity do X here?" from a cached
//! `SelfSubjectRulesReview` per (context, identity, namespace), falling back to a per-check
//! `SelfSubjectAccessReview` when the rules review is incomplete or the scope is cluster-wide.
//! Fails closed: an unanswerable question is never reported as allowed (Constitution VI).
//! Derived data only, never persisted. Lazy per namespace, no idle polling (Constitution V).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::Sender;
use k8s_openapi::api::authorization::v1::{
    ResourceAttributes, SelfSubjectAccessReview, SelfSubjectAccessReviewSpec,
    SelfSubjectRulesReview, SelfSubjectRulesReviewSpec, SubjectRulesReviewStatus,
};
use kube::api::{Api, PostParams};

use crate::protocol::{
    BackendEvent, BackendEventEnvelope, CapabilityQuery, CapabilityResult, CapabilityRule,
    CapabilitySet, CapabilitySource, Correlation, NamespaceAccessEntry, NamespaceAccessResolved,
    Route, SystemEvent, section_keys,
};

use super::{events, state::BackendState};

/// Cache entries older than this are re-fetched (spec FR-011: 60 s TTL).
pub(super) const CAPABILITY_TTL: Duration = Duration::from_secs(60);

type CacheKey = (String, String, Option<String>);

/// Session-scoped capability cache. Interior mutability so read-only handlers can invalidate.
#[derive(Default)]
pub(super) struct CapabilityCache {
    entries: Mutex<HashMap<CacheKey, (CapabilitySet, Instant)>>,
}

impl CapabilityCache {
    pub(super) fn get(
        &self,
        context: &str,
        identity_sub: &str,
        namespace: Option<&str>,
        now: Instant,
    ) -> Option<CapabilitySet> {
        let map = self.entries.lock().ok()?;
        let key = (
            context.to_owned(),
            identity_sub.to_owned(),
            namespace.map(str::to_owned),
        );
        let (set, at) = map.get(&key)?;
        (now.saturating_duration_since(*at) < CAPABILITY_TTL).then(|| set.clone())
    }

    pub(super) fn put(&self, set: CapabilitySet, now: Instant) {
        if let Ok(mut map) = self.entries.lock() {
            let key = (
                set.context.clone(),
                set.identity_sub.clone(),
                set.namespace.clone(),
            );
            map.insert(key, (set, now));
        }
    }

    /// Identity change, context change, or a successful RBAC apply.
    pub(super) fn invalidate_all(&self) {
        if let Ok(mut map) = self.entries.lock() {
            map.clear();
        }
    }
}

/// Pure: the `sub` of the signed-in identity for `issuer_id`; empty when the context is not an
/// OIDC context or that issuer has no live session (kubeconfig identity).
pub(super) fn sub_for_issuer(
    issuer_id: Option<&str>,
    identities: &[crate::protocol::v1::OidcIdentity],
) -> String {
    let Some(issuer_id) = issuer_id else {
        return String::new();
    };
    identities
        .iter()
        .find(|i| i.issuer_id == issuer_id)
        .map(|i| i.sub.clone())
        .unwrap_or_default()
}

/// Identity `sub` the current connection authenticates as: the OIDC session of the active
/// context's issuer, or empty for kubeconfig identities.
fn identity_sub(state: &BackendState) -> String {
    let issuer = state
        .cluster
        .current_context
        .as_deref()
        .and_then(super::kube::oidc_issuer_for_context);
    sub_for_issuer(issuer.as_deref(), &state.oidc.identities())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn verb_ok(have: &[String], want: &str) -> bool {
    have.iter()
        .any(|v| v == "*" || v.eq_ignore_ascii_case(want))
}

/// Pure: does one rule grant `query`? Rules restricted by `resourceNames` never grant a
/// general (unnamed) capability.
fn rule_grants(rule: &CapabilityRule, query: &CapabilityQuery) -> bool {
    if !rule.resource_names.is_empty() || !verb_ok(&rule.verbs, &query.verb) {
        return false;
    }
    let group = query.api_group.as_deref().unwrap_or("");
    if !rule.api_groups.iter().any(|g| g == "*" || g == group) {
        return false;
    }
    let full = match &query.subresource {
        Some(sub) => format!("{}/{sub}", query.resource),
        None => query.resource.clone(),
    };
    rule.resources.iter().any(|r| {
        r == "*"
            || *r == full
            || query
                .subresource
                .as_ref()
                .is_some_and(|sub| *r == format!("*/{sub}"))
    })
}

/// Pure: answer `query` from a resolved set. `None` means the set cannot answer (incomplete
/// and no rule grants it, or the set is for a different scope) and a review is needed.
pub(super) fn evaluate(set: &CapabilitySet, query: &CapabilityQuery) -> Option<CapabilityResult> {
    if set.context != query.context || set.namespace != query.namespace {
        return None;
    }
    if set.rules.iter().any(|r| rule_grants(r, query)) {
        return Some(CapabilityResult {
            allowed: true,
            reason: None,
            source: CapabilitySource::Rules,
            verified: true,
        });
    }
    if set.incomplete {
        return None;
    }
    Some(CapabilityResult {
        allowed: false,
        reason: Some(format!(
            "no RBAC rule grants '{}' on {}",
            query.verb, query.resource
        )),
        source: CapabilitySource::Rules,
        verified: true,
    })
}

/// Fail-closed result for a check that could not be completed.
fn unverified(reason: String) -> CapabilityResult {
    CapabilityResult {
        allowed: false,
        reason: Some(reason),
        source: CapabilitySource::Review,
        verified: false,
    }
}

/// Convert a `SelfSubjectRulesReview` status into a set. Failure yields an empty incomplete set.
pub(super) fn set_from_status(
    context: &str,
    namespace: Option<&str>,
    identity_sub: &str,
    status: Option<SubjectRulesReviewStatus>,
    fetched_at: u64,
) -> CapabilitySet {
    let (rules, incomplete) = match status {
        // Cluster-scoped bucket: a per-namespace review cannot speak for cluster scope.
        Some(s) if namespace.is_some() => (
            s.resource_rules
                .into_iter()
                .map(|r| CapabilityRule {
                    verbs: r.verbs,
                    api_groups: r.api_groups.unwrap_or_default(),
                    resources: r.resources.unwrap_or_default(),
                    resource_names: r.resource_names.unwrap_or_default(),
                })
                .collect(),
            s.incomplete,
        ),
        _ => (Vec::new(), true),
    };
    CapabilitySet {
        context: context.to_owned(),
        namespace: namespace.map(str::to_owned),
        identity_sub: identity_sub.to_owned(),
        rules,
        incomplete,
        fetched_at,
    }
}

async fn fetch_set(
    state: &BackendState,
    context: &str,
    namespace: Option<&str>,
    sub: &str,
) -> CapabilitySet {
    let status = match (state.cluster.client_or_err(), namespace) {
        (Ok(client), Some(ns)) => {
            let review = SelfSubjectRulesReview {
                metadata: Default::default(),
                spec: SelfSubjectRulesReviewSpec {
                    namespace: Some(ns.to_owned()),
                },
                status: None,
            };
            Api::<SelfSubjectRulesReview>::all(client)
                .create(&PostParams::default(), &review)
                .await
                .ok()
                .and_then(|r| r.status)
        }
        _ => None,
    };
    set_from_status(context, namespace, sub, status, unix_now())
}

fn emit_system(state: &BackendState, sender: &Sender<BackendEventEnvelope>, ev: SystemEvent) {
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(ev),
    );
}

/// `ResolveCapabilities`: cache-first, then one rules review. Failure emits an incomplete,
/// empty set (never allowed-by-default).
pub(super) async fn handle_resolve_capabilities(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: String,
    namespace: Option<String>,
) {
    let sub = identity_sub(state);
    let cached = state
        .capabilities
        .get(&context, &sub, namespace.as_deref(), Instant::now());
    let set = if let Some(set) = cached {
        set
    } else {
        let set = fetch_set(state, &context, namespace.as_deref(), &sub).await;
        // Never cache a failed review: retry on the next request.
        if !(set.incomplete && set.rules.is_empty()) {
            state.capabilities.put(set.clone(), Instant::now());
        }
        set
    };
    emit_system(state, sender, SystemEvent::CapabilitiesResolved(set));
}

/// `CheckCapability`: rules-first, `SelfSubjectAccessReview` fallback, fail closed.
pub(super) async fn handle_check_capability(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    query: CapabilityQuery,
) {
    let result = check(state, &query).await;
    emit_system(
        state,
        sender,
        SystemEvent::CapabilityChecked { query, result },
    );
}

async fn check(state: &BackendState, query: &CapabilityQuery) -> CapabilityResult {
    if state.cluster.current_context.as_deref() != Some(query.context.as_str()) {
        return unverified("the query is for a context that is not connected".to_owned());
    }
    let sub = identity_sub(state);
    let cached = state.capabilities.get(
        &query.context,
        &sub,
        query.namespace.as_deref(),
        Instant::now(),
    );
    if let Some(result) = cached.and_then(|set| evaluate(&set, query)) {
        return result;
    }
    let client = match state.cluster.client_or_err() {
        Ok(c) => c,
        Err(e) => return unverified(format!("{e:#}")),
    };
    let review = SelfSubjectAccessReview {
        metadata: Default::default(),
        spec: SelfSubjectAccessReviewSpec {
            resource_attributes: Some(ResourceAttributes {
                group: query.api_group.clone(),
                resource: Some(query.resource.clone()),
                subresource: query.subresource.clone(),
                verb: Some(query.verb.clone()),
                namespace: query.namespace.clone(),
                ..Default::default()
            }),
            non_resource_attributes: None,
        },
        status: None,
    };
    match Api::<SelfSubjectAccessReview>::all(client)
        .create(&PostParams::default(), &review)
        .await
    {
        Ok(r) => {
            let s = r.status.unwrap_or_default();
            CapabilityResult {
                allowed: s.allowed,
                reason: if s.allowed {
                    None
                } else {
                    s.reason.or(s.evaluation_error)
                },
                source: CapabilitySource::Review,
                verified: true,
            }
        }
        Err(err) => unverified(err.to_string()),
    }
}

// ---- 011/015 access-set queries (shared with the MCP guard) -------------------------------

/// Does `entry` grant `verb` on `resource`? `*` is honoured in both positions, and a
/// `group/resource`-qualified entry matches a bare `resource` request for the core group.
pub(super) fn entry_allows(entry: &NamespaceAccessEntry, verb: &str, resource: &str) -> bool {
    let verb_ok = entry
        .verbs
        .iter()
        .any(|v| v == "*" || v.eq_ignore_ascii_case(verb));
    let resource_ok = entry.resources.iter().any(|r| {
        r == "*" || r == resource || r.rsplit_once('/').is_some_and(|(_, tail)| tail == resource)
    });
    verb_ok && resource_ok
}

/// Is `verb` on `resource` granted by a folded 011 access set? `namespace = None` needs a
/// cluster-wide grant; a namespaced call is allowed by either that or a grant in that namespace.
pub(super) fn access_allows(
    resolved: &NamespaceAccessResolved,
    namespace: Option<&str>,
    verb: &str,
    resource: &str,
) -> bool {
    let cluster_wide_ok = resolved
        .cluster_wide
        .iter()
        .any(|e| entry_allows(e, verb, resource));
    match namespace {
        None => cluster_wide_ok,
        Some(ns) => {
            cluster_wide_ok
                || resolved
                    .per_namespace
                    .iter()
                    .any(|e| e.namespace == ns && entry_allows(e, verb, resource))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::authorization::v1::ResourceRule;

    fn q(ns: Option<&str>, verb: &str, res: &str) -> CapabilityQuery {
        CapabilityQuery {
            context: "c".into(),
            namespace: ns.map(str::to_owned),
            verb: verb.into(),
            resource: res.into(),
            api_group: None,
            subresource: None,
        }
    }

    fn rule(verbs: &[&str], groups: &[&str], res: &[&str]) -> CapabilityRule {
        CapabilityRule {
            verbs: verbs.iter().map(|s| (*s).to_owned()).collect(),
            api_groups: groups.iter().map(|s| (*s).to_owned()).collect(),
            resources: res.iter().map(|s| (*s).to_owned()).collect(),
            resource_names: vec![],
        }
    }

    fn set(rules: Vec<CapabilityRule>, incomplete: bool) -> CapabilitySet {
        CapabilitySet {
            context: "c".into(),
            namespace: Some("dev".into()),
            identity_sub: String::new(),
            rules,
            incomplete,
            fetched_at: 0,
        }
    }

    #[test]
    fn resolves_from_rules_review() {
        let s = set(vec![rule(&["get", "list"], &[""], &["pods"])], false);
        assert!(
            evaluate(&s, &q(Some("dev"), "get", "pods"))
                .unwrap()
                .allowed
        );
        let denied = evaluate(&s, &q(Some("dev"), "delete", "pods")).unwrap();
        assert!(!denied.allowed && denied.verified);
    }

    #[test]
    fn wildcards_and_subresources() {
        let s = set(vec![rule(&["*"], &["apps"], &["deployments/scale"])], false);
        let mut scale = q(Some("dev"), "update", "deployments");
        scale.api_group = Some("apps".into());
        scale.subresource = Some("scale".into());
        assert!(evaluate(&s, &scale).unwrap().allowed);
        let s2 = set(vec![rule(&["create"], &[""], &["*/exec"])], false);
        let mut exec = q(Some("dev"), "create", "pods");
        exec.subresource = Some("exec".into());
        assert!(evaluate(&s2, &exec).unwrap().allowed);
    }

    #[test]
    fn resource_names_never_grant_general_capability() {
        let mut r = rule(&["delete"], &[""], &["pods"]);
        r.resource_names = vec!["only-this".into()];
        let s = set(vec![r], false);
        assert!(
            !evaluate(&s, &q(Some("dev"), "delete", "pods"))
                .unwrap()
                .allowed
        );
    }

    #[test]
    fn incomplete_falls_back_and_never_denies_from_partial_rules() {
        let s = set(vec![rule(&["get"], &[""], &["pods"])], true);
        assert!(
            evaluate(&s, &q(Some("dev"), "get", "pods"))
                .unwrap()
                .allowed
        );
        assert!(evaluate(&s, &q(Some("dev"), "delete", "pods")).is_none());
    }

    #[test]
    fn wrong_scope_is_not_answerable() {
        let s = set(vec![rule(&["*"], &["*"], &["*"])], false);
        assert!(evaluate(&s, &q(Some("other"), "get", "pods")).is_none());
        assert!(evaluate(&s, &q(None, "get", "nodes")).is_none());
    }

    #[test]
    fn failed_review_fails_closed() {
        let s = set_from_status("c", Some("dev"), "", None, 1);
        assert!(s.incomplete && s.rules.is_empty());
        assert!(evaluate(&s, &q(Some("dev"), "get", "pods")).is_none());
        let cluster = set_from_status("c", None, "", Some(Default::default()), 1);
        assert!(cluster.incomplete && cluster.rules.is_empty());
        let u = unverified("x".into());
        assert!(!u.allowed && !u.verified);
    }

    #[test]
    fn status_maps_rules_and_incompleteness() {
        let status = SubjectRulesReviewStatus {
            incomplete: true,
            resource_rules: vec![ResourceRule {
                verbs: vec!["get".into()],
                api_groups: None,
                resources: Some(vec!["pods".into()]),
                resource_names: None,
            }],
            ..Default::default()
        };
        let s = set_from_status("c", Some("dev"), "sub", Some(status), 5);
        assert!(s.incomplete);
        assert_eq!(s.rules[0].resources, vec!["pods"]);
        assert_eq!(s.identity_sub, "sub");
    }

    #[test]
    fn cache_ttl_and_invalidation() {
        let cache = CapabilityCache::default();
        let t0 = Instant::now();
        cache.put(set(vec![], false), t0);
        assert!(cache.get("c", "", Some("dev"), t0).is_some());
        assert!(
            cache.get("c", "", Some("qa"), t0).is_none(),
            "lazy per namespace"
        );
        assert!(cache.get("c", "other-sub", Some("dev"), t0).is_none());
        assert!(
            cache
                .get(
                    "c",
                    "",
                    Some("dev"),
                    t0 + CAPABILITY_TTL + Duration::from_secs(1)
                )
                .is_none()
        );
        cache.invalidate_all();
        assert!(cache.get("c", "", Some("dev"), t0).is_none());
    }

    #[test]
    fn two_identities_get_separate_cache_entries() {
        use crate::protocol::v1::{OidcIdentity, OidcIdentityState};
        let ident = |issuer: &str, sub: &str| OidcIdentity {
            issuer_id: issuer.into(),
            sub: sub.into(),
            username: sub.into(),
            email: None,
            groups: vec![],
            expires_at: 0,
            state: OidcIdentityState::SignedIn,
        };
        let ids = [ident("rauthy", "alice-sub"), ident("other", "bob-sub")];
        assert_eq!(sub_for_issuer(Some("rauthy"), &ids), "alice-sub");
        assert_eq!(sub_for_issuer(Some("other"), &ids), "bob-sub");
        assert_eq!(sub_for_issuer(Some("gone"), &ids), "");
        assert_eq!(sub_for_issuer(None, &ids), "");

        let cache = CapabilityCache::default();
        let now = Instant::now();
        for sub in ["alice-sub", "bob-sub"] {
            let mut s = set(vec![rule(&["get"], &[""], &["pods"])], false);
            s.identity_sub = sub.into();
            s.rules[0].resources = vec![sub.into()];
            cache.put(s, now);
        }
        let a = cache.get("c", "alice-sub", Some("dev"), now).unwrap();
        let b = cache.get("c", "bob-sub", Some("dev"), now).unwrap();
        assert_eq!(a.rules[0].resources, vec!["alice-sub"]);
        assert_eq!(b.rules[0].resources, vec!["bob-sub"]);
        assert!(cache.get("c", "", Some("dev"), now).is_none());
    }
}
