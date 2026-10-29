//! Round-trip tests for the build identity, update notice and self-update commands and events
//! (spec 061 T014).

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

const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn all_commands() -> Vec<Cmd> {
    vec![
        Cmd::GetKeelBuild,
        Cmd::GetUpdateStatus,
        Cmd::CheckForUpdates,
        Cmd::DismissUpdateDot {
            digest: DIGEST.into(),
        },
        Cmd::PreviewKeelUpdate {
            digest: DIGEST.into(),
        },
    ]
}

fn running() -> KeelBuild {
    KeelBuild {
        version: Some("0.2.0".into()),
        revision: Some("abc1234def".into()),
        built_at: Some("2026-10-01T12:00:00Z".into()),
        digest: None,
        display: "0.2.0".into(),
    }
}

fn status(update_path: UpdatePath) -> UpdateStatus {
    UpdateStatus {
        enabled: true,
        image: "docker.io/mbround18/keel:latest".into(),
        state: UpdateCheckState::Checked,
        outcome: Some(UpdateOutcome::UpdateAvailable),
        cant_tell_reason: None,
        checked_at: Some("2026-10-28T12:00:00Z".into()),
        next_check_at: Some("2026-10-29T12:00:00Z".into()),
        manual_after: Some("2026-10-28T12:01:00Z".into()),
        last_error: None,
        running: running(),
        available: Some(AvailableImage {
            digest: DIGEST.into(),
            version: Some("0.3.0".into()),
            revision: Some("fedcba9".into()),
            built_at: Some("2026-10-27T12:00:00Z".into()),
            release_notes: Some("https://example.net/releases/0.3.0".into()),
            release_notes_note: None,
        }),
        digest_compare: false,
        update_path,
        update_run: Some(Uuid::from_u128(7)),
    }
}

#[test]
fn commands_round_trip() {
    for cmd in &all_commands() {
        round_trip(cmd);
    }
    assert_eq!(
        round_trip(&Cmd::GetKeelBuild),
        json!({"type": "GetKeelBuild"})
    );
    assert_eq!(
        round_trip(&Cmd::DismissUpdateDot {
            digest: DIGEST.into()
        }),
        json!({"type": "DismissUpdateDot", "data": {"digest": DIGEST}})
    );
    let names: Vec<_> = all_commands().iter().map(Cmd::name).collect();
    assert_eq!(
        names,
        [
            "get_keel_build",
            "get_update_status",
            "check_for_updates",
            "dismiss_update_dot",
            "preview_keel_update"
        ]
    );
}

#[test]
fn none_are_token_actions_or_streaming() {
    for c in &all_commands() {
        assert!(!TOKEN_ACTIONS.contains(&c.name()), "{}", c.name());
        assert!(!c.is_streaming());
    }
}

#[test]
fn unknown_and_missing_fields_are_refused() {
    for v in [
        json!({"type": "DismissUpdateDot", "data": {"digest": DIGEST, "x": 1}}),
        json!({"type": "PreviewKeelUpdate", "data": {}}),
        json!({"type": "GetKeelBuild", "data": {"x": 1}}),
    ] {
        assert!(serde_json::from_value::<Cmd>(v.clone()).is_err(), "{v}");
    }
}

#[test]
fn events_round_trip() {
    let manual = UpdatePath::Manual {
        missing: "no_keel_host".into(),
        digest_ref: format!("mbround18/keel@{DIGEST}"),
        steps: vec!["docker pull …".into(), "docker compose up -d".into()],
    };
    let events = vec![
        Event::KeelBuild(running()),
        Event::KeelBuild(KeelBuild {
            version: None,
            revision: None,
            built_at: None,
            digest: None,
            display: "development build".into(),
        }),
        Event::UpdateStatus(status(UpdatePath::Run)),
        Event::UpdateStatus(status(manual)),
        Event::UpdateStatus(UpdateStatus {
            state: UpdateCheckState::NotChecked,
            outcome: Some(UpdateOutcome::CantTell),
            cant_tell_reason: Some("no_revision_label".into()),
            last_error: Some(UpdateError {
                code: "registry_unreachable".into(),
                at: "2026-10-28T12:00:00Z".into(),
            }),
            available: None,
            update_run: None,
            ..status(UpdatePath::Run)
        }),
        Event::UpdateStatusLine(UpdateStatusLine {
            update_available: true,
            checked_at: Some("2026-10-28T12:00:00Z".into()),
        }),
    ];
    for e in &events {
        round_trip(e);
    }

    let v = round_trip(&events[0]);
    assert_eq!(v["type"], "KeelBuild");
    assert_eq!(v["data"]["display"], "0.2.0");
    assert_eq!(v["data"]["digest"], json!(null));

    let v = round_trip(&events[2]);
    assert_eq!(v["data"]["state"], "checked");
    assert_eq!(v["data"]["outcome"], "update_available");
    assert_eq!(v["data"]["update_path"], json!({"kind": "run"}));
    assert_eq!(v["data"]["update_run"], Uuid::from_u128(7).to_string());

    let v = round_trip(&events[3]);
    assert_eq!(v["data"]["update_path"]["kind"], "manual");
    assert_eq!(v["data"]["update_path"]["missing"], "no_keel_host");

    let v = round_trip(&events[4]);
    assert_eq!(v["data"]["state"], "not_checked");
    assert_eq!(v["data"]["outcome"], "cant_tell");
    assert_eq!(v["data"]["last_error"]["code"], "registry_unreachable");

    // The admin line carries nothing about the available image.
    let v = round_trip(&events[5]);
    let keys: Vec<_> = v["data"].as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["checked_at", "update_available"]);
}

#[test]
fn every_outcome_is_snake_case() {
    for (o, s) in [
        (UpdateOutcome::UpToDate, "up_to_date"),
        (UpdateOutcome::UpdateAvailable, "update_available"),
        (UpdateOutcome::OlderBuild, "older_build"),
        (UpdateOutcome::Rebuilt, "rebuilt"),
        (UpdateOutcome::CantTell, "cant_tell"),
    ] {
        assert_eq!(round_trip(&o), json!(s));
    }
}

#[test]
fn me_carries_the_version_and_dismissed_dot() {
    let me = Me {
        id: Uuid::from_u128(1),
        display_name: "Ada".into(),
        email: None,
        instance_role: InstanceRole::Owner,
        grants: vec![],
        verification: Verification {
            verified: false,
            at: None,
            window_secs: 300,
            enrolled: true,
            required: false,
            recovering: false,
        },
        keel_version: Some("0.2.0".into()),
        update_dot_dismissed: Some(DIGEST.into()),
    };
    let v = round_trip(&Event::Me(me));
    assert_eq!(v["data"]["keel_version"], "0.2.0");
    assert_eq!(v["data"]["update_dot_dismissed"], DIGEST);
}

#[test]
fn run_info_carries_params() {
    let id = Uuid::from_u128;
    let mut run = RunInfo {
        id: id(1),
        workspace: id(2),
        binding: Some(id(3)),
        job: Some(id(4)),
        job_name: "keel-server".into(),
        trigger: RunTrigger::Manual,
        requested_by: Some(id(5)),
        commit: "a".repeat(40),
        inventory: "fleet".into(),
        inventory_revision: None,
        newer_inventory_revision: false,
        limit: None,
        hosts: vec!["keel-1".into()],
        policy: None,
        state: RunPhase::Queued,
        reason: None,
        created_at: "2026-10-28T12:00:00Z".into(),
        started_at: None,
        ended_at: None,
        summary: None,
        params: BTreeMap::new(),
    };
    // Empty when there are none, and still on the wire.
    assert_eq!(round_trip(&run)["params"], json!({}));
    run.params
        .insert("keel_server_image_digest".into(), DIGEST.into());
    assert_eq!(
        round_trip(&run)["params"],
        json!({"keel_server_image_digest": DIGEST})
    );
}
