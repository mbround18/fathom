//! Per-user authorization (spec 036 R10, FR-017): resolve which namespaces a caller may `list`
//! via `SubjectAccessReview`, cache for <= 60 s (bounded), coalesce concurrent identical
//! resolutions, and fail closed on any error. Every response/frame is filtered with the
//! [`VisibleScope`] produced here.
//!
//! Conservative rule: a namespace is visible only when the caller may `list` **every** watched
//! namespaced kind there. Cluster-scoped objects (Nodes) need cluster-level `list`.

use crate::identity::CallerIdentity;
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use fathom::protocol::v1::{DerivedControllerEvent, VisibleScope};
use futures::stream::{self, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;

/// Watched namespaced kinds as (API group, resource) for `list` checks.
pub const NAMESPACED_RESOURCES: &[(&str, &str)] = &[
    ("apps", "deployments"),
    ("apps", "statefulsets"),
    ("apps", "daemonsets"),
    ("apps", "replicasets"),
    ("batch", "jobs"),
    ("batch", "cronjobs"),
    ("autoscaling", "horizontalpodautoscalers"),
    ("", "pods"),
    ("", "events"),
    ("", "services"),
];

/// Cluster-scoped kinds the daemon exposes.
pub const CLUSTER_RESOURCES: &[(&str, &str)] = &[("", "nodes")];

const SAR_CONCURRENCY: usize = 16;

/// A set of kinds a scope is resolved over. `cache_tag` distinguishes cache slots; the
/// default (all watched kinds) uses the bare identity key.
#[derive(Debug, Clone, Copy)]
pub struct ResourceSet {
    pub namespaced: &'static [(&'static str, &'static str)],
    pub cluster: &'static [(&'static str, &'static str)],
    pub cache_tag: &'static str,
}

/// Every kind the daemon exposes (events, topology, metrics).
pub const ALL_KINDS: ResourceSet = ResourceSet {
    namespaced: NAMESPACED_RESOURCES,
    cluster: CLUSTER_RESOURCES,
    cache_tag: "",
};

/// Only `list pods` (the pods snapshot).
pub const PODS_ONLY: ResourceSet = ResourceSet {
    namespaced: &[("", "pods")],
    cluster: &[],
    cache_tag: "pods",
};

/// Only `list events` (the events snapshot).
pub const EVENTS_ONLY: ResourceSet = ResourceSet {
    namespaced: &[("", "events")],
    cluster: &[],
    cache_tag: "events",
};

/// One `list` access question. `namespace: None` on a namespaced resource means all namespaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessCheck {
    pub group: &'static str,
    pub resource: &'static str,
    pub namespace: Option<String>,
    pub cluster_scoped: bool,
}

#[async_trait]
pub trait AccessReviewer: Send + Sync {
    /// Whether `id` may `list` per `check`. Errors are propagated (fail closed by the caller).
    async fn allowed(&self, id: &CallerIdentity, check: &AccessCheck) -> Result<bool>;
}

/// Real reviewer: creates `SubjectAccessReview` objects as the caller's identity.
pub struct KubeReviewer {
    client: kube::Client,
}

impl KubeReviewer {
    pub fn new(client: kube::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl AccessReviewer for KubeReviewer {
    async fn allowed(&self, id: &CallerIdentity, check: &AccessCheck) -> Result<bool> {
        use k8s_openapi::api::authorization::v1::{
            ResourceAttributes, SubjectAccessReview, SubjectAccessReviewSpec,
        };
        let sar = SubjectAccessReview {
            spec: SubjectAccessReviewSpec {
                user: Some(id.user.clone()),
                groups: Some(id.groups.clone()),
                extra: if id.extra.is_empty() {
                    None
                } else {
                    Some(id.extra.clone().into_iter().collect())
                },
                resource_attributes: Some(ResourceAttributes {
                    verb: Some("list".into()),
                    group: Some(check.group.into()),
                    resource: Some(check.resource.into()),
                    namespace: check.namespace.clone(),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        let api: kube::Api<SubjectAccessReview> = kube::Api::all(self.client.clone());
        let out = api
            .create(&kube::api::PostParams::default(), &sar)
            .await
            .context("SubjectAccessReview")?;
        let st = out
            .status
            .context("SubjectAccessReview returned no status")?;
        if let Some(e) = st.evaluation_error.filter(|e| !e.is_empty()) {
            // A partial evaluation error must never be read as "allowed".
            if !st.allowed {
                anyhow::bail!("SubjectAccessReview evaluation error: {e}");
            }
        }
        Ok(st.allowed)
    }
}

struct Cached {
    at: Instant,
    scope: VisibleScope,
}

type Slot = Arc<tokio::sync::Mutex<Option<Cached>>>;

pub struct ScopeResolver {
    reviewer: Arc<dyn AccessReviewer>,
    ttl: Duration,
    max_entries: usize,
    cache: std::sync::Mutex<HashMap<String, Slot>>,
}

impl ScopeResolver {
    pub fn new(reviewer: Arc<dyn AccessReviewer>, ttl: Duration, max_entries: usize) -> Self {
        Self {
            reviewer,
            ttl,
            max_entries: max_entries.max(1),
            cache: Default::default(),
        }
    }

    pub fn cache_len(&self) -> usize {
        self.cache.lock().map(|c| c.len()).unwrap_or(0)
    }

    fn slot(&self, key: &str) -> Slot {
        let mut c = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(s) = c.get(key) {
            return s.clone();
        }
        if c.len() >= self.max_entries {
            // Drop expired/empty slots first, then the oldest, so the cache stays bounded.
            let now = Instant::now();
            let ttl = self.ttl;
            c.retain(|_, s| match s.try_lock() {
                Ok(g) => g.as_ref().is_some_and(|x| now.duration_since(x.at) < ttl),
                Err(_) => true, // resolution in flight
            });
            while c.len() >= self.max_entries {
                let victim = c
                    .iter()
                    .filter_map(|(k, s)| {
                        s.try_lock()
                            .ok()
                            .and_then(|g| g.as_ref().map(|x| (k.clone(), x.at)))
                    })
                    .min_by_key(|(_, at)| *at)
                    .map(|(k, _)| k);
                match victim {
                    Some(k) => {
                        c.remove(&k);
                    }
                    None => break,
                }
            }
        }
        let s: Slot = Arc::default();
        c.insert(key.to_string(), s.clone());
        s
    }

    /// Resolve the caller's visible scope over `universe` (known namespaces). `Err` => the
    /// caller must return nothing (fail closed); errors are not cached.
    pub async fn resolve(&self, id: &CallerIdentity, universe: &[String]) -> Result<VisibleScope> {
        self.resolve_for(id, universe, &ALL_KINDS).await
    }

    /// Like [`Self::resolve`] for an explicit resource set: a namespace is visible when the
    /// caller may `list` every one of `set.namespaced` there; `cluster_scoped` needs `list` on
    /// every one of `set.cluster` and is `false` when that is empty. The warm snapshots use a
    /// single-kind set so they match what a direct `list` of that kind returns.
    pub async fn resolve_for(
        &self,
        id: &CallerIdentity,
        universe: &[String],
        set: &ResourceSet,
    ) -> Result<VisibleScope> {
        let key = id.key();
        let slot_key = match set.cache_tag {
            "" => key.clone(),
            tag => format!("{key}|{tag}"),
        };
        let slot = self.slot(&slot_key);
        let mut guard = slot.lock().await;
        if let Some(c) = guard.as_ref()
            && Instant::now().duration_since(c.at) < self.ttl
        {
            return Ok(c.scope.clone());
        }
        let scope = self.compute(id, &key, universe, set).await?;
        *guard = Some(Cached {
            at: Instant::now(),
            scope: scope.clone(),
        });
        Ok(scope)
    }

    async fn all_allowed(&self, id: &CallerIdentity, checks: Vec<AccessCheck>) -> Result<bool> {
        let mut s = stream::iter(checks)
            .map(|c| async move { self.reviewer.allowed(id, &c).await })
            .buffer_unordered(SAR_CONCURRENCY);
        let mut all = true;
        // Drain fully so an error anywhere fails closed rather than short-circuiting on `false`.
        while let Some(r) = s.next().await {
            all &= r?;
        }
        Ok(all)
    }

    async fn compute(
        &self,
        id: &CallerIdentity,
        key: &str,
        universe: &[String],
        set: &ResourceSet,
    ) -> Result<VisibleScope> {
        let cluster_checks: Vec<AccessCheck> = set
            .cluster
            .iter()
            .map(|(g, r)| AccessCheck {
                group: g,
                resource: r,
                namespace: None,
                cluster_scoped: true,
            })
            .collect();
        let cluster_scoped =
            !cluster_checks.is_empty() && self.all_allowed(id, cluster_checks).await?;

        let ns_checks = |ns: Option<&str>| -> Vec<AccessCheck> {
            set.namespaced
                .iter()
                .map(|(g, r)| AccessCheck {
                    group: g,
                    resource: r,
                    namespace: ns.map(str::to_string),
                    cluster_scoped: false,
                })
                .collect()
        };

        let mut namespaces = Vec::new();
        if self.all_allowed(id, ns_checks(None)).await? {
            namespaces = universe.to_vec();
        } else {
            for ns in universe {
                if self.all_allowed(id, ns_checks(Some(ns))).await? {
                    namespaces.push(ns.clone());
                }
            }
        }
        namespaces.sort();
        Ok(VisibleScope {
            identity_key: key.to_string(),
            namespaces,
            cluster_scoped,
            resolved_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        })
    }
}

/// The empty scope used when identity is missing/unverified.
pub fn empty_scope() -> VisibleScope {
    VisibleScope {
        identity_key: String::new(),
        namespaces: vec![],
        cluster_scoped: false,
        resolved_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
    }
}

/// `None` namespace = cluster-scoped object.
pub fn namespace_visible(scope: &VisibleScope, namespace: Option<&str>) -> bool {
    match namespace {
        None => scope.cluster_scoped,
        Some(ns) => scope
            .namespaces
            .binary_search_by(|n| n.as_str().cmp(ns))
            .is_ok(),
    }
}

pub fn event_visible(scope: &VisibleScope, e: &DerivedControllerEvent) -> bool {
    namespace_visible(scope, e.namespace.as_deref())
}
