//! Namespace-scoped RBAC (031): listing for an identity that cannot list cluster-wide.
//!
//! A user bound by `RoleBinding`s (edit in `team-a`, view in `default`, nothing cluster-scoped)
//! gets `403 Forbidden` on every `Api::all` list and on `list namespaces`. Fathom must still work
//! for them: namespaced kinds are listed one namespace at a time over the namespaces the user can
//! reach, and a denied cluster-scoped kind is reported as "not permitted" instead of an error.
//!
//! * [`is_forbidden`] / [`anyhow_is_forbidden`]: is an error an RBAC denial?
//! * [`NamespaceScope`]: shared per-connection record of whether namespaces could be listed and,
//!   if not, which namespaces the user reaches (the kubeconfig context's namespace plus any the
//!   user typed into the namespace picker).
//! * [`list_scoped`]: list a namespaced kind over the filter, the reachable namespaces, or the
//!   whole cluster, skipping namespaces that deny the list.
//! * [`ForbiddenLatch`]: log a background poller's denial once, then back off quietly.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures::future::join_all;
use kube::api::{Api, ListParams};

/// Is `err` an RBAC denial (`403 Forbidden`) from the API server?
pub(super) fn is_forbidden(err: &kube::Error) -> bool {
    matches!(err, kube::Error::Api(resp) if resp.code == 403)
}

/// [`is_forbidden`] for an error anywhere in an `anyhow` chain (a `?`-propagated list failure,
/// possibly wrapped in `.context(..)`).
pub(super) fn anyhow_is_forbidden(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<kube::Error>()
            .is_some_and(is_forbidden)
    })
}

/// What the current identity may list, shared by commands and background pollers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ScopeState {
    /// `list namespaces` was denied: namespaced kinds are listed per namespace over
    /// [`ScopeState::reachable`] rather than cluster-wide.
    pub(super) restricted: bool,
    /// Namespaces the user reaches when restricted, sorted and de-duplicated.
    pub(super) reachable: Vec<String>,
}

/// Cheap-to-clone handle on the connection's [`ScopeState`].
#[derive(Clone, Debug, Default)]
pub(super) struct NamespaceScope(Arc<RwLock<ScopeState>>);

impl NamespaceScope {
    pub(super) fn snapshot(&self) -> ScopeState {
        self.0.read().map(|s| s.clone()).unwrap_or_default()
    }

    /// Back to "unknown / unrestricted", for a new connection.
    pub(super) fn reset(&self) {
        if let Ok(mut s) = self.0.write() {
            *s = ScopeState::default();
        }
    }

    /// Record that namespaces cannot be listed; `seed` (the context namespace) is reachable.
    pub(super) fn restrict(&self, seed: &str) {
        if let Ok(mut s) = self.0.write() {
            s.restricted = true;
            add_sorted(&mut s.reachable, seed);
        }
    }

    /// When restricted, remember namespaces the user asked for explicitly (typed into the
    /// picker). Returns true when the reachable set grew.
    pub(super) fn remember(&self, namespaces: &[String]) -> bool {
        let Ok(mut s) = self.0.write() else {
            return false;
        };
        if !s.restricted {
            return false;
        }
        let before = s.reachable.len();
        for ns in namespaces {
            add_sorted(&mut s.reachable, ns);
        }
        s.reachable.len() != before
    }
}

fn add_sorted(list: &mut Vec<String>, ns: &str) {
    let ns = ns.trim();
    if ns.is_empty() {
        return;
    }
    if let Err(at) = list.binary_search_by(|n| n.as_str().cmp(ns)) {
        list.insert(at, ns.to_owned());
    }
}

/// How to list a namespaced kind.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ListPlan {
    ClusterWide,
    PerNamespace(Vec<String>),
}

/// The namespace filter wins; else a restricted identity lists its reachable namespaces; else
/// one cluster-wide list.
pub(super) fn plan(filter: &[String], scope: &ScopeState) -> ListPlan {
    if !filter.is_empty() {
        ListPlan::PerNamespace(filter.to_vec())
    } else if scope.restricted {
        ListPlan::PerNamespace(scope.reachable.clone())
    } else {
        ListPlan::ClusterWide
    }
}

/// Merge per-namespace list results: a namespace that denies the list is skipped, any other
/// error fails the whole list, and the list is denied only when every namespace denied it.
pub(super) fn merge_per_namespace<K>(results: Vec<kube::Result<Vec<K>>>) -> kube::Result<Vec<K>> {
    let mut items = Vec::new();
    let mut denied = None;
    let mut any_ok = false;
    for result in results {
        match result {
            Ok(list) => {
                any_ok = true;
                items.extend(list);
            }
            Err(e) if is_forbidden(&e) => {
                denied.get_or_insert(e);
            }
            Err(e) => return Err(e),
        }
    }
    match denied {
        Some(e) if !any_ok => Err(e),
        _ => Ok(items),
    }
}

/// List `K` per [`plan`]: one list per namespace (concurrently) or one cluster-wide list.
pub(super) async fn list_scoped<K>(
    client: kube::Client,
    filter: &[String],
    scope: &ScopeState,
) -> kube::Result<Vec<K>>
where
    K: kube::Resource<Scope = k8s_openapi::NamespaceResourceScope>
        + Clone
        + serde::de::DeserializeOwned
        + std::fmt::Debug,
    K::DynamicType: Default,
{
    list_scoped_with(client, filter, scope, &ListParams::default()).await
}

/// [`list_scoped`] with explicit `params` (a label or field selector).
pub(super) async fn list_scoped_with<K>(
    client: kube::Client,
    filter: &[String],
    scope: &ScopeState,
    params: &ListParams,
) -> kube::Result<Vec<K>>
where
    K: kube::Resource<Scope = k8s_openapi::NamespaceResourceScope>
        + Clone
        + serde::de::DeserializeOwned
        + std::fmt::Debug,
    K::DynamicType: Default,
{
    match plan(filter, scope) {
        ListPlan::ClusterWide => Ok(Api::<K>::all(client).list(params).await?.items),
        ListPlan::PerNamespace(namespaces) => {
            let lists = namespaces.iter().map(|ns| {
                let api = Api::<K>::namespaced(client.clone(), ns);
                async move { api.list(params).await.map(|l| l.items) }
            });
            merge_per_namespace(join_all(lists).await)
        }
    }
}

/// [`list_scoped`] for a namespaced dynamic kind (a CRD such as the VPA).
pub(super) async fn list_scoped_dynamic(
    client: kube::Client,
    resource: &kube::discovery::ApiResource,
    filter: &[String],
    scope: &ScopeState,
) -> kube::Result<Vec<kube::api::DynamicObject>> {
    let params = ListParams::default();
    match plan(filter, scope) {
        ListPlan::ClusterWide => Ok(Api::all_with(client, resource).list(&params).await?.items),
        ListPlan::PerNamespace(namespaces) => {
            let lists = namespaces.iter().map(|ns| {
                let api = Api::namespaced_with(client.clone(), ns, resource);
                let params = &params;
                async move { api.list(params).await.map(|l| l.items) }
            });
            merge_per_namespace(join_all(lists).await)
        }
    }
}

/// Per-connection helper for a background poller: the first denial is logged, later ones are
/// silent, and the poll interval stretches to [`ForbiddenLatch::BACKOFF`] while denied.
#[derive(Debug, Default)]
pub(super) struct ForbiddenLatch {
    denied: bool,
}

impl ForbiddenLatch {
    pub(super) const BACKOFF: Duration = Duration::from_secs(300);

    /// Record a failed poll. Returns true when the error should be surfaced (a non-RBAC error);
    /// a denial is logged once and never surfaced.
    pub(super) fn on_error(&mut self, what: &str, err: &anyhow::Error) -> bool {
        if !anyhow_is_forbidden(err) {
            return true;
        }
        if !self.denied {
            tracing::info!("{what}: not permitted for this identity; backing off ({err:#})");
        }
        self.denied = true;
        false
    }

    pub(super) fn on_ok(&mut self) {
        self.denied = false;
    }

    /// The delay before the next poll: `normal`, or the backoff while denied.
    pub(super) fn next_delay(&self, normal: Duration) -> Duration {
        if self.denied { Self::BACKOFF } else { normal }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api_err(code: u16) -> kube::Error {
        kube::Error::Api(kube::core::ErrorResponse {
            status: "Failure".into(),
            message: "denied".into(),
            reason: if code == 403 { "Forbidden" } else { "Other" }.into(),
            code,
        })
    }

    #[test]
    fn forbidden_is_detected_directly_and_through_anyhow_context() {
        assert!(is_forbidden(&api_err(403)));
        assert!(!is_forbidden(&api_err(404)));
        let wrapped = anyhow::Error::from(api_err(403)).context("Failed to list nodes");
        assert!(anyhow_is_forbidden(&wrapped));
        assert!(!anyhow_is_forbidden(&anyhow::anyhow!("403 in a string")));
        assert!(!anyhow_is_forbidden(&anyhow::Error::from(api_err(500))));
    }

    #[test]
    fn plan_prefers_filter_then_reachable_then_cluster() {
        let open = ScopeState::default();
        let restricted = ScopeState {
            restricted: true,
            reachable: vec!["a".into(), "b".into()],
        };
        assert_eq!(plan(&[], &open), ListPlan::ClusterWide);
        assert_eq!(
            plan(&[], &restricted),
            ListPlan::PerNamespace(vec!["a".into(), "b".into()])
        );
        assert_eq!(
            plan(&["x".into()], &restricted),
            ListPlan::PerNamespace(vec!["x".into()])
        );
        assert_eq!(
            plan(&["x".into()], &open),
            ListPlan::PerNamespace(vec!["x".into()])
        );
    }

    #[test]
    fn merge_skips_denied_namespaces() {
        let merged = merge_per_namespace(vec![Ok(vec![1]), Err(api_err(403)), Ok(vec![2, 3])]);
        assert_eq!(merged.unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn merge_is_denied_only_when_every_namespace_denies() {
        let merged = merge_per_namespace::<u8>(vec![Err(api_err(403)), Err(api_err(403))]);
        assert!(merged.is_err_and(|e| is_forbidden(&e)));
        assert_eq!(merge_per_namespace::<u8>(vec![]).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn merge_fails_on_a_non_rbac_error() {
        let merged = merge_per_namespace(vec![Ok(vec![1]), Err(api_err(500))]);
        assert!(merged.is_err_and(|e| !is_forbidden(&e)));
    }

    #[test]
    fn scope_remembers_typed_namespaces_only_when_restricted() {
        let scope = NamespaceScope::default();
        assert!(!scope.remember(&["demo".into()]));
        assert!(!scope.snapshot().restricted);
        scope.restrict("team-a");
        assert!(scope.remember(&["demo".into(), "team-a".into()]));
        assert!(!scope.remember(&["demo".into()]));
        assert_eq!(scope.snapshot().reachable, vec!["demo", "team-a"]);
        scope.reset();
        assert_eq!(scope.snapshot(), ScopeState::default());
    }

    #[test]
    fn latch_logs_once_and_backs_off_while_denied() {
        let mut latch = ForbiddenLatch::default();
        let normal = Duration::from_secs(20);
        let denied = anyhow::Error::from(api_err(403));
        assert!(!latch.on_error("scan", &denied));
        assert_eq!(latch.next_delay(normal), ForbiddenLatch::BACKOFF);
        assert!(latch.on_error("scan", &anyhow::anyhow!("boom")));
        latch.on_ok();
        assert_eq!(latch.next_delay(normal), normal);
    }
}
