//! T011: status-diff derivation and coalescing.

use chrono::{Duration, TimeZone, Utc};
use fathom::protocol::v1::{DerivedCategory, DerivedOrigin, DerivedSeverity};
use fathom_daemon::derive::*;
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, StatefulSet};
use k8s_openapi::api::autoscaling::v2::HorizontalPodAutoscaler;
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{Event, Pod};
use serde_json::{Value, json};

fn de<T: serde::de::DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).unwrap()
}

fn meta(kind_ns: &str) -> Value {
    json!({"name":"x","namespace":kind_ns,"uid":"uid-x","generation":1})
}

fn dep(gen_: i64, obs: i64, replicas: i32, updated: i32, avail: i32, conds: Value) -> Deployment {
    de(json!({
        "metadata": {"name":"web","namespace":"a","uid":"d1","generation":gen_},
        "spec": {"replicas": replicas, "selector": {}, "template": {"spec": {"containers": []}}},
        "status": {"observedGeneration": obs, "replicas": replicas, "updatedReplicas": updated,
                   "availableReplicas": avail, "conditions": conds}
    }))
}

#[test]
fn deployment_progress_deadline_exceeded_is_an_error_once() {
    let ok = json!([{"type":"Progressing","status":"True","reason":"NewReplicaSetAvailable"}]);
    let bad = json!([{"type":"Progressing","status":"False","reason":"ProgressDeadlineExceeded",
        "message":"ReplicaSet \"web-1\" has timed out progressing."}]);
    let evs = derive_deployment(
        &dep(2, 2, 3, 1, 2, ok.clone()),
        &dep(2, 2, 3, 1, 2, bad.clone()),
    );
    assert_eq!(evs.len(), 1);
    let e = &evs[0];
    assert_eq!(e.origin, DerivedOrigin::Derived);
    assert_eq!(e.category, DerivedCategory::Rollout);
    assert_eq!(e.severity, DerivedSeverity::Error);
    assert_eq!(e.reason, "ProgressDeadlineExceeded");
    assert_eq!(e.message, "ReplicaSet \"web-1\" has timed out progressing.");
    assert_eq!(
        (e.kind.as_str(), e.name.as_str(), e.uid.as_str()),
        ("Deployment", "web", "d1")
    );
    assert_eq!(e.namespace.as_deref(), Some("a"));
    assert_eq!(e.repeat, 1);
    // already failing before: no repeat event
    assert!(
        derive_deployment(&dep(2, 2, 3, 1, 2, bad.clone()), &dep(2, 2, 3, 1, 2, bad)).is_empty()
    );
}

#[test]
fn deployment_rollout_started_and_complete() {
    let c = json!([]);
    let started = derive_deployment(
        &dep(1, 1, 3, 3, 3, c.clone()),
        &dep(2, 1, 3, 0, 3, c.clone()),
    );
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].reason, "RolloutStarted");
    assert_eq!(started[0].severity, DerivedSeverity::Info);
    let done = derive_deployment(
        &dep(2, 2, 3, 2, 2, c.clone()),
        &dep(2, 2, 3, 3, 3, c.clone()),
    );
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].reason, "RolloutComplete");
    assert!(derive_deployment(&dep(2, 2, 3, 3, 3, c.clone()), &dep(2, 2, 3, 3, 3, c)).is_empty());
}

#[test]
fn statefulset_rollout() {
    let mk = |gen_: i64, obs: i64, upd: i32, ready: i32| -> StatefulSet {
        de(json!({
            "metadata": {"name":"db","namespace":"a","uid":"s1","generation":gen_},
            "spec": {"replicas": 3, "selector": {}, "serviceName":"db", "template": {"spec": {"containers": []}}},
            "status": {"observedGeneration": obs, "replicas":3, "updatedReplicas": upd, "readyReplicas": ready}
        }))
    };
    assert_eq!(
        derive_statefulset(&mk(1, 1, 3, 3), &mk(2, 1, 0, 3))[0].reason,
        "RolloutStarted"
    );
    let done = derive_statefulset(&mk(2, 2, 1, 2), &mk(2, 2, 3, 3));
    assert_eq!(done[0].reason, "RolloutComplete");
    assert_eq!(done[0].kind, "StatefulSet");
}

#[test]
fn daemonset_rollout_complete() {
    let mk = |upd: i32, avail: i32| -> DaemonSet {
        de(json!({
            "metadata": {"name":"agent","namespace":"a","uid":"ds1","generation":2},
            "spec": {"selector": {}, "template": {"spec": {"containers": []}}},
            "status": {"observedGeneration": 2, "desiredNumberScheduled": 3, "currentNumberScheduled": 3,
                       "numberMisscheduled": 0, "numberReady": avail,
                       "updatedNumberScheduled": upd, "numberAvailable": avail}
        }))
    };
    let d = derive_daemonset(&mk(1, 2), &mk(3, 3));
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].reason, "RolloutComplete");
    assert_eq!(d[0].kind, "DaemonSet");
}

fn job(conds: Value) -> Job {
    de(json!({
        "metadata": {"name":"j","namespace":"a","uid":"j1"},
        "spec": {"template": {"spec": {"containers": [], "restartPolicy":"Never"}}},
        "status": {"conditions": conds}
    }))
}

#[test]
fn job_complete_and_failed() {
    let c = derive_job(
        &job(json!([])),
        &job(json!([{"type":"Complete","status":"True"}])),
    );
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].category, DerivedCategory::Job);
    assert_eq!(c[0].reason, "Complete");
    assert_eq!(c[0].severity, DerivedSeverity::Info);
    let f = derive_job(
        &job(json!([])),
        &job(
            json!([{"type":"Failed","status":"True","reason":"BackoffLimitExceeded",
                     "message":"Job has reached the specified backoff limit"}]),
        ),
    );
    assert_eq!(f[0].reason, "BackoffLimitExceeded");
    assert_eq!(f[0].severity, DerivedSeverity::Error);
    assert_eq!(f[0].message, "Job has reached the specified backoff limit");
    // status False is not an event; unchanged is not an event
    assert!(
        derive_job(
            &job(json!([])),
            &job(json!([{"type":"Failed","status":"False"}]))
        )
        .is_empty()
    );
    let same = job(json!([{"type":"Complete","status":"True"}]));
    assert!(derive_job(&same, &same).is_empty());
}

#[test]
fn cronjob_scheduled() {
    let mk = |t: Option<&str>| -> CronJob {
        de(json!({
            "metadata": {"name":"cj","namespace":"a","uid":"c1"},
            "spec": {"schedule":"* * * * *", "jobTemplate": {"spec": {"template": {"spec": {"containers": [], "restartPolicy":"Never"}}}}},
            "status": t.map(|t| json!({"lastScheduleTime": t})).unwrap_or(json!({}))
        }))
    };
    let e = derive_cronjob(&mk(None), &mk(Some("2026-01-01T00:00:00Z")));
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].category, DerivedCategory::Cronjob);
    assert_eq!(e[0].reason, "Scheduled");
    assert!(
        derive_cronjob(
            &mk(Some("2026-01-01T00:00:00Z")),
            &mk(Some("2026-01-01T00:00:00Z"))
        )
        .is_empty()
    );
}

fn hpa(cur: i32, des: i32, conds: Value) -> HorizontalPodAutoscaler {
    de(json!({
        "metadata": {"name":"h","namespace":"a","uid":"h1"},
        "spec": {"maxReplicas": 10, "scaleTargetRef": {"kind":"Deployment","name":"web"}},
        "status": {"currentReplicas": cur, "desiredReplicas": des, "conditions": conds}
    }))
}

#[test]
fn hpa_scaling_and_limited() {
    let e = derive_hpa(&hpa(2, 2, json!([])), &hpa(4, 5, json!([])));
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].category, DerivedCategory::Scaling);
    assert_eq!(e[0].reason, "ScaledUp");
    assert!(e[0].message.contains("2") && e[0].message.contains("4"));
    assert_eq!(
        derive_hpa(&hpa(4, 4, json!([])), &hpa(2, 2, json!([])))[0].reason,
        "ScaledDown"
    );
    let lim = json!([{"type":"ScalingLimited","status":"True","reason":"TooManyReplicas","message":"the desired replica count is more than the maximum"}]);
    let e = derive_hpa(&hpa(4, 4, json!([])), &hpa(4, 4, lim));
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].reason, "TooManyReplicas");
    assert_eq!(e[0].severity, DerivedSeverity::Warning);
}

fn pod(ready: bool, restarts: i32, waiting: Option<Value>, last_term: Option<Value>) -> Pod {
    let mut cs = json!({"name":"app","image":"i","imageID":"","ready":ready,"restartCount":restarts,
                        "state": {}});
    if let Some(w) = waiting {
        cs["state"] = json!({"waiting": w});
    }
    if let Some(t) = last_term {
        cs["lastState"] = json!({"terminated": t});
    }
    de(json!({
        "metadata": {"name":"p","namespace":"a","uid":"p1"},
        "spec": {"containers": []},
        "status": {"phase":"Running",
                   "conditions":[{"type":"Ready","status": if ready {"True"} else {"False"}}],
                   "containerStatuses":[cs]}
    }))
}

#[test]
fn pod_crashloop_oomkilled_and_ready() {
    let e = derive_pod(
        &pod(true, 0, None, None),
        &pod(
            false,
            0,
            Some(
                json!({"reason":"CrashLoopBackOff","message":"back-off 5m0s restarting failed container"}),
            ),
            None,
        ),
    );
    let crash = e
        .iter()
        .find(|x| x.reason == "CrashLoopBackOff")
        .expect("crash");
    assert_eq!(crash.severity, DerivedSeverity::Error);
    assert_eq!(crash.category, DerivedCategory::Pod);
    assert_eq!(crash.message, "back-off 5m0s restarting failed container");
    assert!(
        e.iter()
            .any(|x| x.reason == "NotReady" && x.severity == DerivedSeverity::Warning)
    );

    let oom = derive_pod(
        &pod(true, 1, None, None),
        &pod(
            true,
            2,
            None,
            Some(json!({"exitCode":137,"reason":"OOMKilled"})),
        ),
    );
    assert!(
        oom.iter()
            .any(|x| x.reason == "OOMKilled" && x.severity == DerivedSeverity::Error)
    );

    let ready = derive_pod(&pod(false, 0, None, None), &pod(true, 0, None, None));
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].reason, "Ready");
    assert_eq!(ready[0].severity, DerivedSeverity::Info);

    // still crash-looping: no duplicate event
    let w = json!({"reason":"CrashLoopBackOff","message":"m"});
    assert!(
        derive_pod(
            &pod(false, 3, Some(w.clone()), None),
            &pod(false, 3, Some(w), None)
        )
        .is_empty()
    );
}

#[test]
fn raw_events_are_forwarded_verbatim() {
    let e: Event = de(json!({
        "metadata": {"name":"ev","namespace":"a","uid":"e1"},
        "involvedObject": {"kind":"Pod","name":"p","namespace":"a","uid":"p1"},
        "type":"Warning","reason":"FailedScheduling","message":"0/3 nodes are available","count":3
    }));
    let d = raw_event(&e).unwrap();
    assert_eq!(d.origin, DerivedOrigin::Raw);
    assert_eq!(d.category, DerivedCategory::Event);
    assert_eq!(d.severity, DerivedSeverity::Warning);
    assert_eq!(
        (d.kind.as_str(), d.name.as_str(), d.uid.as_str()),
        ("Pod", "p", "p1")
    );
    assert_eq!(d.reason, "FailedScheduling");
    assert_eq!(d.message, "0/3 nodes are available");
    assert_eq!(d.repeat, 3);
    let n: Event = de(json!({
        "metadata": {"name":"ev","namespace":"a"},
        "involvedObject": {"kind":"Pod","name":"p"},"type":"Normal","reason":"Pulled","message":"ok"
    }));
    let d = raw_event(&n).unwrap();
    assert_eq!(d.severity, DerivedSeverity::Info);
    assert_eq!(d.repeat, 1);
}

#[test]
fn raw_event_without_involved_name_is_skipped() {
    let e: Event = de(json!({"metadata": {"name":"ev"}, "involvedObject": {}}));
    assert!(raw_event(&e).is_none());
}

#[test]
fn messages_are_capped_at_4kib_on_a_char_boundary() {
    assert_eq!(cap_message("short"), "short");
    let long = "é".repeat(5000);
    let c = cap_message(&long);
    assert!(c.len() <= 4096, "{}", c.len());
    assert!(c.ends_with("…truncated"));
    assert!(c.is_char_boundary(c.len()));
    let exactly = "a".repeat(4096);
    assert_eq!(cap_message(&exactly), exactly);
}

fn t(s: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + s, 0).unwrap()
}

fn one() -> fathom::protocol::v1::DerivedControllerEvent {
    derive_deployment(
        &dep(2, 2, 3, 1, 2, json!([])),
        &dep(
            2,
            2,
            3,
            1,
            2,
            json!([{"type":"Progressing","status":"False","reason":"ProgressDeadlineExceeded"}]),
        ),
    )
    .remove(0)
}

#[test]
fn coalescer_emits_first_then_counts_repeats() {
    let mut c = Coalescer::new(Duration::seconds(5));
    let first = c.offer(one(), t(0)).expect("first passes");
    assert_eq!(first.repeat, 1);
    assert!(c.offer(one(), t(1)).is_none());
    assert!(c.offer(one(), t(2)).is_none());
    let next = c.offer(one(), t(6)).expect("window over");
    assert_eq!(next.repeat, 3, "two suppressed + this one");
    // different reason is a different key
    let mut other = one();
    other.reason = "Other".into();
    assert!(c.offer(other, t(6)).is_some());
}

#[test]
fn coalescer_flushes_trailing_suppressed_count() {
    let mut c = Coalescer::new(Duration::seconds(5));
    c.offer(one(), t(0));
    c.offer(one(), t(1));
    assert!(c.flush_due(t(3)).is_empty(), "window not over");
    let f = c.flush_due(t(6));
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].repeat, 1);
    assert!(c.flush_due(t(20)).is_empty());
    let _ = meta("a");
}
