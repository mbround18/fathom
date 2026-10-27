//! Round-trip tests for the run, job and policy commands and events (spec 055 T071).

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

fn run_info() -> RunInfo {
    RunInfo {
        id: id(1),
        workspace: id(2),
        binding: Some(id(3)),
        job: Some(id(4)),
        job_name: "web".into(),
        trigger: RunTrigger::Delivery,
        requested_by: None,
        commit: "a".repeat(40),
        inventory: "prod".into(),
        inventory_revision: Some(id(5)),
        newer_inventory_revision: true,
        limit: Some("web:!web-3".into()),
        hosts: vec!["web-1".into(), "web-2".into()],
        policy: Some(id(6)),
        state: RunPhase::Failed,
        reason: Some(RunReason::RunnerLost),
        created_at: "2026-10-27T10:00:00Z".into(),
        started_at: Some("2026-10-27T10:00:01Z".into()),
        ended_at: None,
        summary: Some(RunSummary {
            hosts: 2,
            succeeded: 1,
            lost: 1,
            stats: HostStats {
                ok: 3,
                changed: 1,
                ..HostStats::default()
            },
            ..RunSummary::default()
        }),
    }
}

#[test]
fn run_commands() {
    let all = vec![
        Cmd::ListRuns {
            workspace: id(2),
            binding: None,
            host: Some(ObjectName::new("web-1").unwrap()),
            actor: None,
            state: Some(RunPhase::Running),
            from: None,
            to: None,
        },
        Cmd::GetRun { run: id(1) },
        Cmd::SubscribeRun {
            run: id(1),
            from_seq: 42,
        },
        Cmd::UnsubscribeRun { run: id(1) },
        Cmd::PreviewRunStart {
            job: id(4),
            limit: Some(Pattern::new("web-*").unwrap()),
        },
        Cmd::PreviewCancel { run: id(1) },
        Cmd::ListJobs { binding: id(3) },
        Cmd::ListPolicies { workspace: id(2) },
        Cmd::PreviewPolicyUpsert(PolicySpec {
            workspace: id(2),
            id: None,
            priority: 10,
            job: Some(Pattern::new("db-*").unwrap()),
            inventory: Some(ObjectName::new("prod").unwrap()),
            limit: None,
            requires_approval: true,
            approvals_needed: 2,
            four_eyes: true,
            approval_ttl_secs: 3600,
        }),
        Cmd::PreviewPolicyRemove { policy: id(6) },
        Cmd::ExplainPolicy {
            job: id(4),
            limit: None,
        },
    ];
    for cmd in &all {
        round_trip(cmd);
    }
    let v = round_trip(&Cmd::SubscribeRun {
        run: id(1),
        from_seq: 7,
    });
    assert_eq!(v["type"], "SubscribeRun");
    assert_eq!(v["data"]["from_seq"], 7);
    // `from_seq` defaults to the start.
    let c: Cmd =
        serde_json::from_value(json!({"type": "SubscribeRun", "data": {"run": id(1).to_string()}}))
            .unwrap();
    assert_eq!(
        c,
        Cmd::SubscribeRun {
            run: id(1),
            from_seq: 0
        }
    );
}

#[test]
fn only_subscriptions_stream_and_only_reads_are_token_actions() {
    let streaming = [
        Cmd::SubscribeRun {
            run: id(1),
            from_seq: 0,
        },
        Cmd::UnsubscribeRun { run: id(1) },
    ];
    for c in &streaming {
        assert!(c.is_streaming(), "{}", c.name());
        assert!(!TOKEN_ACTIONS.contains(&c.name()));
    }
    for name in [
        "list_runs",
        "get_run",
        "list_jobs",
        "list_policies",
        "explain_policy",
    ] {
        assert!(TOKEN_ACTIONS.contains(&name), "{name}");
    }
    for name in [
        "preview_run_start",
        "preview_cancel",
        "preview_policy_upsert",
        "preview_policy_remove",
    ] {
        assert!(!TOKEN_ACTIONS.contains(&name), "{name}");
    }
}

#[test]
fn inputs_are_bounded_and_unknown_fields_refused() {
    assert!(Pattern::new("").is_err());
    assert!(Pattern::new("a\nb").is_err());
    assert!(Pattern::new("x".repeat(257)).is_err());
    assert!(
        serde_json::from_value::<Cmd>(json!({
            "type": "GetRun", "data": {"run": id(1).to_string(), "extra": 1}
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<Cmd>(json!({
            "type": "PreviewRunStart", "data": {"job": id(1).to_string(), "limit": ""}
        }))
        .is_err()
    );
}

#[test]
fn run_events() {
    let events = vec![
        Event::Runs {
            items: vec![run_info()],
        },
        Event::Run {
            run: run_info(),
            hosts: vec![RunHostInfo {
                host: id(7),
                name: "web-1".into(),
                state: HostPhase::Failed,
                refusal: None,
                started_at: Some("2026-10-27T10:00:01Z".into()),
                ended_at: Some("2026-10-27T10:00:09Z".into()),
                stats: Some(HostStats {
                    unreachable: 1,
                    ..HostStats::default()
                }),
                first_failure: Some(FirstFailure {
                    task: "Gathering Facts".into(),
                    module: None,
                    message: "ssh: connect to host web-1 port 22: No route to host".into(),
                    rc: Some(255),
                    unreachable: true,
                }),
            }],
        },
        Event::RunSubscribed { run: id(1), seq: 9 },
        Event::RunEvent {
            run: id(1),
            seq: 10,
            at: "2026-10-27T10:00:02Z".into(),
            kind: "task_start".into(),
            host: Some("web-1".into()),
            task: Some("install nginx".into()),
            data: "{}".into(),
        },
        Event::RunLog {
            run: id(1),
            seq: 11,
            stream: LogStream::Stdout,
            text: "PLAY [web]\n".into(),
        },
        Event::RunState {
            run: id(1),
            state: RunPhase::Queued,
            reason: None,
        },
        Event::Jobs {
            binding: id(3),
            items: vec![JobInfo {
                id: id(4),
                binding: id(3),
                name: "web".into(),
                playbook: "playbooks/web.yaml".into(),
                profiles: vec!["web".into()],
                inventory: "prod".into(),
                limit: None,
                paths: Some(vec!["playbooks/**".into()]),
                credentials: vec![],
                blocked: None,
                defined_at_commit: "a".repeat(40),
            }],
        },
        Event::Policies {
            workspace: id(2),
            items: vec![PolicyInfo {
                id: id(6),
                workspace: id(2),
                priority: i32::MAX,
                job: None,
                inventory: None,
                limit: None,
                requires_approval: true,
                approvals_needed: 1,
                four_eyes: false,
                approval_ttl_secs: 86_400,
                catch_all: true,
            }],
        },
        Event::PolicyExplanation {
            job: id(4),
            limit: None,
            winner: id(6),
            requires_approval: true,
            reason: "catch-all".into(),
            checks: vec![PolicyCheck {
                policy: id(6),
                priority: i32::MAX,
                matched: true,
                why: "matches everything".into(),
            }],
        },
    ];
    for e in &events {
        round_trip(e);
    }
    let v = round_trip(&Event::RunState {
        run: id(1),
        state: RunPhase::PendingApproval,
        reason: Some(RunReason::InventoryInvalid),
    });
    assert_eq!(v["data"]["state"], "pending_approval");
    assert_eq!(v["data"]["reason"], "inventory_invalid");
}
