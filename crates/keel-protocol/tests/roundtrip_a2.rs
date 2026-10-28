//! Round-trip tests for runs outside Keel (spec 058 amendment A2, T044): `HostInfo`'s
//! `changed_outside`, `HostDetail`'s newest entries, the `ListHosts` filter and the
//! `HostChangedOutside` event.

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

fn host_info() -> HostInfo {
    HostInfo {
        id: id(1),
        workspace: id(2),
        inventory_name: "web-1".into(),
        state: HostStatus::Online,
        profiles: vec!["base".into()],
        allowed_profiles: vec!["base".into()],
        require_signed: false,
        runner_version: Some("0.1.0".into()),
        ansible_version: Some("2.21.4".into()),
        enrolled_at: "2026-10-27T10:00:00Z".into(),
        last_seen_at: None,
        revoked_at: None,
        changed_outside: true,
    }
}

#[test]
fn a_host_says_whether_it_changed_outside_keel() {
    let v = round_trip(&host_info());
    assert_eq!(v["changed_outside"], true);
}

#[test]
fn host_detail_carries_the_hosts_claims() {
    let detail = HostDetail {
        host: host_info(),
        python_version: None,
        override_quorum: None,
        ledger_head: None,
        certs: vec![],
        outside_runs: vec![
            OutsideRunInfo {
                at: "2026-10-27T12:00:00Z".into(),
                user: "root".into(),
                playbook: "playbooks/debian.yaml".into(),
                commit: Some("a".repeat(40)),
                dirty: true,
                check: false,
                received_at: "2026-10-27T12:01:00Z".into(),
            },
            OutsideRunInfo {
                at: "2026-10-27T11:00:00Z".into(),
                user: "deploy".into(),
                playbook: "site.yaml".into(),
                commit: None,
                dirty: false,
                check: true,
                received_at: "2026-10-27T12:01:00Z".into(),
            },
        ],
    };
    let v = round_trip(&Event::Host(detail));
    assert_eq!(
        v["data"]["outside_runs"][0]["playbook"],
        "playbooks/debian.yaml"
    );
    assert_eq!(
        v["data"]["outside_runs"][1]["commit"],
        serde_json::Value::Null
    );
    assert_eq!(v["data"]["host"]["changed_outside"], true);
}

#[test]
fn list_hosts_filters_on_changed_outside() {
    let v = round_trip(&Cmd::ListHosts {
        workspace: id(2),
        profile: None,
        drift: None,
        changed_outside: Some(true),
    });
    assert_eq!(v["data"]["changed_outside"], true);
    // Optional: an older client leaves it out.
    let c: Cmd = serde_json::from_value(
        json!({"type": "ListHosts", "data": {"workspace": id(2).to_string()}}),
    )
    .unwrap();
    assert_eq!(
        c,
        Cmd::ListHosts {
            workspace: id(2),
            profile: None,
            drift: None,
            changed_outside: None,
        }
    );
}

#[test]
fn host_changed_outside_is_pushed() {
    let v = round_trip(&Event::HostChangedOutside { host: id(1) });
    assert_eq!(
        v,
        json!({"type": "HostChangedOutside", "data": {"host": id(1).to_string()}})
    );
}
