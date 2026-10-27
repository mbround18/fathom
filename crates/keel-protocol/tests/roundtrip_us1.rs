//! Round-trip tests for the binding, receiver, delivery and credential commands and the
//! `DeliveryReceived` and `BindingSynced` events (spec 055 T042).

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

fn spec() -> BindingSpec {
    BindingSpec {
        project: id(1),
        url: RepoUrl::new("git@git.example.com:ops/infra.git").unwrap(),
        branch: BranchName::new("main").unwrap(),
        transport: Transport::Ssh,
        credential: Some(id(2)),
        known_host: Some(KnownHost::new("git.example.com ssh-ed25519 AAAAC3Nz").unwrap()),
        poll_interval: None,
        allow_force_push: false,
        signed_mode: true,
        override_quorum: 2,
    }
}

fn binding_info() -> BindingInfo {
    BindingInfo {
        id: id(3),
        project: id(1),
        workspace: id(9),
        url: "https://git.example.com/ops/infra.git".into(),
        branch: "main".into(),
        transport: Transport::Https,
        credential: None,
        known_host: None,
        poll_interval: Some(300),
        allow_force_push: false,
        signed_mode: false,
        override_quorum: 1,
        status: ActiveStatus::Active,
        last_commit: Some("a".repeat(40)),
        last_synced_at: Some("2026-10-27T10:00:00Z".into()),
        last_sync_error: Some("manifest_invalid".into()),
        manifest_problems: vec![ManifestProblem {
            at: "jobs.web.playbook".into(),
            message: "'x.yaml' does not exist at this commit".into(),
        }],
    }
}

fn delivery() -> DeliveryInfo {
    DeliveryInfo {
        id: id(4),
        receiver: id(5),
        binding: Some(id(3)),
        provider_delivery_id: "72d3162e-cc78-11e3-81ab-4c9367dc0958".into(),
        received_at: "2026-10-27T10:00:00Z".into(),
        decision: DeliveryDecision::Accepted,
        refusal: None,
        sync_state: Some(SyncState::HistoryRewritten),
        commit: Some("b".repeat(40)),
    }
}

#[test]
fn binding_commands() {
    let v = round_trip(&Cmd::PreviewBindingCreate(spec()));
    assert_eq!(v["type"], "PreviewBindingCreate");
    assert_eq!(v["data"]["transport"], "ssh");
    assert_eq!(v["data"]["url"], "git@git.example.com:ops/infra.git");
    assert_eq!(v["data"]["poll_interval"], serde_json::Value::Null);

    // Optional fields may be omitted on the wire.
    let cmd: Cmd = serde_json::from_value(json!({
        "type": "PreviewBindingCreate",
        "data": {
            "project": id(1), "url": "https://git.example.com/a/b", "branch": "main",
            "transport": "https", "allow_force_push": false, "signed_mode": false,
            "override_quorum": 1
        }
    }))
    .unwrap();
    let Cmd::PreviewBindingCreate(s) = cmd else {
        panic!()
    };
    assert_eq!(s.credential, None);

    round_trip(&Cmd::PreviewBindingUpdate(BindingUpdate {
        binding: id(3),
        branch: BranchName::new("release").unwrap(),
        credential: None,
        known_host: None,
        poll_interval: Some(60),
        allow_force_push: true,
        override_quorum: 1,
    }));
    for cmd in [
        Cmd::ListBindings { workspace: id(9) },
        Cmd::GetBinding { binding: id(3) },
        Cmd::PreviewSignedMode {
            binding: id(3),
            on: false,
        },
        Cmd::PreviewBindingDisable { binding: id(3) },
        Cmd::PreviewBindingEnable { binding: id(3) },
        Cmd::PreviewBindingRemove { binding: id(3) },
        Cmd::SyncBinding { binding: id(3) },
    ] {
        round_trip(&cmd);
        assert!(!cmd.is_streaming());
    }
    assert_eq!(Cmd::SyncBinding { binding: id(3) }.name(), "sync_binding");
    assert_eq!(
        Cmd::PreviewBindingCreate(spec()).name(),
        "preview_binding_create"
    );
}

#[test]
fn receiver_and_delivery_commands() {
    let v = round_trip(&Cmd::PreviewReceiverCreate {
        workspace: id(9),
        name: ObjectName::new("forgejo-main").unwrap(),
        provider: ReceiverProvider::Forgejo,
    });
    assert_eq!(v["data"]["provider"], "forgejo");
    let v = round_trip(&Cmd::PreviewReceiverRotate {
        receiver: id(5),
        previous_valid_for: None,
    });
    assert_eq!(v["data"]["previous_valid_for"], serde_json::Value::Null);
    let v = round_trip(&Cmd::ListDeliveries {
        workspace: id(9),
        receiver: Some(id(5)),
        binding: None,
        decision: Some(DeliveryDecision::Refused),
        from: Some(Instant::new("2026-10-01T00:00:00Z").unwrap()),
        to: None,
    });
    assert_eq!(v["data"]["decision"], "refused");
    for cmd in [
        Cmd::ListReceivers { workspace: id(9) },
        Cmd::PreviewReceiverDisable { receiver: id(5) },
        Cmd::PreviewReceiverRemove { receiver: id(5) },
        Cmd::PreviewReceiverBind {
            receiver: id(5),
            binding: id(3),
        },
        Cmd::PreviewReceiverUnbind {
            receiver: id(5),
            binding: id(3),
        },
    ] {
        round_trip(&cmd);
        assert!(!cmd.is_streaming());
    }
    assert_eq!(
        Cmd::PreviewReceiverBind {
            receiver: id(5),
            binding: id(3)
        }
        .name(),
        "preview_receiver_bind"
    );
}

#[test]
fn credential_commands_never_print_values() {
    let cmd = Cmd::PreviewCredentialCreate {
        workspace: id(9),
        name: ObjectName::new("prod-vault").unwrap(),
        kind: CredentialKind::VaultPassword,
        value: SecretValue::new("hunter2").unwrap(),
    };
    let v = round_trip(&cmd);
    assert_eq!(v["data"]["kind"], "vault_password");
    assert!(!format!("{cmd:?}").contains("hunter2"));
    round_trip(&Cmd::PreviewCredentialRotate {
        credential: id(6),
        value: SecretValue::new("x").unwrap(),
    });
    round_trip(&Cmd::PreviewCredentialGrant {
        credential: id(6),
        binding: id(3),
        profiles: vec![ObjectName::new("debian").unwrap()],
    });
    round_trip(&Cmd::PreviewCredentialRevoke {
        credential: id(6),
        binding: id(3),
    });
    round_trip(&Cmd::PreviewCredentialRemove { credential: id(6) });
    round_trip(&Cmd::ListCredentials { workspace: id(9) });

    let v = round_trip(&Event::Credentials {
        items: vec![CredentialInfo {
            id: id(6),
            workspace: id(9),
            name: "prod-vault".into(),
            kind: CredentialKind::GitSshKey,
            fingerprint: "SHA256:0123456789abcdef".into(),
            created_at: "2026-10-27T10:00:00Z".into(),
            rotated_at: None,
            grants: vec![CredentialGrantInfo {
                binding: id(3),
                profiles: vec![],
            }],
        }],
    });
    assert!(v["data"]["items"][0].get("value").is_none());
}

#[test]
fn inputs_are_bounded_and_unknown_fields_refused() {
    assert!(RepoUrl::new("x".repeat(3000)).is_err());
    assert!(BranchName::new("-x").is_err());
    assert!(ObjectName::new("UPPER").is_err());
    assert!(SecretValue::new("x".repeat(70 * 1024)).is_err());
    let bad = json!({ "type": "SyncBinding", "data": { "binding": id(3), "force": true } });
    assert!(serde_json::from_value::<Cmd>(bad).is_err());
    let bad = json!({ "type": "PreviewReceiverCreate",
        "data": { "workspace": id(9), "name": "ok", "provider": "bitbucket" } });
    assert!(serde_json::from_value::<Cmd>(bad).is_err());
}

#[test]
fn binding_events() {
    let v = round_trip(&Event::BindingSynced {
        binding: id(3),
        commit: "c".repeat(40),
        jobs_changed: true,
        inventories_changed: false,
    });
    assert_eq!(v["type"], "BindingSynced");
    assert_eq!(v["data"]["jobs_changed"], true);

    round_trip(&Event::Bindings {
        items: vec![binding_info()],
    });
    let v = round_trip(&Event::Binding {
        binding: binding_info(),
        jobs: vec![JobSummary {
            name: "web-deploy".into(),
            playbook: "playbooks/web.yaml".into(),
            profiles: vec!["debian".into()],
            inventory: "prod".into(),
            blocked: Some("credential_unavailable".into()),
            defined_at_commit: "a".repeat(40),
        }],
    });
    assert_eq!(
        v["data"]["binding"]["manifest_problems"][0]["at"],
        "jobs.web.playbook"
    );
}

#[test]
fn receiver_and_delivery_events() {
    let v = round_trip(&EventEnvelope::push(Event::DeliveryReceived {
        delivery: delivery(),
    }));
    assert!(v.get("reply_to").is_none());
    assert_eq!(v["event"]["type"], "DeliveryReceived");
    assert_eq!(
        v["event"]["data"]["delivery"]["sync_state"],
        "history_rewritten"
    );

    let mut refused = delivery();
    refused.decision = DeliveryDecision::Refused;
    refused.refusal = Some(DeliveryRefusal::NoMatchingBinding);
    refused.sync_state = None;
    let v = round_trip(&Event::Deliveries {
        items: vec![refused],
    });
    assert_eq!(v["data"]["items"][0]["refusal"], "no_matching_binding");

    let v = round_trip(&Event::ReceiverSecret {
        url: "https://keel.example.com/hooks/0190".into(),
        secret: "s3cr3t".into(),
    });
    assert_eq!(v["type"], "ReceiverSecret");
    round_trip(&Event::Receivers {
        items: vec![ReceiverInfo {
            id: id(5),
            workspace: id(9),
            name: "forgejo-main".into(),
            provider: ReceiverProvider::Generic,
            status: ActiveStatus::Disabled,
            url: "https://keel.example.com/hooks/5".into(),
            previous_expires_at: Some("2026-10-28T10:00:00Z".into()),
            bindings: vec![id(3)],
            created_at: "2026-10-27T10:00:00Z".into(),
            rotated_at: None,
        }],
    });
}
