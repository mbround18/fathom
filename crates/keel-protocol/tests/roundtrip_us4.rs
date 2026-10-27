//! Round-trip tests for the inventory commands and events (spec 055 US4).

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

fn text(s: &str) -> InventoryText {
    InventoryText::new(s).unwrap()
}

fn info() -> InventoryInfo {
    InventoryInfo {
        id: id(1),
        workspace: id(9),
        name: "prod".into(),
        source: InventorySource::Managed,
        binding: None,
        path: None,
        status: InventoryStatus::Active,
        head_seq: Some(2),
        head_status: Some(RevisionStatus::Valid),
        host_count: 3,
        has_schema: true,
        created_at: "2026-10-27T10:00:00Z".into(),
    }
}

fn summary(seq: u32) -> InventoryRevisionSummary {
    InventoryRevisionSummary {
        seq,
        status: RevisionStatus::Invalid,
        content_hash: "a".repeat(64),
        source_commit: Some("b".repeat(40)),
        author: None,
        restored_from: Some(1),
        host_count: 3,
        created_at: "2026-10-27T10:00:00Z".into(),
    }
}

fn problem() -> InventoryProblem {
    InventoryProblem {
        at: "hosts.w1.vars.http_port".into(),
        message: "expected integer".into(),
    }
}

#[test]
fn inventory_commands() {
    let cmds = [
        Cmd::ListInventories { workspace: id(9) },
        Cmd::GetInventoryRevision {
            inventory: id(1),
            seq: Some(2),
        },
        Cmd::GetInventoryRevision {
            inventory: id(1),
            seq: None,
        },
        Cmd::DiffInventory {
            inventory: id(1),
            from: 1,
            to: 2,
        },
        Cmd::PreviewInventoryCreate {
            workspace: id(9),
            name: ObjectName::new("prod").unwrap(),
            content: text("web:\n  hosts:\n    w1:\n"),
            var_schema: Some(text("{\"type\":\"object\"}")),
        },
        Cmd::PreviewInventoryEdit {
            inventory: id(1),
            base_seq: 2,
            content: text("web:\n"),
        },
        Cmd::PreviewInventoryRestore {
            inventory: id(1),
            seq: 1,
        },
        Cmd::PreviewInventoryDeprecate { inventory: id(1) },
        Cmd::ValidateInventory {
            workspace: id(9),
            content: text("web:\n"),
            inventory: Some(id(1)),
        },
    ];
    let names: Vec<&str> = cmds.iter().map(Cmd::name).collect();
    assert_eq!(
        names,
        [
            "list_inventories",
            "get_inventory_revision",
            "get_inventory_revision",
            "diff_inventory",
            "preview_inventory_create",
            "preview_inventory_edit",
            "preview_inventory_restore",
            "preview_inventory_deprecate",
            "validate_inventory",
        ]
    );
    for cmd in &cmds {
        round_trip(cmd);
        assert!(!cmd.is_streaming());
    }

    // Optional fields may be omitted on the wire.
    let cmd: Cmd = serde_json::from_value(json!({
        "type": "PreviewInventoryCreate",
        "data": { "workspace": id(9), "name": "prod", "content": "" }
    }))
    .unwrap();
    assert!(matches!(
        cmd,
        Cmd::PreviewInventoryCreate {
            var_schema: None,
            ..
        }
    ));
    let cmd: Cmd = serde_json::from_value(json!({
        "type": "GetInventoryRevision", "data": { "inventory": id(1) }
    }))
    .unwrap();
    assert!(matches!(cmd, Cmd::GetInventoryRevision { seq: None, .. }));

    // Names follow the object-name rule; oversized text is refused.
    assert!(
        serde_json::from_value::<Cmd>(json!({
            "type": "PreviewInventoryCreate",
            "data": { "workspace": id(9), "name": "Prod", "content": "" }
        }))
        .is_err()
    );
    assert!(InventoryText::new("x".repeat(MAX_INVENTORY_TEXT_BYTES + 1)).is_err());
    assert_eq!(format!("{:?}", text("secret: x")), "InventoryText(9 bytes)");
}

#[test]
fn token_actions_hold_only_inventory_reads() {
    for read in [
        "list_inventories",
        "get_inventory_revision",
        "diff_inventory",
        "validate_inventory",
    ] {
        assert!(TOKEN_ACTIONS.contains(&read), "{read}");
    }
    for write in [
        "preview_inventory_create",
        "preview_inventory_edit",
        "preview_inventory_restore",
        "preview_inventory_deprecate",
    ] {
        assert!(!TOKEN_ACTIONS.contains(&write), "{write}");
    }
}

#[test]
fn inventory_events() {
    let v = round_trip(&Event::Inventories {
        items: vec![
            info(),
            InventoryInfo {
                source: InventorySource::Repo,
                binding: Some(id(3)),
                path: Some("inventories/prod/hosts.yaml".into()),
                status: InventoryStatus::Deprecated,
                head_seq: None,
                head_status: None,
                ..info()
            },
        ],
    });
    assert_eq!(v["data"]["items"][0]["source"], "managed");
    assert_eq!(v["data"]["items"][1]["status"], "deprecated");

    let v = round_trip(&Event::InventoryRevision {
        inventory: info(),
        revision: InventoryRevisionInfo {
            summary: summary(2),
            source: Some("web:\n".into()),
            content: "{}".into(),
            hosts: vec![InventoryHostInfo {
                host: "w1".into(),
                groups: vec!["web".into()],
            }],
            errors: vec![problem()],
            warnings: vec![],
        },
        history: vec![summary(2), summary(1)],
    });
    assert_eq!(v["data"]["revision"]["summary"]["status"], "invalid");

    let v = round_trip(&Event::InventoryDiff {
        inventory: id(1),
        from: 1,
        to: 2,
        changes: vec![InventoryChange {
            path: "hosts.w1.vars.http_port".into(),
            kind: InventoryChangeKind::Changed,
            before: Some("80".into()),
            after: Some("8080".into()),
        }],
    });
    assert_eq!(v["data"]["changes"][0]["kind"], "changed");

    round_trip(&Event::InventoryValidation {
        content_hash: "c".repeat(64),
        hosts: vec![],
        errors: vec![problem()],
        warnings: vec![problem()],
    });
}
