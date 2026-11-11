//! Round-trip tests for spec 066: workspace primary repository, forge connection, catalog, job
//! drafts, host add and proposed changes (T002).

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

fn repo_path(p: &str) -> RepoPath {
    serde_json::from_value(json!(p)).unwrap()
}

#[test]
fn commands_round_trip_with_their_wire_names() {
    let all = vec![
        (
            Cmd::PreviewWorkspacePrimary {
                workspace: id(1),
                binding: Some(id(2)),
            },
            "preview_workspace_primary",
        ),
        (
            Cmd::PreviewWorkspacePrimary {
                workspace: id(1),
                binding: None,
            },
            "preview_workspace_primary",
        ),
        (
            Cmd::PreviewWorkspaceRepository {
                workspace: id(1),
                spec: RepositorySpec {
                    url: RepoUrl::new("https://git.example/ops/ansible.git").unwrap(),
                    branch: BranchName::new("main").unwrap(),
                    transport: Transport::Https,
                    credential: Some(id(3)),
                    known_host: None,
                    poll_interval: Some(300),
                    allow_force_push: false,
                    signed_mode: false,
                    override_quorum: 1,
                },
            },
            "preview_workspace_repository",
        ),
        (
            Cmd::GetForgeConnection { workspace: id(1) },
            "get_forge_connection",
        ),
        (
            Cmd::PreviewForgeConnection {
                workspace: id(1),
                provider: ForgeProvider::Forgejo,
                api_base: Some(ApiBase::new("https://git.example/api/v1").unwrap()),
                credential: id(4),
            },
            "preview_forge_connection",
        ),
        (
            Cmd::PreviewForgeRemove { workspace: id(1) },
            "preview_forge_remove",
        ),
        (Cmd::ProbeForge { workspace: id(1) }, "probe_forge"),
        (Cmd::GetCatalog { workspace: id(1) }, "get_catalog"),
        (
            Cmd::DraftJob {
                workspace: id(1),
                playbook: repo_path("playbooks/provision.yaml"),
            },
            "draft_job",
        ),
        (
            Cmd::PreviewHostAdd {
                workspace: id(1),
                inventory_name: HostName::new("newbox").unwrap(),
                groups: HostGroups(vec![GroupName::new("compose_hosts").unwrap()]),
                profiles: HostProfiles(vec![ProfileName::new("prox-vm").unwrap()]),
                inventory: Some(repo_path("inventory/hosts.yml")),
            },
            "preview_host_add",
        ),
        (
            Cmd::PreviewSopsRecipient { host: id(5) },
            "preview_sops_recipient",
        ),
        (
            Cmd::PreviewJobPropose {
                workspace: id(1),
                playbook: repo_path("playbooks/provision.yaml"),
                name: ObjectName::new("provision").unwrap(),
            },
            "preview_job_propose",
        ),
    ];
    for (cmd, name) in &all {
        round_trip(cmd);
        assert_eq!(cmd.name(), *name);
        assert!(!cmd.is_streaming(), "{name}");
    }
    assert!(TOKEN_ACTIONS.contains(&"get_catalog"));
    assert!(TOKEN_ACTIONS.contains(&"draft_job"));
    assert!(!TOKEN_ACTIONS.contains(&"preview_host_add"));
}

#[test]
fn optional_fields_may_be_left_out() {
    let cmd: Cmd = serde_json::from_value(json!({
        "type": "PreviewWorkspacePrimary", "data": {"workspace": id(1).to_string()}
    }))
    .unwrap();
    assert_eq!(
        cmd,
        Cmd::PreviewWorkspacePrimary {
            workspace: id(1),
            binding: None
        }
    );
    let cmd: Cmd = serde_json::from_value(json!({
        "type": "PreviewHostAdd", "data": {
            "workspace": id(1).to_string(), "inventory_name": "newbox",
            "groups": ["compose_hosts"], "profiles": []
        }
    }))
    .unwrap();
    assert!(matches!(
        cmd,
        Cmd::PreviewHostAdd {
            inventory: None,
            ..
        }
    ));
}

#[test]
fn names_are_checked() {
    assert!(GroupName::new("compose_hosts").is_ok());
    assert!(GroupName::new("_meta").is_ok());
    assert!(GroupName::new("").is_err());
    assert!(GroupName::new("-x").is_err());
    assert!(GroupName::new("a b").is_err());
    assert!(GroupName::new("a".repeat(256)).is_err());
    assert!(ApiBase::new("https://api.github.com").is_ok());
    assert!(ApiBase::new("ftp://x.example").is_err());
    assert!(ApiBase::new("https://x .example").is_err());
    // Duplicate groups are refused.
    let r: Result<Cmd, _> = serde_json::from_value(json!({
        "type": "PreviewHostAdd", "data": {
            "workspace": id(1).to_string(), "inventory_name": "newbox",
            "groups": ["a", "a"], "profiles": []
        }
    }));
    assert!(r.is_err());
    // RepositorySpec has no `project`.
    let r: Result<RepositorySpec, _> = serde_json::from_value(json!({
        "project": id(1).to_string(), "url": "https://x.example/r.git", "branch": "main",
        "transport": "https", "allow_force_push": false, "signed_mode": false,
        "override_quorum": 1
    }));
    assert!(r.is_err());
}

#[test]
fn events_round_trip() {
    let view = CatalogView {
        binding: id(2),
        commit: "a".repeat(40),
        built_at: "2026-11-11T10:00:00Z".into(),
        stale: false,
        error: None,
        playbooks: vec![CatalogPlaybook {
            path: "playbooks/provision.yaml".into(),
            plays: vec![CatalogPlay {
                name: Some("Provision".into()),
                hosts: "all".into(),
            }],
            roles: vec!["base".into()],
            imports: vec![],
            vars_files: vec![],
            unresolved: vec!["{{ role }}".into()],
            jobs: vec!["provision".into()],
        }],
        roles: vec![CatalogRole {
            name: "base".into(),
            path: "roles/base".into(),
            description: None,
            defaults: vec!["base_user".into()],
            dependencies: vec![],
            reached_by: vec!["playbooks/provision.yaml".into()],
            external: false,
        }],
        inventories: vec![CatalogInventory {
            path: "inventory/hosts.yml".into(),
            kind: CatalogInventoryKind::Yaml,
            groups: vec![CatalogGroup {
                name: "compose_hosts".into(),
                parent: Some("all".into()),
                hosts: vec!["web".into()],
                children: vec![],
                vars_file: Some("inventory/group_vars/compose_hosts.yml".into()),
            }],
        }],
        secrets: CatalogSecrets {
            rules_file: Some(".sops.yaml".into()),
            rules: vec![SopsRule {
                index: 0,
                path_regex: "secrets/.*".into(),
                age: vec!["age1x".into()],
                pgp: vec![],
            }],
            files: vec![SopsFile {
                path: "secrets/a.sops.yaml".into(),
                rule: Some(0),
                age: vec!["age1x".into()],
                pgp: vec![],
                lastmodified: None,
                keys: vec!["db.password".into()],
                drift: Some(SopsDrift {
                    missing: vec![],
                    extra: vec![],
                }),
                problem: None,
                read_by: vec!["provision".into()],
            }],
        },
        problems: vec![CatalogProblem {
            path: "x.yml".into(),
            line: Some(3),
            message: "bad".into(),
        }],
        skipped: CatalogSkipped::default(),
        hosts: vec![HostDecrypt {
            host: id(5),
            inventory_name: "web".into(),
            age_recipient: None,
            decrypts: vec![],
        }],
    };
    let v = round_trip(&Event::Catalog(view));
    assert_eq!(v["type"], "Catalog");
    assert_eq!(v["data"]["inventories"][0]["kind"], "yaml");

    round_trip(&Event::JobDraft {
        yaml: "provision:\n  playbook: playbooks/provision.yaml\n".into(),
        notes: vec!["profiles guessed".into()],
    });
    let v = round_trip(&Event::ForgeConnection(None));
    assert_eq!(v, json!({"type": "ForgeConnection", "data": null}));
    let v = round_trip(&Event::ForgeConnection(Some(ForgeConnectionInfo {
        workspace: id(1),
        provider: ForgeProvider::Github,
        api_base: "https://api.github.com".into(),
        credential: id(4),
        probed_at: None,
        probe_error: None,
        created_at: "2026-11-11T10:00:00Z".into(),
    })));
    assert_eq!(v["data"]["provider"], "github");
    round_trip(&Event::ForgeProbe {
        ok: true,
        can_push: false,
        message: Some("read only".into()),
    });
    let v = round_trip(&Event::HostAdded {
        enrollment: EnrollmentIssued {
            id: id(6),
            workspace: id(1),
            inventory_name: "newbox".into(),
            profiles: vec!["prox-vm".into()],
            expires_at: "2026-11-11T11:00:00Z".into(),
            token: EnrollToken("kenr_test".into()),
            snippet: String::new(),
            instructions: String::new(),
            instructions_unavailable: false,
        },
        change: ChangeOutcome::Opened {
            url: "https://git.example/ops/ansible/pulls/7".into(),
            branch: "keel/add-host-newbox".into(),
        },
    });
    assert_eq!(v["data"]["change"]["kind"], "opened");
    let v = round_trip(&Event::ChangeProposed(ChangeOutcome::Manual {
        diff: "--- a\n+++ b\n".into(),
        reason: "forge_not_configured".into(),
    }));
    assert_eq!(v["data"]["kind"], "manual");
    round_trip(&Event::ChangeProposed(ChangeOutcome::Failed {
        diff: String::new(),
        error: "forge_refused".into(),
    }));
}

#[test]
fn new_fields_on_existing_types() {
    // Preview.warnings is left out when empty, and defaults when absent.
    let p: Preview = serde_json::from_value(json!({
        "preview_id": id(1).to_string(), "action": "run_start", "summary": "s",
        "affected": [], "step_up": false, "expires_at": "2026-11-11T10:00:00Z"
    }))
    .unwrap();
    assert!(p.warnings.is_empty());
    assert!(serde_json::to_value(&p).unwrap().get("warnings").is_none());
    let p = Preview {
        warnings: vec!["web cannot decrypt secrets/a.sops.yaml".into()],
        ..p
    };
    assert_eq!(
        round_trip(&p)["warnings"][0],
        "web cannot decrypt secrets/a.sops.yaml"
    );
    assert_eq!(
        serde_json::to_value(CredentialKind::ForgeToken).unwrap(),
        "forge_token"
    );
    assert_eq!(
        serde_json::to_value(HostInventoryStatus::PendingInventory).unwrap(),
        "pending_inventory"
    );
}
