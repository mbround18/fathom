//! Long-lived read-only watches (FR-003, FR-018): Deployments, StatefulSets, DaemonSets,
//! ReplicaSets, Jobs, CronJobs, HPAs, Pods, Events, plus Nodes and Services for topology.
//! Secrets and ConfigMaps are never watched. Each watch feeds status-diff derivation (event
//! log) and the topology graph; `Init`/`InitApply` after the first list is a baseline and
//! produces no derived events, but a relist after a gap diffs against what was seen before.

use crate::config::DaemonConfig;
use crate::derive::{
    Coalescer, derive_cronjob, derive_daemonset, derive_deployment, derive_hpa, derive_job,
    derive_pod, derive_statefulset, raw_event,
};
use crate::hub::{Hub, SyncTracker};
use crate::topology::Obj;
use chrono::Utc;
use fathom::protocol::v1::{DerivedControllerEvent, TopologyHealth};
use futures::StreamExt;
use k8s_openapi::NamespaceResourceScope;
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::autoscaling::v2::HorizontalPodAutoscaler;
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{Event, Node, Pod, Service};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::runtime::WatchStreamExt;
use kube::runtime::watcher::{self, Event as WEvent};
use kube::{Api, Client, Resource, ResourceExt};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

/// Kinds watched (the `kinds` capability), in FR-003 order.
pub const WATCHED_KINDS: &[&str] = &[
    "Deployment",
    "StatefulSet",
    "DaemonSet",
    "ReplicaSet",
    "Job",
    "CronJob",
    "HorizontalPodAutoscaler",
    "Pod",
    "Event",
];

/// Extra watches used only for topology.
pub const TOPOLOGY_KINDS: &[&str] = &["Node", "Service"];

pub struct Ctx {
    pub hub: Arc<Hub>,
    pub sync: Arc<SyncTracker>,
    pub coalescer: Mutex<Coalescer>,
}

impl Ctx {
    fn emit(&self, ev: DerivedControllerEvent) {
        let now = Utc::now();
        let out = self
            .coalescer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .offer(ev, now);
        if let Some(e) = out {
            self.hub.append_event(e, now);
        }
    }

    /// Flush trailing coalesced counts; call about once a second.
    pub fn flush(&self) {
        let now = Utc::now();
        let due = self
            .coalescer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .flush_due(now);
        for e in due {
            self.hub.append_event(e, now);
        }
    }
}

/// Sync keys the tracker must see before `/readyz` is 200 for this config.
pub fn sync_keys(cfg: &DaemonConfig) -> Vec<String> {
    let mut keys = Vec::new();
    for k in WATCHED_KINDS.iter().chain(TOPOLOGY_KINDS) {
        if cfg.namespaces.is_empty() {
            keys.push(k.to_string());
        } else if *k != "Node" {
            keys.extend(cfg.namespaces.iter().map(|ns| format!("{k}/{ns}")));
        }
    }
    keys
}

fn owners(m: &ObjectMeta) -> Vec<String> {
    m.owner_references
        .iter()
        .flatten()
        .map(|o| o.uid.clone())
        .collect()
}

fn base(kind: &str, m: &ObjectMeta, health: TopologyHealth) -> Option<Obj> {
    Some(Obj {
        uid: m.uid.clone()?,
        kind: kind.into(),
        namespace: m.namespace.clone(),
        name: m.name.clone()?,
        health,
        owners: owners(m),
        labels: BTreeMap::new(),
        selector: None,
        node_name: None,
        scale_target: None,
    })
}

pub fn pod_health(p: &Pod) -> TopologyHealth {
    let Some(st) = p.status.as_ref() else {
        return TopologyHealth::Unknown;
    };
    if st.phase.as_deref() == Some("Failed") {
        return TopologyHealth::Error;
    }
    let bad = st.container_statuses.iter().flatten().any(|c| {
        matches!(
            c.state
                .as_ref()
                .and_then(|s| s.waiting.as_ref())
                .and_then(|w| w.reason.as_deref()),
            Some(
                "CrashLoopBackOff"
                    | "ErrImagePull"
                    | "ImagePullBackOff"
                    | "CreateContainerConfigError"
            )
        )
    });
    if bad {
        return TopologyHealth::Error;
    }
    if st.phase.as_deref() == Some("Succeeded") {
        return TopologyHealth::Ok;
    }
    let ready = st
        .conditions
        .iter()
        .flatten()
        .find(|c| c.type_ == "Ready")
        .map(|c| c.status == "True");
    match ready {
        Some(true) => TopologyHealth::Ok,
        Some(false) => TopologyHealth::Warning,
        None => TopologyHealth::Unknown,
    }
}

fn project_pod(p: &Pod) -> Option<Obj> {
    let mut o = base("Pod", &p.metadata, pod_health(p))?;
    o.labels = p.metadata.labels.clone().unwrap_or_default();
    o.node_name = p.spec.as_ref().and_then(|s| s.node_name.clone());
    Some(o)
}

pub fn workload_health(replicas: i32, ready: i32, failed: bool) -> TopologyHealth {
    if failed {
        TopologyHealth::Error
    } else if ready >= replicas {
        TopologyHealth::Ok
    } else {
        TopologyHealth::Warning
    }
}

fn project_deployment(d: &Deployment) -> Option<Obj> {
    let want = d.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
    let st = d.status.as_ref();
    let failed = st.and_then(|s| s.conditions.as_ref()).is_some_and(|c| {
        c.iter().any(|c| {
            c.type_ == "Progressing" && c.reason.as_deref() == Some("ProgressDeadlineExceeded")
        })
    });
    let avail = st.and_then(|s| s.available_replicas).unwrap_or(0);
    base(
        "Deployment",
        &d.metadata,
        workload_health(want, avail, failed),
    )
}

fn project_sts(s: &StatefulSet) -> Option<Obj> {
    let want = s.spec.as_ref().and_then(|x| x.replicas).unwrap_or(1);
    let ready = s
        .status
        .as_ref()
        .and_then(|x| x.ready_replicas)
        .unwrap_or(0);
    base(
        "StatefulSet",
        &s.metadata,
        workload_health(want, ready, false),
    )
}

fn project_ds(d: &DaemonSet) -> Option<Obj> {
    let st = d.status.as_ref();
    let want = st.map_or(0, |s| s.desired_number_scheduled);
    let ready = st.map_or(0, |s| s.number_ready);
    base(
        "DaemonSet",
        &d.metadata,
        workload_health(want, ready, false),
    )
}

fn project_rs(r: &ReplicaSet) -> Option<Obj> {
    let want = r.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
    let ready = r
        .status
        .as_ref()
        .and_then(|s| s.ready_replicas)
        .unwrap_or(0);
    base(
        "ReplicaSet",
        &r.metadata,
        workload_health(want, ready, false),
    )
}

fn project_job(j: &Job) -> Option<Obj> {
    let failed = j
        .status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .is_some_and(|c| c.iter().any(|c| c.type_ == "Failed" && c.status == "True"));
    base(
        "Job",
        &j.metadata,
        if failed {
            TopologyHealth::Error
        } else {
            TopologyHealth::Ok
        },
    )
}

fn project_cronjob(c: &CronJob) -> Option<Obj> {
    base("CronJob", &c.metadata, TopologyHealth::Ok)
}

fn project_hpa(h: &HorizontalPodAutoscaler) -> Option<Obj> {
    let mut o = base("HorizontalPodAutoscaler", &h.metadata, TopologyHealth::Ok)?;
    let t = h.spec.as_ref().map(|s| &s.scale_target_ref)?;
    o.scale_target = Some((t.kind.clone(), t.name.clone()));
    Some(o)
}

fn project_service(s: &Service) -> Option<Obj> {
    let mut o = base("Service", &s.metadata, TopologyHealth::Ok)?;
    o.selector = Some(
        s.spec
            .as_ref()
            .and_then(|x| x.selector.clone())
            .unwrap_or_default(),
    );
    Some(o)
}

pub fn node_health(n: &Node) -> TopologyHealth {
    match n
        .status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .and_then(|c| c.iter().find(|c| c.type_ == "Ready"))
        .map(|c| c.status.as_str())
    {
        Some("True") => TopologyHealth::Ok,
        Some("False") => TopologyHealth::Error,
        Some(_) => TopologyHealth::Warning,
        None => TopologyHealth::Unknown,
    }
}

fn project_node(n: &Node) -> Option<Obj> {
    base("Node", &n.metadata, node_health(n))
}

type Derive<K> = fn(&K, &K) -> Vec<DerivedControllerEvent>;
type Project<K> = fn(&K) -> Option<Obj>;

fn slim<K: Resource>(mut o: K) -> K {
    o.meta_mut().managed_fields = None;
    o
}

async fn watch_kind<K>(
    api: Api<K>,
    sync_key: String,
    ctx: Arc<Ctx>,
    derive: Option<Derive<K>>,
    project: Project<K>,
) where
    K: Resource<DynamicType = ()>
        + Clone
        + serde::de::DeserializeOwned
        + Debug
        + Send
        + Sync
        + 'static,
{
    let mut seen: HashMap<String, K> = HashMap::new();
    let mut listed: HashSet<String> = HashSet::new();
    let mut stream = watcher::watcher(api, watcher::Config::default())
        .default_backoff()
        .boxed();
    while let Some(ev) = stream.next().await {
        match ev {
            Ok(WEvent::Init) => listed.clear(),
            Ok(WEvent::InitApply(o) | WEvent::Apply(o)) => {
                let o = slim(o);
                let uid = o.uid().unwrap_or_default();
                listed.insert(uid.clone());
                if let (Some(d), Some(prev)) = (derive, seen.get(&uid)) {
                    for e in d(prev, &o) {
                        ctx.emit(e);
                    }
                }
                if let Some(p) = project(&o) {
                    ctx.hub.topology_upsert(p, Utc::now());
                }
                seen.insert(uid, o);
            }
            Ok(WEvent::InitDone) => {
                let gone: Vec<String> = seen
                    .keys()
                    .filter(|u| !listed.contains(*u))
                    .cloned()
                    .collect();
                for u in gone {
                    seen.remove(&u);
                    ctx.hub.topology_remove(&u, Utc::now());
                }
                ctx.sync.mark(&sync_key);
            }
            Ok(WEvent::Delete(o)) => {
                let uid = o.uid().unwrap_or_default();
                seen.remove(&uid);
                ctx.hub.topology_remove(&uid, Utc::now());
            }
            Err(e) => tracing::warn!(kind = %sync_key, error = %e, "watch error (retrying)"),
        }
    }
}

async fn watch_events(api: Api<Event>, sync_key: String, ctx: Arc<Ctx>) {
    // uid -> resourceVersion, so a relist does not re-forward what was already forwarded.
    let mut forwarded: HashMap<String, String> = HashMap::new();
    let mut first_list = true;
    let mut stream = watcher::watcher(api, watcher::Config::default())
        .default_backoff()
        .boxed();
    while let Some(ev) = stream.next().await {
        match ev {
            Ok(WEvent::Init) => {}
            Ok(WEvent::InitApply(e) | WEvent::Apply(e)) => {
                let uid = e.uid().unwrap_or_default();
                let rv = e.resource_version().unwrap_or_default();
                let unseen = forwarded.get(&uid) != Some(&rv);
                forwarded.insert(uid, rv);
                // The first list is history the daemon did not witness: baseline only.
                if unseen
                    && !first_list
                    && let Some(d) = raw_event(&e)
                {
                    ctx.hub.append_event(d, Utc::now());
                }
            }
            Ok(WEvent::InitDone) => {
                first_list = false;
                ctx.sync.mark(&sync_key);
            }
            Ok(WEvent::Delete(e)) => {
                forwarded.remove(&e.uid().unwrap_or_default());
            }
            Err(e) => tracing::warn!(kind = %sync_key, error = %e, "event watch error (retrying)"),
        }
    }
}

fn ns_apis<K>(client: &Client, cfg: &DaemonConfig) -> Vec<(Api<K>, String)>
where
    K: Resource<DynamicType = (), Scope = NamespaceResourceScope>
        + Clone
        + serde::de::DeserializeOwned
        + Debug,
{
    let kind = K::kind(&());
    if cfg.namespaces.is_empty() {
        vec![(Api::all(client.clone()), kind.to_string())]
    } else {
        cfg.namespaces
            .iter()
            .map(|ns| (Api::namespaced(client.clone(), ns), format!("{kind}/{ns}")))
            .collect()
    }
}

fn spawn_kind<K>(
    client: &Client,
    cfg: &DaemonConfig,
    ctx: &Arc<Ctx>,
    derive: Option<Derive<K>>,
    project: Project<K>,
) where
    K: Resource<DynamicType = (), Scope = NamespaceResourceScope>
        + Clone
        + serde::de::DeserializeOwned
        + Debug
        + Send
        + Sync
        + 'static,
{
    for (api, key) in ns_apis::<K>(client, cfg) {
        tokio::spawn(watch_kind(api, key, ctx.clone(), derive, project));
    }
}

/// Start every watch plus the coalescer flush task. Returns immediately; readiness is
/// reported through `ctx.sync`.
pub fn spawn_all(client: Client, cfg: &DaemonConfig, ctx: Arc<Ctx>) {
    spawn_kind::<Deployment>(
        &client,
        cfg,
        &ctx,
        Some(derive_deployment),
        project_deployment,
    );
    spawn_kind::<StatefulSet>(&client, cfg, &ctx, Some(derive_statefulset), project_sts);
    spawn_kind::<DaemonSet>(&client, cfg, &ctx, Some(derive_daemonset), project_ds);
    spawn_kind::<ReplicaSet>(&client, cfg, &ctx, None, project_rs);
    spawn_kind::<Job>(&client, cfg, &ctx, Some(derive_job), project_job);
    spawn_kind::<CronJob>(&client, cfg, &ctx, Some(derive_cronjob), project_cronjob);
    spawn_kind::<HorizontalPodAutoscaler>(&client, cfg, &ctx, Some(derive_hpa), project_hpa);
    spawn_kind::<Pod>(&client, cfg, &ctx, Some(derive_pod), project_pod);
    spawn_kind::<Service>(&client, cfg, &ctx, None, project_service);
    for (api, key) in ns_apis::<Event>(&client, cfg) {
        tokio::spawn(watch_events(api, key, ctx.clone()));
    }
    if cfg.namespaces.is_empty() {
        tokio::spawn(watch_kind(
            Api::<Node>::all(client),
            "Node".into(),
            ctx.clone(),
            None,
            project_node,
        ));
    } else {
        tracing::info!("namespace-scoped mode: Nodes are not watched (cluster-scoped)");
    }
    let flush = ctx;
    tokio::spawn(async move {
        let mut t = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            t.tick().await;
            flush.flush();
        }
    });
}
