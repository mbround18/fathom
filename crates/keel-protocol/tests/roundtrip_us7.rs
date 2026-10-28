//! Round-trip tests for the host and SSH CA commands and events (spec 055 T078).

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

fn host_info() -> HostInfo {
    HostInfo {
        id: id(1),
        workspace: id(2),
        inventory_name: "web-1.example".into(),
        state: HostStatus::Online,
        profiles: vec!["base".into(), "web".into()],
        allowed_profiles: vec!["base".into(), "web".into(), "db".into()],
        require_signed: true,
        runner_version: Some("0.1.0".into()),
        ansible_version: Some("2.19.0".into()),
        enrolled_at: "2026-10-27T10:00:00Z".into(),
        last_seen_at: Some("2026-10-27T10:05:00Z".into()),
        revoked_at: None,
    }
}

#[test]
fn host_commands() {
    let all = vec![
        Cmd::ListHosts {
            workspace: id(2),
            profile: Some(ProfileName::new("web").unwrap()),
            drift: None,
        },
        Cmd::GetHost { host: id(1) },
        Cmd::PreviewHostEnroll {
            workspace: id(2),
            inventory_name: HostName::new("web-1.example").unwrap(),
            profiles: profiles(&["base", "web"]),
        },
        Cmd::PreviewHostRevoke { host: id(1) },
        Cmd::GetSshCa,
        Cmd::PreviewSshCaRotate {
            kind: SshCaKind::Runner,
        },
    ];
    for cmd in &all {
        round_trip(cmd);
    }
    let v = round_trip(&Cmd::PreviewSshCaRotate {
        kind: SshCaKind::Host,
    });
    assert_eq!(
        v,
        json!({"type": "PreviewSshCaRotate", "data": {"kind": "host"}})
    );
    assert_eq!(round_trip(&Cmd::GetSshCa), json!({"type": "GetSshCa"}));
    // `profile` and `drift` are optional.
    let c: Cmd = serde_json::from_value(
        json!({"type": "ListHosts", "data": {"workspace": id(2).to_string()}}),
    )
    .unwrap();
    assert_eq!(
        c,
        Cmd::ListHosts {
            workspace: id(2),
            profile: None,
            drift: None
        }
    );
}

#[test]
fn only_reads_are_token_actions() {
    for name in ["list_hosts", "get_host", "get_ssh_ca"] {
        assert!(TOKEN_ACTIONS.contains(&name), "{name}");
    }
    let writes = [
        Cmd::PreviewHostEnroll {
            workspace: id(2),
            inventory_name: HostName::new("web-1").unwrap(),
            profiles: profiles(&[]),
        },
        Cmd::PreviewHostRevoke { host: id(1) },
        Cmd::PreviewSshCaRotate {
            kind: SshCaKind::Host,
        },
    ];
    for c in &writes {
        assert!(!TOKEN_ACTIONS.contains(&c.name()), "{}", c.name());
        assert!(!c.is_streaming());
    }
}

#[test]
fn inputs_are_bounded_and_unknown_fields_refused() {
    assert!(HostName::new("").is_err());
    assert!(HostName::new("-web").is_err());
    assert!(HostName::new("web 1").is_err());
    assert!(HostName::new("x".repeat(254)).is_err());
    assert!(HostName::new("Web-1.example_x").is_ok());
    let enroll = |profiles: serde_json::Value| {
        serde_json::from_value::<Cmd>(json!({
            "type": "PreviewHostEnroll",
            "data": {"workspace": id(2).to_string(), "inventory_name": "web-1", "profiles": profiles}
        }))
    };
    assert!(enroll(json!(["base"])).is_ok());
    assert!(enroll(json!(["base", "base"])).is_err(), "duplicates");
    assert!(enroll(json!(["Bad!"])).is_err());
    let many: Vec<String> = (0..=MAX_HOST_PROFILES).map(|i| format!("p{i}")).collect();
    assert!(enroll(json!(many)).is_err());
    assert!(
        serde_json::from_value::<Cmd>(json!({
            "type": "GetHost", "data": {"host": id(1).to_string(), "extra": 1}
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<Cmd>(json!({
            "type": "PreviewSshCaRotate", "data": {"kind": "user"}
        }))
        .is_err()
    );
}

#[test]
fn host_events() {
    let events = vec![
        Event::Hosts {
            items: vec![host_info()],
        },
        Event::Host(HostDetail {
            host: HostInfo {
                state: HostStatus::Revoked,
                revoked_at: Some("2026-10-27T11:00:00Z".into()),
                ..host_info()
            },
            python_version: Some("3.12.3".into()),
            override_quorum: Some(1),
            ledger_head: Some("b".repeat(64)),
            certs: vec![HostCertInfo {
                serial: u64::MAX.to_string(),
                principals: vec!["host:web-1.example".into(), "profile:web".into()],
                valid_after: "2026-10-27T10:00:00Z".into(),
                valid_before: "2026-10-28T10:00:00Z".into(),
                revoked_at: Some("2026-10-27T11:00:00Z".into()),
            }],
        }),
        Event::HostEnrollment {
            id: id(3),
            workspace: id(2),
            inventory_name: "web-1.example".into(),
            profiles: vec!["base".into()],
            expires_at: "2026-10-27T10:15:00Z".into(),
            token: EnrollToken("keel-enroll-v1.abc.SHA256:xyz".into()),
        },
        Event::SshCa {
            cas: vec![
                SshCaInfo {
                    id: id(4),
                    kind: SshCaKind::Host,
                    state: SshCaState::Active,
                    fingerprint: "SHA256:aaa".into(),
                    public_key: "ssh-ed25519 AAAA keel-host-ca".into(),
                    not_after: None,
                    created_at: "2026-10-27T09:00:00Z".into(),
                },
                SshCaInfo {
                    id: id(5),
                    kind: SshCaKind::Runner,
                    state: SshCaState::Retiring,
                    fingerprint: "SHA256:bbb".into(),
                    public_key: "ssh-ed25519 BBBB keel-runner-ca".into(),
                    not_after: Some("2026-10-28T09:00:00Z".into()),
                    created_at: "2026-10-27T09:00:00Z".into(),
                },
            ],
            cert_authority: "@cert-authority * ssh-ed25519 AAAA keel-host-ca".into(),
        },
        Event::HostState {
            host: id(1),
            state: HostStatus::Offline,
        },
    ];
    for e in &events {
        round_trip(e);
    }
    let v = round_trip(&Event::HostState {
        host: id(1),
        state: HostStatus::Online,
    });
    assert_eq!(v["type"], "HostState");
    assert_eq!(v["data"]["state"], "online");
}

#[test]
fn the_enrollment_token_never_shows_in_debug_output() {
    let e = Event::HostEnrollment {
        id: id(3),
        workspace: id(2),
        inventory_name: "web-1".into(),
        profiles: vec![],
        expires_at: "2026-10-27T10:15:00Z".into(),
        token: EnrollToken("keel-enroll-v1.topsecret.SHA256:xyz".into()),
    };
    assert!(!format!("{e:?}").contains("topsecret"));
}
