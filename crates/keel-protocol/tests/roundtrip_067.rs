//! Round-trip tests for spec 067: zones, relays, runner paths, relay-link keys and host queries
//! (T028).

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

fn zone_name(n: &str) -> ZoneName {
    ZoneName::new(n).unwrap()
}

fn relay_info(state: RelayState) -> RelayInfo {
    RelayInfo {
        id: id(10),
        workspace: id(1),
        zone: id(2),
        name: zone_name("edge-1"),
        dial_address: "relay.example:2222".into(),
        host_key_fingerprint: "SHA256:abc".into(),
        host: Some(id(3)),
        state,
        since: Some("2026-11-17T10:00:00Z".into()),
        rtt_ms: Some(12),
        version: Some("0.1.0".into()),
        streams: Some(2),
        last_error: None,
        revoked_at: None,
    }
}

#[test]
fn commands_round_trip_with_their_wire_names() {
    let desc = ZoneDescription::new("Behind the office NAT").unwrap();
    let all = vec![
        (Cmd::ListZones { workspace: id(1) }, "list_zones"),
        (
            Cmd::PreviewZoneCreate {
                workspace: id(1),
                name: zone_name("office"),
                description: desc.clone(),
            },
            "preview_zone_create",
        ),
        (
            Cmd::PreviewZoneUpdate {
                zone: id(2),
                name: zone_name("office-2"),
                description: desc,
            },
            "preview_zone_update",
        ),
        (
            Cmd::PreviewZoneRemove { zone: id(2) },
            "preview_zone_remove",
        ),
        (
            Cmd::PreviewHostZone {
                host: id(3),
                zone: Some(id(2)),
            },
            "preview_host_zone",
        ),
        (
            Cmd::PreviewHostZone {
                host: id(3),
                zone: None,
            },
            "preview_host_zone",
        ),
        (Cmd::ListRelays { workspace: id(1) }, "list_relays"),
        (
            Cmd::PreviewRelayRegister {
                workspace: id(1),
                zone: id(2),
                name: zone_name("edge-1"),
                dial_address: "relay.example:2222".into(),
                host_key: "ssh-ed25519 AAAA relay".into(),
                host: None,
            },
            "preview_relay_register",
        ),
        (
            Cmd::PreviewRelayUpdate {
                relay: id(10),
                zone: id(2),
                name: zone_name("edge-1"),
                dial_address: "relay.example:2223".into(),
                host_key: "ssh-ed25519 AAAA relay".into(),
                host: Some(id(3)),
            },
            "preview_relay_update",
        ),
        (
            Cmd::PreviewRelayRevoke { relay: id(10) },
            "preview_relay_revoke",
        ),
        (
            Cmd::PreviewRelayRelease { relay: id(10) },
            "preview_relay_release",
        ),
        (Cmd::GetRelayLinkKeys, "get_relay_link_keys"),
        (
            Cmd::PreviewRelayLinkKeyRotate,
            "preview_relay_link_key_rotate",
        ),
        (
            Cmd::PreviewRelayLinkKeyActivate {
                fingerprint: "SHA256:x".into(),
            },
            "preview_relay_link_key_activate",
        ),
        (
            Cmd::PreviewRelayLinkKeyRetire {
                fingerprint: "SHA256:x".into(),
            },
            "preview_relay_link_key_retire",
        ),
    ];
    for (cmd, name) in &all {
        let v = round_trip(cmd);
        assert_eq!(cmd.name(), *name);
        assert!(!cmd.is_streaming(), "{name}");
        assert!(v["type"].is_string(), "{v}");
    }
    assert!(TOKEN_ACTIONS.contains(&"list_zones"));
    assert!(TOKEN_ACTIONS.contains(&"list_relays"));
    assert!(!TOKEN_ACTIONS.contains(&"preview_relay_register"));
    assert!(!TOKEN_ACTIONS.contains(&"get_relay_link_keys"));

    let v = round_trip(&Cmd::PreviewHostZone {
        host: id(3),
        zone: None,
    });
    assert_eq!(
        v,
        json!({"type": "PreviewHostZone", "data": {"host": id(3), "zone": null}})
    );
}

#[test]
fn names_are_checked() {
    for ok in ["a", "edge-1", "0zone", &"a".repeat(63)] {
        assert!(ZoneName::new(ok).is_ok(), "{ok}");
    }
    for bad in [
        "",
        "direct",
        "-edge",
        "Edge",
        "edge_1",
        "edge.1",
        &"a".repeat(64),
    ] {
        assert!(ZoneName::new(bad).is_err(), "{bad}");
    }
    assert!(ZoneDescription::new("é".repeat(500)).is_ok());
    assert!(ZoneDescription::new("é".repeat(501)).is_err());
    let bad = json!({"type": "PreviewZoneCreate",
        "data": {"workspace": id(1), "name": "direct", "description": ""}});
    assert!(serde_json::from_value::<Cmd>(bad).is_err());
}

#[test]
fn events_round_trip() {
    let v = round_trip(&Event::Zones {
        items: vec![ZoneInfo {
            id: id(2),
            workspace: id(1),
            name: zone_name("office"),
            description: String::new(),
            hosts: 3,
            relays: 1,
        }],
    });
    assert_eq!(v["type"], "Zones");
    for state in [
        RelayState::Connecting,
        RelayState::Connected,
        RelayState::Unreachable,
        RelayState::Quarantined,
        RelayState::Revoked,
    ] {
        round_trip(&Event::Relays {
            items: vec![relay_info(state)],
        });
    }
    let v = round_trip(&Event::RelayChanged(relay_info(RelayState::Unreachable)));
    assert_eq!(v["data"]["state"], "unreachable");
    let v = round_trip(&Event::RelayLinkKeys {
        items: vec![RelayLinkKey {
            fingerprint: "SHA256:x".into(),
            public_line: "ssh-ed25519 AAAA keel-relay-link".into(),
            state: RelayLinkKeyState::Pending,
        }],
    });
    assert_eq!(v["data"]["items"][0]["state"], "pending");

    for reach in [
        HostReach::Connected {
            path: RunnerPath::Direct,
        },
        HostReach::Connected {
            path: RunnerPath::Relay { relay: id(10) },
        },
        HostReach::Disconnected,
        HostReach::RelayUnreachable {
            relays: vec![id(10), id(11)],
        },
        HostReach::RelayRevoked { relay: id(10) },
        HostReach::NoPath,
    ] {
        round_trip(&Event::HostReachChanged { host: id(3), reach });
    }
    let v = round_trip(&Event::HostReachChanged {
        host: id(3),
        reach: HostReach::Connected {
            path: RunnerPath::Relay { relay: id(10) },
        },
    });
    assert_eq!(
        v["data"]["reach"],
        json!({"kind": "connected", "path": {"kind": "relay", "relay": id(10)}})
    );
    let v = round_trip(&Event::HostReachChanged {
        host: id(3),
        reach: HostReach::NoPath,
    });
    assert_eq!(v["data"]["reach"], json!({"kind": "no_path"}));

    for (outcome, kind) in [
        (None, QueryKind::Facts),
        (Some(QueryOutcome::Answered), QueryKind::AppliedRevision),
        (Some(QueryOutcome::Refused), QueryKind::UnitState),
        (Some(QueryOutcome::Timeout), QueryKind::DiskHealth),
        (Some(QueryOutcome::NoCapability), QueryKind::PathFree),
        (Some(QueryOutcome::Disconnected), QueryKind::Facts),
    ] {
        round_trip(&Event::HostQueryAnswered(HostQuery {
            id: id(20),
            host: id(3),
            kind,
            params: json!({"path": "/var"}),
            asked_by: "owner".into(),
            asked_at: "2026-11-17T10:00:00Z".into(),
            outcome,
            answer: outcome.map(|_| json!({"free": 10})),
            refusal: None,
            answered_at: None,
        }));
    }
}

#[test]
fn host_info_without_zone_or_reach_still_parses() {
    let v = json!({
        "id": id(3), "workspace": id(1), "inventory_name": "web-1", "state": "online",
        "profiles": [], "allowed_profiles": [], "require_signed": false,
        "runner_version": null, "ansible_version": null,
        "enrolled_at": "2026-11-17T10:00:00Z", "last_seen_at": null, "revoked_at": null,
        "changed_outside": false, "age_recipient": null, "inventory_status": "unknown"
    });
    let info: HostInfo = serde_json::from_value(v).unwrap();
    assert_eq!(info.zone, None);
    assert_eq!(info.reach, HostReach::Disconnected);
}
