//! Round-trip tests for spec 068's wire types (converge, check runs, required checks, profile
//! suggestions) and the defaults that keep older payloads readable (T012).

use std::collections::BTreeMap;

use keel_core::uuid::Uuid;
use keel_protocol::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;

fn round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(
    value: &T,
) -> serde_json::Value {
    let text = serde_json::to_string(value).unwrap();
    let back: T = serde_json::from_str(&text).unwrap();
    assert_eq!(&back, value, "{text}");
    serde_json::from_str(&text).unwrap()
}

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn step(position: u16, job: &str) -> PlannedStep {
    PlannedStep {
        position,
        job: job.into(),
        via: vec!["web".into(), "docker".into()],
        params: BTreeMap::from([("channel".into(), "stable".into())]),
        credential_names: vec!["cf-token".into()],
    }
}

fn run_info() -> RunInfo {
    RunInfo {
        id: id(1),
        workspace: id(2),
        binding: Some(id(3)),
        job: None,
        job_name: "converge".into(),
        trigger: RunTrigger::Manual,
        requested_by: Some(id(9)),
        commit: "a".repeat(40),
        inventory: "fleet".into(),
        inventory_revision: Some(id(4)),
        newer_inventory_revision: false,
        limit: None,
        hosts: vec!["web-1".into()],
        policy: None,
        state: RunPhase::WaitingChecks,
        reason: None,
        created_at: "2026-11-17T10:00:00Z".into(),
        started_at: None,
        ended_at: None,
        summary: None,
        params: BTreeMap::new(),
        approval: None,
        kind: RunKind::Converge,
        check: true,
        parent_run: None,
    }
}

#[test]
fn converge_plan_and_target_round_trip() {
    for hosts in [
        TargetSelection::All,
        TargetSelection::Hosts {
            ids: vec![id(7), id(8)],
        },
    ] {
        round_trip(&ConvergeTarget {
            binding_id: id(3),
            hosts,
            check: false,
        });
    }
    let v = round_trip(&TargetSelection::All);
    assert_eq!(v, json!({ "kind": "all" }));

    let plan = ConvergePlan {
        binding_id: id(3),
        commit: "a".repeat(40),
        inventory_revision: id(4),
        check: true,
        hosts: vec![HostPlan {
            host_id: id(7),
            host: "web-1".into(),
            profiles: vec!["web".into()],
            steps: vec![step(1, "docker-install"), step(2, "web-deploy")],
            refusals: vec![StepRefusal {
                job: "k3s-agent".into(),
                reason: StepRefusalReason::OutsideCeiling,
            }],
        }],
        policy: ApprovalTerms {
            requires_approval: true,
            approvals_needed: 2,
            four_eyes: true,
            approval_ttl_secs: 3600,
        },
        review: vec![ReviewGroupSummary {
            job: "web-deploy".into(),
            commit: "a".repeat(40),
            needs_review: false,
        }],
        replaces: vec![id(11)],
        blocked_by: vec![],
    };
    let v = round_trip(&plan);
    assert_eq!(v["hosts"][0]["refusals"][0]["reason"], "outside_ceiling");
    // A target without `check` reads as a real run.
    let t: ConvergeTarget =
        serde_json::from_value(json!({ "binding_id": id(3), "hosts": { "kind": "all" } })).unwrap();
    assert!(!t.check);
}

#[test]
fn converge_view_round_trips_with_every_step_state() {
    let states = [
        StepState::Blocked,
        StepState::Queued,
        StepState::Sent,
        StepState::Running,
        StepState::Succeeded,
        StepState::Failed,
        StepState::Refused,
        StepState::Lost,
        StepState::Cancelled,
        StepState::Skipped,
        StepState::Covered,
    ];
    let steps = states
        .iter()
        .enumerate()
        .map(|(i, s)| ConvergeStepView {
            position: u16::try_from(i + 1).unwrap(),
            job: format!("j{i}"),
            run_id: id(100 + i as u128),
            state: *s,
            reason: None,
            started_at: Some("2026-11-17T10:00:00Z".into()),
            ended_at: None,
        })
        .collect();
    let view = ConvergeView {
        run: run_info(),
        hosts: vec![ConvergeHostView {
            host_id: id(7),
            host: "web-1".into(),
            state: ConvergeHostState::Holding,
            steps,
        }],
        retry_of: Some(id(12)),
    };
    let v = round_trip(&view);
    assert_eq!(v["run"]["state"], "waiting_checks");
    assert_eq!(v["run"]["kind"], "converge");
    assert_eq!(v["hosts"][0]["steps"][10]["state"], "covered");
    for s in [
        ConvergeHostState::Waiting,
        ConvergeHostState::Holding,
        ConvergeHostState::Succeeded,
        ConvergeHostState::Failed,
        ConvergeHostState::Cancelled,
    ] {
        round_trip(&s);
    }
}

#[test]
fn suggestions_profiles_and_checks_round_trip() {
    round_trip(&ProfileSuggestion {
        host_id: id(7),
        profile: "web".into(),
        action: SuggestionAction::Remove,
        groups: vec!["webservers".into()],
        first_seen_at: "2026-11-17T10:00:00Z".into(),
    });
    round_trip(&HostProfilesPreview {
        change: HostProfilesChange {
            host_id: id(7),
            add: vec!["web".into()],
            remove: vec![],
        },
        outside_ceiling: vec!["web".into()],
        principals_after: vec!["web".into(), "debian".into()],
        plan_after: vec![step(1, "web-deploy")],
    });
    for state in [
        CheckState::Pending,
        CheckState::Success,
        CheckState::Failure,
        CheckState::Error,
        CheckState::Missing,
    ] {
        round_trip(&RunCheckView {
            context: "ci/lint".into(),
            state,
            target_url: None,
        });
    }
    round_trip(&ConvergeHostSummary {
        run_id: id(1),
        state: ConvergeHostState::Waiting,
        step: 2,
        of: 5,
    });
}

#[test]
fn new_variants_are_spelled_snake_case() {
    assert_eq!(json!(RunPhase::WaitingChecks), "waiting_checks");
    for (r, s) in [
        (RunReason::ChecksFailed, "checks_failed"),
        (RunReason::ChecksTimeout, "checks_timeout"),
        (RunReason::Replaced, "replaced"),
        (RunReason::RunnerTooOld, "runner_too_old"),
    ] {
        assert_eq!(json!(r), s);
    }
    for (h, s) in [
        (HostPhase::Blocked, "blocked"),
        (HostPhase::Skipped, "skipped"),
        (HostPhase::Covered, "covered"),
    ] {
        assert_eq!(json!(h), s);
    }
    assert_eq!(json!(CredentialKind::EnvValue), "env_value");
    assert_eq!(json!(RunKind::default()), "normal");
}

#[test]
fn older_payloads_still_parse_with_defaults() {
    // RunInfo from before 068: no kind, check or parent_run.
    let mut v = serde_json::to_value(run_info()).unwrap();
    for k in ["kind", "check", "parent_run"] {
        v.as_object_mut().unwrap().remove(k);
    }
    let r: RunInfo = serde_json::from_value(v).unwrap();
    assert_eq!(
        (r.kind, r.check, r.parent_run),
        (RunKind::Normal, false, None)
    );

    // PreviewRunStart from before 068: no params or check.
    let cmd: Cmd = serde_json::from_value(json!({
        "type": "PreviewRunStart",
        "data": { "job": id(4) },
    }))
    .unwrap();
    match cmd {
        Cmd::PreviewRunStart {
            params,
            check,
            limit,
            ..
        } => assert!(params.is_empty() && !check && limit.is_none()),
        other => panic!("{other:?}"),
    }
    let with = Cmd::PreviewRunStart {
        job: id(4),
        limit: None,
        params: BTreeMap::from([("channel".into(), "beta".into())]),
        check: true,
    };
    round_trip(&with);
}
