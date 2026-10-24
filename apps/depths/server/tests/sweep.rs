//! T075 (SC-006): the permission sweep. Every HTTP route and every protocol command, as each kind
//! of person, against the permission table in `specs/050-depths-stack-manager/data-model.md`.
//!
//! The expectations below are written from that table, not from the server: [`expected`] is an
//! exhaustive match over [`Cmd`] (a new command does not compile until it has a row), and
//! [`every_command_has_a_sample`] checks the samples name every variant serde knows. The HTTP
//! side keeps [`ROUTES`] as a table and [`the_route_table_matches_the_router`] checks it against
//! the router's source.

mod support;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::body::Body;
use depths_protocol::{Cmd, LogTarget, Role, StackOp, StatsTarget, UserId};
use depths_server::limits::{Limits, MAX_KEYS, RateLimiter};
use futures_util::SinkExt as _;
use http::{Method, Request, StatusCode, header};
use serde_json::{Value, json};
use support::ws::*;
use support::{App, ORIGIN, OWNER, app_with, container, json as body_json, text};
use tokio_tungstenite::tungstenite::Message;

/// Signed in with stack `web` assigned (and `db` not).
const USER: &str = "7";
/// An admin who is not an owner: the role, not ownership, is what grants.
const ADMIN: &str = "9";
/// Signed in, never given access.
const PENDING: &str = "777";

fn uid(subject: &str) -> UserId {
    UserId::new(format!("discord:{subject}")).unwrap()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Features {
    shell: bool,
    edit: bool,
    /// `DEPTHS_HOST_PROC` set, to a fake `/proc`.
    host: bool,
}

const ON: Features = Features {
    shell: true,
    edit: true,
    host: true,
};
const OFF: Features = Features {
    shell: false,
    edit: false,
    host: false,
};

/// Stacks `web` and `db` (service `app`, each with a `.env`), container `stray-x-1` outside every
/// stack, `USER` given `web`, `ADMIN` made admin. Rate limits are raised: the sweep signs in far
/// more often than a person would.
fn sweep_app(f: Features) -> App {
    sweep_app_under(f, depths_core::security::ShellPolicy::Strict)
}

/// [`sweep_app`] under a shell policy (051).
fn sweep_app_under(f: Features, policy: depths_core::security::ShellPolicy) -> App {
    let a = app_with(|s| {
        let mut cfg = (*s.cfg).clone();
        cfg.shell_policy = policy;
        cfg.features.shell = f.shell;
        cfg.features.edit = f.edit;
        cfg.host_proc = f.host.then(|| support::fake_host_proc(&cfg));
        s.cfg = Arc::new(cfg);
        let big = 1_000_000;
        s.limits = Arc::new(Limits {
            auth: RateLimiter::new(big, big, MAX_KEYS),
            api: RateLimiter::new(big, big, MAX_KEYS),
            ws_upgrade: RateLimiter::new(big, big, MAX_KEYS),
        });
    });
    a.add_stack("web", COMPOSE, Some("TOKEN=x\n"));
    a.add_stack("db", COMPOSE, Some("TOKEN=x\n"));
    a.set_containers(vec![
        container("web", "app", "running", "Up 1 hour"),
        container("db", "app", "running", "Up 1 hour"),
        container("stray", "x", "running", "Up 1 hour"),
    ]);
    grant(&a, USER, &["web"]);
    a.state.users.put(&uid(ADMIN), Role::Admin, &[]).unwrap();
    a
}

// ---------------------------------------------------------------------------
// The protocol: what the table says
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Who {
    User,
    Admin,
}

impl Who {
    fn subject(self) -> &'static str {
        match self {
            Who::User => USER,
            Who::Admin => ADMIN,
        }
    }
    fn other(self) -> Who {
        match self {
            Who::User => Who::Admin,
            Who::Admin => Who::User,
        }
    }
    /// A stack this person may act on: the user's assigned one, or any for an admin.
    fn own_stack(self) -> &'static str {
        match self {
            Who::User => "web",
            Who::Admin => "db",
        }
    }
}

/// Whose preview, operation or stream a command names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Mine,
    Theirs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    /// A reply of this event type.
    Reply(&'static str),
    Denied(&'static str),
    Invalid,
    /// `PreviewVoid{unknown}`: existence is not disclosed.
    Void,
    /// Accepted with no reply (stream input, stats, cancel).
    Silent,
}

use Outcome::*;

const NOT_PERMITTED: &str = "not_permitted";
const FEATURE_DISABLED: &str = "feature_disabled";
const SHELL_ESCAPE: &str = "shell_escape";
const SHELL_ROOT: &str = "shell_root_admin_only";
const SHELL_UNVERIFIABLE: &str = "shell_unverifiable";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// Stack `web`, or a container in it.
    Assigned,
    /// Stack `db`, or a container in it.
    Unassigned,
    /// A container outside every stack (`stray-x-1`).
    Foreign,
    /// Nothing by that name.
    Missing,
}

fn str_of(v: &impl serde::Serialize) -> String {
    serde_json::to_value(v)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

fn stack_place(name: &impl serde::Serialize) -> Place {
    match str_of(name).as_str() {
        "web" => Place::Assigned,
        "db" => Place::Unassigned,
        _ => Place::Missing,
    }
}

fn container_place(name: &impl serde::Serialize) -> Place {
    match str_of(name).as_str() {
        "web-app-1" => Place::Assigned,
        "db-app-1" => Place::Unassigned,
        "stray-x-1" => Place::Foreign,
        _ => Place::Missing,
    }
}

/// The table's rule for anything aimed at a stack or a container: a user only inside their
/// stacks, and a user never learns whether something they may not see exists (`Denied`); an
/// admin anywhere, and told when it is not there (`Invalid`).
fn gate(who: Who, place: Place, ok: Outcome) -> Outcome {
    match (who, place) {
        (Who::Admin, Place::Missing) => Invalid,
        (Who::Admin, _) => ok,
        (Who::User, Place::Assigned) => ok,
        (Who::User, _) => Denied(NOT_PERMITTED),
    }
}

/// Destructive down and stack deletion are admin-only.
fn admin_only(op: &StackOp) -> bool {
    matches!(
        op,
        StackOp::Down {
            remove_volumes: true,
            ..
        } | StackOp::Down {
            remove_images: true,
            ..
        } | StackOp::Delete { .. }
    )
}

fn admin_or_denied(who: Who, ok: Outcome) -> Outcome {
    match who {
        Who::Admin => ok,
        Who::User => Denied(NOT_PERMITTED),
    }
}

/// The permission table as outcomes. No wildcard: a new command fails to compile here.
fn expected(cmd: &Cmd, who: Who, scope: Scope, f: Features) -> Outcome {
    match cmd {
        Cmd::ListStacks => Reply("Stacks"),
        Cmd::GetStack { name } => gate(who, stack_place(name), Reply("StackDetail")),
        // 051 FR-016: whoever may see the stack may lint a draft of it.
        // An admin may lint a stack that does not exist yet: the one they are creating.
        Cmd::LintCompose { stack, .. } => match (who, stack_place(stack)) {
            (Who::Admin, _) | (Who::User, Place::Assigned) => Reply("LintResult"),
            (Who::User, _) => Denied(NOT_PERMITTED),
        },
        // Admin-only, even on an assigned stack.
        Cmd::RevealEnv { name } => match who {
            Who::Admin => gate(who, stack_place(name), Reply("EnvRevealed")),
            Who::User => Denied(NOT_PERMITTED),
        },
        Cmd::PreviewOp { stack, op } => {
            if admin_only(op) && who == Who::User {
                Denied(NOT_PERMITTED)
            } else {
                gate(who, stack_place(stack), Reply("Preview"))
            }
        }
        Cmd::PreviewSave { draft, .. } => {
            if !f.edit {
                Denied(FEATURE_DISABLED)
            } else if draft.base_hash.is_none() {
                // Creating a stack.
                admin_or_denied(who, Reply("Preview"))
            } else {
                gate(who, stack_place(&draft.stack), Reply("Preview"))
            }
        }
        Cmd::Confirm { .. } => match scope {
            Scope::Mine => Reply("OpStarted"),
            Scope::Theirs => Void,
        },
        // Cancel is "actor or admin".
        Cmd::Cancel { .. } => match (scope, who) {
            (Scope::Mine, _) | (Scope::Theirs, Who::Admin) => Silent,
            (Scope::Theirs, Who::User) => Denied(NOT_PERMITTED),
        },
        Cmd::OpenLogs { target, .. } => match target {
            LogTarget::Stack { stack, .. } => gate(who, stack_place(stack), Reply("StreamOpened")),
            LogTarget::Container { container } => {
                gate(who, container_place(container), Reply("StreamOpened"))
            }
        },
        Cmd::OpenShell { container, .. } => {
            if f.shell {
                gate(who, container_place(container), Reply("StreamOpened"))
            } else {
                Denied(FEATURE_DISABLED)
            }
        }
        // Streams belong to their socket: another socket's id is just unknown here.
        Cmd::ShellInput { .. } | Cmd::ShellResize { .. } => match scope {
            Scope::Mine => Silent,
            Scope::Theirs => Invalid,
        },
        Cmd::CloseStream { .. } => match scope {
            Scope::Mine => Reply("StreamClosed"),
            Scope::Theirs => Invalid,
        },
        Cmd::WatchStats { targets } => {
            assert_eq!(targets.len(), 1, "one target per sample");
            match &targets[0] {
                StatsTarget::Stack { stack } => gate(who, stack_place(stack), Silent),
                StatsTarget::Container { container } => {
                    gate(who, container_place(container), Silent)
                }
            }
        }
        Cmd::UnwatchStats => Silent,
        // Admin only, and the permission is checked before the setting.
        Cmd::WatchHost => match who {
            Who::User => Denied(NOT_PERMITTED),
            Who::Admin if f.host => Silent,
            Who::Admin => Denied(FEATURE_DISABLED),
        },
        Cmd::UnwatchHost => Silent,
        Cmd::GetAudit { .. } => admin_or_denied(who, Reply("AuditPage")),
        Cmd::GetUsers => admin_or_denied(who, Reply("Users")),
        Cmd::PutUser { id, .. } | Cmd::RemoveUser { id } => match who {
            Who::User => Denied(NOT_PERMITTED),
            Who::Admin if str_of(id) == format!("discord:{OWNER}") => Denied("owner"),
            Who::Admin => Reply("Users"),
        },
    }
}

// ---------------------------------------------------------------------------
// The protocol: samples
// ---------------------------------------------------------------------------

/// What a sample needs set up first; its result replaces the `"$ID"` in the command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prep {
    None,
    /// The stack's current content hash.
    Hash(&'static str),
    /// A preview of `restart`.
    Preview(Scope),
    /// A running operation.
    Op(Scope),
    /// An open shell.
    Shell(Scope),
}

impl Prep {
    fn scope(self) -> Scope {
        match self {
            Prep::Preview(s) | Prep::Op(s) | Prep::Shell(s) => s,
            Prep::None | Prep::Hash(_) => Scope::Mine,
        }
    }
}

fn cmd(ty: &str, data: Value) -> Value {
    json!({ "type": ty, "data": data })
}

fn bare(ty: &str) -> Value {
    json!({ "type": ty })
}

fn samples() -> Vec<(Prep, Value)> {
    let mut v = vec![(Prep::None, bare("ListStacks"))];
    for s in ["web", "db", "ghost"] {
        v.push((Prep::None, cmd("GetStack", json!({ "name": s }))));
        v.push((
            Prep::None,
            cmd(
                "LintCompose",
                json!({ "stack": s, "compose_text": "services:\n  app:\n    image: busybox\n" }),
            ),
        ));
        v.push((Prep::None, cmd("RevealEnv", json!({ "name": s }))));
        v.push((
            Prep::None,
            cmd(
                "PreviewOp",
                json!({ "stack": s, "op": { "kind": "restart" } }),
            ),
        ));
        v.push((
            Prep::None,
            cmd(
                "OpenLogs",
                json!({ "target": { "kind": "stack", "stack": s }, "tail": 10 }),
            ),
        ));
    }
    for op in [
        json!({ "kind": "down", "options": {} }),
        json!({ "kind": "down", "options": { "remove_volumes": true } }),
        json!({ "kind": "down", "options": { "remove_images": true } }),
        json!({ "kind": "delete", "options": {} }),
    ] {
        v.push((
            Prep::None,
            cmd("PreviewOp", json!({ "stack": "web", "op": op })),
        ));
    }
    v.push((
        Prep::None,
        cmd(
            "PreviewOp",
            json!({ "stack": "db", "op": { "kind": "delete", "options": {} } }),
        ),
    ));
    for s in ["web", "db"] {
        v.push((
            Prep::Hash(s),
            cmd(
                "PreviewSave",
                json!({ "draft": { "stack": s, "base_hash": "$ID", "compose": COMPOSE.replace("nginx", "caddy") }, "deploy": false }),
            ),
        ));
    }
    v.push((
        Prep::None,
        cmd(
            "PreviewSave",
            json!({ "draft": { "stack": "fresh", "compose": COMPOSE }, "deploy": false }),
        ),
    ));
    for c in ["web-app-1", "db-app-1", "stray-x-1", "ghost-1"] {
        v.push((
            Prep::None,
            cmd(
                "OpenLogs",
                json!({ "target": { "kind": "container", "container": c }, "tail": 10 }),
            ),
        ));
        v.push((
            Prep::None,
            cmd(
                "OpenShell",
                json!({ "container": c, "cols": 80, "rows": 24 }),
            ),
        ));
        v.push((
            Prep::None,
            cmd(
                "WatchStats",
                json!({ "targets": [{ "kind": "container", "container": c }] }),
            ),
        ));
    }
    for s in ["web", "db"] {
        v.push((
            Prep::None,
            cmd(
                "WatchStats",
                json!({ "targets": [{ "kind": "stack", "stack": s }] }),
            ),
        ));
    }
    v.push((Prep::None, bare("UnwatchStats")));
    v.push((Prep::None, bare("WatchHost")));
    v.push((Prep::None, bare("UnwatchHost")));
    v.push((Prep::None, cmd("GetAudit", json!({ "filter": {} }))));
    v.push((Prep::None, bare("GetUsers")));
    v.push((
        Prep::None,
        cmd(
            "PutUser",
            json!({ "id": "discord:555", "role": "user", "stacks": ["web"] }),
        ),
    ));
    v.push((
        Prep::None,
        cmd(
            "PutUser",
            json!({ "id": format!("discord:{OWNER}"), "role": "user", "stacks": [] }),
        ),
    ));
    v.push((
        Prep::None,
        cmd("RemoveUser", json!({ "id": format!("discord:{USER}") })),
    ));
    v.push((
        Prep::None,
        cmd("RemoveUser", json!({ "id": format!("discord:{OWNER}") })),
    ));
    for scope in [Scope::Mine, Scope::Theirs] {
        v.push((
            Prep::Preview(scope),
            cmd("Confirm", json!({ "preview_id": "$ID" })),
        ));
        v.push((
            Prep::Op(scope),
            cmd("Cancel", json!({ "operation_id": "$ID" })),
        ));
        v.push((
            Prep::Shell(scope),
            cmd(
                "ShellInput",
                json!({ "stream_id": "$ID", "bytes_b64": b64("ls\n") }),
            ),
        ));
        v.push((
            Prep::Shell(scope),
            cmd(
                "ShellResize",
                json!({ "stream_id": "$ID", "cols": 100, "rows": 30 }),
            ),
        ));
        v.push((
            Prep::Shell(scope),
            cmd("CloseStream", json!({ "stream_id": "$ID" })),
        ));
    }
    v
}

/// `value` with every `"$ID"` replaced by `id`.
fn fill(value: &Value, id: &Value) -> Value {
    match value {
        Value::String(s) if s == "$ID" => id.clone(),
        Value::Array(a) => Value::Array(a.iter().map(|x| fill(x, id)).collect()),
        Value::Object(o) => {
            Value::Object(o.iter().map(|(k, x)| (k.clone(), fill(x, id))).collect())
        }
        other => other.clone(),
    }
}

/// A placeholder for commands sent where nothing could be set up (signed out, no access).
fn dummy(prep: Prep) -> Value {
    match prep {
        Prep::None => Value::Null,
        Prep::Hash(_) => json!("0".repeat(64)),
        Prep::Preview(_) | Prep::Op(_) => json!("00000000-0000-4000-8000-000000000000"),
        Prep::Shell(_) => json!(1),
    }
}

/// The variants serde accepts, from its own "unknown variant" message.
fn declared_variants() -> BTreeSet<String> {
    let err = serde_json::from_value::<Cmd>(json!({ "type": "NoSuchCommand" }))
        .unwrap_err()
        .to_string();
    let list = err
        .split("expected one of")
        .nth(1)
        .unwrap_or_else(|| panic!("unexpected serde message: {err}"));
    list.split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

#[test]
fn every_command_has_a_sample() {
    let declared = declared_variants();
    assert_eq!(declared.len(), 21, "{declared:?}");
    let sampled: BTreeSet<String> = samples()
        .iter()
        .map(|(_, c)| c["type"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(sampled, declared);
}

// ---------------------------------------------------------------------------
// The protocol: running a sample
// ---------------------------------------------------------------------------

/// Never a real stream: the reply to it marks the end of the replies to what came before.
const SENTINEL_STREAM: u64 = 4_000_000_000;
const SENTINEL_ID: u64 = 999_999;

/// Every reply to request `id`. The socket answers a command before reading the next frame, so
/// the sentinel's reply arrives after all of them, and "no reply" is a definite answer.
async fn replies(ws: &mut Socket, id: u64, cmd: &Value) -> Vec<Value> {
    ws.send(Message::text(json!({ "id": id, "cmd": cmd }).to_string()))
        .await
        .unwrap();
    let sentinel = json!({ "type": "CloseStream", "data": { "stream_id": SENTINEL_STREAM } });
    ws.send(Message::text(
        json!({ "id": SENTINEL_ID, "cmd": sentinel }).to_string(),
    ))
    .await
    .unwrap();
    let mut out = Vec::new();
    loop {
        let e = next(ws).await;
        if e["reply_to"] == SENTINEL_ID {
            assert_eq!(e["event"]["type"], "Invalid", "sentinel: {e}");
            return out;
        }
        if e["reply_to"] == id {
            out.push(e);
        }
    }
}

fn observe(replies: &[Value]) -> Result<Outcome, String> {
    match replies {
        [] => Ok(Silent),
        [e] => {
            let ty = e["event"]["type"].as_str().unwrap_or_default();
            let d = &e["event"]["data"];
            Ok(match ty {
                "Denied" => match d["reason"].as_str() {
                    Some(NOT_PERMITTED) => Denied(NOT_PERMITTED),
                    Some(FEATURE_DISABLED) => Denied(FEATURE_DISABLED),
                    Some("owner") => Denied("owner"),
                    Some(SHELL_ESCAPE) => Denied(SHELL_ESCAPE),
                    Some(SHELL_ROOT) => Denied(SHELL_ROOT),
                    Some(SHELL_UNVERIFIABLE) => Denied(SHELL_UNVERIFIABLE),
                    _ => return Err(format!("{e}")),
                },
                "Invalid" => Invalid,
                "PreviewVoid" if d["reason"] == "unknown" => Void,
                "Stacks" | "StackDetail" | "EnvRevealed" | "Preview" | "OpStarted"
                | "StreamOpened" | "StreamClosed" | "AuditPage" | "Users" | "LintResult" => {
                    Reply(match ty {
                        "Stacks" => "Stacks",
                        "StackDetail" => "StackDetail",
                        "EnvRevealed" => "EnvRevealed",
                        "Preview" => "Preview",
                        "OpStarted" => "OpStarted",
                        "StreamOpened" => "StreamOpened",
                        "StreamClosed" => "StreamClosed",
                        "AuditPage" => "AuditPage",
                        "LintResult" => "LintResult",
                        _ => "Users",
                    })
                }
                _ => return Err(format!("{e}")),
            })
        }
        many => Err(format!("{} replies: {many:?}", many.len())),
    }
}

/// Live sockets kept open for one sample; their servers stop on drop.
#[derive(Default)]
struct Held {
    sockets: Vec<Socket>,
    servers: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for Held {
    fn drop(&mut self) {
        for s in &self.servers {
            s.abort();
        }
    }
}

/// Opens a socket as `subject`, keeping its server in `held`.
async fn socket(a: &App, subject: &str, held: &mut Held) -> Socket {
    let (ws, server, _) = open(a, subject).await;
    held.servers.push(server);
    ws
}

/// Runs `prep` with `me` as the actor (the `Theirs` cases act as the other person, on another
/// socket kept in `held`); returns what replaces `"$ID"`.
async fn prepare(a: &App, me: &mut Socket, who: Who, prep: Prep, held: &mut Held) -> Value {
    let restart = |stack: &str| {
        cmd(
            "PreviewOp",
            json!({ "stack": stack, "op": { "kind": "restart" } }),
        )
    };
    let shell = |c: &str| {
        cmd(
            "OpenShell",
            json!({ "container": c, "cols": 80, "rows": 24 }),
        )
    };
    match prep {
        Prep::None => Value::Null,
        Prep::Hash(stack) => {
            let mut probe = socket(a, OWNER, held).await;
            let d = ask(&mut probe, 1, cmd("GetStack", json!({ "name": stack }))).await;
            d["event"]["data"]["content_hash"].clone()
        }
        Prep::Preview(scope) | Prep::Op(scope) => {
            let (actor, stack) = match scope {
                Scope::Mine => (who, who.own_stack()),
                Scope::Theirs => (who.other(), who.other().own_stack()),
            };
            let mut theirs = match scope {
                Scope::Mine => None,
                Scope::Theirs => Some(socket(a, actor.subject(), held).await),
            };
            let ws = theirs.as_mut().unwrap_or(me);
            let p = ask(ws, 101, restart(stack)).await;
            assert_eq!(p["event"]["type"], "Preview", "prep: {p}");
            let preview = p["event"]["data"]["preview_id"].clone();
            let id = if matches!(prep, Prep::Op(_)) {
                a.runtime.hold.store(true, Ordering::SeqCst);
                let s = ask(ws, 102, cmd("Confirm", json!({ "preview_id": preview }))).await;
                assert_eq!(s["event"]["type"], "OpStarted", "prep: {s}");
                s["event"]["data"]["operation_id"].clone()
            } else {
                preview
            };
            held.sockets.extend(theirs);
            id
        }
        Prep::Shell(scope) => {
            let c = match scope {
                Scope::Mine => format!("{}-app-1", who.own_stack()),
                Scope::Theirs => format!("{}-app-1", who.other().own_stack()),
            };
            let mut theirs = match scope {
                Scope::Mine => None,
                Scope::Theirs => Some(socket(a, who.other().subject(), held).await),
            };
            let ws = theirs.as_mut().unwrap_or(me);
            let r = ask(ws, 103, shell(&c)).await;
            assert_eq!(r["event"]["type"], "StreamOpened", "prep: {r}");
            let id = r["event"]["data"]["stream_id"].clone();
            held.sockets.extend(theirs);
            id
        }
    }
}

/// Runs every sample as `who` on a fresh app each; returns the mismatches.
async fn sweep(who: Who, f: Features) -> Vec<String> {
    let mut wrong = Vec::new();
    for (prep, template) in samples() {
        if !f.shell && matches!(prep, Prep::Shell(_)) {
            // No shell can be open to aim these at; `OpenShell` itself is swept.
            continue;
        }
        let a = sweep_app(f);
        let mut held = Held::default();
        let mut ws = socket(&a, who.subject(), &mut held).await;
        let id = prepare(&a, &mut ws, who, prep, &mut held).await;
        let sent = fill(&template, &id);
        let typed: Cmd = serde_json::from_value(sent.clone())
            .unwrap_or_else(|e| panic!("sample does not parse: {sent}: {e}"));
        let want = expected(&typed, who, prep.scope(), f);
        let got = observe(&replies(&mut ws, 1, &sent).await);
        if got.as_ref() != Ok(&want) {
            wrong.push(format!(
                "{who:?} {:?} {sent}: want {want:?}, got {got:?}",
                prep.scope()
            ));
        }
        a.runtime.hold.store(false, Ordering::SeqCst);
    }
    wrong
}

#[tokio::test]
async fn every_command_as_a_user_matches_the_table() {
    let wrong = sweep(Who::User, ON).await;
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}

#[tokio::test]
async fn every_command_as_an_admin_matches_the_table() {
    let wrong = sweep(Who::Admin, ON).await;
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}

#[tokio::test]
async fn every_command_with_shell_and_edit_off_matches_the_table() {
    let mut wrong = sweep(Who::User, OFF).await;
    wrong.extend(sweep(Who::Admin, OFF).await);
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}

// ---------------------------------------------------------------------------
// 051: the container gate on OpenShell, every policy x role x place x level
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Ok,
    Root,
    Escape,
    /// The inspect at the moment of the request fails.
    Unknown,
}

/// 051 FR-008 to FR-010, written from `contracts/protocol.md` rather than the server: the role
/// table first, then the container's level under the policy.
fn shell_expected(
    who: Who,
    place: Place,
    level: Level,
    policy: depths_core::security::ShellPolicy,
) -> Outcome {
    use depths_core::security::ShellPolicy::{Relaxed, Strict};
    let role = gate(who, place, Reply("StreamOpened"));
    if role != Reply("StreamOpened") {
        return role;
    }
    match (policy, level, who) {
        (_, Level::Unknown, _) => Denied(SHELL_UNVERIFIABLE),
        (Strict, Level::Escape, _) | (Relaxed, Level::Escape, Who::User) => Denied(SHELL_ESCAPE),
        (Strict, Level::Root, Who::User) => Denied(SHELL_ROOT),
        _ => Reply("StreamOpened"),
    }
}

#[tokio::test]
async fn every_shell_matches_the_policy_and_level_table() {
    use depths_core::security::ShellPolicy::{Relaxed, Strict};
    let mut wrong = Vec::new();
    for policy in [Strict, Relaxed] {
        for level in [Level::Ok, Level::Root, Level::Escape, Level::Unknown] {
            let a = sweep_app_under(ON, policy);
            let inspect = match level {
                Level::Ok | Level::Unknown => json!({ "Config": { "User": "1000" } }),
                Level::Root => json!({ "Config": { "User": "0" } }),
                // A runtime socket is an escape even read-only (051 FR-004).
                Level::Escape => json!({
                    "Config": { "User": "1000" },
                    "Mounts": [{ "Type": "bind", "Source": "/var/run/docker.sock",
                        "Destination": "/var/run/docker.sock", "RW": false }],
                }),
            };
            for id in ["web-app-id", "db-app-id", "stray-x-id"] {
                a.set_inspect(id, inspect.clone());
            }
            a.runtime
                .inspect_fails
                .store(level == Level::Unknown, Ordering::SeqCst);
            for who in [Who::User, Who::Admin] {
                let mut held = Held::default();
                let mut ws = socket(&a, who.subject(), &mut held).await;
                for (n, c) in ["web-app-1", "db-app-1", "stray-x-1", "nope-1"]
                    .into_iter()
                    .enumerate()
                {
                    let sent = cmd(
                        "OpenShell",
                        json!({ "container": c, "cols": 80, "rows": 24 }),
                    );
                    let want = shell_expected(who, container_place(&c), level, policy);
                    let got = observe(&replies(&mut ws, n as u64 + 1, &sent).await);
                    if got.as_ref() != Ok(&want) {
                        wrong.push(format!(
                            "{policy:?} {level:?} {who:?} {c}: want {want:?}, got {got:?}"
                        ));
                    }
                }
            }
        }
    }
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}

/// What the runtime has been asked to do, to show a refused command reached none of it.
fn runtime_calls(a: &App) -> (usize, usize, usize, usize) {
    let r = &a.runtime;
    (
        r.ran.lock().unwrap().len(),
        r.follows.lock().unwrap().len(),
        r.shells.lock().unwrap().len(),
        r.stats_calls.lock().unwrap().len(),
    )
}

/// The command is never answered and the socket closes with `code`.
async fn closes_unanswered(ws: &mut Socket, code: u16, what: &str) {
    use futures_util::StreamExt as _;
    loop {
        match tokio::time::timeout(WAIT, ws.next()).await {
            Ok(Some(Ok(Message::Close(Some(f))))) => {
                assert_eq!(u16::from(f.code), code, "{what}");
                return;
            }
            Ok(Some(Ok(Message::Text(t)))) => {
                let e: Value = serde_json::from_str(t.as_str()).unwrap();
                assert!(
                    e["reply_to"].is_null(),
                    "{what}: answered before closing: {e}"
                );
            }
            Ok(Some(Ok(_))) => {}
            other => panic!("{what}: no close frame: {other:?}"),
        }
    }
}

#[tokio::test]
async fn signed_out_every_command_closes_4401_unanswered() {
    let a = sweep_app(ON);
    for (prep, template) in samples() {
        let sent = fill(&template, &dummy(prep));
        for cookie in [None, Some("__Host-depths_session=forged")] {
            let (mut ws, server) = connect(&a, cookie).await;
            // The server may already have closed; what matters is that nothing answers.
            let _ = ws
                .send(Message::text(json!({ "id": 1, "cmd": sent }).to_string()))
                .await;
            closes_unanswered(&mut ws, 4401, &format!("{cookie:?} {sent}")).await;
            server.abort();
        }
    }
    assert_eq!(runtime_calls(&a), (0, 0, 0, 0));
}

#[tokio::test]
async fn a_pending_person_is_closed_4403_after_the_hello() {
    let a = sweep_app(ON);
    for (prep, template) in samples() {
        let sent = fill(&template, &dummy(prep));
        let (mut ws, server, _) = open(&a, PENDING).await;
        let _ = ws
            .send(Message::text(json!({ "id": 1, "cmd": sent }).to_string()))
            .await;
        closes_unanswered(&mut ws, 4403, &format!("{sent}")).await;
        server.abort();
    }
    assert_eq!(runtime_calls(&a), (0, 0, 0, 0));
}

/// Access removed while a socket is open (no `users_changed` to warn it): the next command,
/// whatever it is, closes `4403` with no reply and reaches nothing.
#[tokio::test]
async fn losing_access_mid_socket_closes_4403_on_any_command() {
    for (prep, template) in samples() {
        let a = sweep_app(ON);
        let mut held = Held::default();
        let mut ws = socket(&a, USER, &mut held).await;
        // Set up as the user while they still had access (their preview, op, shell).
        let id = match prep {
            Prep::Preview(_) | Prep::Op(_) | Prep::Shell(_) => {
                prepare(&a, &mut ws, Who::User, prep, &mut held).await
            }
            Prep::None | Prep::Hash(_) => dummy(prep),
        };
        let sent = fill(&template, &id);
        let before = runtime_calls(&a);
        a.state.users.remove(&uid(USER)).unwrap();
        ws.send(Message::text(json!({ "id": 1, "cmd": sent }).to_string()))
            .await
            .unwrap();
        closes_unanswered(&mut ws, 4403, &format!("{sent}")).await;
        assert_eq!(runtime_calls(&a), before, "{sent}");
        a.runtime.hold.store(false, Ordering::SeqCst);
    }
}

/// Revocation while streams are open: every stream on the stack closes as `failed`, and the
/// stats watch drops the stack's containers.
#[tokio::test]
async fn revoked_access_closes_every_stream_as_failed() {
    let a = sweep_app(ON);
    grant(&a, USER, &["web", "db"]);
    let mut held = Held::default();
    let mut ws = socket(&a, USER, &mut held).await;
    let logs = ask(
        &mut ws,
        1,
        cmd(
            "OpenLogs",
            json!({ "target": { "kind": "stack", "stack": "web" }, "tail": 10 }),
        ),
    )
    .await;
    let logs = logs["event"]["data"]["stream_id"].clone();
    let sh = ask(
        &mut ws,
        2,
        cmd(
            "OpenShell",
            json!({ "container": "web-app-1", "cols": 80, "rows": 24 }),
        ),
    )
    .await;
    let sh = sh["event"]["data"]["stream_id"].clone();
    assert!(logs.is_u64() && sh.is_u64(), "{logs} {sh}");
    let kept = ask(
        &mut ws,
        3,
        cmd(
            "OpenLogs",
            json!({ "target": { "kind": "stack", "stack": "db" }, "tail": 10 }),
        ),
    )
    .await;
    let kept = kept["event"]["data"]["stream_id"].clone();
    let w = replies(
        &mut ws,
        4,
        &cmd(
            "WatchStats",
            json!({ "targets": [{ "kind": "stack", "stack": "web" }, { "kind": "stack", "stack": "db" }] }),
        ),
    )
    .await;
    assert!(w.is_empty(), "{w:?}");

    grant(&a, USER, &["db"]);
    let _ = a.state.users_changed.send(());
    let mut closed = BTreeSet::new();
    while closed.len() < 2 {
        let e = next_of(&mut ws, "StreamClosed").await;
        assert_eq!(e["event"]["data"]["reason"]["kind"], "failed", "{e}");
        closed.insert(e["event"]["data"]["stream_id"].to_string());
    }
    let want: BTreeSet<String> = [logs.to_string(), sh.to_string()].into();
    assert_eq!(closed, want);
    // The stream on the stack still granted stays open.
    let r = replies(
        &mut ws,
        5,
        &cmd("CloseStream", json!({ "stream_id": kept })),
    )
    .await;
    assert_eq!(r.len(), 1, "{r:?}");
    assert_eq!(r[0]["event"]["type"], "StreamClosed", "{r:?}");
    assert_eq!(r[0]["event"]["data"]["reason"]["kind"], "closed", "{r:?}");
    until("a stats reading without web's container", || {
        a.runtime
            .stats_calls
            .lock()
            .unwrap()
            .last()
            .is_some_and(|ids| ids == &["db-app-id".to_owned()])
    })
    .await;
}

/// A preview made while permitted cannot be confirmed after the stack is taken away.
#[tokio::test]
async fn confirm_rechecks_permission() {
    let a = sweep_app(ON);
    let mut held = Held::default();
    let mut ws = socket(&a, USER, &mut held).await;
    let p = ask(
        &mut ws,
        1,
        cmd(
            "PreviewOp",
            json!({ "stack": "web", "op": { "kind": "restart" } }),
        ),
    )
    .await;
    let preview = p["event"]["data"]["preview_id"].clone();
    grant(&a, USER, &["db"]);
    let r = replies(
        &mut ws,
        2,
        &cmd("Confirm", json!({ "preview_id": preview })),
    )
    .await;
    assert_eq!(observe(&r), Ok(Denied(NOT_PERMITTED)), "{r:?}");
    assert!(a.runtime.ran.lock().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Visitor {
    SignedOut,
    Pending,
    User,
    Admin,
}

impl Visitor {
    fn subject(self) -> Option<&'static str> {
        match self {
            Visitor::SignedOut => None,
            Visitor::Pending => Some(PENDING),
            Visitor::User => Some(USER),
            Visitor::Admin => Some(ADMIN),
        }
    }
}

const VISITORS: [Visitor; 4] = [
    Visitor::SignedOut,
    Visitor::Pending,
    Visitor::User,
    Visitor::Admin,
];

/// One router entry. `kind` is the routing function as written in `http.rs` (`get`, `post`,
/// `any`), or `nest` / `fallback` for the static services; `probe` is a request path for it.
struct Route {
    kind: &'static str,
    path: &'static str,
    probe: &'static str,
}

const FALLBACK: &str = "<fallback>";

const ROUTES: &[Route] = &[
    Route {
        kind: "get",
        path: "/healthz",
        probe: "/healthz",
    },
    Route {
        kind: "get",
        path: "/readyz",
        probe: "/readyz",
    },
    Route {
        kind: "get",
        path: "/auth/login",
        probe: "/auth/login?provider=discord",
    },
    Route {
        kind: "get",
        path: "/auth/login/{provider}",
        probe: "/auth/login/discord",
    },
    Route {
        kind: "get",
        path: "/auth/callback/{provider}",
        probe: "/auth/callback/discord?code=7&state=forged",
    },
    Route {
        kind: "get",
        path: "/api/v1/me",
        probe: "/api/v1/me",
    },
    Route {
        kind: "get",
        path: "/api/v1/audit/export",
        probe: "/api/v1/audit/export",
    },
    Route {
        kind: "post",
        path: "/auth/logout",
        probe: "/auth/logout",
    },
    Route {
        kind: "post",
        path: "/auth/refresh",
        probe: "/auth/refresh",
    },
    Route {
        kind: "get",
        path: "/api/v1/ws",
        probe: "/api/v1/ws",
    },
    Route {
        kind: "any",
        path: "/api",
        probe: "/api",
    },
    Route {
        kind: "any",
        path: "/api/{*rest}",
        probe: "/api/v1/stacks",
    },
    Route {
        kind: "any",
        path: "/auth",
        probe: "/auth",
    },
    Route {
        kind: "any",
        path: "/auth/{*rest}",
        probe: "/auth/admin",
    },
    Route {
        kind: "nest",
        path: "/assets",
        probe: "/assets/app-abc.js",
    },
    Route {
        kind: "fallback",
        path: FALLBACK,
        probe: "/stacks/web",
    },
];

/// The table's status for `visitor` on `route`'s own method.
fn http_expected(route: &Route, v: Visitor) -> u16 {
    let signed_in = v != Visitor::SignedOut;
    match route.path {
        "/healthz" | "/readyz" => 200,
        "/auth/login" | "/auth/login/{provider}" => 303,
        // No login cookie for this state: refused, whoever asks.
        "/auth/callback/{provider}" => 400,
        "/api/v1/me" | "/auth/logout" | "/auth/refresh" => {
            if signed_in {
                200
            } else {
                401
            }
        }
        "/api/v1/audit/export" => match v {
            Visitor::SignedOut => 401,
            Visitor::Pending | Visitor::User => 403,
            Visitor::Admin => 200,
        },
        // A plain GET is not an upgrade; who you are is checked after the Hello.
        "/api/v1/ws" => 426,
        "/api" | "/api/{*rest}" | "/auth" | "/auth/{*rest}" => 404,
        "/assets" | FALLBACK => 200,
        other => panic!("no expectation for {other}"),
    }
}

/// A signed-in visitor's session cookie and CSRF token.
struct Visit {
    cookie: Option<String>,
    csrf: Option<String>,
}

async fn visit(a: &App, v: Visitor) -> Visit {
    match v.subject() {
        None => Visit {
            cookie: None,
            csrf: None,
        },
        Some(s) => {
            let cookie = a.sign_in(s).await;
            let csrf = csrf_of(a, &cookie).await;
            Visit {
                cookie: Some(cookie),
                csrf: Some(csrf),
            }
        }
    }
}

async fn call(
    a: &App,
    method: &Method,
    uri: &str,
    cookie: Option<&str>,
    headers: &[(&str, &str)],
) -> http::Response<Body> {
    let mut b = Request::builder().method(method.clone()).uri(uri);
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    a.send(b.body(Body::empty()).unwrap()).await
}

#[tokio::test]
async fn every_route_as_every_visitor_matches_the_table() {
    let mut wrong = Vec::new();
    for route in ROUTES {
        for v in VISITORS {
            // Fresh each time: logout ends the session it is given.
            let a = sweep_app(ON);
            let s = visit(&a, v).await;
            let method = if route.kind == "post" {
                Method::POST
            } else {
                Method::GET
            };
            let mut h = vec![("origin", ORIGIN)];
            if let Some(t) = &s.csrf {
                h.push(("x-csrf-token", t.as_str()));
            }
            let r = call(&a, &method, route.probe, s.cookie.as_deref(), &h).await;
            let got = r.status().as_u16();
            let want = http_expected(route, v);
            if got != want {
                wrong.push(format!(
                    "{v:?} {method} {}: want {want}, got {got}",
                    route.probe
                ));
                continue;
            }
            if route.path == "/api/v1/me" && got == 200 {
                let me = body_json(r).await;
                let (role, stacks) = match v {
                    Visitor::Pending => ("pending", json!([])),
                    Visitor::User => ("user", json!(["web"])),
                    _ => ("admin", json!([])),
                };
                assert_eq!(me["role"], role, "{v:?}: {me}");
                assert_eq!(me["stacks"], stacks, "{v:?}: {me}");
            } else if route.path == "/api/v1/audit/export" && got == 403 {
                assert_eq!(text(r).await, r#"{"error":"forbidden"}"#, "{v:?}");
            }
        }
    }
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}

#[tokio::test]
async fn the_ws_upgrade_refuses_a_foreign_or_missing_origin_for_everyone() {
    let a = sweep_app(ON);
    for v in VISITORS {
        let s = visit(&a, v).await;
        for origin in [None, Some("https://evil.example")] {
            let h: Vec<(&str, &str)> = origin.map(|o| ("origin", o)).into_iter().collect();
            let r = call(&a, &Method::GET, "/api/v1/ws", s.cookie.as_deref(), &h).await;
            assert_eq!(r.status(), StatusCode::FORBIDDEN, "{v:?} {origin:?}");
        }
    }
}

/// Every route, every unsafe method: no session is a 401, and a session without the right
/// token, Origin and fetch site is `403 {"error":"csrf"}`, admin included, and changes nothing.
/// With everything right, a route that only reads still never succeeds.
#[tokio::test]
async fn every_mutation_needs_the_csrf_token_and_origin() {
    let a = sweep_app(ON);
    let mut visits = Vec::new();
    for v in [Visitor::Pending, Visitor::User, Visitor::Admin] {
        visits.push((v, visit(&a, v).await));
    }
    let mut wrong = Vec::new();
    for route in ROUTES {
        for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            let uri = route.probe;
            let r = call(&a, &method, uri, None, &[("origin", ORIGIN)]).await;
            if r.status() != StatusCode::UNAUTHORIZED {
                wrong.push(format!("SignedOut {method} {uri}: {}", r.status()));
            }
            for (v, s) in &visits {
                let cookie = s.cookie.as_deref();
                let token = s.csrf.as_deref().unwrap();
                let refused: [(&str, Vec<(&str, &str)>); 5] = [
                    ("no token", vec![("origin", ORIGIN)]),
                    (
                        "wrong token",
                        vec![("origin", ORIGIN), ("x-csrf-token", "d3JvbmctdG9rZW4")],
                    ),
                    ("no origin", vec![("x-csrf-token", token)]),
                    (
                        "foreign origin",
                        vec![("origin", "https://evil.example"), ("x-csrf-token", token)],
                    ),
                    (
                        "cross-site fetch",
                        vec![
                            ("origin", ORIGIN),
                            ("sec-fetch-site", "cross-site"),
                            ("x-csrf-token", token),
                        ],
                    ),
                ];
                for (why, h) in refused {
                    let r = call(&a, &method, uri, cookie, &h).await;
                    let status = r.status();
                    let body = text(r).await;
                    if status != StatusCode::FORBIDDEN || body != r#"{"error":"csrf"}"# {
                        wrong.push(format!("{v:?} {method} {uri} {why}: {status} {body}"));
                    }
                }
            }
            // Admin, everything right.
            let (_, s) = &visits[2];
            let own = route.kind == "post" && method == Method::POST;
            if !own {
                let token = s.csrf.as_deref().unwrap();
                let h = [("origin", ORIGIN), ("x-csrf-token", token)];
                let r = call(&a, &method, uri, s.cookie.as_deref(), &h).await;
                let st = r.status().as_u16();
                if st != 404 && st != 405 {
                    wrong.push(format!("Admin {method} {uri} with a valid token: {st}"));
                }
            }
        }
    }
    // Nothing above ended a session.
    for (v, s) in &visits {
        let r = a.get("/api/v1/me", s.cookie.as_deref()).await;
        assert_eq!(r.status(), StatusCode::OK, "{v:?}'s session survived");
    }
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}

/// The router's own entries, read from its source: `.route("<path>", <kind>(`,
/// `.nest_service("<path>"` and `.fallback_service(`.
fn router_entries() -> BTreeSet<(String, String)> {
    let src = include_str!("../src/http.rs");
    let src = src.split("#[cfg(test)]").next().unwrap();
    let mut out = BTreeSet::new();
    for (marker, kind) in [(".route(\"", None), (".nest_service(\"", Some("nest"))] {
        for chunk in src.split(marker).skip(1) {
            let (path, rest) = chunk.split_once('"').unwrap();
            let kind = match kind {
                Some(k) => k.to_owned(),
                None => rest
                    .trim_start_matches(',')
                    .trim_start()
                    .split('(')
                    .next()
                    .unwrap()
                    .to_owned(),
            };
            out.insert((kind, path.to_owned()));
        }
    }
    if src.contains(".fallback_service(") {
        out.insert(("fallback".to_owned(), FALLBACK.to_owned()));
    }
    out
}

#[test]
fn the_route_table_matches_the_router() {
    let table: BTreeSet<(String, String)> = ROUTES
        .iter()
        .map(|r| (r.kind.to_owned(), r.path.to_owned()))
        .collect();
    assert_eq!(table.len(), ROUTES.len(), "duplicate rows");
    assert_eq!(router_entries(), table);
}
