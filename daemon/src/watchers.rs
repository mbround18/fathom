//! Long-lived read-only watches (FR-003, FR-018): Deployments, StatefulSets, DaemonSets,
//! ReplicaSets, Jobs, CronJobs, HPAs, Pods, Events, plus Nodes and Services for topology.
//! Secrets and ConfigMaps are never watched. Each watch feeds status-diff derivation (event
//! log) and the topology graph; `Init`/`InitApply` after the first list is a baseline and
//! produces no derived events, but a relist after a gap diffs against what was seen before.

use crate::authz::{EVENTS_ONLY, PODS_ONLY};
use crate::config::DaemonConfig;
use crate::derive::{
    Coalescer, derive_cronjob, derive_daemonset, derive_deployment, derive_hpa, derive_job,
    derive_pod, derive_statefulset, raw_event,
};
use crate::hub::{Hub, SyncTracker};
use crate::snapshot::{SnapshotSource, SnapshotStore};
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
use kube::api::{ListParams, WatchEvent, WatchParams};
use kube::runtime::WatchStreamExt;
use kube::runtime::watcher::{self, Event as WEvent};
use kube::{Api, Client, Resource, ResourceExt};
use serde::Serialize;
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

/// One watch event, from either `kube::runtime::watcher` or [`list_watch`] (which also
/// surfaces bookmarks and watch (re)establishment, needed by the warm snapshots).
#[derive(Debug)]
pub enum Ev<K> {
    /// A (re)list starts.
    Init,
    InitApply(K),
    /// The (re)list finished, at this list resourceVersion when known.
    InitDone(Option<String>),
    Apply(K),
    Delete(K),
    Bookmark(String),
    /// A watch connection is open again.
    Established,
    Error(String),
}

impl<K> From<WEvent<K>> for Ev<K> {
    fn from(e: WEvent<K>) -> Self {
        match e {
            WEvent::Init => Ev::Init,
            WEvent::InitApply(o) => Ev::InitApply(o),
            WEvent::InitDone => Ev::InitDone(None),
            WEvent::Apply(o) => Ev::Apply(o),
            WEvent::Delete(o) => Ev::Delete(o),
        }
    }
}

const LIST_PAGE: u32 = 500;
const RETRY_MIN: std::time::Duration = std::time::Duration::from_millis(500);
const RETRY_MAX: std::time::Duration = std::time::Duration::from_secs(30);

fn is_gone(e: &kube::Error) -> bool {
    matches!(e, kube::Error::Api(s) if s.code == 410)
}

/// Paginated LIST, then WATCH from the list's resourceVersion with bookmarks, re-watching from
/// the last seen version when a watch ends and relisting on `410 Gone`. Every event is handed
/// to `on` in API-server order. Never returns.
pub async fn list_watch<K>(api: Api<K>, mut on: impl FnMut(Ev<K>))
where
    K: Resource + Clone + serde::de::DeserializeOwned + Debug + Send + 'static,
{
    let mut backoff = RETRY_MIN;
    let pause = |backoff: &mut std::time::Duration| {
        let d = *backoff;
        *backoff = (*backoff * 2).min(RETRY_MAX);
        tokio::time::sleep(d)
    };
    'relist: loop {
        on(Ev::Init);
        let mut lp = ListParams::default().limit(LIST_PAGE);
        let list_rv = loop {
            match api.list(&lp).await {
                Ok(l) => {
                    for o in l.items {
                        on(Ev::InitApply(o));
                    }
                    match l.metadata.continue_.filter(|c| !c.is_empty()) {
                        Some(c) => lp = lp.continue_token(&c),
                        None => break l.metadata.resource_version,
                    }
                }
                Err(e) => {
                    on(Ev::Error(format!("list: {e}")));
                    pause(&mut backoff).await;
                    continue 'relist;
                }
            }
        };
        on(Ev::InitDone(list_rv.clone()));
        let mut rv = list_rv.unwrap_or_default();
        loop {
            let stream = match api.watch(&WatchParams::default().timeout(290), &rv).await {
                Ok(s) => s,
                Err(e) => {
                    on(Ev::Error(format!("watch: {e}")));
                    pause(&mut backoff).await;
                    if is_gone(&e) {
                        continue 'relist;
                    }
                    continue;
                }
            };
            on(Ev::Established);
            backoff = RETRY_MIN;
            let mut stream = std::pin::pin!(stream);
            while let Some(ev) = stream.next().await {
                match ev {
                    Ok(WatchEvent::Added(o) | WatchEvent::Modified(o)) => {
                        if let Some(v) = o.resource_version() {
                            rv = v;
                        }
                        on(Ev::Apply(o));
                    }
                    Ok(WatchEvent::Deleted(o)) => {
                        if let Some(v) = o.resource_version() {
                            rv = v;
                        }
                        on(Ev::Delete(o));
                    }
                    Ok(WatchEvent::Bookmark(b)) => {
                        rv = b.metadata.resource_version.clone();
                        on(Ev::Bookmark(rv.clone()));
                    }
                    Ok(WatchEvent::Error(e)) => {
                        on(Ev::Error(format!("watch: {} ({})", e.message, e.code)));
                        pause(&mut backoff).await;
                        if e.code == 410 {
                            continue 'relist;
                        }
                        break;
                    }
                    Err(e) => {
                        on(Ev::Error(format!("watch stream: {e}")));
                        let gone = is_gone(&e);
                        pause(&mut backoff).await;
                        if gone {
                            continue 'relist;
                        }
                        break;
                    }
                }
            }
            // A normal end (server-side timeout) re-watches from `rv` without a relist.
        }
    }
}

/// Per-watch state for one kind (one partition): derivation baseline, topology projection and
/// optionally a warm snapshot store. Objects are shared (`Arc`) between the baseline and the
/// snapshot, so snapshotting costs no second copy.
struct KindWatch<K> {
    key: String,
    ctx: Arc<Ctx>,
    derive: Option<Derive<K>>,
    project: Project<K>,
    snapshot: Option<Arc<SnapshotStore<K>>>,
    seen: HashMap<String, Arc<K>>,
    listed: HashSet<String>,
}

impl<K> KindWatch<K>
where
    K: Resource + Serialize + Send + Sync + 'static,
{
    fn upsert(&mut self, o: K, initial: bool) {
        let o = Arc::new(slim(o));
        let uid = o.uid().unwrap_or_default();
        if initial {
            self.listed.insert(uid.clone());
        }
        if let (Some(d), Some(prev)) = (self.derive, self.seen.get(&uid)) {
            for e in d(prev, &o) {
                self.ctx.emit(e);
            }
        }
        if let Some(p) = (self.project)(&o) {
            self.ctx.hub.topology_upsert(p, Utc::now());
        }
        if let Some(s) = &self.snapshot {
            if initial {
                s.init_apply(&self.key, o.clone());
            } else {
                s.apply(&self.key, o.clone());
            }
        }
        self.seen.insert(uid, o);
    }

    fn handle(&mut self, ev: Ev<K>) {
        let snap = self.snapshot.clone();
        match ev {
            Ev::Init => {
                self.listed.clear();
                if let Some(s) = &snap {
                    s.init(&self.key);
                }
            }
            Ev::InitApply(o) => self.upsert(o, true),
            Ev::Apply(o) => self.upsert(o, false),
            Ev::InitDone(rv) => {
                let gone: Vec<String> = self
                    .seen
                    .keys()
                    .filter(|u| !self.listed.contains(*u))
                    .cloned()
                    .collect();
                for u in gone {
                    self.seen.remove(&u);
                    self.ctx.hub.topology_remove(&u, Utc::now());
                }
                if let Some(s) = &snap {
                    s.init_done(&self.key, rv);
                }
                self.ctx.sync.mark(&self.key);
            }
            Ev::Delete(o) => {
                let uid = o.uid().unwrap_or_default();
                self.seen.remove(&uid);
                self.ctx.hub.topology_remove(&uid, Utc::now());
                if let Some(s) = &snap {
                    s.delete(&self.key, &o);
                }
            }
            Ev::Bookmark(rv) => {
                if let Some(s) = &snap {
                    s.bookmark(&self.key, rv);
                }
            }
            Ev::Established => {
                if let Some(s) = &snap {
                    s.established(&self.key);
                }
            }
            Ev::Error(e) => {
                if let Some(s) = &snap {
                    s.lost(&self.key, Utc::now());
                }
                tracing::warn!(kind = %self.key, error = %e, "watch error (retrying)");
            }
        }
    }
}

async fn watch_kind<K>(
    api: Api<K>,
    key: String,
    ctx: Arc<Ctx>,
    derive: Option<Derive<K>>,
    project: Project<K>,
    snapshot: Option<Arc<SnapshotStore<K>>>,
) where
    K: Resource<DynamicType = ()>
        + Clone
        + serde::de::DeserializeOwned
        + Serialize
        + Debug
        + Send
        + Sync
        + 'static,
{
    let mut w = KindWatch {
        key,
        ctx,
        derive,
        project,
        snapshot,
        seen: HashMap::new(),
        listed: HashSet::new(),
    };
    if w.snapshot.is_some() {
        // Snapshot kinds need bookmarks and watch health, which `kube::runtime::watcher` hides.
        list_watch(api, |ev| w.handle(ev)).await;
        return;
    }
    let mut stream = watcher::watcher(api, watcher::Config::default())
        .default_backoff()
        .boxed();
    while let Some(ev) = stream.next().await {
        match ev {
            Ok(e) => w.handle(e.into()),
            Err(e) => w.handle(Ev::Error(e.to_string())),
        }
    }
}

/// Raw Events: forwarded to the event log (never on the first list: that is history the
/// daemon did not witness) and kept in the warm events snapshot.
struct EventWatch {
    key: String,
    ctx: Arc<Ctx>,
    snapshot: Arc<SnapshotStore<Event>>,
    /// uid -> resourceVersion, so a relist does not re-forward what was already forwarded.
    forwarded: HashMap<String, String>,
    first_list: bool,
}

impl EventWatch {
    fn forward(&mut self, e: Event) -> Arc<Event> {
        let e = Arc::new(slim(e));
        let uid = e.uid().unwrap_or_default();
        let rv = e.resource_version().unwrap_or_default();
        let unseen = self.forwarded.get(&uid) != Some(&rv);
        self.forwarded.insert(uid, rv);
        // The first list is history the daemon did not witness: baseline only.
        if unseen
            && !self.first_list
            && let Some(d) = raw_event(&e)
        {
            self.ctx.hub.append_event(d, Utc::now());
        }
        e
    }

    fn handle(&mut self, ev: Ev<Event>) {
        let s = self.snapshot.clone();
        match ev {
            Ev::Init => s.init(&self.key),
            Ev::InitApply(e) => {
                let e = self.forward(e);
                s.init_apply(&self.key, e);
            }
            Ev::Apply(e) => {
                let e = self.forward(e);
                s.apply(&self.key, e);
            }
            Ev::InitDone(rv) => {
                self.first_list = false;
                s.init_done(&self.key, rv);
                self.ctx.sync.mark(&self.key);
            }
            Ev::Delete(e) => {
                self.forwarded.remove(&e.uid().unwrap_or_default());
                s.delete(&self.key, &e);
            }
            Ev::Bookmark(rv) => s.bookmark(&self.key, rv),
            Ev::Established => s.established(&self.key),
            Ev::Error(e) => {
                s.lost(&self.key, Utc::now());
                tracing::warn!(kind = %self.key, error = %e, "event watch error (retrying)");
            }
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

/// Watch keys (= snapshot partitions) for kind `K` under this config.
fn partitions<K>(cfg: &DaemonConfig) -> Vec<String>
where
    K: Resource<DynamicType = ()>,
{
    let kind = K::kind(&());
    if cfg.namespaces.is_empty() {
        vec![kind.to_string()]
    } else {
        cfg.namespaces
            .iter()
            .map(|ns| format!("{kind}/{ns}"))
            .collect()
    }
}

/// The warm snapshot stores (US4): Pods and Events, fed by their watches.
pub struct Snapshots {
    pub pods: Arc<SnapshotStore<Pod>>,
    pub events: Arc<SnapshotStore<Event>>,
}

impl Snapshots {
    pub fn new(cfg: &DaemonConfig) -> Self {
        let now = Utc::now();
        Self {
            pods: Arc::new(SnapshotStore::new(
                "pods",
                &PODS_ONLY,
                &partitions::<Pod>(cfg),
                now,
            )),
            events: Arc::new(SnapshotStore::new(
                "events",
                &EVENTS_ONLY,
                &partitions::<Event>(cfg),
                now,
            )),
        }
    }

    /// For [`crate::http::AppState::with_snapshots`].
    pub fn sources(&self) -> Vec<Arc<dyn SnapshotSource>> {
        vec![self.pods.clone(), self.events.clone()]
    }
}

fn spawn_kind<K>(
    client: &Client,
    cfg: &DaemonConfig,
    ctx: &Arc<Ctx>,
    derive: Option<Derive<K>>,
    project: Project<K>,
    snapshot: Option<Arc<SnapshotStore<K>>>,
) where
    K: Resource<DynamicType = (), Scope = NamespaceResourceScope>
        + Clone
        + serde::de::DeserializeOwned
        + Serialize
        + Debug
        + Send
        + Sync
        + 'static,
{
    for (api, key) in ns_apis::<K>(client, cfg) {
        tokio::spawn(watch_kind(
            api,
            key,
            ctx.clone(),
            derive,
            project,
            snapshot.clone(),
        ));
    }
}

/// Start every watch plus the coalescer flush task. Returns immediately; readiness is
/// reported through `ctx.sync`, snapshot state through `snaps`.
pub fn spawn_all(client: Client, cfg: &DaemonConfig, ctx: Arc<Ctx>, snaps: &Snapshots) {
    spawn_kind::<Deployment>(
        &client,
        cfg,
        &ctx,
        Some(derive_deployment),
        project_deployment,
        None,
    );
    spawn_kind::<StatefulSet>(
        &client,
        cfg,
        &ctx,
        Some(derive_statefulset),
        project_sts,
        None,
    );
    spawn_kind::<DaemonSet>(&client, cfg, &ctx, Some(derive_daemonset), project_ds, None);
    spawn_kind::<ReplicaSet>(&client, cfg, &ctx, None, project_rs, None);
    spawn_kind::<Job>(&client, cfg, &ctx, Some(derive_job), project_job, None);
    spawn_kind::<CronJob>(
        &client,
        cfg,
        &ctx,
        Some(derive_cronjob),
        project_cronjob,
        None,
    );
    spawn_kind::<HorizontalPodAutoscaler>(&client, cfg, &ctx, Some(derive_hpa), project_hpa, None);
    spawn_kind::<Pod>(
        &client,
        cfg,
        &ctx,
        Some(derive_pod),
        project_pod,
        Some(snaps.pods.clone()),
    );
    spawn_kind::<Service>(&client, cfg, &ctx, None, project_service, None);
    for (api, key) in ns_apis::<Event>(&client, cfg) {
        let mut w = EventWatch {
            key,
            ctx: ctx.clone(),
            snapshot: snaps.events.clone(),
            forwarded: HashMap::new(),
            first_list: true,
        };
        tokio::spawn(async move { list_watch(api, |ev| w.handle(ev)).await });
    }
    if cfg.namespaces.is_empty() {
        tokio::spawn(watch_kind(
            Api::<Node>::all(client),
            "Node".into(),
            ctx.clone(),
            None,
            project_node,
            None,
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
