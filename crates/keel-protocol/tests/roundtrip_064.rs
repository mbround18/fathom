//! Round-trip tests for run approval: the approve, deny and count commands, the
//! `PendingApprovals` event and `RunInfo.approval` (spec 064 T006).

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

#[test]
fn approval_commands_round_trip() {
    let all = vec![
        Cmd::PreviewRunApprove { run: id(1) },
        Cmd::PreviewRunDeny {
            run: id(1),
            reason: Some(ApprovalReason::new("Not during the freeze.\nRetry Monday.").unwrap()),
        },
        Cmd::PreviewRunDeny {
            run: id(1),
            reason: None,
        },
        Cmd::CountPendingApprovals,
    ];
    for cmd in &all {
        round_trip(cmd);
        assert!(!cmd.is_streaming(), "{}", cmd.name());
    }
    assert_eq!(all[0].name(), "preview_run_approve");
    assert_eq!(all[1].name(), "preview_run_deny");
    assert_eq!(all[3].name(), "count_pending_approvals");
    // The reason may be left out.
    let cmd: Cmd = serde_json::from_value(json!({
        "type": "PreviewRunDeny", "data": {"run": id(1).to_string()}
    }))
    .unwrap();
    assert_eq!(
        cmd,
        Cmd::PreviewRunDeny {
            run: id(1),
            reason: None
        }
    );
}

#[test]
fn deny_reason_is_checked() {
    let deny = |reason: String| {
        serde_json::from_value::<Cmd>(json!({
            "type": "PreviewRunDeny", "data": {"run": id(1).to_string(), "reason": reason}
        }))
    };
    assert!(deny("x".repeat(MAX_APPROVAL_REASON_CHARS)).is_ok());
    assert!(deny("é".repeat(MAX_APPROVAL_REASON_CHARS)).is_ok());
    assert!(deny("x".repeat(MAX_APPROVAL_REASON_CHARS + 1)).is_err());
    assert!(deny(String::new()).is_err());
    assert!(deny("   \n\t".into()).is_err());
    assert!(deny("bad\u{7}bell".into()).is_err());
    assert!(deny("tab\tand\nnewline".into()).is_ok());
}

#[test]
fn approval_tokens_are_not_granted_to_api_tokens() {
    for name in [
        "preview_run_approve",
        "preview_run_deny",
        "count_pending_approvals",
    ] {
        assert!(!TOKEN_ACTIONS.contains(&name), "{name}");
    }
}

#[test]
fn pending_approvals_event_round_trips() {
    let ev = Event::PendingApprovals {
        counts: vec![
            WorkspaceCount {
                workspace: id(2),
                count: 3,
            },
            WorkspaceCount {
                workspace: id(9),
                count: 0,
            },
        ],
    };
    let v = round_trip(&ev);
    assert_eq!(v["type"], "PendingApprovals");
    assert_eq!(v["data"]["counts"][0]["count"], 3);
}

fn run() -> RunInfo {
    RunInfo {
        id: id(1),
        workspace: id(2),
        binding: Some(id(3)),
        job: Some(id(4)),
        job_name: "web".into(),
        trigger: RunTrigger::Manual,
        requested_by: Some(id(5)),
        commit: "a".repeat(40),
        inventory: "prod".into(),
        inventory_revision: None,
        newer_inventory_revision: false,
        limit: None,
        hosts: vec!["web-1".into()],
        policy: Some(id(6)),
        state: RunPhase::PendingApproval,
        reason: None,
        created_at: "2026-10-31T09:00:00Z".into(),
        started_at: None,
        ended_at: None,
        summary: None,
        params: BTreeMap::new(),
        approval: None,
    }
}

#[test]
fn run_info_carries_its_approval() {
    let mut r = run();
    // Absent before the run was held, and an older server's reply still parses.
    assert_eq!(round_trip(&r)["approval"], json!(null));
    let mut v = serde_json::to_value(&r).unwrap();
    v.as_object_mut().unwrap().remove("approval");
    assert_eq!(serde_json::from_value::<RunInfo>(v).unwrap(), r);

    r.approval = Some(RunApproval {
        needed: 2,
        four_eyes: true,
        expires_at: "2026-11-01T09:00:00Z".into(),
        approvals: vec![ApprovalEntry {
            user: id(7),
            name: "Ada".into(),
            role: Some("approver".into()),
            at: "2026-10-31T09:05:00Z".into(),
            reason: None,
        }],
        denial: None,
    });
    let v = round_trip(&r);
    assert_eq!(v["approval"]["needed"], 2);
    assert_eq!(v["approval"]["approvals"][0]["name"], "Ada");

    r.state = RunPhase::Denied;
    r.reason = Some(RunReason::ApprovalDenied);
    r.approval.as_mut().unwrap().denial = Some(ApprovalEntry {
        user: id(8),
        name: "Deleted user".into(),
        role: None,
        at: "2026-10-31T09:06:00Z".into(),
        reason: Some("Not during the freeze.".into()),
    });
    let v = round_trip(&r);
    assert_eq!(v["approval"]["denial"]["reason"], "Not during the freeze.");
}
