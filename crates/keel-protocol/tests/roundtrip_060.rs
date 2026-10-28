//! Round-trip tests for the settings, enrolment, admin token and System workspace commands and
//! events (spec 060 T006).

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

fn profiles(names: &[&str]) -> HostProfiles {
    HostProfiles(
        names
            .iter()
            .map(|n| ProfileName::new(*n).unwrap())
            .collect(),
    )
}

fn all_commands() -> Vec<Cmd> {
    vec![
        Cmd::GetSettings,
        Cmd::PreviewSettingsUpdate {
            template: "%t".into(),
            instructions: "# Enrol\n".into(),
            version: 3,
        },
        Cmd::ListEnrollments {
            state: Some(EnrollmentState::Pending),
            workspace: Some(id(2)),
        },
        Cmd::PreviewEnrollmentRevoke { enrollment: id(3) },
        Cmd::PreviewEnrollmentRoll { enrollment: id(3) },
        Cmd::ListAllApiTokens {
            holder: Some(id(1)),
            state: Some(ApiTokenState::Expired),
        },
        Cmd::GetSystem,
        Cmd::PreviewSystemConfigure {
            allowed_profiles: profiles(&["keel"]),
            version: 0,
        },
        Cmd::PreviewSystemRemove,
    ]
}

#[test]
fn commands_round_trip() {
    for cmd in &all_commands() {
        round_trip(cmd);
    }
    assert_eq!(
        round_trip(&Cmd::GetSettings),
        json!({"type": "GetSettings"})
    );
    assert_eq!(
        round_trip(&Cmd::ListEnrollments {
            state: Some(EnrollmentState::Revoked),
            workspace: None,
        }),
        json!({"type": "ListEnrollments", "data": {"state": "revoked", "workspace": null}})
    );
    // The filters are optional.
    let c: Cmd = serde_json::from_value(json!({"type": "ListAllApiTokens", "data": {}})).unwrap();
    assert_eq!(
        c,
        Cmd::ListAllApiTokens {
            holder: None,
            state: None
        }
    );
    let c: Cmd = serde_json::from_value(json!({"type": "ListEnrollments", "data": {}})).unwrap();
    assert_eq!(
        c,
        Cmd::ListEnrollments {
            state: None,
            workspace: None
        }
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
fn unknown_fields_and_bad_profiles_are_refused() {
    for v in [
        json!({"type": "PreviewEnrollmentRevoke", "data": {"enrollment": id(3), "x": 1}}),
        json!({"type": "PreviewSystemConfigure", "data": {"allowed_profiles": ["a", "a"], "version": 0}}),
        json!({"type": "PreviewSettingsUpdate", "data": {"template": "%t", "version": 1}}),
    ] {
        assert!(serde_json::from_value::<Cmd>(v.clone()).is_err(), "{v}");
    }
}

#[test]
fn events_round_trip() {
    let token = ApiTokenInfo {
        id: id(5),
        name: "ci".into(),
        owner: id(1),
        workspace: id(2),
        workspace_slug: "home".into(),
        actions: vec!["list_runs".into()],
        expires_at: "2026-11-28T12:00:00Z".into(),
        last_used_at: None,
        revoked: false,
    };
    let events = vec![
        Event::Settings {
            template: "%t".into(),
            instructions: String::new(),
            version: 0,
            is_default: true,
            runner_address: Some("keel.example.net:7831".into()),
            public_url: "https://keel.example.net".into(),
            role_mapping: Some(RoleMappingInfo {
                provider: "keycloak".into(),
                claim: "roles".into(),
                owner: vec!["keel-owner".into()],
                admin: vec![],
                user: vec!["keel-user".into()],
                required: true,
            }),
            warnings: vec!["idp_names_no_owner".into()],
        },
        Event::Enrollments {
            items: vec![EnrollmentInfo {
                id: id(3),
                workspace: id(2),
                workspace_slug: "home".into(),
                inventory_name: "web-01".into(),
                profiles: vec!["base".into()],
                state: EnrollmentState::Used,
                created_by: Some("Ada".into()),
                created_at: "2026-10-28T12:00:00Z".into(),
                expires_at: "2026-10-28T12:15:00Z".into(),
                host: Some(id(4)),
                rolled_from: None,
            }],
        },
        Event::AllApiTokens {
            items: vec![AdminApiTokenInfo {
                token,
                holder: id(1),
                holder_name: "Ada".into(),
                workspace_slug: "home".into(),
                state: ApiTokenState::Active,
            }],
        },
        Event::System {
            configured: true,
            workspace: Some(id(9)),
            allowed_profiles: vec!["keel".into()],
            version: 2,
            bindings: vec![id(10)],
            hosts: vec![],
        },
        Event::System {
            configured: false,
            workspace: None,
            allowed_profiles: vec![],
            version: 0,
            bindings: vec![],
            hosts: vec![],
        },
        Event::HostEnrollment {
            id: id(3),
            workspace: id(2),
            inventory_name: "web-01".into(),
            profiles: vec![],
            expires_at: "2026-10-28T12:15:00Z".into(),
            token: EnrollToken("keel-enroll-v1.s.f".into()),
            snippet: "make keel-node".into(),
            instructions: "**hi**".into(),
            instructions_unavailable: true,
        },
    ];
    for e in &events {
        round_trip(e);
    }
    let v = round_trip(&events[1]);
    assert_eq!(v["data"]["items"][0]["state"], "used");
    let v = round_trip(&events[2]);
    assert_eq!(v["data"]["items"][0]["state"], "active");
    let v = round_trip(&events[5]);
    assert_eq!(v["data"]["instructions_unavailable"], true);
    assert_eq!(v["data"]["snippet"], "make keel-node");
}

#[test]
fn role_source_and_system_flag_serialise() {
    assert_eq!(serde_json::to_value(RoleSource::Idp).unwrap(), "idp");
    assert_eq!(serde_json::to_value(RoleSource::Keel).unwrap(), "keel");
    for (s, want) in [
        (EnrollmentState::Pending, "pending"),
        (EnrollmentState::Burned, "burned"),
        (EnrollmentState::Expired, "expired"),
    ] {
        assert_eq!(serde_json::to_value(s).unwrap(), want);
    }
    assert_eq!(
        serde_json::to_value(ApiTokenState::Revoked).unwrap(),
        "revoked"
    );
}
