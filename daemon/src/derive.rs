//! Status-diff derivation of controller-level events and coalescing (spec 036 R4, FR-003).
//!
//! Each `derive_*` compares the previously seen object with the new one and returns the
//! transitions worth telling an operator. Messages are taken verbatim from status/Events
//! (capped at 4 KiB); no Secret data is ever read or emitted (Secrets are not watched).

use chrono::{DateTime, Duration, Utc};
use fathom::protocol::v1::{
    DerivedCategory, DerivedControllerEvent, DerivedOrigin, DerivedSeverity,
};
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, StatefulSet};
use k8s_openapi::api::autoscaling::v2::HorizontalPodAutoscaler;
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{Event, Pod};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use std::collections::HashMap;

pub const MAX_MESSAGE_BYTES: usize = 4096;
const TRUNC_MARKER: &str = "…truncated";

/// Cap a message at 4 KiB total (marker included) on a char boundary.
pub fn cap_message(s: &str) -> String {
    if s.len() <= MAX_MESSAGE_BYTES {
        return s.to_string();
    }
    let mut end = MAX_MESSAGE_BYTES - TRUNC_MARKER.len();
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{TRUNC_MARKER}", &s[..end])
}

fn mk(
    kind: &str,
    m: &ObjectMeta,
    category: DerivedCategory,
    severity: DerivedSeverity,
    reason: &str,
    message: &str,
) -> DerivedControllerEvent {
    DerivedControllerEvent {
        origin: DerivedOrigin::Derived,
        kind: kind.to_string(),
        namespace: m.namespace.clone(),
        name: m.name.clone().unwrap_or_default(),
        uid: m.uid.clone().unwrap_or_default(),
        category,
        severity,
        reason: reason.to_string(),
        message: cap_message(message),
        repeat: 1,
    }
}

/// Normalized condition across the differently typed k8s condition structs.
struct Cond {
    type_: String,
    status: String,
    reason: String,
    message: String,
}

impl Cond {
    fn is_true(&self) -> bool {
        self.status == "True"
    }
}

fn find<'a>(cs: &'a [Cond], t: &str) -> Option<&'a Cond> {
    cs.iter().find(|c| c.type_ == t)
}

/// True when `t` is present with status `True` now and was not before.
fn newly_true<'a>(prev: &[Cond], new: &'a [Cond], t: &str) -> Option<&'a Cond> {
    let n = find(new, t).filter(|c| c.is_true())?;
    (!find(prev, t).is_some_and(Cond::is_true)).then_some(n)
}

fn started(kind: &str, m: &ObjectMeta, prev: &ObjectMeta) -> Option<DerivedControllerEvent> {
    let (a, b) = (prev.generation?, m.generation?);
    (a != b).then(|| {
        mk(
            kind,
            m,
            DerivedCategory::Rollout,
            DerivedSeverity::Info,
            "RolloutStarted",
            &format!("generation {a} -> {b}"),
        )
    })
}

fn complete_event(
    kind: &str,
    m: &ObjectMeta,
    done: (bool, bool),
    msg: &str,
) -> Option<DerivedControllerEvent> {
    (!done.0 && done.1).then(|| {
        mk(
            kind,
            m,
            DerivedCategory::Rollout,
            DerivedSeverity::Info,
            "RolloutComplete",
            msg,
        )
    })
}

fn deployment_done(d: &Deployment) -> bool {
    let want = d.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
    let st = d.status.as_ref();
    let g = |f: fn(&k8s_openapi::api::apps::v1::DeploymentStatus) -> Option<i32>| {
        st.and_then(f).unwrap_or(0)
    };
    st.and_then(|s| s.observed_generation).unwrap_or(0) >= d.metadata.generation.unwrap_or(0)
        && g(|s| s.updated_replicas) == want
        && g(|s| s.available_replicas) == want
        && g(|s| s.replicas) == want
}

fn deployment_conds(d: &Deployment) -> Vec<Cond> {
    d.status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .into_iter()
        .flatten()
        .map(|c| Cond {
            type_: c.type_.clone(),
            status: c.status.clone(),
            reason: c.reason.clone().unwrap_or_default(),
            message: c.message.clone().unwrap_or_default(),
        })
        .collect()
}

pub fn derive_deployment(prev: &Deployment, new: &Deployment) -> Vec<DerivedControllerEvent> {
    let mut out = Vec::new();
    out.extend(started("Deployment", &new.metadata, &prev.metadata));
    let (pc, nc) = (deployment_conds(prev), deployment_conds(new));
    let failing = |cs: &[Cond]| {
        cs.iter()
            .any(|c| c.type_ == "Progressing" && c.reason == "ProgressDeadlineExceeded")
    };
    if failing(&nc) && !failing(&pc) {
        let msg = find(&nc, "Progressing")
            .map(|c| c.message.as_str())
            .unwrap_or("");
        out.push(mk(
            "Deployment",
            &new.metadata,
            DerivedCategory::Rollout,
            DerivedSeverity::Error,
            "ProgressDeadlineExceeded",
            msg,
        ));
    }
    let want = new.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
    out.extend(complete_event(
        "Deployment",
        &new.metadata,
        (deployment_done(prev), deployment_done(new)),
        &format!("{want} replicas updated and available"),
    ));
    out
}

pub fn derive_statefulset(prev: &StatefulSet, new: &StatefulSet) -> Vec<DerivedControllerEvent> {
    let done = |s: &StatefulSet| {
        let want = s.spec.as_ref().and_then(|x| x.replicas).unwrap_or(1);
        let st = s.status.as_ref();
        st.and_then(|x| x.observed_generation).unwrap_or(0) >= s.metadata.generation.unwrap_or(0)
            && st.and_then(|x| x.updated_replicas).unwrap_or(0) == want
            && st.and_then(|x| x.ready_replicas).unwrap_or(0) == want
    };
    let mut out = Vec::new();
    out.extend(started("StatefulSet", &new.metadata, &prev.metadata));
    out.extend(complete_event(
        "StatefulSet",
        &new.metadata,
        (done(prev), done(new)),
        "all replicas updated and ready",
    ));
    out
}

pub fn derive_daemonset(prev: &DaemonSet, new: &DaemonSet) -> Vec<DerivedControllerEvent> {
    let done = |s: &DaemonSet| {
        let Some(st) = s.status.as_ref() else {
            return false;
        };
        let want = st.desired_number_scheduled;
        st.observed_generation.unwrap_or(0) >= s.metadata.generation.unwrap_or(0)
            && st.updated_number_scheduled.unwrap_or(0) == want
            && st.number_available.unwrap_or(0) == want
    };
    let mut out = Vec::new();
    out.extend(started("DaemonSet", &new.metadata, &prev.metadata));
    out.extend(complete_event(
        "DaemonSet",
        &new.metadata,
        (done(prev), done(new)),
        "all nodes updated and available",
    ));
    out
}

pub fn derive_job(prev: &Job, new: &Job) -> Vec<DerivedControllerEvent> {
    let conds = |j: &Job| -> Vec<Cond> {
        j.status
            .as_ref()
            .and_then(|s| s.conditions.as_ref())
            .into_iter()
            .flatten()
            .map(|c| Cond {
                type_: c.type_.clone(),
                status: c.status.clone(),
                reason: c.reason.clone().unwrap_or_default(),
                message: c.message.clone().unwrap_or_default(),
            })
            .collect()
    };
    let (pc, nc) = (conds(prev), conds(new));
    let mut out = Vec::new();
    for (t, sev) in [
        ("Complete", DerivedSeverity::Info),
        ("Failed", DerivedSeverity::Error),
    ] {
        if let Some(c) = newly_true(&pc, &nc, t) {
            let reason = if c.reason.is_empty() { t } else { &c.reason };
            out.push(mk(
                "Job",
                &new.metadata,
                DerivedCategory::Job,
                sev,
                reason,
                &c.message,
            ));
        }
    }
    out
}

pub fn derive_cronjob(prev: &CronJob, new: &CronJob) -> Vec<DerivedControllerEvent> {
    let last = |c: &CronJob| c.status.as_ref().and_then(|s| s.last_schedule_time.clone());
    match (last(prev), last(new)) {
        (p, Some(n)) if p.as_ref() != Some(&n) => vec![mk(
            "CronJob",
            &new.metadata,
            DerivedCategory::Cronjob,
            DerivedSeverity::Info,
            "Scheduled",
            &format!("last schedule {}", n.0),
        )],
        _ => vec![],
    }
}

pub fn derive_hpa(
    prev: &HorizontalPodAutoscaler,
    new: &HorizontalPodAutoscaler,
) -> Vec<DerivedControllerEvent> {
    let cur = |h: &HorizontalPodAutoscaler| h.status.as_ref().and_then(|s| s.current_replicas);
    let conds = |h: &HorizontalPodAutoscaler| -> Vec<Cond> {
        h.status
            .as_ref()
            .and_then(|s| s.conditions.as_ref())
            .into_iter()
            .flatten()
            .map(|c| Cond {
                type_: c.type_.clone(),
                status: c.status.clone(),
                reason: c.reason.clone().unwrap_or_default(),
                message: c.message.clone().unwrap_or_default(),
            })
            .collect()
    };
    let mut out = Vec::new();
    if let (Some(a), Some(b)) = (cur(prev), cur(new))
        && a != b
    {
        let want = new.status.as_ref().map_or(b, |s| s.desired_replicas);
        out.push(mk(
            "HorizontalPodAutoscaler",
            &new.metadata,
            DerivedCategory::Scaling,
            DerivedSeverity::Info,
            if b > a { "ScaledUp" } else { "ScaledDown" },
            &format!("replicas {a} -> {b} (desired {want})"),
        ));
    }
    if let Some(c) = newly_true(&conds(prev), &conds(new), "ScalingLimited") {
        let reason = if c.reason.is_empty() {
            "ScalingLimited"
        } else {
            &c.reason
        };
        out.push(mk(
            "HorizontalPodAutoscaler",
            &new.metadata,
            DerivedCategory::Scaling,
            DerivedSeverity::Warning,
            reason,
            &c.message,
        ));
    }
    out
}

fn pod_ready(p: &Pod) -> Option<bool> {
    p.status
        .as_ref()?
        .conditions
        .as_ref()?
        .iter()
        .find(|c| c.type_ == "Ready")
        .map(|c| c.status == "True")
}

fn container_statuses(p: &Pod) -> &[k8s_openapi::api::core::v1::ContainerStatus] {
    p.status
        .as_ref()
        .and_then(|s| s.container_statuses.as_deref())
        .unwrap_or(&[])
}

pub fn derive_pod(prev: &Pod, new: &Pod) -> Vec<DerivedControllerEvent> {
    let mut out = Vec::new();
    let pe = |sev, reason: &str, msg: &str| {
        mk("Pod", &new.metadata, DerivedCategory::Pod, sev, reason, msg)
    };
    let prev_cs = container_statuses(prev);
    for cs in container_statuses(new) {
        let old = prev_cs.iter().find(|c| c.name == cs.name);
        let waiting = |c: &k8s_openapi::api::core::v1::ContainerStatus| {
            c.state.as_ref().and_then(|s| s.waiting.clone())
        };
        if let Some(w) = waiting(cs)
            && w.reason.as_deref() == Some("CrashLoopBackOff")
            && old.and_then(waiting).and_then(|w| w.reason).as_deref() != Some("CrashLoopBackOff")
        {
            out.push(pe(
                DerivedSeverity::Error,
                "CrashLoopBackOff",
                w.message.as_deref().unwrap_or(""),
            ));
        }
        if let Some(old) = old
            && cs.restart_count > old.restart_count
        {
            let term = cs.last_state.as_ref().and_then(|s| s.terminated.as_ref());
            if term.and_then(|t| t.reason.as_deref()) == Some("OOMKilled") {
                let msg = term
                    .and_then(|t| t.message.clone())
                    .unwrap_or_else(|| format!("container {} was OOMKilled", cs.name));
                out.push(pe(DerivedSeverity::Error, "OOMKilled", &msg));
            }
        }
    }
    match (pod_ready(prev), pod_ready(new)) {
        (Some(false), Some(true)) => out.push(pe(DerivedSeverity::Info, "Ready", "")),
        (Some(true), Some(false)) => {
            let msg = new
                .status
                .as_ref()
                .and_then(|s| s.conditions.as_ref())
                .and_then(|cs| cs.iter().find(|c| c.type_ == "Ready"))
                .and_then(|c| c.message.clone())
                .unwrap_or_default();
            out.push(pe(DerivedSeverity::Warning, "NotReady", &msg));
        }
        _ => {}
    }
    out
}

/// Forward a core/v1 Event as a `raw` entry. `None` when it names no involved object.
pub fn raw_event(e: &Event) -> Option<DerivedControllerEvent> {
    let name = e.involved_object.name.clone().filter(|n| !n.is_empty())?;
    let sev = if e.type_.as_deref() == Some("Warning") {
        DerivedSeverity::Warning
    } else {
        DerivedSeverity::Info
    };
    Some(DerivedControllerEvent {
        origin: DerivedOrigin::Raw,
        kind: e.involved_object.kind.clone().unwrap_or_default(),
        namespace: e
            .involved_object
            .namespace
            .clone()
            .or_else(|| e.metadata.namespace.clone()),
        name,
        uid: e.involved_object.uid.clone().unwrap_or_default(),
        category: DerivedCategory::Event,
        severity: sev,
        reason: e.reason.clone().unwrap_or_default(),
        message: cap_message(e.message.as_deref().unwrap_or("")),
        repeat: e.count.unwrap_or(1).max(1) as u32,
    })
}

struct Slot {
    last_emit: DateTime<Utc>,
    pending: u32,
    last: DerivedControllerEvent,
}

/// Per (uid, reason): at most one event per window; suppressed occurrences are counted into
/// `repeat` on the next emission (or a later [`Coalescer::flush_due`]).
pub struct Coalescer {
    window: Duration,
    slots: HashMap<(String, String), Slot>,
}

impl Coalescer {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            slots: HashMap::new(),
        }
    }

    pub fn offer(
        &mut self,
        mut ev: DerivedControllerEvent,
        now: DateTime<Utc>,
    ) -> Option<DerivedControllerEvent> {
        let key = (ev.uid.clone(), ev.reason.clone());
        match self.slots.get_mut(&key) {
            Some(s) if now.signed_duration_since(s.last_emit) < self.window => {
                s.pending += 1;
                s.last = ev;
                None
            }
            Some(s) => {
                ev.repeat = ev.repeat.max(1) + s.pending;
                s.pending = 0;
                s.last_emit = now;
                s.last = ev.clone();
                Some(ev)
            }
            None => {
                self.slots.insert(
                    key,
                    Slot {
                        last_emit: now,
                        pending: 0,
                        last: ev.clone(),
                    },
                );
                Some(ev)
            }
        }
    }

    /// Emit trailing suppressed counts whose window has elapsed and drop idle slots.
    pub fn flush_due(&mut self, now: DateTime<Utc>) -> Vec<DerivedControllerEvent> {
        let mut out = Vec::new();
        let window = self.window;
        self.slots.retain(|_, s| {
            let elapsed = now.signed_duration_since(s.last_emit);
            if s.pending > 0 && elapsed >= window {
                let mut e = s.last.clone();
                e.repeat = s.pending;
                out.push(e);
                s.pending = 0;
                s.last_emit = now;
                return true;
            }
            !(s.pending == 0 && elapsed > window * 12)
        });
        out
    }
}
