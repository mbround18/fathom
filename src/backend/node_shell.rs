//! 037-node-shell: a privileged debug pod pinned to one node, with layered guaranteed cleanup.
//!
//! The backend owns the session lifecycle; the UI is a view of `NodeShellStatus` events.
//! Cluster access goes through the small [`NodeShellApi`] trait so the pod builder, the
//! create -> Running wait and every cleanup path are unit-tested against a fake API.
//!
//! Cleanup layers (research.md R3): explicit delete on every normal end path (all funnel into
//! [`end_active`]), a bounded delete on context switch / app exit using the client captured when
//! the session started, the pod's `activeDeadlineSeconds`, and a label-based orphan sweep the
//! operator confirms by name.
//!
//! Deviation from the plan: the wait for `Running` polls a single pod with `GET` once per second
//! (bounded by the start timeout) instead of opening a watch; it only runs while starting.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use async_trait::async_trait;
use crossbeam_channel::Sender;
use k8s_openapi::api::authorization::v1::ResourceAttributes;
use k8s_openapi::api::core::v1::{
    Container, HostPathVolumeSource, LocalObjectReference, Pod, PodSpec, SecurityContext,
    Toleration, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, DeleteParams, ListParams, PostParams};
use tokio::task::JoinHandle;

use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, NodeShellAccessCheck, NodeShellCheckOutcome,
    NodeShellPhase, NodeShellPodRef, NodeShellSettings, OrphanedNodeShellPod, Route, SystemEvent,
    section_keys,
};

use super::state::BackendState;
use super::{events, persistence, rbac_access};

pub(super) const LABEL_MANAGED_BY: (&str, &str) = ("app.kubernetes.io/managed-by", "fathom");
pub(super) const LABEL_NODE_SHELL: &str = "fathom.io/node-shell";
pub(super) const LABEL_NODE: &str = "fathom.io/node";
pub(super) const ANNOTATION_CREATED_AT: &str = "fathom.io/created-at";
pub(super) const ORPHAN_SELECTOR: &str = "fathom.io/node-shell=true";
pub(super) const CONTAINER_NAME: &str = "shell";

/// Bound on the delete issued from a context switch (research.md R3).
pub(super) const CONTEXT_SWITCH_BOUND: Duration = Duration::from_secs(5);
/// Bound on the delete issued from app exit; kept inside the Tauri shutdown grace.
pub(super) const APP_EXIT_BOUND: Duration = Duration::from_secs(4);

// ------------------------------------------------------------------------------------------
// Cluster access seam
// ------------------------------------------------------------------------------------------

#[async_trait]
pub(super) trait NodeShellApi: Send + Sync {
    async fn create(&self, pod: &Pod) -> Result<Pod, kube::Error>;
    async fn get(&self, namespace: &str, name: &str) -> Result<Pod, kube::Error>;
    async fn delete(&self, namespace: &str, name: &str) -> Result<(), kube::Error>;
    /// Pods carrying the node-shell label, in `namespace` or in all namespaces.
    async fn list(&self, namespace: Option<&str>) -> Result<Vec<Pod>, kube::Error>;
    /// One `SelfSubjectAccessReview`; `Ok((allowed, reason))`.
    async fn review(
        &self,
        attrs: ResourceAttributes,
    ) -> Result<(bool, Option<String>), kube::Error>;
}

pub(super) struct KubeApi(pub(super) kube::Client);

#[async_trait]
impl NodeShellApi for KubeApi {
    async fn create(&self, pod: &Pod) -> Result<Pod, kube::Error> {
        let ns = pod.metadata.namespace.clone().unwrap_or_default();
        Api::<Pod>::namespaced(self.0.clone(), &ns)
            .create(&PostParams::default(), pod)
            .await
    }
    async fn get(&self, namespace: &str, name: &str) -> Result<Pod, kube::Error> {
        Api::<Pod>::namespaced(self.0.clone(), namespace)
            .get(name)
            .await
    }
    async fn delete(&self, namespace: &str, name: &str) -> Result<(), kube::Error> {
        Api::<Pod>::namespaced(self.0.clone(), namespace)
            .delete(name, &DeleteParams::default())
            .await
            .map(|_| ())
    }
    async fn list(&self, namespace: Option<&str>) -> Result<Vec<Pod>, kube::Error> {
        let api: Api<Pod> = match namespace {
            Some(ns) => Api::namespaced(self.0.clone(), ns),
            None => Api::all(self.0.clone()),
        };
        Ok(api
            .list(&ListParams::default().labels(ORPHAN_SELECTOR))
            .await?
            .items)
    }
    async fn review(
        &self,
        attrs: ResourceAttributes,
    ) -> Result<(bool, Option<String>), kube::Error> {
        let status = rbac_access::review_resource_access(self.0.clone(), attrs).await?;
        Ok((status.allowed, status.reason.or(status.evaluation_error)))
    }
}

fn is_not_found(err: &kube::Error) -> bool {
    matches!(err, kube::Error::Api(ae) if ae.code == 404)
}

fn is_conflict(err: &kube::Error) -> bool {
    matches!(err, kube::Error::Api(ae) if ae.code == 409)
}

// ------------------------------------------------------------------------------------------
// Pod construction
// ------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(super) struct StartParams {
    pub(super) node: String,
    pub(super) namespace: String,
    pub(super) image: String,
    pub(super) image_pull_secret: Option<String>,
    pub(super) deadline_seconds: u32,
}

fn fnv6(s: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{:06x}", h & 0x00ff_ffff)
}

/// `fathom-node-shell-<node-hash>-<suffix>`: at most 63 characters and DNS-safe for any node name.
pub(super) fn pod_name(node: &str, suffix: &str) -> String {
    format!("fathom-node-shell-{}-{}", fnv6(node), suffix)
}

/// Five random base-36 characters (no extra dependency: std's per-process random hasher).
pub(super) fn random_suffix() -> String {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    let mut n = hasher.finish();
    let alphabet = b"abcdefghijklmnopqrstuvwxyz0123456789";
    (0..5)
        .map(|_| {
            let c = alphabet[(n % 36) as usize] as char;
            n /= 36;
            c
        })
        .collect()
}

/// The debug pod, matching `kubectl debug node/<n> --profile=sysadmin` (research.md R1).
pub(super) fn build_pod(name: &str, params: &StartParams, created_at: &str) -> Pod {
    let labels: BTreeMap<String, String> = [
        (LABEL_MANAGED_BY.0.to_owned(), LABEL_MANAGED_BY.1.to_owned()),
        (LABEL_NODE_SHELL.to_owned(), "true".to_owned()),
        // Label values are limited to 63 chars of [A-Za-z0-9-_.]; node names are DNS subdomains
        // (up to 253), so an over-long name is truncated here and kept whole in the annotation.
        (LABEL_NODE.to_owned(), label_value(&params.node)),
    ]
    .into();
    let annotations: BTreeMap<String, String> = [
        (ANNOTATION_CREATED_AT.to_owned(), created_at.to_owned()),
        ("fathom.io/node-full".to_owned(), params.node.clone()),
    ]
    .into();
    Pod {
        metadata: ObjectMeta {
            name: Some(name.to_owned()),
            namespace: Some(params.namespace.clone()),
            labels: Some(labels),
            annotations: Some(annotations),
            ..Default::default()
        },
        spec: Some(PodSpec {
            node_name: Some(params.node.clone()),
            host_pid: Some(true),
            host_network: Some(true),
            host_ipc: Some(true),
            restart_policy: Some("Never".to_owned()),
            active_deadline_seconds: Some(i64::from(params.deadline_seconds)),
            // The keep-alive `sleep` ignores SIGTERM as PID 1; do not wait 30 s to delete.
            termination_grace_period_seconds: Some(0),
            tolerations: Some(vec![Toleration {
                operator: Some("Exists".to_owned()),
                ..Default::default()
            }]),
            image_pull_secrets: params
                .image_pull_secret
                .as_ref()
                .map(|n| vec![LocalObjectReference { name: n.clone() }]),
            containers: vec![Container {
                name: CONTAINER_NAME.to_owned(),
                image: Some(params.image.clone()),
                command: Some(vec!["sleep".to_owned(), "infinity".to_owned()]),
                stdin: Some(true),
                tty: Some(true),
                security_context: Some(SecurityContext {
                    privileged: Some(true),
                    ..Default::default()
                }),
                volume_mounts: Some(vec![VolumeMount {
                    name: "host".to_owned(),
                    mount_path: "/host".to_owned(),
                    ..Default::default()
                }]),
                ..Default::default()
            }],
            volumes: Some(vec![Volume {
                name: "host".to_owned(),
                host_path: Some(HostPathVolumeSource {
                    path: "/".to_owned(),
                    type_: None,
                }),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn label_value(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .take(63)
        .collect();
    cleaned
        .trim_matches(|c: char| !c.is_ascii_alphanumeric())
        .to_owned()
}

// ------------------------------------------------------------------------------------------
// Observing the pod
// ------------------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Observed {
    Pending(Option<String>),
    Running,
    /// The pod reached a terminal phase before it was usable.
    Terminal(String),
}

/// Map a pod's status to what the operator should see. Conditions and waiting reasons are
/// carried verbatim (FR-010).
pub(super) fn observe(pod: &Pod) -> Observed {
    let Some(status) = pod.status.as_ref() else {
        return Observed::Pending(None);
    };
    let phase = status.phase.as_deref().unwrap_or("Pending");
    match phase {
        "Running" => return Observed::Running,
        "Failed" | "Succeeded" => {
            let detail = [status.reason.clone(), status.message.clone()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(": ");
            return Observed::Terminal(if detail.is_empty() {
                format!("Pod entered phase {phase}")
            } else {
                format!("Pod entered phase {phase}: {detail}")
            });
        }
        _ => {}
    }
    Observed::Pending(pending_detail(pod))
}

fn pending_detail(pod: &Pod) -> Option<String> {
    let status = pod.status.as_ref()?;
    let mut parts: Vec<String> = Vec::new();
    for cs in status.container_statuses.iter().flatten() {
        if let Some(w) = cs.state.as_ref().and_then(|s| s.waiting.as_ref()) {
            let reason = w.reason.clone().unwrap_or_default();
            match w.message.as_deref() {
                Some(m) if !m.is_empty() => parts.push(format!("{reason}: {m}")),
                _ => parts.push(reason),
            }
        }
    }
    for c in status.conditions.iter().flatten() {
        if c.status == "False" {
            let mut s = c.type_.clone();
            if let Some(r) = &c.reason {
                s.push_str(&format!(" ({r})"));
            }
            if let Some(m) = &c.message {
                s.push_str(&format!(": {m}"));
            }
            parts.push(s);
        }
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Timing {
    pub(super) start_timeout: Duration,
    pub(super) poll: Duration,
    pub(super) delete_bound: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            start_timeout: Duration::from_secs(90),
            poll: Duration::from_secs(1),
            delete_bound: Duration::from_secs(10),
        }
    }
}

/// Wait until the pod is Running. `Err(detail)` on a terminal phase, API error, or timeout;
/// `on_pending` is called only when the pending detail changes.
pub(super) async fn wait_running(
    api: &dyn NodeShellApi,
    namespace: &str,
    name: &str,
    timing: Timing,
    mut on_pending: impl FnMut(Option<String>) + Send,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timing.start_timeout;
    let mut last: Option<Option<String>> = None;
    let mut last_detail: Option<String>;
    loop {
        match api.get(namespace, name).await {
            Ok(pod) => match observe(&pod) {
                Observed::Running => return Ok(()),
                Observed::Terminal(detail) => return Err(detail),
                Observed::Pending(detail) => {
                    if last.as_ref() != Some(&detail) {
                        on_pending(detail.clone());
                        last = Some(detail.clone());
                    }
                    last_detail = detail;
                }
            },
            Err(e) => return Err(format!("Failed to read pod {namespace}/{name}: {e}")),
        }
        if tokio::time::Instant::now() + timing.poll > deadline {
            return Err(match last_detail {
                Some(d) => format!(
                    "Timed out after {}s waiting for the pod to start: {d}",
                    timing.start_timeout.as_secs()
                ),
                None => format!(
                    "Timed out after {}s waiting for the pod to start",
                    timing.start_timeout.as_secs()
                ),
            });
        }
        tokio::time::sleep(timing.poll).await;
    }
}

// ------------------------------------------------------------------------------------------
// Events
// ------------------------------------------------------------------------------------------

#[derive(Clone)]
pub(super) struct Emitter {
    pub(super) seq: Arc<AtomicU64>,
    pub(super) sender: Sender<BackendEventEnvelope>,
    pub(super) context: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct Ident {
    pub(super) session_id: u64,
    pub(super) node: String,
    pub(super) namespace: String,
    pub(super) pod: String,
}

impl Emitter {
    fn route(&self) -> Route {
        Route::cluster(section_keys::nodes(), self.context.clone())
    }

    pub(super) fn status(
        &self,
        id: &Ident,
        phase: NodeShellPhase,
        detail: Option<String>,
        leftover: bool,
    ) {
        events::emit(
            &self.seq,
            &self.sender,
            self.route(),
            Correlation::None,
            BackendEvent::System(SystemEvent::NodeShellStatus {
                session_id: id.session_id,
                phase,
                node: id.node.clone(),
                namespace: id.namespace.clone(),
                pod: id.pod.clone(),
                detail,
                leftover,
            }),
        );
    }
}

// ------------------------------------------------------------------------------------------
// Session state
// ------------------------------------------------------------------------------------------

pub(super) struct NodeShellSession {
    ident: Ident,
    api: Arc<dyn NodeShellApi>,
    emitter: Emitter,
    phase: NodeShellPhase,
    leftover: bool,
    watch_task: Option<JoinHandle<()>>,
}

#[derive(Default)]
pub(super) struct NodeShellState {
    session: Option<NodeShellSession>,
    next_session_id: u64,
}

impl NodeShellState {
    pub(super) fn abort_task(&mut self) {
        if let Some(t) = self.session.as_mut().and_then(|s| s.watch_task.take()) {
            t.abort();
        }
    }
}

/// Delete the pod and report the outcome as events. `Ok` means the pod is gone (deleted or
/// already NotFound); `Err` carries the verbatim failure and the pod is left over.
async fn teardown(
    api: &dyn NodeShellApi,
    emitter: &Emitter,
    id: &Ident,
    bound: Duration,
) -> Result<(), String> {
    emitter.status(id, NodeShellPhase::Terminating, None, false);
    let result = match tokio::time::timeout(bound, api.delete(&id.namespace, &id.pod)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) if is_not_found(&e) => Ok(()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err(format!(
            "Timed out after {}s deleting pod {}/{}",
            bound.as_secs(),
            id.namespace,
            id.pod
        )),
    };
    match &result {
        Ok(()) => emitter.status(id, NodeShellPhase::Removed, None, false),
        Err(detail) => emitter.status(id, NodeShellPhase::Failed, Some(detail.clone()), true),
    }
    result
}

/// End the active session by deleting its pod. Every normal end path calls this: explicit stop
/// (which also covers tab close and terminal exit), supersede, context switch, and app exit.
/// On failure the session is kept (`leftover`) so Stop can be retried. Returns whether the
/// session is fully gone.
pub(super) async fn end_active(state: &mut BackendState, bound: Duration) -> bool {
    let Some(mut sess) = state.node_shell.session.take() else {
        return true;
    };
    if let Some(t) = sess.watch_task.take() {
        t.abort();
    }
    if sess.phase == NodeShellPhase::Attached {
        state.exec_session.stop();
    }
    match teardown(sess.api.as_ref(), &sess.emitter, &sess.ident, bound).await {
        Ok(()) => true,
        Err(_) => {
            sess.leftover = true;
            sess.phase = NodeShellPhase::Failed;
            state.node_shell.session = Some(sess);
            false
        }
    }
}

/// `StopNodeShell`: a stale `session_id` (superseded or already removed) is ignored.
pub(super) async fn stop(state: &mut BackendState, session_id: u64) {
    match &state.node_shell.session {
        Some(s) if s.ident.session_id == session_id => {
            end_active(state, Timing::default().delete_bound).await;
        }
        _ => tracing::debug!("StopNodeShell for stale session {session_id} ignored"),
    }
}

/// Called after a successful `StartPodExec` attach: if it targets the session's pod, the
/// session is now `Attached`.
pub(super) fn note_exec_ready(state: &mut BackendState, namespace: &str, pod: &str) {
    if let Some(s) = state.node_shell.session.as_mut()
        && s.ident.namespace == namespace
        && s.ident.pod == pod
        && matches!(s.phase, NodeShellPhase::Pending | NodeShellPhase::Running)
    {
        s.phase = NodeShellPhase::Attached;
        s.emitter
            .status(&s.ident, NodeShellPhase::Attached, None, false);
    }
}

/// `StartNodeShell` against the live cluster.
pub(super) async fn start(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    params: StartParams,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    start_with_api(
        state,
        sender,
        Arc::new(KubeApi(client)),
        params,
        Timing::default(),
    )
    .await
}

pub(super) async fn start_with_api(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    api: Arc<dyn NodeShellApi>,
    params: StartParams,
    timing: Timing,
) -> anyhow::Result<()> {
    // Singleton: the previous session's pod is deleted before a new one is created.
    if !end_active(state, timing.delete_bound).await {
        anyhow::bail!(
            "The previous node shell pod could not be deleted; retry stopping it before starting another"
        );
    }

    state.node_shell.next_session_id += 1;
    let session_id = state.node_shell.next_session_id;
    let emitter = Emitter {
        seq: state.cluster.seq.clone(),
        sender: sender.clone(),
        context: state.cluster.current_context.clone(),
    };
    let created_at = chrono::Utc::now().to_rfc3339();

    let mut ident = Ident {
        session_id,
        node: params.node.clone(),
        namespace: params.namespace.clone(),
        pod: pod_name(&params.node, &random_suffix()),
    };
    emitter.status(&ident, NodeShellPhase::Creating, None, false);

    // A name collision is retried once with a fresh suffix, then reported.
    let mut attempt = 0;
    loop {
        let pod = build_pod(&ident.pod, &params, &created_at);
        match api.create(&pod).await {
            Ok(_) => break,
            Err(e) if is_conflict(&e) && attempt == 0 => {
                attempt += 1;
                ident.pod = pod_name(&params.node, &random_suffix());
            }
            Err(e) => {
                // Nothing was created: verbatim API/admission error, no cleanup needed.
                emitter.status(&ident, NodeShellPhase::Failed, Some(e.to_string()), false);
                return Ok(());
            }
        }
    }
    emitter.status(&ident, NodeShellPhase::Pending, None, false);

    let task = {
        let api = api.clone();
        let emitter = emitter.clone();
        let ident = ident.clone();
        tokio::spawn(async move {
            let result = wait_running(api.as_ref(), &ident.namespace, &ident.pod, timing, |d| {
                emitter.status(&ident, NodeShellPhase::Pending, d, false);
            })
            .await;
            match result {
                Ok(()) => emitter.status(&ident, NodeShellPhase::Running, None, false),
                Err(detail) => {
                    emitter.status(&ident, NodeShellPhase::Failed, Some(detail), false);
                    // Failure mapping always cleans up (FR-010); a delete failure is reported
                    // as a leftover by `teardown`.
                    let _ = teardown(api.as_ref(), &emitter, &ident, timing.delete_bound).await;
                }
            }
        })
    };

    state.node_shell.session = Some(NodeShellSession {
        ident,
        api,
        emitter,
        phase: NodeShellPhase::Pending,
        leftover: false,
        watch_task: Some(task),
    });
    Ok(())
}

// ------------------------------------------------------------------------------------------
// Access check
// ------------------------------------------------------------------------------------------

/// The five permissions Node Shell needs (FR-004): `(verb, resource, subresource, namespaced)`.
const REQUIRED_ACCESS: [(&str, &str, Option<&str>, bool); 5] = [
    ("create", "pods", None, true),
    ("delete", "pods", None, true),
    ("get", "pods", None, true),
    ("create", "pods", Some("exec"), true),
    ("get", "nodes", None, false),
];

pub(super) async fn check_access(
    api: &dyn NodeShellApi,
    namespace: &str,
) -> Vec<NodeShellAccessCheck> {
    let mut out = Vec::with_capacity(REQUIRED_ACCESS.len());
    for (verb, resource, sub, namespaced) in REQUIRED_ACCESS {
        let ns = namespaced.then(|| namespace.to_owned());
        let attrs = ResourceAttributes {
            // Core API group.
            group: Some(String::new()),
            resource: Some(resource.to_owned()),
            subresource: sub.map(str::to_owned),
            verb: Some(verb.to_owned()),
            namespace: ns.clone(),
            ..Default::default()
        };
        let outcome = match api.review(attrs).await {
            Ok((true, _)) => NodeShellCheckOutcome::Allowed,
            Ok((false, reason)) => NodeShellCheckOutcome::Denied { reason },
            // Fail safe: an errored review is never treated as allowed.
            Err(e) => NodeShellCheckOutcome::Error {
                message: e.to_string(),
            },
        };
        out.push(NodeShellAccessCheck {
            verb: verb.to_owned(),
            resource: resource.to_owned(),
            subresource: sub.map(str::to_owned),
            namespace: ns,
            outcome,
        });
    }
    out
}

pub(super) async fn handle_check_access(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: String,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let checks = check_access(&KubeApi(client), &namespace).await;
    events::emit(
        &state.cluster.seq,
        sender,
        Route::cluster(section_keys::nodes(), state.cluster.current_context.clone()),
        Correlation::None,
        BackendEvent::System(SystemEvent::NodeShellAccessChecked { namespace, checks }),
    );
    Ok(())
}

// ------------------------------------------------------------------------------------------
// Orphan sweep
// ------------------------------------------------------------------------------------------

pub(super) fn orphan_from_pod(pod: &Pod) -> OrphanedNodeShellPod {
    let labels = pod.metadata.labels.clone().unwrap_or_default();
    let annotations = pod.metadata.annotations.clone().unwrap_or_default();
    OrphanedNodeShellPod {
        namespace: pod.metadata.namespace.clone().unwrap_or_default(),
        name: pod.metadata.name.clone().unwrap_or_default(),
        node: pod
            .spec
            .as_ref()
            .and_then(|s| s.node_name.clone())
            .or_else(|| labels.get(LABEL_NODE).cloned()),
        phase: pod.status.as_ref().and_then(|s| s.phase.clone()),
        created_at: annotations.get(ANNOTATION_CREATED_AT).cloned().or_else(|| {
            pod.metadata
                .creation_timestamp
                .as_ref()
                .map(|t| super::workload_common::kube_rfc3339(&t))
        }),
    }
}

/// Fathom node-shell pods that the active session does not own (terminal-phase included).
pub(super) async fn list_orphans(
    api: &dyn NodeShellApi,
    namespace: Option<&str>,
    active: Option<(&str, &str)>,
) -> Result<Vec<OrphanedNodeShellPod>, kube::Error> {
    Ok(api
        .list(namespace)
        .await?
        .iter()
        .filter(|p| {
            p.metadata
                .labels
                .as_ref()
                .is_some_and(|l| l.get(LABEL_NODE_SHELL).is_some_and(|v| v == "true"))
        })
        .map(orphan_from_pod)
        .filter(|o| active != Some((o.namespace.as_str(), o.name.as_str())))
        .collect())
}

/// Delete operator-confirmed orphans. Refuses pods that do not carry the node-shell label or
/// that belong to the active session; NotFound counts as success. Returns per-pod failures.
pub(super) async fn delete_orphans(
    api: &dyn NodeShellApi,
    pods: &[NodeShellPodRef],
    active: Option<(&str, &str)>,
) -> Vec<String> {
    let mut failures = Vec::new();
    for p in pods {
        if active == Some((p.namespace.as_str(), p.name.as_str())) {
            failures.push(format!(
                "{}/{}: belongs to the active node shell session; stop the session instead",
                p.namespace, p.name
            ));
            continue;
        }
        match api.get(&p.namespace, &p.name).await {
            Ok(pod) => {
                let is_ours = pod
                    .metadata
                    .labels
                    .as_ref()
                    .is_some_and(|l| l.get(LABEL_NODE_SHELL).is_some_and(|v| v == "true"));
                if !is_ours {
                    failures.push(format!(
                        "{}/{}: not a Fathom node shell pod; refusing to delete",
                        p.namespace, p.name
                    ));
                    continue;
                }
            }
            Err(e) if is_not_found(&e) => continue,
            Err(e) => {
                failures.push(format!("{}/{}: {e}", p.namespace, p.name));
                continue;
            }
        }
        match api.delete(&p.namespace, &p.name).await {
            Ok(()) => {}
            Err(e) if is_not_found(&e) => {}
            Err(e) => failures.push(format!("{}/{}: {e}", p.namespace, p.name)),
        }
    }
    failures
}

fn active_key(state: &BackendState) -> Option<(String, String)> {
    state
        .node_shell
        .session
        .as_ref()
        .map(|s| (s.ident.namespace.clone(), s.ident.pod.clone()))
}

pub(super) async fn handle_list_orphans(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: Option<String>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let active = active_key(state);
    let pods = list_orphans(
        &KubeApi(client),
        namespace.as_deref(),
        active.as_ref().map(|(a, b)| (a.as_str(), b.as_str())),
    )
    .await?;
    emit_orphans(state, sender, pods);
    Ok(())
}

fn emit_orphans(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    pods: Vec<OrphanedNodeShellPod>,
) {
    events::emit(
        &state.cluster.seq,
        sender,
        Route::cluster(section_keys::nodes(), state.cluster.current_context.clone()),
        Correlation::None,
        BackendEvent::System(SystemEvent::NodeShellOrphans { pods }),
    );
}

pub(super) async fn handle_delete_orphans(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    pods: Vec<NodeShellPodRef>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api = KubeApi(client);
    let active = active_key(state);
    let active_ref = active.as_ref().map(|(a, b)| (a.as_str(), b.as_str()));
    let failures = delete_orphans(&api, &pods, active_ref).await;
    // Re-list so the UI reflects what really remains, even after partial failure.
    if let Ok(remaining) = list_orphans(&api, None, active_ref).await {
        emit_orphans(state, sender, remaining);
    }
    if failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("{}", failures.join("\n"))
    }
}

// ------------------------------------------------------------------------------------------
// Settings
// ------------------------------------------------------------------------------------------

pub(super) fn handle_get_settings(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    let context = state.cluster.current_context.clone().unwrap_or_default();
    let settings = persistence::load_node_shell_settings(&context);
    emit_settings(state, sender, context, settings);
}

pub(super) fn handle_set_settings(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    settings: NodeShellSettings,
) -> anyhow::Result<()> {
    let context = state.cluster.current_context.clone().unwrap_or_default();
    let settings = settings.normalized().map_err(anyhow::Error::msg)?;
    persistence::save_node_shell_settings(&context, &settings)?;
    emit_settings(state, sender, context, settings);
    Ok(())
}

fn emit_settings(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: String,
    settings: NodeShellSettings,
) {
    events::emit(
        &state.cluster.seq,
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::NodeShellSettingsLoaded { context, settings }),
    );
}

// ------------------------------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{
        ContainerState, ContainerStateWaiting, ContainerStatus, PodCondition, PodStatus,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;

    fn params() -> StartParams {
        StartParams {
            node: "worker-1".to_owned(),
            namespace: "default".to_owned(),
            image: "busybox:stable".to_owned(),
            image_pull_secret: None,
            deadline_seconds: 14_400,
        }
    }

    // ---- pod builder (T007, T019) ----

    #[test]
    fn pod_is_privileged_with_host_namespaces_and_node_pin() {
        let mut p = params();
        p.image_pull_secret = Some("regcred".to_owned());
        let pod = build_pod("fathom-node-shell-abc-12345", &p, "2026-01-01T00:00:00Z");
        let spec = pod.spec.as_ref().unwrap();
        assert_eq!(spec.node_name.as_deref(), Some("worker-1"));
        assert_eq!(spec.host_pid, Some(true));
        assert_eq!(spec.host_network, Some(true));
        assert_eq!(spec.host_ipc, Some(true));
        assert_eq!(spec.restart_policy.as_deref(), Some("Never"));
        let tol = &spec.tolerations.as_ref().unwrap()[0];
        assert_eq!(tol.operator.as_deref(), Some("Exists"));
        assert!(tol.key.is_none() && tol.effect.is_none());
        assert_eq!(spec.image_pull_secrets.as_ref().unwrap()[0].name, "regcred");
        let c = &spec.containers[0];
        assert_eq!(c.image.as_deref(), Some("busybox:stable"));
        assert_eq!(c.security_context.as_ref().unwrap().privileged, Some(true));
        assert_eq!(c.volume_mounts.as_ref().unwrap()[0].mount_path, "/host");
        assert_eq!(
            spec.volumes.as_ref().unwrap()[0]
                .host_path
                .as_ref()
                .unwrap()
                .path,
            "/"
        );
        let labels = pod.metadata.labels.as_ref().unwrap();
        assert_eq!(labels["app.kubernetes.io/managed-by"], "fathom");
        assert_eq!(labels["fathom.io/node-shell"], "true");
        assert_eq!(labels["fathom.io/node"], "worker-1");
        assert!(
            pod.metadata
                .annotations
                .as_ref()
                .unwrap()
                .contains_key("fathom.io/created-at")
        );
    }

    #[test]
    fn deadline_is_always_present() {
        for secs in [300_u32, 14_400, 86_400] {
            let mut p = params();
            p.deadline_seconds = secs;
            let pod = build_pod("n", &p, "t");
            assert_eq!(
                pod.spec.unwrap().active_deadline_seconds,
                Some(i64::from(secs))
            );
        }
    }

    #[test]
    fn no_pull_secret_means_no_field() {
        let pod = build_pod("n", &params(), "t");
        assert!(pod.spec.unwrap().image_pull_secrets.is_none());
    }

    #[test]
    fn pod_names_fit_dns_limits_for_long_or_odd_nodes() {
        let long = "a".repeat(253);
        for node in ["n", "ip-10-0-0-1.ec2.internal", long.as_str()] {
            let name = pod_name(node, &random_suffix());
            assert!(name.len() <= 63, "{name}");
            assert!(
                name.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            );
        }
        assert_ne!(random_suffix(), random_suffix());
        let mut p = params();
        p.node = long.clone();
        let labels = build_pod("n", &p, "t").metadata.labels.unwrap();
        assert!(labels["fathom.io/node"].len() <= 63);
    }

    // ---- observation ----

    fn pod_with_status(status: PodStatus) -> Pod {
        Pod {
            status: Some(status),
            ..Default::default()
        }
    }

    #[test]
    fn observe_maps_phases_and_surfaces_reasons() {
        assert_eq!(
            observe(&pod_with_status(PodStatus {
                phase: Some("Running".to_owned()),
                ..Default::default()
            })),
            Observed::Running
        );
        let waiting = pod_with_status(PodStatus {
            phase: Some("Pending".to_owned()),
            container_statuses: Some(vec![ContainerStatus {
                state: Some(ContainerState {
                    waiting: Some(ContainerStateWaiting {
                        reason: Some("ImagePullBackOff".to_owned()),
                        message: Some("Back-off pulling image \"nope\"".to_owned()),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ..Default::default()
        });
        match observe(&waiting) {
            Observed::Pending(Some(d)) => {
                assert!(
                    d.contains("ImagePullBackOff") && d.contains("Back-off pulling"),
                    "{d}"
                );
            }
            other => panic!("{other:?}"),
        }
        let unschedulable = pod_with_status(PodStatus {
            phase: Some("Pending".to_owned()),
            conditions: Some(vec![PodCondition {
                type_: "PodScheduled".to_owned(),
                status: "False".to_owned(),
                reason: Some("Unschedulable".to_owned()),
                message: Some("0/1 nodes are available".to_owned()),
                ..Default::default()
            }]),
            ..Default::default()
        });
        match observe(&unschedulable) {
            Observed::Pending(Some(d)) => assert!(d.contains("Unschedulable"), "{d}"),
            other => panic!("{other:?}"),
        }
        match observe(&pod_with_status(PodStatus {
            phase: Some("Failed".to_owned()),
            reason: Some("DeadlineExceeded".to_owned()),
            ..Default::default()
        })) {
            Observed::Terminal(d) => assert!(d.contains("DeadlineExceeded")),
            other => panic!("{other:?}"),
        }
        assert_eq!(observe(&Pod::default()), Observed::Pending(None));
    }

    // ---- fake cluster ----

    #[derive(Clone)]
    struct ApiErr(u16, &'static str);
    impl ApiErr {
        fn into_kube(self) -> kube::Error {
            kube::Error::Api(
                kube::core::Status::failure(self.1, "X")
                    .with_code(self.0)
                    .boxed(),
            )
        }
    }

    type ReviewResult = Result<(bool, Option<String>), ApiErr>;

    #[derive(Default)]
    struct FakeApi {
        pods: Mutex<HashMap<(String, String), Pod>>,
        create_errs: Mutex<Vec<ApiErr>>,
        delete_err: Mutex<Option<ApiErr>>,
        /// Phase reported for pods on `get` (default Running).
        phase: Mutex<Option<PodStatus>>,
        deletes: Mutex<Vec<String>>,
        creates: Mutex<Vec<String>>,
        review_script: Mutex<HashMap<String, ReviewResult>>,
    }

    #[async_trait]
    impl NodeShellApi for FakeApi {
        async fn create(&self, pod: &Pod) -> Result<Pod, kube::Error> {
            if let Some(e) = {
                let mut errs = self.create_errs.lock().unwrap();
                (!errs.is_empty()).then(|| errs.remove(0))
            } {
                return Err(e.into_kube());
            }
            let key = (
                pod.metadata.namespace.clone().unwrap(),
                pod.metadata.name.clone().unwrap(),
            );
            self.creates.lock().unwrap().push(key.1.clone());
            self.pods.lock().unwrap().insert(key, pod.clone());
            Ok(pod.clone())
        }
        async fn get(&self, ns: &str, name: &str) -> Result<Pod, kube::Error> {
            let mut pod = self
                .pods
                .lock()
                .unwrap()
                .get(&(ns.to_owned(), name.to_owned()))
                .cloned()
                .ok_or_else(|| ApiErr(404, "not found").into_kube())?;
            pod.status = Some(self.phase.lock().unwrap().clone().unwrap_or(PodStatus {
                phase: Some("Running".to_owned()),
                ..Default::default()
            }));
            Ok(pod)
        }
        async fn delete(&self, ns: &str, name: &str) -> Result<(), kube::Error> {
            self.deletes.lock().unwrap().push(format!("{ns}/{name}"));
            if let Some(e) = self.delete_err.lock().unwrap().clone() {
                return Err(e.into_kube());
            }
            self.pods
                .lock()
                .unwrap()
                .remove(&(ns.to_owned(), name.to_owned()))
                .map(|_| ())
                .ok_or_else(|| ApiErr(404, "not found").into_kube())
        }
        async fn list(&self, ns: Option<&str>) -> Result<Vec<Pod>, kube::Error> {
            Ok(self
                .pods
                .lock()
                .unwrap()
                .iter()
                .filter(|((n, _), _)| ns.is_none_or(|w| w == n))
                .map(|(_, p)| p.clone())
                .collect())
        }
        async fn review(
            &self,
            attrs: ResourceAttributes,
        ) -> Result<(bool, Option<String>), kube::Error> {
            let key = format!(
                "{}/{}/{}",
                attrs.verb.unwrap(),
                attrs.resource.unwrap(),
                attrs.subresource.unwrap_or_default()
            );
            match self.review_script.lock().unwrap().get(&key).cloned() {
                Some(Ok(v)) => Ok(v),
                Some(Err(e)) => Err(e.into_kube()),
                None => Ok((true, None)),
            }
        }
    }

    fn tiny() -> Timing {
        Timing {
            start_timeout: Duration::from_millis(80),
            poll: Duration::from_millis(5),
            delete_bound: Duration::from_millis(200),
        }
    }

    fn harness() -> (
        BackendState,
        Sender<BackendEventEnvelope>,
        crossbeam_channel::Receiver<BackendEventEnvelope>,
    ) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut state = BackendState::new();
        state.cluster.current_context = Some("kind-test".to_owned());
        (state, tx, rx)
    }

    /// `(session_id, phase, detail, leftover)` for every status event so far.
    fn statuses(
        rx: &crossbeam_channel::Receiver<BackendEventEnvelope>,
    ) -> Vec<(u64, NodeShellPhase, Option<String>, bool)> {
        rx.try_iter()
            .filter_map(|e| match e.event {
                BackendEvent::System(SystemEvent::NodeShellStatus {
                    session_id,
                    phase,
                    detail,
                    leftover,
                    ..
                }) => Some((session_id, phase, detail, leftover)),
                _ => None,
            })
            .collect()
    }

    fn phases(v: &[(u64, NodeShellPhase, Option<String>, bool)]) -> Vec<NodeShellPhase> {
        v.iter().map(|s| s.1).collect()
    }

    async fn started(
        api: &Arc<FakeApi>,
    ) -> (
        BackendState,
        crossbeam_channel::Receiver<BackendEventEnvelope>,
        Sender<BackendEventEnvelope>,
    ) {
        let (mut state, tx, rx) = harness();
        start_with_api(&mut state, &tx, api.clone(), params(), tiny())
            .await
            .unwrap();
        // Let the watcher reach Running.
        for _ in 0..50 {
            if statuses_peek(&rx).contains(&NodeShellPhase::Running) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        (state, rx, tx)
    }

    fn statuses_peek(
        rx: &crossbeam_channel::Receiver<BackendEventEnvelope>,
    ) -> Vec<NodeShellPhase> {
        // Non-destructive: re-collect by draining into a buffer is unnecessary here, since the
        // helpers below only need to know whether Running was seen; drain and re-inject nothing.
        // (Callers use `statuses` for the full log afterwards, so keep a side log.)
        PEEK.with(|p| {
            let mut p = p.borrow_mut();
            p.extend(phases(&statuses(rx)));
            p.clone()
        })
    }

    thread_local! {
        static PEEK: std::cell::RefCell<Vec<NodeShellPhase>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    // ---- US1: create -> Running -> Attached (T011) ----

    #[tokio::test]
    async fn create_watch_running_then_attach() {
        PEEK.with(|p| p.borrow_mut().clear());
        let api = Arc::new(FakeApi::default());
        let (mut state, rx, _tx) = started(&api).await;
        assert_eq!(api.creates.lock().unwrap().len(), 1);
        let seen = PEEK.with(|p| p.borrow().clone());
        assert_eq!(seen.first(), Some(&NodeShellPhase::Creating));
        assert!(seen.contains(&NodeShellPhase::Pending));
        assert!(seen.contains(&NodeShellPhase::Running));

        let pod = state.node_shell.session.as_ref().unwrap().ident.pod.clone();
        // An exec on some other pod does not mark the session attached.
        note_exec_ready(&mut state, "default", "someone-else");
        assert!(statuses(&rx).is_empty());
        note_exec_ready(&mut state, "default", &pod);
        assert_eq!(phases(&statuses(&rx)), vec![NodeShellPhase::Attached]);
    }

    #[tokio::test]
    async fn stale_stop_is_discarded() {
        let api = Arc::new(FakeApi::default());
        let (mut state, rx, _tx) = started(&api).await;
        let _ = statuses(&rx);
        stop(&mut state, 999).await;
        assert!(api.deletes.lock().unwrap().is_empty());
        assert!(state.node_shell.session.is_some());
    }

    // ---- US2: one test per end path (T015) ----

    fn assert_clean_end(
        api: &FakeApi,
        state: &BackendState,
        rx: &crossbeam_channel::Receiver<BackendEventEnvelope>,
    ) {
        assert!(state.node_shell.session.is_none());
        assert!(api.pods.lock().unwrap().is_empty(), "pod must be gone");
        let ph = phases(&statuses(rx));
        let n = ph.len();
        assert_eq!(
            ph[n - 2..],
            [NodeShellPhase::Terminating, NodeShellPhase::Removed]
        );
    }

    #[tokio::test]
    async fn end_path_stop() {
        let api = Arc::new(FakeApi::default());
        let (mut state, rx, _tx) = started(&api).await;
        let _ = statuses(&rx);
        let id = state.node_shell.session.as_ref().unwrap().ident.session_id;
        stop(&mut state, id).await;
        assert_clean_end(&api, &state, &rx);
    }

    #[tokio::test]
    async fn end_path_tab_close_while_pending_cancels_watcher_and_deletes() {
        let api = Arc::new(FakeApi::default());
        *api.phase.lock().unwrap() = Some(PodStatus {
            phase: Some("Pending".to_owned()),
            ..Default::default()
        });
        let (mut state, tx, rx) = harness();
        let mut t = tiny();
        t.start_timeout = Duration::from_secs(30);
        start_with_api(&mut state, &tx, api.clone(), params(), t)
            .await
            .unwrap();
        let id = state.node_shell.session.as_ref().unwrap().ident.session_id;
        stop(&mut state, id).await;
        assert_clean_end(&api, &state, &rx);
    }

    #[tokio::test]
    async fn end_path_terminal_exit_stops_attached_session() {
        let api = Arc::new(FakeApi::default());
        let (mut state, rx, _tx) = started(&api).await;
        let pod = state.node_shell.session.as_ref().unwrap().ident.pod.clone();
        note_exec_ready(&mut state, "default", &pod);
        let _ = statuses(&rx);
        // The UI sends StopNodeShell when the stream closes with "exit".
        let id = state.node_shell.session.as_ref().unwrap().ident.session_id;
        stop(&mut state, id).await;
        assert_clean_end(&api, &state, &rx);
    }

    #[tokio::test]
    async fn end_path_supersede_deletes_first_before_creating_second() {
        let api = Arc::new(FakeApi::default());
        let (mut state, rx, tx) = started(&api).await;
        let first = api.creates.lock().unwrap()[0].clone();
        let _ = statuses(&rx);
        start_with_api(&mut state, &tx, api.clone(), params(), tiny())
            .await
            .unwrap();
        assert_eq!(api.deletes.lock().unwrap()[0], format!("default/{first}"));
        let creates = api.creates.lock().unwrap().clone();
        assert_eq!(creates.len(), 2);
        assert!(
            !api.pods
                .lock()
                .unwrap()
                .contains_key(&("default".to_owned(), first))
        );
        let log = statuses(&rx);
        let removed = log
            .iter()
            .position(|s| s.1 == NodeShellPhase::Removed)
            .unwrap();
        let creating = log
            .iter()
            .position(|s| s.1 == NodeShellPhase::Creating)
            .unwrap();
        assert!(
            removed < creating,
            "old pod removed before new Creating: {log:?}"
        );
        assert_eq!(log[creating].0, 2, "new session gets a new generation id");
    }

    #[tokio::test]
    async fn end_path_context_switch_uses_captured_client() {
        let api = Arc::new(FakeApi::default());
        let (mut state, rx, _tx) = started(&api).await;
        let _ = statuses(&rx);
        // The session's own api (the old cluster's client) is used even after the backend's
        // active context changed.
        state.cluster.current_context = Some("kind-other".to_owned());
        assert!(end_active(&mut state, CONTEXT_SWITCH_BOUND).await);
        assert_clean_end(&api, &state, &rx);
    }

    #[tokio::test]
    async fn end_path_app_exit_is_bounded() {
        let api = Arc::new(FakeApi::default());
        let (mut state, rx, _tx) = started(&api).await;
        let _ = statuses(&rx);
        assert!(end_active(&mut state, APP_EXIT_BOUND).await);
        assert_clean_end(&api, &state, &rx);
    }

    #[tokio::test]
    async fn delete_timeout_is_a_leftover_not_a_hang() {
        struct Hang(FakeApi);
        #[async_trait]
        impl NodeShellApi for Hang {
            async fn create(&self, p: &Pod) -> Result<Pod, kube::Error> {
                self.0.create(p).await
            }
            async fn get(&self, n: &str, m: &str) -> Result<Pod, kube::Error> {
                self.0.get(n, m).await
            }
            async fn delete(&self, _: &str, _: &str) -> Result<(), kube::Error> {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                Ok(())
            }
            async fn list(&self, n: Option<&str>) -> Result<Vec<Pod>, kube::Error> {
                self.0.list(n).await
            }
            async fn review(
                &self,
                a: ResourceAttributes,
            ) -> Result<(bool, Option<String>), kube::Error> {
                self.0.review(a).await
            }
        }
        let (mut state, tx, rx) = harness();
        start_with_api(
            &mut state,
            &tx,
            Arc::new(Hang(FakeApi::default())),
            params(),
            tiny(),
        )
        .await
        .unwrap();
        let _ = statuses(&rx);
        assert!(!end_active(&mut state, Duration::from_millis(30)).await);
        let log = statuses(&rx);
        let last = log.last().unwrap();
        assert_eq!(last.1, NodeShellPhase::Failed);
        assert!(last.3, "leftover");
        assert!(last.2.as_ref().unwrap().contains("Timed out"));
    }

    #[tokio::test]
    async fn delete_fails_leftover_then_retry_succeeds() {
        let api = Arc::new(FakeApi::default());
        let (mut state, rx, _tx) = started(&api).await;
        let _ = statuses(&rx);
        *api.delete_err.lock().unwrap() = Some(ApiErr(403, "pods \"x\" is forbidden"));
        let id = state.node_shell.session.as_ref().unwrap().ident.session_id;
        stop(&mut state, id).await;
        let log = statuses(&rx);
        let last = log.last().unwrap();
        assert_eq!(last.1, NodeShellPhase::Failed);
        assert!(last.3, "leftover flag set");
        assert!(
            last.2.as_ref().unwrap().contains("forbidden"),
            "verbatim: {last:?}"
        );
        assert!(
            !phases(&log).contains(&NodeShellPhase::Removed),
            "Removed only after the backend confirms deletion"
        );
        assert!(state.node_shell.session.is_some(), "kept for retry");

        *api.delete_err.lock().unwrap() = None;
        stop(&mut state, id).await;
        assert_clean_end(&api, &state, &rx);
    }

    #[tokio::test]
    async fn not_found_on_delete_counts_as_removed() {
        let api = Arc::new(FakeApi::default());
        let (mut state, rx, _tx) = started(&api).await;
        let _ = statuses(&rx);
        // Someone else already deleted it: the fake now answers 404.
        api.pods.lock().unwrap().clear();
        let id = state.node_shell.session.as_ref().unwrap().ident.session_id;
        stop(&mut state, id).await;
        assert_clean_end(&api, &state, &rx);
    }

    // ---- US3: failure mapping (T023) ----

    #[tokio::test]
    async fn admission_error_is_verbatim_and_creates_nothing() {
        let api = Arc::new(FakeApi::default());
        api.create_errs.lock().unwrap().push(ApiErr(
            403,
            "pods \"x\" is forbidden: violates PodSecurity \"restricted:latest\": privileged",
        ));
        let (mut state, tx, rx) = harness();
        start_with_api(&mut state, &tx, api.clone(), params(), tiny())
            .await
            .unwrap();
        let log = statuses(&rx);
        assert_eq!(
            phases(&log),
            vec![NodeShellPhase::Creating, NodeShellPhase::Failed]
        );
        assert!(log[1].2.as_ref().unwrap().contains("violates PodSecurity"));
        assert!(!log[1].3);
        assert!(api.deletes.lock().unwrap().is_empty());
        assert!(state.node_shell.session.is_none());
    }

    #[tokio::test]
    async fn name_collision_retried_once_then_reported() {
        let api = Arc::new(FakeApi::default());
        api.create_errs
            .lock()
            .unwrap()
            .push(ApiErr(409, "already exists"));
        let (mut state, tx, rx) = harness();
        start_with_api(&mut state, &tx, api.clone(), params(), tiny())
            .await
            .unwrap();
        assert_eq!(
            api.creates.lock().unwrap().len(),
            1,
            "second attempt succeeded"
        );
        let _ = statuses(&rx);

        let api = Arc::new(FakeApi::default());
        api.create_errs
            .lock()
            .unwrap()
            .push(ApiErr(409, "already exists"));
        api.create_errs
            .lock()
            .unwrap()
            .push(ApiErr(409, "already exists again"));
        let (mut state, tx, rx) = harness();
        start_with_api(&mut state, &tx, api.clone(), params(), tiny())
            .await
            .unwrap();
        let log = statuses(&rx);
        assert_eq!(log.last().unwrap().1, NodeShellPhase::Failed);
        assert!(
            log.last()
                .unwrap()
                .2
                .as_ref()
                .unwrap()
                .contains("already exists again")
        );
    }

    #[tokio::test]
    async fn pending_timeout_shows_conditions_and_deletes_the_pod() {
        let api = Arc::new(FakeApi::default());
        *api.phase.lock().unwrap() = Some(PodStatus {
            phase: Some("Pending".to_owned()),
            container_statuses: Some(vec![ContainerStatus {
                state: Some(ContainerState {
                    waiting: Some(ContainerStateWaiting {
                        reason: Some("ImagePullBackOff".to_owned()),
                        message: None,
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ..Default::default()
        });
        let (mut state, tx, rx) = harness();
        start_with_api(&mut state, &tx, api.clone(), params(), tiny())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        let log = statuses(&rx);
        let failed = log.iter().find(|s| s.1 == NodeShellPhase::Failed).unwrap();
        assert!(
            failed.2.as_ref().unwrap().contains("ImagePullBackOff"),
            "{failed:?}"
        );
        assert!(phases(&log).ends_with(&[NodeShellPhase::Terminating, NodeShellPhase::Removed]));
        assert!(api.pods.lock().unwrap().is_empty());
        // Pending detail was reported (once, deduplicated).
        assert_eq!(
            log.iter()
                .filter(|s| s.1 == NodeShellPhase::Pending && s.2.is_some())
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn pod_that_fails_on_its_own_is_cleaned_up() {
        let api = Arc::new(FakeApi::default());
        *api.phase.lock().unwrap() = Some(PodStatus {
            phase: Some("Failed".to_owned()),
            reason: Some("NodeAffinity".to_owned()),
            ..Default::default()
        });
        let (mut state, tx, rx) = harness();
        start_with_api(&mut state, &tx, api.clone(), params(), tiny())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let log = statuses(&rx);
        assert!(log.iter().any(|s| s.1 == NodeShellPhase::Failed
            && s.2.as_ref().is_some_and(|d| d.contains("NodeAffinity"))));
        assert!(api.pods.lock().unwrap().is_empty());
    }

    // ---- access check (T020) ----

    #[tokio::test]
    async fn access_check_all_allowed() {
        let api = FakeApi::default();
        let checks = check_access(&api, "default").await;
        assert_eq!(checks.len(), 5);
        assert!(checks.iter().all(NodeShellAccessCheck::is_allowed));
        let nodes = checks.iter().find(|c| c.resource == "nodes").unwrap();
        assert_eq!(nodes.namespace, None);
        let exec = checks
            .iter()
            .find(|c| c.subresource.as_deref() == Some("exec"))
            .unwrap();
        assert_eq!(exec.namespace.as_deref(), Some("default"));
    }

    #[tokio::test]
    async fn access_check_per_permission_denied_and_error_fail_safe() {
        let api = FakeApi::default();
        {
            let mut s = api.review_script.lock().unwrap();
            s.insert(
                "create/pods/exec".to_owned(),
                Ok((false, Some("no exec".to_owned()))),
            );
            s.insert("get/nodes/".to_owned(), Err(ApiErr(500, "boom")));
        }
        let checks = check_access(&api, "default").await;
        let by = |v: &str, r: &str, sub: Option<&str>| {
            checks
                .iter()
                .find(|c| c.verb == v && c.resource == r && c.subresource.as_deref() == sub)
                .unwrap()
        };
        assert!(by("create", "pods", None).is_allowed());
        assert_eq!(
            by("create", "pods", Some("exec")).outcome,
            NodeShellCheckOutcome::Denied {
                reason: Some("no exec".to_owned())
            }
        );
        match &by("get", "nodes", None).outcome {
            NodeShellCheckOutcome::Error { message } => assert!(message.contains("boom")),
            other => panic!("{other:?}"),
        }
        assert!(
            !checks.iter().all(NodeShellAccessCheck::is_allowed),
            "errors fail safe"
        );
    }

    // ---- orphans (T018) ----

    #[tokio::test]
    async fn orphan_listing_includes_terminal_phase_and_excludes_active() {
        let api = FakeApi::default();
        for (name, phase) in [("a", "Failed"), ("b", "Running")] {
            let mut pod = build_pod(name, &params(), "2026-01-01T00:00:00Z");
            pod.status = Some(PodStatus {
                phase: Some(phase.to_owned()),
                ..Default::default()
            });
            api.pods
                .lock()
                .unwrap()
                .insert(("default".to_owned(), name.to_owned()), pod);
        }
        // An unrelated pod is never listed.
        api.pods.lock().unwrap().insert(
            ("default".to_owned(), "web".to_owned()),
            Pod {
                metadata: ObjectMeta {
                    name: Some("web".to_owned()),
                    namespace: Some("default".to_owned()),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let mut all = list_orphans(&api, None, None).await.unwrap();
        all.sort_by(|x, y| x.name.cmp(&y.name));
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].phase.as_deref(), Some("Failed"));
        assert_eq!(all[0].node.as_deref(), Some("worker-1"));
        let without = list_orphans(&api, None, Some(("default", "b")))
            .await
            .unwrap();
        assert_eq!(without.len(), 1);
    }

    #[tokio::test]
    async fn orphan_delete_refuses_foreign_and_active_pods_and_tolerates_notfound() {
        let api = FakeApi::default();
        api.pods.lock().unwrap().insert(
            ("default".to_owned(), "ours".to_owned()),
            build_pod("ours", &params(), "t"),
        );
        api.pods.lock().unwrap().insert(
            ("default".to_owned(), "web".to_owned()),
            Pod {
                metadata: ObjectMeta {
                    name: Some("web".to_owned()),
                    namespace: Some("default".to_owned()),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let r = |n: &str| NodeShellPodRef {
            namespace: "default".to_owned(),
            name: n.to_owned(),
        };
        let failures = delete_orphans(
            &api,
            &[r("ours"), r("web"), r("gone"), r("active")],
            Some(("default", "active")),
        )
        .await;
        assert_eq!(failures.len(), 2, "{failures:?}");
        assert!(
            failures
                .iter()
                .any(|f| f.contains("web") && f.contains("not a Fathom"))
        );
        assert!(
            failures
                .iter()
                .any(|f| f.contains("active node shell session"))
        );
        assert_eq!(
            *api.deletes.lock().unwrap(),
            vec!["default/ours".to_owned()]
        );
        assert!(
            api.pods
                .lock()
                .unwrap()
                .contains_key(&("default".to_owned(), "web".to_owned()))
        );
    }

    /// Live validation against a real cluster (ignored by default):
    /// `FATHOM_NS_LIVE_CONTEXT=kind-fathom-dev FATHOM_NS_LIVE_NODE=<node> cargo test --lib live_node_shell -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "needs a live cluster"]
    async fn live_node_shell_create_attach_delete() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let context = std::env::var("FATHOM_NS_LIVE_CONTEXT").expect("FATHOM_NS_LIVE_CONTEXT");
        let node = std::env::var("FATHOM_NS_LIVE_NODE").expect("FATHOM_NS_LIVE_NODE");
        let cfg = kube::Config::from_kubeconfig(&kube::config::KubeConfigOptions {
            context: Some(context.clone()),
            ..Default::default()
        })
        .await
        .expect("kubeconfig");
        let client = kube::Client::try_from(cfg).expect("client");

        let checks = check_access(&KubeApi(client.clone()), "default").await;
        println!("access checks: {checks:?}");

        let (mut state, tx, rx) = harness();
        state.cluster.current_context = Some(context);
        state.cluster.client = Some(client.clone());
        let params = StartParams {
            node,
            namespace: "default".to_owned(),
            image: "busybox:stable".to_owned(),
            image_pull_secret: None,
            deadline_seconds: 600,
        };
        start(&mut state, &tx, params).await.unwrap();
        let (pod, ns) = {
            let s = state.node_shell.session.as_ref().unwrap();
            (s.ident.pod.clone(), s.ident.namespace.clone())
        };
        println!("created pod {ns}/{pod}");

        let mut seen: Vec<NodeShellPhase> = Vec::new();
        for _ in 0..240 {
            seen.extend(phases(&statuses(&rx)));
            if seen.contains(&NodeShellPhase::Running) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        assert!(seen.contains(&NodeShellPhase::Running), "phases: {seen:?}");

        super::super::exec::start_exec(
            &mut state,
            &tx,
            ns.clone(),
            pod.clone(),
            Some("shell".to_owned()),
        )
        .await
        .unwrap();
        let mut ready = false;
        for _ in 0..80 {
            for e in rx.try_iter() {
                match e.event {
                    BackendEvent::Stream(crate::protocol::StreamEvent::PodExecReady { .. }) => {
                        ready = true
                    }
                    BackendEvent::System(SystemEvent::NodeShellStatus {
                        phase: NodeShellPhase::Attached,
                        ..
                    }) => seen.push(NodeShellPhase::Attached),
                    _ => {}
                }
            }
            if ready && seen.contains(&NodeShellPhase::Attached) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        assert!(ready, "PodExecReady not seen");
        assert!(seen.contains(&NodeShellPhase::Attached), "phases: {seen:?}");

        let id = state.node_shell.session.as_ref().unwrap().ident.session_id;
        stop(&mut state, id).await;
        assert!(state.node_shell.session.is_none());
        let api: Api<Pod> = Api::namespaced(client, &ns);
        let gone = matches!(api.get(&pod).await, Err(e) if is_not_found(&e));
        assert!(gone, "pod {ns}/{pod} still exists after stop");
        println!("pod deleted");
    }
}
