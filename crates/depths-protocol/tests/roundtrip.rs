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
        Cmd::Cancel { .. } => "Cancel",
        Cmd::OpenLogs { .. } => "OpenLogs",
        Cmd::OpenShell { .. } => "OpenShell",
        Cmd::ShellInput { .. } => "ShellInput",
        Cmd::ShellResize { .. } => "ShellResize",
        Cmd::WatchStats { .. } => "WatchStats",
        Cmd::UnwatchStats => "UnwatchStats",
        Cmd::CloseStream { .. } => "CloseStream",
        Cmd::GetAudit { .. } => "GetAudit",
        Cmd::GetUsers => "GetUsers",
        Cmd::PutUser { .. } => "PutUser",
        Cmd::RemoveUser { .. } => "RemoveUser",
    }
}

const CMD_VARIANTS: usize = 18;
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
            preview_id: preview_id()
        })
        .unwrap(),
        json!({ "type": "Confirm", "data": { "preview_id": "00000000-0000-0000-0000-000000000001" } })
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
            containers: vec![container()],
        }],
        running_services: 1,
        total_services: 2,
        modified_at: Some("2026-10-23T09:00:00Z".into()),
        read_error: None,
        read_only_reason: Some("stack directory is a symlink".into()),
        too_large: false,
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
        }),
        Event::StackDetail(StackDetail {
            stack: Stack {
                too_large: true,
                ..stack_summary()
            },
            compose_text: None,
            env: Some(EnvText::Plain("PORT=80\n".into())),
            content_hash: "b".repeat(64),
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
        }),
        Event::ValidationFailed {
            errors: vec![
                ValidationError {
                    line: Some(3),
                    message: "services.app.image must be a string".into(),
                },
                ValidationError {
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
        },
        Event::Busy {
            stack: "web".into(),
            operation_id: operation_id(),
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
                },
            ],
            next_cursor: Some("c2VnLTE".into()),
        },
        Event::Users(users.clone()),
        Event::UsersChanged(users),
        Event::SessionExpiring {
            at: "2026-10-23T17:00:00Z".into(),
        },
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
        Event::AuditPage { .. } => "AuditPage",
        Event::Users(_) => "Users",
        Event::UsersChanged(_) => "UsersChanged",
        Event::SessionExpiring { .. } => "SessionExpiring",
    }
}

const EVENT_VARIANTS: usize = 24;

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
    ] {
        roundtrip(&Event::Denied { reason });
    }
    for reason in [
        PreviewVoidReason::Unknown,
        PreviewVoidReason::Used,
        PreviewVoidReason::Expired,
        PreviewVoidReason::StackChanged,
    ] {
        roundtrip(&Event::PreviewVoid { reason });
    }
    for reason in [StreamCloseReason::Ended, StreamCloseReason::Closed] {
        roundtrip(&reason);
    }
}

#[test]
fn event_wire_shape_is_pinned() {
    assert_eq!(
        serde_json::to_value(EventEnvelope {
            reply_to: Some(RequestId(4)),
            event: Event::Denied {
                reason: DenyReason::NotPermitted
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
        },
    });
}
