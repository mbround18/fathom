//! Serde round-trips for every command and event variant (spec 050 T034), and the parser's
//! refusals: unknown fields, unknown variants and out-of-bounds values never reach the server.
//!
//! `all_cmds` and `all_events` are checked against exhaustive matches, so a new variant without a
//! sample here fails to compile.

use depths_protocol::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::fmt::Debug;
use uuid::Uuid;

fn roundtrip<T: Serialize + DeserializeOwned + PartialEq + Debug>(value: &T) {
    let text = serde_json::to_string(value).unwrap();
    let back: T = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{text}: {e}"));
    assert_eq!(&back, value, "{text}");
}

fn rejects<T: DeserializeOwned + Debug>(value: Value) {
    let parsed = serde_json::from_value::<T>(value.clone());
    assert!(parsed.is_err(), "accepted {value}: {parsed:?}");
}

fn accepts<T: DeserializeOwned + Debug>(value: Value) {
    if let Err(e) = serde_json::from_value::<T>(value.clone()) {
        panic!("refused {value}: {e}");
    }
}

fn stack(name: &str) -> StackName {
    StackName::new(name).unwrap()
}

fn hash() -> ContentHash {
    ContentHash::new("a".repeat(64)).unwrap()
}

fn preview_id() -> PreviewId {
    PreviewId(Uuid::from_u128(1))
}

fn operation_id() -> OperationId {
    OperationId(Uuid::from_u128(2))
}

fn mount_id() -> MountId {
    MountId::compute(MountKind::Volume, "valheim_saves")
}

fn rel(path: &str) -> RelPath {
    RelPath::parse(path).unwrap()
}

fn fingerprint() -> Fingerprint {
    Fingerprint::of_content(b"port = 2456\n", 1_700_000_000_000_000_000, 42)
}

fn version_id() -> VersionId {
    VersionId::new(format!("{}-3", "b".repeat(64))).unwrap()
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn all_cmds() -> Vec<Cmd> {
    vec![
        Cmd::ListStacks,
        Cmd::GetStack { name: stack("web") },
        Cmd::RevealEnv { name: stack("web") },
        Cmd::PreviewOp {
            stack: stack("web"),
            op: StackOp::Up,
        },
        Cmd::PreviewOp {
            stack: stack("web"),
            op: StackOp::Stop,
        },
        Cmd::PreviewOp {
            stack: stack("web"),
            op: StackOp::Restart,
        },
        Cmd::PreviewOp {
            stack: stack("web"),
            op: StackOp::Down {
                remove_volumes: true,
                remove_images: false,
            },
        },
        Cmd::PreviewOp {
            stack: stack("web"),
            op: StackOp::Pull,
        },
        Cmd::PreviewOp {
            stack: stack("web"),
            op: StackOp::Update,
        },
        Cmd::PreviewOp {
            stack: stack("web"),
            op: StackOp::Delete {
                remove_directory: true,
            },
        },
        Cmd::PreviewOp {
            stack: stack("web"),
            op: StackOp::ServiceUp {
                service: ServiceName::new("app").unwrap(),
            },
        },
        Cmd::PreviewOp {
            stack: stack("web"),
            op: StackOp::ServiceStop {
                service: ServiceName::new("app").unwrap(),
            },
        },
        Cmd::PreviewOp {
            stack: stack("web"),
            op: StackOp::ServiceRestart {
                service: ServiceName::new("app.v2").unwrap(),
            },
        },
        Cmd::PreviewSave {
            draft: StackDraft {
                stack: stack("web"),
                base_hash: Some(hash()),
                compose: "services:\r\n  app:\r\n    image: busybox # keep\r\n".into(),
                env: Some(String::new()),
            },
            deploy: true,
        },
        Cmd::PreviewSave {
            draft: StackDraft {
                stack: stack("new-stack"),
                base_hash: None,
                compose: "\u{feff}services: {}\n".into(),
                env: None,
            },
            deploy: false,
        },
        Cmd::Confirm {
            preview_id: preview_id(),
            acknowledge_findings: false,
        },
        Cmd::Confirm {
            preview_id: preview_id(),
            acknowledge_findings: true,
        },
        Cmd::LintCompose {
            stack: stack("web"),
            compose_text: "services:\n  app:\n    image: busybox\n    privileged: true\n".into(),
        },
        Cmd::Cancel {
            operation_id: operation_id(),
        },
        Cmd::OpenLogs {
            target: LogTarget::Stack {
                stack: stack("web"),
                service: Some(ServiceName::new("app").unwrap()),
            },
            tail: LogTail::new(10_000).unwrap(),
            since: Some(Timestamp::new("2026-10-23T12:00:00Z").unwrap()),
        },
        Cmd::OpenLogs {
            target: LogTarget::Stack {
                stack: stack("web"),
                service: None,
            },
            tail: LogTail::new(0).unwrap(),
            since: None,
        },
        Cmd::OpenLogs {
            target: LogTarget::Container {
                container: ContainerRef::new("web-app-1").unwrap(),
            },
            tail: LogTail::new(200).unwrap(),
            since: None,
        },
        Cmd::OpenShell {
            container: ContainerRef::new("0123abcd").unwrap(),
            shell: Some(Shell::Bash),
            cols: 120,
            rows: 40,
        },
        Cmd::OpenShell {
            container: ContainerRef::new("web-app-1").unwrap(),
            shell: None,
            cols: 80,
            rows: 24,
        },
        Cmd::ShellInput {
            stream_id: StreamId(3),
            bytes_b64: ShellBytes::new("bHMgLWxhCg==").unwrap(),
        },
        Cmd::ShellResize {
            stream_id: StreamId(3),
            cols: 100,
            rows: 30,
        },
        Cmd::WatchStats {
            targets: vec![
                StatsTarget::Stack {
                    stack: stack("web"),
                },
                StatsTarget::Container {
                    container: ContainerRef::new("web-app-1").unwrap(),
                },
            ],
        },
        Cmd::UnwatchStats,
        Cmd::WatchHost,
        Cmd::UnwatchHost,
        Cmd::CloseStream {
            stream_id: StreamId(3),
        },
        Cmd::GetAudit {
            filter: AuditFilter {
                stack: Some(stack("web")),
                actor: Some(UserId::new("oidc:abc|123@x").unwrap()),
                action: Some(AuditAction::Down),
                from: Some(Timestamp::new("2026-10-01T00:00:00+02:00").unwrap()),
                to: Some(Timestamp::new("2026-10-23T00:00:00Z").unwrap()),
            },
            cursor: Some(AuditCursor::new("c2VnLTE6bGluZS00Mg").unwrap()),
        },
        Cmd::GetAudit {
            filter: AuditFilter::default(),
            cursor: None,
        },
        Cmd::GetUsers,
        Cmd::PutUser {
            id: UserId::new("discord:123456789012345678").unwrap(),
            role: Role::User,
            stacks: vec![stack("web"), stack("minecraft")],
        },
        Cmd::RemoveUser {
            id: UserId::new("github:583231").unwrap(),
        },
        Cmd::ListMounts {
            stack: stack("valheim"),
        },
        Cmd::ListDir {
            stack: stack("valheim"),
            mount: mount_id(),
            path: RelPath::root(),
            req: 1,
        },
        Cmd::ReadFile {
            stack: stack("valheim"),
            mount: mount_id(),
            path: rel("config/server.cfg"),
            req: 2,
        },
        Cmd::FolderSize {
            stack: stack("valheim"),
            mount: mount_id(),
            path: rel("worlds"),
            req: 3,
        },
        Cmd::PreviewFileOp {
            stack: stack("valheim"),
            op: FileOp::Save {
                mount: mount_id(),
                path: rel("config/server.cfg"),
                text: "port = 2457\n".into(),
                base: fingerprint(),
                bom: false,
                eol: LineEnding::Crlf,
            },
        },
        Cmd::PreviewFileOp {
            stack: stack("valheim"),
            op: FileOp::Create {
                mount: mount_id(),
                parent: rel("config"),
                name: Name::new("admins.txt").unwrap(),
                kind: CreateKind::File,
            },
        },
        Cmd::PreviewFileOp {
            stack: stack("valheim"),
            op: FileOp::Rename {
                mount: mount_id(),
                from: rel("config/a.cfg"),
                to: rel("config/b.cfg"),
            },
        },
        Cmd::PreviewFileOp {
            stack: stack("valheim"),
            op: FileOp::Delete {
                mount: mount_id(),
                path: rel("worlds"),
                raw_name: Some(RawName::from_bytes(b"old\xff".to_vec()).unwrap()),
            },
        },
        Cmd::PreviewFileOp {
            stack: stack("valheim"),
            op: FileOp::Restore {
                mount: mount_id(),
                path: rel("config/server.cfg"),
                version: version_id(),
            },
        },
        Cmd::PreviewFileOp {
            stack: stack("valheim"),
            op: FileOp::Upload {
                mount: mount_id(),
                dir: rel("worlds"),
                files: vec![UploadFile {
                    name: Name::new("Dedicated.db").unwrap(),
                    size: 1024,
                }],
            },
        },
        Cmd::ListHistory {
            stack: stack("valheim"),
            mount: mount_id(),
            path: rel("config/server.cfg"),
        },
        Cmd::ReadVersion {
            stack: stack("valheim"),
            mount: mount_id(),
            path: rel("config/server.cfg"),
            version: version_id(),
            req: 4,
        },
    ]
}

/// Fails to compile when a `Cmd` variant has no sample above.
fn cmd_covered(cmd: &Cmd) -> &'static str {
    match cmd {
        Cmd::ListStacks => "ListStacks",
        Cmd::GetStack { .. } => "GetStack",
        Cmd::RevealEnv { .. } => "RevealEnv",
        Cmd::PreviewOp { .. } => "PreviewOp",
        Cmd::PreviewSave { .. } => "PreviewSave",
        Cmd::Confirm { .. } => "Confirm",
        Cmd::LintCompose { .. } => "LintCompose",
        Cmd::Cancel { .. } => "Cancel",
        Cmd::OpenLogs { .. } => "OpenLogs",
        Cmd::OpenShell { .. } => "OpenShell",
        Cmd::ShellInput { .. } => "ShellInput",
        Cmd::ShellResize { .. } => "ShellResize",
        Cmd::WatchStats { .. } => "WatchStats",
        Cmd::UnwatchStats => "UnwatchStats",
        Cmd::WatchHost => "WatchHost",
        Cmd::UnwatchHost => "UnwatchHost",
        Cmd::CloseStream { .. } => "CloseStream",
        Cmd::GetAudit { .. } => "GetAudit",
        Cmd::GetUsers => "GetUsers",
        Cmd::PutUser { .. } => "PutUser",
        Cmd::RemoveUser { .. } => "RemoveUser",
        Cmd::ListMounts { .. } => "ListMounts",
        Cmd::ListDir { .. } => "ListDir",
        Cmd::ReadFile { .. } => "ReadFile",
        Cmd::FolderSize { .. } => "FolderSize",
        Cmd::PreviewFileOp { .. } => "PreviewFileOp",
        Cmd::ListHistory { .. } => "ListHistory",
        Cmd::ReadVersion { .. } => "ReadVersion",
    }
}

const CMD_VARIANTS: usize = 28;
const FILE_OP_VARIANTS: usize = 6;
const STACK_OP_VARIANTS: usize = 10;

#[test]
fn every_cmd_roundtrips_in_an_envelope() {
    let cmds = all_cmds();
    let mut seen: Vec<_> = cmds.iter().map(cmd_covered).collect();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), CMD_VARIANTS);

    let mut ops: Vec<_> = cmds
        .iter()
        .filter_map(|c| match c {
            Cmd::PreviewOp { op, .. } => Some(op.kind()),
            _ => None,
        })
        .map(|k| format!("{k:?}"))
        .collect();
    ops.sort();
    ops.dedup();
    assert_eq!(ops.len(), STACK_OP_VARIANTS);

    let mut file_ops: Vec<_> = cmds
        .iter()
        .filter_map(|c| match c {
            Cmd::PreviewFileOp { op, .. } => Some(std::mem::discriminant(op)),
            _ => None,
        })
        .collect();
    file_ops.dedup();
    assert_eq!(file_ops.len(), FILE_OP_VARIANTS);

    for (i, cmd) in cmds.into_iter().enumerate() {
        roundtrip(&CmdEnvelope {
            id: RequestId(i as u64),
            cmd,
        });
    }
}

#[test]
fn hello_roundtrips() {
    roundtrip(&Hello {
        protocol_version: PROTOCOL_VERSION,
        csrf_token: CsrfToken::new("Zm9vYmFyYmF6cXV4LXF1dXhfY29yZ2VncmF1bHRnYXJw").unwrap(),
    });
}

#[test]
fn cmd_wire_shape_is_pinned() {
    assert_eq!(
        serde_json::to_value(Cmd::ListStacks).unwrap(),
        json!({ "type": "ListStacks" })
    );
    assert_eq!(
        serde_json::to_value(Cmd::PreviewOp {
            stack: stack("web"),
            op: StackOp::Down {
                remove_volumes: false,
                remove_images: true,
            },
        })
        .unwrap(),
        json!({ "type": "PreviewOp", "data": { "stack": "web", "op": {
            "kind": "down", "options": { "remove_volumes": false, "remove_images": true }
        } } })
    );
    assert_eq!(
        serde_json::to_value(Cmd::Confirm {
            preview_id: preview_id(),
            acknowledge_findings: false,
        })
        .unwrap(),
        json!({ "type": "Confirm", "data": { "preview_id": "00000000-0000-0000-0000-000000000001" } })
    );
    // 051 FR-018: the acknowledgement is sent only when given, and defaults to off.
    assert_eq!(
        serde_json::to_value(Cmd::Confirm {
            preview_id: preview_id(),
            acknowledge_findings: true,
        })
        .unwrap(),
        json!({ "type": "Confirm", "data": {
            "preview_id": "00000000-0000-0000-0000-000000000001", "acknowledge_findings": true
        } })
    );
    // Options default to off when absent.
    let parsed: Cmd = serde_json::from_value(json!({
        "type": "PreviewOp", "data": { "stack": "web", "op": { "kind": "down", "options": {} } }
    }))
    .unwrap();
    assert_eq!(
        parsed,
        Cmd::PreviewOp {
            stack: stack("web"),
            op: StackOp::Down {
                remove_volumes: false,
                remove_images: false
            }
        }
    );
}

// ---------------------------------------------------------------------------
// Refusals: unknown fields and variants
// ---------------------------------------------------------------------------

#[test]
fn unknown_fields_are_rejected_everywhere_a_client_writes() {
    // Controls: the same frames without the extra field parse.
    accepts::<CmdEnvelope>(json!({ "id": 1, "cmd": { "type": "ListStacks" } }));
    accepts::<Cmd>(json!({ "type": "GetStack", "data": { "name": "web" } }));
    accepts::<Cmd>(
        json!({ "type": "PreviewSave", "data": { "deploy": false, "draft": {
        "stack": "web", "compose": ""
    } } }),
    );
    accepts::<Cmd>(
        json!({ "type": "OpenLogs", "data": { "tail": 10, "target": {
        "kind": "container", "container": "x"
    } } }),
    );
    accepts::<Cmd>(json!({ "type": "GetAudit", "data": { "filter": { "stack": "web" } } }));
    accepts::<Cmd>(json!({ "type": "GetAudit", "data": {} }));

    // Envelope and Hello.
    rejects::<CmdEnvelope>(json!({ "id": 1, "cmd": { "type": "ListStacks" }, "admin": true }));
    rejects::<Hello>(json!({ "protocol_version": 1, "csrf_token": "abc", "role": "admin" }));
    // Next to the tag.
    rejects::<Cmd>(json!({ "type": "ListStacks", "extra": 1 }));
    // Inside a variant.
    rejects::<Cmd>(json!({ "type": "GetStack", "data": { "name": "web", "path": "/etc" } }));
    rejects::<Cmd>(json!({ "type": "Confirm", "data": {
        "preview_id": "00000000-0000-0000-0000-000000000001", "force": true
    } }));
    // Nested types.
    rejects::<Cmd>(
        json!({ "type": "PreviewSave", "data": { "deploy": false, "draft": {
        "stack": "web", "compose": "", "path": "../../etc/passwd"
    } } }),
    );
    rejects::<Cmd>(
        json!({ "type": "PreviewOp", "data": { "stack": "web", "op": {
        "kind": "down", "options": { "remove_volumes": true, "argv": ["rm", "-rf", "/"] }
    } } }),
    );
    rejects::<Cmd>(
        json!({ "type": "PreviewOp", "data": { "stack": "web", "op": {
        "kind": "up", "extra": 1
    } } }),
    );
    rejects::<Cmd>(
        json!({ "type": "OpenLogs", "data": { "tail": 10, "target": {
        "kind": "container", "container": "x", "follow_host": true
    } } }),
    );
    rejects::<Cmd>(json!({ "type": "WatchStats", "data": { "targets": [
        { "kind": "stack", "stack": "web", "interval": 0 }
    ] } }));
    rejects::<Cmd>(
        json!({ "type": "GetAudit", "data": { "filter": { "stack": "web", "sql": "1=1" } } }),
    );
}

#[test]
fn unknown_variants_are_rejected() {
    rejects::<Cmd>(json!({ "type": "Exec", "data": { "argv": ["sh"] } }));
    rejects::<Cmd>(json!({ "type": "liststacks" }));
    rejects::<Cmd>(json!({ "data": {} }));
    rejects::<Cmd>(json!("ListStacks"));
    rejects::<Cmd>(
        json!({ "type": "PreviewOp", "data": { "stack": "web", "op": { "kind": "save" } } }),
    );
    rejects::<Cmd>(
        json!({ "type": "PreviewOp", "data": { "stack": "web", "op": { "kind": "prune" } } }),
    );
    rejects::<Cmd>(
        json!({ "type": "OpenLogs", "data": { "tail": 1, "target": { "kind": "host" } } }),
    );
    rejects::<Cmd>(json!({ "type": "OpenShell", "data": {
        "container": "x", "shell": "/bin/sh -c id", "cols": 80, "rows": 24
    } }));
    rejects::<Cmd>(
        json!({ "type": "PutUser", "data": { "id": "github:1", "role": "root", "stacks": [] } }),
    );
    rejects::<Cmd>(json!({ "type": "GetAudit", "data": { "filter": { "action": "drop_table" } } }));
}

#[test]
fn unit_variants_refuse_a_payload() {
    rejects::<Cmd>(json!({ "type": "ListStacks", "data": { "all": true } }));
    rejects::<Cmd>(json!({ "type": "GetUsers", "data": [1] }));
}

#[test]
fn missing_required_fields_are_rejected() {
    rejects::<CmdEnvelope>(json!({ "cmd": { "type": "ListStacks" } }));
    rejects::<Hello>(json!({ "protocol_version": 1 }));
    rejects::<Cmd>(json!({ "type": "GetStack", "data": {} }));
    rejects::<Cmd>(
        json!({ "type": "OpenLogs", "data": { "target": { "kind": "container", "container": "x" } } }),
    );
}

// ---------------------------------------------------------------------------
// Refusals: bounds
// ---------------------------------------------------------------------------

#[test]
fn stack_names_follow_the_compose_rule() {
    for ok in ["web", "0abc", "a_b-c", &"a".repeat(MAX_NAME_BYTES)] {
        assert!(StackName::new(ok).is_ok(), "{ok}");
    }
    for bad in [
        "",
        "Web",
        "-web",
        "_web",
        "..",
        "../etc",
        "a/b",
        "/abs",
        "a b",
        "web\n",
        "wéb",
        &"a".repeat(MAX_NAME_BYTES + 1),
    ] {
        assert!(StackName::new(bad).is_err(), "{bad:?}");
        rejects::<Cmd>(json!({ "type": "GetStack", "data": { "name": bad } }));
    }
}

#[test]
fn service_and_container_names_cannot_read_as_flags() {
    for bad in ["", "-f", "--help", ".hidden", "a/b", "a b", "a;b"] {
        assert!(ServiceName::new(bad).is_err(), "{bad:?}");
        assert!(ContainerRef::new(bad).is_err(), "{bad:?}");
    }
    assert!(ServiceName::new("App.v2_x-1").is_ok());
    assert!(ContainerRef::new("a".repeat(64)).is_ok());
    assert!(ContainerRef::new("a".repeat(MAX_NAME_BYTES + 1)).is_err());
}

#[test]
fn user_ids_are_provider_qualified() {
    for ok in [
        "discord:123456789012345678",
        "github:583231",
        "oidc:248289761001",
        "oidc:auth0|abc@example.com",
    ] {
        assert!(UserId::new(ok).is_ok(), "{ok}");
    }
    for bad in [
        "",
        "583231",
        "github:octocat",
        "discord:",
        "discord:12a",
        "ldap:1",
        "oidc:has space",
        "oidc:tab\t",
        &format!("oidc:{}", "x".repeat(MAX_SUBJECT_BYTES + 1)),
    ] {
        assert!(UserId::new(bad).is_err(), "{bad:?}");
        rejects::<Cmd>(json!({ "type": "RemoveUser", "data": { "id": bad } }));
    }
}

#[test]
fn hashes_tokens_and_timestamps_are_checked() {
    assert!(ContentHash::new("A".repeat(64)).is_err());
    assert!(ContentHash::new("a".repeat(63)).is_err());
    assert!(ContentHash::new("g".repeat(64)).is_err());

    assert!(CsrfToken::new("").is_err());
    assert!(CsrfToken::new("a+b").is_err());
    assert!(CsrfToken::new("a".repeat(MAX_CSRF_TOKEN_BYTES + 1)).is_err());

    assert!(Timestamp::new("yesterday").is_err());
    assert!(Timestamp::new("2026-10-23").is_err());
    assert!(Timestamp::new("10m").is_err());

    assert!(AuditCursor::new("../x").is_err());
    assert!(AuditCursor::new("a".repeat(MAX_CURSOR_BYTES + 1)).is_err());

    rejects::<Cmd>(json!({ "type": "Confirm", "data": { "preview_id": "not-a-uuid" } }));
    rejects::<Cmd>(json!({ "type": "Cancel", "data": { "operation_id": 5 } }));
    rejects::<Cmd>(
        json!({ "type": "PreviewSave", "data": { "deploy": false, "draft": {
        "stack": "web", "compose": "", "base_hash": "deadbeef"
    } } }),
    );
}

#[test]
fn shell_input_is_bounded_base64() {
    assert!(ShellBytes::new("").is_ok());
    assert!(ShellBytes::new("AAAA").is_ok());
    assert!(ShellBytes::new("AA==").is_ok());
    assert!(ShellBytes::new("AAA").is_err());
    assert!(ShellBytes::new("A===").is_err());
    assert!(ShellBytes::new("AA-_").is_err());
    assert!(ShellBytes::new("A".repeat(MAX_SHELL_INPUT_B64_BYTES)).is_ok());
    assert!(ShellBytes::new("A".repeat(MAX_SHELL_INPUT_B64_BYTES + 4)).is_err());
}

#[test]
fn numbers_and_lists_are_bounded() {
    assert!(LogTail::new(MAX_LOG_TAIL).is_ok());
    rejects::<Cmd>(json!({ "type": "OpenLogs", "data": {
        "tail": MAX_LOG_TAIL + 1, "target": { "kind": "container", "container": "x" }
    } }));
    rejects::<Cmd>(json!({ "type": "OpenLogs", "data": {
        "tail": -1, "target": { "kind": "container", "container": "x" }
    } }));
    rejects::<Cmd>(
        json!({ "type": "ShellResize", "data": { "stream_id": 1, "cols": 70000, "rows": 1 } }),
    );

    let targets = |n: usize| {
        json!({ "type": "WatchStats", "data": {
        "targets": vec![json!({ "kind": "stack", "stack": "web" }); n]
    } })
    };
    assert!(serde_json::from_value::<Cmd>(targets(MAX_STATS_TARGETS)).is_ok());
    rejects::<Cmd>(targets(MAX_STATS_TARGETS + 1));

    let stacks = |n: usize| {
        json!({ "type": "PutUser", "data": {
        "id": "github:1", "role": "user", "stacks": vec!["web"; n]
    } })
    };
    assert!(serde_json::from_value::<Cmd>(stacks(MAX_USER_STACKS)).is_ok());
    rejects::<Cmd>(stacks(MAX_USER_STACKS + 1));
}

#[test]
fn close_codes_match_the_contract() {
    assert_eq!(CloseCode::Unauthenticated.code(), 4401);
    assert_eq!(CloseCode::Forbidden.code(), 4403);
    assert_eq!(CloseCode::TooMany.code(), 4429);
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

fn container() -> Container {
    Container {
        id: "0123abcd".repeat(8),
        name: "web-app-1".into(),
        image: "busybox:1.37".into(),
        state: RuntimeContainerState::Running,
        health: Some(ContainerHealth::Healthy),
        status: "Up 3 hours".into(),
        created_at: "2026-10-23T09:00:00Z".into(),
        command: "sleep infinity".into(),
        ports: vec![PublishedPort {
            container_port: 80,
            host_port: Some(8080),
            host_ip: Some("0.0.0.0".into()),
            protocol: "tcp".into(),
        }],
        project: Some("web".into()),
        service: Some("app".into()),
        cluster: None,
        adoption: AdoptionState::Adopted,
    }
}

fn stack_summary() -> Stack {
    Stack {
        name: "web".into(),
        valid_name: true,
        compose_file: ComposeFileName::ComposeYaml,
        shadowed_files: vec![ComposeFileName::DockerComposeYml],
        has_env: true,
        content_hash: "a".repeat(64),
        status: StackStatus::Partial,
        unhealthy: true,
        services: vec![ServiceSummary {
            name: "app".into(),
            image: Some("busybox:1.37".into()),
            containers: vec![
                StackContainer {
                    container: container(),
                    security: Some(ContainerSecurity {
                        level: SecurityLevel::Escape,
                        reasons: vec![SecurityReason {
                            code: "privileged".into(),
                            severity: SecuritySeverity::Escape,
                            message: "runs privileged".into(),
                        }],
                        shell: ShellRule::Refused,
                    }),
                },
                container().into(),
            ],
        }],
        running_services: 1,
        total_services: 2,
        modified_at: Some("2026-10-23T09:00:00Z".into()),
        read_error: None,
        invalid_reason: None,
        read_only_reason: Some("stack directory is a symlink".into()),
        too_large: false,
        security: Some(SecurityLevel::Escape),
    }
}

fn stack_list() -> StackList {
    StackList {
        root: "/opt/stacks".into(),
        stacks: vec![stack_summary()],
        foreign: vec![ForeignProject {
            name: "manual".into(),
            working_dir: Some("/opt/stacks/web".into()),
            containers: vec![container()],
            elsewhere_of: Some("web".into()),
        }],
        standalone: vec![container()],
        warnings: vec!["web: docker-compose.yml is shadowed by compose.yaml".into()],
        stale: false,
        as_of: "2026-10-23T09:00:00Z".into(),
    }
}

fn user(role: Role) -> User {
    User {
        id: "oidc:248289761001".into(),
        provider: Provider::Oidc,
        issuer: Some("https://id.example.com".into()),
        subject: "248289761001".into(),
        username: Some("ana".into()),
        email: Some("ana@example.com".into()),
        role,
        stacks: vec!["web".into(), "gone".into()],
        owner: false,
        pinned_from: None,
        first_seen: None,
        last_seen: None,
    }
}

fn actor() -> Actor {
    Actor {
        provider: Provider::Github,
        subject: "583231".into(),
        username: "octocat".into(),
    }
}

fn all_events() -> Vec<Event> {
    let users = UserList {
        users: vec![
            User {
                owner: true,
                pinned_from: Some("github:octocat".into()),
                provider: Provider::Github,
                issuer: None,
                ..user(Role::Admin)
            },
            user(Role::User),
        ],
        pending: vec![user(Role::Pending)],
    };
    vec![
        Event::Stacks(stack_list()),
        Event::StacksChanged(StackList {
            stale: true,
            ..stack_list()
        }),
        Event::RuntimeUnavailable {
            reason: "Cannot connect to the Docker daemon".into(),
        },
        Event::StackDetail(StackDetail {
            stack: stack_summary(),
            compose_text: Some("services: {}\n".into()),
            env: Some(EnvText::Masked("DB_PASSWORD=********\nPORT=80\n".into())),
            content_hash: "a".repeat(64),
            compose_warnings: vec![warning()],
        }),
        Event::StackDetail(StackDetail {
            stack: Stack {
                too_large: true,
                ..stack_summary()
            },
            compose_text: None,
            env: Some(EnvText::Plain("PORT=80\n".into())),
            content_hash: "b".repeat(64),
            compose_warnings: vec![],
        }),
        Event::EnvRevealed {
            name: "web".into(),
            env_text: "DB_PASSWORD=hunter2\n".into(),
        },
        Event::Preview(Preview {
            preview_id: preview_id(),
            stack: "web".into(),
            kind: OperationKind::Save,
            steps: vec![
                Step {
                    argv: vec![
                        "docker".into(),
                        "compose".into(),
                        "-f".into(),
                        "-".into(),
                        "config".into(),
                        "--quiet".into(),
                    ],
                    cwd: "/opt/stacks/web".into(),
                    stdin: StepStdin::Draft,
                },
                Step {
                    argv: vec!["docker".into(), "compose".into(), "up".into(), "-d".into()],
                    cwd: "/opt/stacks/web".into(),
                    stdin: StepStdin::None,
                },
            ],
            file_diff: Some("--- a/compose.yaml\n+++ b/compose.yaml\n".into()),
            reaches: Reach {
                containers: vec!["web-app-1".into()],
                networks: vec!["web_default".into()],
                volumes: vec![],
            },
            destructive: false,
            stack_hash: "a".repeat(64),
            expires_at: "2026-10-23T09:05:00Z".into(),
            warnings: vec![warning()],
            ack_required: true,
        }),
        Event::ValidationFailed {
            errors: vec![
                ValidationError {
                    file: DraftFile::Compose,
                    line: Some(3),
                    message: "services.app.image must be a string".into(),
                },
                ValidationError {
                    file: DraftFile::Env,
                    line: None,
                    message: "yaml: did not find expected key".into(),
                },
            ],
        },
        Event::Conflict {
            current_text: "services: {}\n".into(),
            current_env: Some(EnvText::Masked("API_TOKEN=********\n".into())),
            current_hash: "c".repeat(64),
        },
        Event::PreviewVoid {
            reason: PreviewVoidReason::StackChanged,
        },
        Event::LintResult {
            stack: "web".into(),
            findings: vec![warning()],
        },
        Event::OpStarted {
            operation_id: operation_id(),
        },
        Event::OpOutput {
            operation_id: operation_id(),
            chunk: " Container web-app-1  Started\n".into(),
        },
        Event::OpFinished {
            operation_id: operation_id(),
            outcome: OpOutcome::Failed {
                exit_code: Some(1),
                stderr: "no such service: ap\n".into(),
            },
        },
        Event::OpFinished {
            operation_id: operation_id(),
            outcome: OpOutcome::Succeeded,
        },
        Event::OpFinished {
            operation_id: operation_id(),
            outcome: OpOutcome::Cancelled,
        },
        Event::Denied {
            reason: DenyReason::NotPermitted,
            detail: None,
        },
        Event::Denied {
            reason: DenyReason::ShellEscape,
            detail: Some("runs privileged".into()),
        },
        Event::Busy {
            stack: "web".into(),
            operation_id: operation_id(),
            kind: OperationKind::Restart,
            actor: actor(),
        },
        Event::Invalid {
            reason: "unknown stack".into(),
        },
        Event::StreamOpened {
            stream_id: StreamId(9),
        },
        Event::LogChunk {
            stream_id: StreamId(9),
            lines: vec!["app-1  | hello".into(), String::new()],
        },
        Event::ShellOutput {
            stream_id: StreamId(9),
            bytes_b64: "G1szMm0kG1swbSA=".into(),
        },
        Event::StreamClosed {
            stream_id: StreamId(9),
            reason: StreamCloseReason::Disconnected,
        },
        Event::StreamClosed {
            stream_id: StreamId(9),
            reason: StreamCloseReason::Failed("container is not running".into()),
        },
        Event::StatsSample {
            samples: vec![UsageSample {
                container_id: "0123abcd".into(),
                at: "2026-10-23T09:00:02Z".into(),
                cpu_percent: Some(12.5),
                memory_bytes: Some(1 << 20),
                memory_limit_bytes: None,
            }],
        },
        Event::HostSample(HostSample {
            at: "2026-10-23T09:00:02Z".into(),
            cpu_percent: Some(7.5),
            cpu_cores: Some(32),
            load: Some(HostLoad {
                one: 1.5,
                five: 1.25,
                fifteen: 0.5,
            }),
            memory: Some(HostMemory {
                total_bytes: 32 << 30,
                available_bytes: 20 << 30,
                used_bytes: 12 << 30,
            }),
            swap: None,
            uptime_secs: Some(86_400),
            disk: Some(HostDisk {
                total_bytes: 1 << 40,
                used_bytes: 1 << 39,
                free_bytes: 1 << 38,
            }),
            containers_running: 3,
            containers_stopped: 1,
        }),
        Event::HostSample(HostSample {
            at: "2026-10-23T09:00:04Z".into(),
            cpu_percent: None,
            cpu_cores: None,
            load: None,
            memory: None,
            swap: Some(HostSwap {
                total_bytes: 8 << 30,
                free_bytes: 8 << 30,
                used_bytes: 0,
            }),
            uptime_secs: None,
            disk: None,
            containers_running: 0,
            containers_stopped: 0,
        }),
        Event::AuditPage {
            entries: vec![
                AuditEntry {
                    at: "2026-10-23T09:00:00Z".into(),
                    actor: actor(),
                    action: AuditAction::Down,
                    stack: Some("web".into()),
                    preview_id: Some(preview_id()),
                    operation_id: Some(operation_id()),
                    argv: vec![vec!["docker".into(), "compose".into(), "down".into()]],
                    target: None,
                    diff_summary: None,
                    outcome: AuditOutcome::Failed {
                        exit_code: Some(1),
                        stderr_tail: "permission denied".into(),
                    },
                    source_ip: "203.0.113.7".into(),
                    network: None,
                    security: None,
                },
                AuditEntry {
                    at: "2026-10-23T09:01:00Z".into(),
                    actor: actor(),
                    action: AuditAction::UserChanged,
                    stack: None,
                    preview_id: None,
                    operation_id: None,
                    argv: vec![],
                    target: Some(UserChange {
                        id: "discord:1".into(),
                        before: Some(Grant {
                            role: Role::Pending,
                            stacks: vec![],
                        }),
                        after: Some(Grant {
                            role: Role::User,
                            stacks: vec!["web".into()],
                        }),
                    }),
                    diff_summary: Some("compose.yaml: +1 -1; .env: PORT".into()),
                    outcome: AuditOutcome::Succeeded,
                    source_ip: "203.0.113.7".into(),
                    network: None,
                    security: None,
                },
                AuditEntry {
                    at: "2026-10-23T09:02:00Z".into(),
                    actor: actor(),
                    action: AuditAction::Denied,
                    stack: Some("db".into()),
                    preview_id: None,
                    operation_id: None,
                    argv: vec![],
                    target: None,
                    diff_summary: None,
                    outcome: AuditOutcome::Denied {
                        reason: "not_permitted".into(),
                    },
                    source_ip: "203.0.113.7".into(),
                    network: Some(AuditNetwork {
                        network: Some("198.51.100.7/32".into()),
                        cmd: Some("open_shell".into()),
                        ..AuditNetwork::default()
                    }),
                    security: Some(AuditSecurity {
                        level: SecurityLevel::Root,
                        reasons: vec!["root_user".into()],
                        acknowledged: false,
                    }),
                },
            ],
            next_cursor: Some("c2VnLTE".into()),
        },
        Event::Users(users.clone()),
        Event::UsersChanged(users),
        Event::SessionExpiring {
            at: "2026-10-23T17:00:00Z".into(),
        },
        Event::Mounts {
            stack: "valheim".into(),
            mounts: vec![
                Mount::new(
                    MountKind::Volume,
                    "valheim_saves".into(),
                    vec![MountService {
                        service: "server".into(),
                        destination: "/config".into(),
                        read_only: false,
                    }],
                    MountState::Ready,
                    Browsable::yes(),
                    vec!["server".into()],
                ),
                Mount::new(
                    MountKind::Tmpfs,
                    String::new(),
                    vec![],
                    MountState::Ready,
                    Browsable::no(NotBrowsableCode::Tmpfs, "tmpfs lives in memory"),
                    vec![],
                ),
            ],
        },
        Event::DirPage {
            req: 1,
            mount: mount_id(),
            path: RelPath::root(),
            entries: vec![Entry {
                name: "old\u{fffd}".into(),
                name_b64: Some(RawName::from_bytes(b"old\xff".to_vec()).unwrap()),
                kind: EntryKind::Link,
                link: Some(LinkInfo {
                    target: "/etc/passwd".into(),
                    inside: false,
                }),
                size: 0,
                mtime: "2026-10-23T12:00:00Z".into(),
                uid: 1000,
                gid: 1000,
                mode: 0o644,
                nlink: 1,
                write_blocked: Some(WriteBlocked::OutsideLink),
            }],
            page: 0,
            last: true,
        },
        Event::FileContent {
            req: 2,
            mount: mount_id(),
            path: rel("config/server.cfg"),
            version: Some(version_id()),
            file: FileContent::build(
                fingerprint(),
                Owner {
                    uid: 1000,
                    gid: 1000,
                    mode: 0o644,
                },
                None,
                Some(b"port = 2456\n"),
                1 << 20,
            ),
        },
        Event::FolderSize {
            req: 3,
            bytes: 1 << 30,
            entries: 812,
            capped: false,
        },
        Event::FilePreview(FilePreview {
            preview_id: preview_id(),
            stack: "valheim".into(),
            mount: mount_id(),
            summary: "Upload 1 file to worlds".into(),
            diff: None,
            entries: Some(10_000),
            entries_capped: true,
            uploads: vec![UploadTarget {
                name: Name::new("Dedicated.db").unwrap(),
                size: 1024,
                exists: true,
                fingerprint: Some(fingerprint()),
            }],
            running: vec!["server".into()],
            expires_at: "2026-10-23T12:05:00Z".into(),
        }),
        Event::FileOpDone {
            preview_id: preview_id(),
            mount: mount_id(),
            path: rel("config/server.cfg"),
            fingerprint: Some(fingerprint()),
            running: vec!["server".into()],
        },
        Event::History {
            mount: mount_id(),
            path: rel("config/server.cfg"),
            versions: vec![FileVersion {
                id: version_id(),
                at: "2026-10-23T11:00:00Z".into(),
                author: "oidc:abc".into(),
                size: 12,
                reason: VersionReason::Save,
            }],
        },
        Event::FileRefused(FileRefusal::ChangedOnDisk {
            current: fingerprint(),
        }),
    ]
}

/// Fails to compile when an `Event` variant has no sample above.
fn event_covered(event: &Event) -> &'static str {
    match event {
        Event::Stacks(_) => "Stacks",
        Event::StacksChanged(_) => "StacksChanged",
        Event::RuntimeUnavailable { .. } => "RuntimeUnavailable",
        Event::StackDetail(_) => "StackDetail",
        Event::EnvRevealed { .. } => "EnvRevealed",
        Event::Preview(_) => "Preview",
        Event::ValidationFailed { .. } => "ValidationFailed",
        Event::Conflict { .. } => "Conflict",
        Event::PreviewVoid { .. } => "PreviewVoid",
        Event::LintResult { .. } => "LintResult",
        Event::OpStarted { .. } => "OpStarted",
        Event::OpOutput { .. } => "OpOutput",
        Event::OpFinished { .. } => "OpFinished",
        Event::Denied { .. } => "Denied",
        Event::Busy { .. } => "Busy",
        Event::Invalid { .. } => "Invalid",
        Event::StreamOpened { .. } => "StreamOpened",
        Event::LogChunk { .. } => "LogChunk",
        Event::ShellOutput { .. } => "ShellOutput",
        Event::StreamClosed { .. } => "StreamClosed",
        Event::StatsSample { .. } => "StatsSample",
        Event::HostSample(_) => "HostSample",
        Event::AuditPage { .. } => "AuditPage",
        Event::Users(_) => "Users",
        Event::UsersChanged(_) => "UsersChanged",
        Event::SessionExpiring { .. } => "SessionExpiring",
        Event::Mounts { .. } => "Mounts",
        Event::DirPage { .. } => "DirPage",
        Event::FileContent { .. } => "FileContent",
        Event::FolderSize { .. } => "FolderSize",
        Event::FilePreview(_) => "FilePreview",
        Event::FileOpDone { .. } => "FileOpDone",
        Event::History { .. } => "History",
        Event::FileRefused(_) => "FileRefused",
    }
}

const EVENT_VARIANTS: usize = 34;

#[test]
fn every_event_roundtrips_in_an_envelope() {
    let events = all_events();
    let mut seen: Vec<_> = events.iter().map(event_covered).collect();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), EVENT_VARIANTS);

    for (i, event) in events.into_iter().enumerate() {
        roundtrip(&EventEnvelope {
            reply_to: (i % 2 == 0).then_some(RequestId(i as u64)),
            event,
        });
    }
}

#[test]
fn every_deny_and_void_reason_roundtrips() {
    for reason in [
        DenyReason::NotPermitted,
        DenyReason::Pending,
        DenyReason::FeatureDisabled,
        DenyReason::Owner,
        DenyReason::RoleNotAssignable,
        DenyReason::ShellEscape,
        DenyReason::ShellRootAdminOnly,
        DenyReason::ShellUnverifiable,
        DenyReason::ComposeBlocked,
    ] {
        roundtrip(&Event::Denied {
            reason,
            detail: None,
        });
    }
    for reason in [
        PreviewVoidReason::Unknown,
        PreviewVoidReason::Used,
        PreviewVoidReason::Expired,
        PreviewVoidReason::StackChanged,
    ] {
        roundtrip(&Event::PreviewVoid { reason });
    }
    for reason in [
        StreamCloseReason::Ended,
        StreamCloseReason::Closed,
        StreamCloseReason::ContainerRecreated,
        StreamCloseReason::ContainerRemoved,
    ] {
        roundtrip(&reason);
    }
}

#[test]
fn event_wire_shape_is_pinned() {
    assert_eq!(
        serde_json::to_value(EventEnvelope {
            reply_to: Some(RequestId(4)),
            event: Event::Denied {
                reason: DenyReason::NotPermitted,
                detail: None,
            },
        })
        .unwrap(),
        json!({ "reply_to": 4, "event": { "type": "Denied", "data": { "reason": "not_permitted" } } })
    );
    assert_eq!(
        serde_json::to_value(EventEnvelope {
            reply_to: None,
            event: Event::StreamClosed {
                stream_id: StreamId(2),
                reason: StreamCloseReason::Disconnected,
            },
        })
        .unwrap(),
        json!({ "event": { "type": "StreamClosed", "data": {
            "stream_id": 2, "reason": { "kind": "disconnected" }
        } } })
    );
    assert_eq!(
        serde_json::to_value(OpOutcome::Cancelled).unwrap(),
        json!({ "kind": "cancelled" })
    );
}

#[test]
fn me_roundtrips() {
    roundtrip(&Me {
        identity: MeIdentity {
            provider: Provider::Discord,
            subject: "123456789012345678".into(),
            username: "ana".into(),
            email: None,
        },
        csrf_token: "abc".into(),
        role: Role::Pending,
        owner: false,
        stacks: vec![],
        features: Features {
            shell: false,
            edit: true,
            host_metrics: false,
            shell_policy: ShellPolicy::Strict,
            compose_policy: ComposePolicy::Enforce,
            daemon: Some(DaemonMode::Rootful),
            files: FilesMode::ReadOnly,
            files_reason: Some("openat2 is missing".into()),
        },
        network: TrustDecision {
            trusted: false,
            reason: TrustReason::Maturing,
            matures_at: Some("2026-10-30T09:00:00Z".into()),
            network: "198.51.100.7/32".into(),
        },
        needs_answer: false,
        proxy_warning: false,
        limits: MeLimits {
            files: FileLimits {
                editor_max: 1 << 20,
                download_max: 1 << 32,
                upload_max: 1 << 30,
            },
        },
    });
}

#[test]
fn files_mode_and_limits_default_when_absent() {
    // An older server sends neither: files are off.
    let features: Features = serde_json::from_value(json!({
        "shell": false, "edit": false, "host_metrics": false
    }))
    .unwrap();
    assert_eq!(features.files, FilesMode::Off);
    assert_eq!(features.files_reason, None);
    assert_eq!(
        serde_json::to_value(FilesMode::ReadOnly).unwrap(),
        json!("read_only")
    );
}

fn warning() -> ComposeWarning {
    ComposeWarning {
        service: "app".into(),
        code: "privileged".into(),
        severity: SecuritySeverity::Escape,
        message: "runs privileged: it can reach the host".into(),
        line: Some(4),
    }
}

#[test]
fn security_wire_shapes_are_pinned() {
    // 051: a rated container is the container's fields plus `security`; an unrated one has none.
    let rated = StackContainer {
        container: container(),
        security: Some(ContainerSecurity {
            level: SecurityLevel::Root,
            reasons: vec![SecurityReason {
                code: "root_user".into(),
                severity: SecuritySeverity::Root,
                message: "runs as root".into(),
            }],
            shell: ShellRule::AdminOnly,
        }),
    };
    let v = serde_json::to_value(&rated).unwrap();
    assert_eq!(v["id"], serde_json::to_value(container()).unwrap()["id"]);
    assert_eq!(
        v["security"],
        json!({ "level": "root", "shell": "admin_only", "reasons": [
            { "code": "root_user", "severity": "root", "message": "runs as root" }
        ] })
    );
    let plain = serde_json::to_value(StackContainer::from(container())).unwrap();
    assert!(plain.get("security").is_none());
    assert_eq!(plain, serde_json::to_value(container()).unwrap());
    // Older payloads without the new fields still parse.
    let mut old = serde_json::to_value(stack_summary()).unwrap();
    old.as_object_mut().unwrap().remove("security");
    let back: Stack = serde_json::from_value(old).unwrap();
    assert_eq!(back.security, None);
    assert_eq!(
        serde_json::to_value(DenyReason::ShellUnverifiable).unwrap(),
        json!("shell_unverifiable")
    );
    assert_eq!(
        serde_json::to_value(StreamCloseReason::ContainerRecreated).unwrap(),
        json!({ "kind": "container_recreated" })
    );
}

// ---------------------------------------------------------------------------
// Files (spec 053)
// ---------------------------------------------------------------------------

#[test]
fn every_file_refusal_roundtrips() {
    for refusal in [
        FileRefusal::FeatureReadOnly,
        FileRefusal::HelperUnavailable {
            reason: "no docker".into(),
        },
        FileRefusal::NotBrowsable {
            reason: NotBrowsable {
                code: NotBrowsableCode::RiskyBind,
                message: "/etc".into(),
            },
        },
        FileRefusal::ReadOnlyMount,
        FileRefusal::NotCreated,
        FileRefusal::Missing,
        FileRefusal::OutsideMount,
        FileRefusal::WriteBlocked {
            reason: WriteBlocked::HardLink,
        },
        FileRefusal::ChangedOnDisk {
            current: fingerprint(),
        },
        FileRefusal::OwnerChanged,
        FileRefusal::TooLarge { bytes: Some(9) },
        FileRefusal::TooLarge { bytes: None },
        FileRefusal::NotText,
        FileRefusal::Exists,
        FileRefusal::NotFound,
        FileRefusal::InvalidName,
        FileRefusal::MixedOwners,
        FileRefusal::NotConfirmed,
        FileRefusal::Partial {
            removed: 3,
            failed_at: "worlds/locked".into(),
            errno: "EACCES".into(),
            message: "Permission denied".into(),
        },
        FileRefusal::OsError {
            errno: "ENOSPC".into(),
            message: "No space left on device".into(),
        },
    ] {
        roundtrip(&Event::FileRefused(refusal));
    }
}

#[test]
fn file_wire_shape_is_pinned() {
    let mount = mount_id();
    assert_eq!(
        serde_json::to_value(Cmd::ListDir {
            stack: stack("valheim"),
            mount: mount.clone(),
            path: RelPath::root(),
            req: 7,
        })
        .unwrap(),
        json!({ "type": "ListDir", "data": {
            "stack": "valheim", "mount": mount.as_str(), "path": "", "req": 7
        } })
    );
    assert_eq!(
        serde_json::to_value(Cmd::PreviewFileOp {
            stack: stack("valheim"),
            op: FileOp::Rename {
                mount: mount.clone(),
                from: rel("a.cfg"),
                to: rel("b.cfg"),
            },
        })
        .unwrap(),
        json!({ "type": "PreviewFileOp", "data": { "stack": "valheim", "op": {
            "kind": "rename", "options": { "mount": mount.as_str(), "from": "a.cfg", "to": "b.cfg" }
        } } })
    );
    assert_eq!(
        serde_json::to_value(Event::FileRefused(FileRefusal::OsError {
            errno: "EACCES".into(),
            message: "Permission denied".into(),
        }))
        .unwrap(),
        json!({ "type": "FileRefused", "data": {
            "kind": "os_error", "errno": "EACCES", "message": "Permission denied"
        } })
    );
    assert_eq!(
        serde_json::to_value(Event::FileRefused(FileRefusal::Exists)).unwrap(),
        json!({ "type": "FileRefused", "data": { "kind": "exists" } })
    );
}

#[test]
fn file_cmds_refuse_bad_values() {
    let mount = mount_id();
    let list_dir = |path: Value, mount: Value| {
        json!({ "type": "ListDir", "data": {
            "stack": "valheim", "mount": mount, "path": path, "req": 1
        } })
    };
    accepts::<Cmd>(list_dir(json!("config"), json!(mount.as_str())));
    for path in ["/etc", "../x", "a/../../b", "a\u{0}b"] {
        rejects::<Cmd>(list_dir(json!(path), json!(mount.as_str())));
    }
    // Empty and `.` components are normalised away, not refused.
    let Cmd::ListDir { path, .. } =
        serde_json::from_value(list_dir(json!("a/./b//"), json!(mount.as_str()))).unwrap()
    else {
        unreachable!()
    };
    assert_eq!(path.as_str(), "a/b");
    rejects::<Cmd>(list_dir(json!("config"), json!("not a mount id")));
    rejects::<Cmd>(json!({ "type": "ListDir", "data": {
        "stack": "valheim", "mount": mount.as_str(), "path": "", "req": 1, "extra": true
    } }));

    let read_version = |version: &str| {
        json!({ "type": "ReadVersion", "data": {
            "stack": "valheim", "mount": mount.as_str(), "path": "a", "version": version, "req": 1
        } })
    };
    accepts::<Cmd>(read_version(version_id().as_str()));
    for version in [
        String::new(),
        "b".repeat(64),
        format!("{}-", "b".repeat(64)),
        format!("{}-x", "b".repeat(64)),
        format!("{}-1", "B".repeat(64)),
        format!("{}-1", "b".repeat(63)),
        format!("{}-{}", "b".repeat(64), "1".repeat(40)),
    ] {
        rejects::<Cmd>(read_version(&version));
    }

    let upload = |files: usize| {
        let files: Vec<_> = (0..files)
            .map(|i| json!({ "name": format!("f{i}"), "size": 1 }))
            .collect();
        json!({ "type": "PreviewFileOp", "data": { "stack": "valheim", "op": {
            "kind": "upload", "options": { "mount": mount.as_str(), "dir": "", "files": files }
        } } })
    };
    accepts::<Cmd>(upload(MAX_UPLOAD_FILES));
    rejects::<Cmd>(upload(MAX_UPLOAD_FILES + 1));
    // A file name is one component.
    rejects::<Cmd>(
        json!({ "type": "PreviewFileOp", "data": { "stack": "valheim", "op": {
        "kind": "upload", "options": { "mount": mount.as_str(), "dir": "", "files": [
            { "name": "a/b", "size": 1 }
        ] }
    } } }),
    );
}
