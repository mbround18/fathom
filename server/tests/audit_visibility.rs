//! Hosted audit attribution and visibility (spec 031, T049/T050): a mutation made through a
//! user's backend is recorded in the shared hosted audit log naming the real signed-in user
//! (also in impersonation mode, where the API server only ever sees the service account), a
//! refused mutation is recorded as a failure (spec 026 outcome rules), an ordinary user sees
//! only their own entries, members of `audit_viewer_groups` see all, and the protocol has no
//! way to edit or delete entries.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::IntoResponse;
use fathom::protocol::{AuditLogEntry, AuditOutcome, BackendCmd};
use fathom_server::audit::AUDIT_FILE;
use fathom_server::config::{AccessModel, HostedConfig};
use fathom_server::host::HostedTenantHost;
use fathom_server::oidc::OidcClient;
use fathom_server::session::{Identity, Session, SessionConfig, SessionStore, Tokens};
use fathom_server::ws::{TenantHost, UserConn};
use secrecy::SecretString;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

type Seen = Arc<Mutex<Vec<(Method, String, HeaderMap)>>>;

/// Stand-in API server: records every request and refuses all of them the way the real API
/// server refuses an RBAC-denied request.
async fn fake_api(
    State(seen): State<Seen>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> impl IntoResponse {
    let user = headers
        .get("impersonate-user")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("system:serviceaccount:fathom:fathom")
        .to_owned();
    seen.lock()
        .unwrap()
        .push((method.clone(), uri.path().to_owned(), headers));
    let body = serde_json::json!({
        "kind": "Status",
        "apiVersion": "v1",
        "metadata": {},
        "status": "Failure",
        "message": format!("pods is forbidden: User \"{user}\" cannot {method} resource \"pods\""),
        "reason": "Forbidden",
        "code": 403
    });
    (
        StatusCode::FORBIDDEN,
        [("content-type", "application/json")],
        body.to_string(),
    )
}

async fn start_api() -> (String, Seen) {
    let seen: Seen = Arc::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().fallback(fake_api).with_state(seen.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (url, seen)
}

fn cfg(issuer: &str, state_dir: &std::path::Path) -> HostedConfig {
    let issuer = issuer.to_owned();
    let state = state_dir.to_string_lossy().into_owned();
    HostedConfig::from_lookup(
        move |k| match k {
            "FATHOM_EXTERNAL_URL" => Some("http://localhost:8080".into()),
            "FATHOM_OIDC_ISSUER" => Some(issuer.clone()),
            "FATHOM_OIDC_CLIENT_ID" => Some(common::CLIENT_ID.into()),
            "FATHOM_ALLOW_INSECURE_LOCALHOST" => Some("1".into()),
            "FATHOM_ACCESS_MODEL" => Some("impersonation".into()),
            "FATHOM_USERNAME_PREFIX" => Some("oidc:".into()),
            "FATHOM_GROUPS_PREFIX" => Some("oidc:".into()),
            "FATHOM_AUDIT_VIEWER_GROUPS" => Some("auditors".into()),
            "FATHOM_STATE_DIR" => Some(state.clone()),
            _ => None,
        },
        true,
    )
    .unwrap()
}

fn identity(name: &str, groups: &[&str]) -> Identity {
    Identity {
        provider: Default::default(),
        issuer: "https://idp".into(),
        sub: format!("sub-{name}"),
        username: name.into(),
        groups: groups.iter().map(|g| (*g).to_owned()).collect(),
        email: None,
    }
}

fn tokens() -> Tokens {
    Tokens {
        id_token: SecretString::from("user-id-token".to_owned()),
        access_token: SecretString::from("user-access-token".to_owned()),
        refresh_token: None,
        expires_at: tokio::time::Instant::now() + Duration::from_secs(3600),
    }
}

struct Tab {
    conn: Box<dyn UserConn>,
    frames: UnboundedReceiver<String>,
}

async fn open_tab(host: &Arc<HostedTenantHost>, session: &Session) -> Tab {
    let (tx, frames) = unbounded_channel();
    let conn = host.attach(session, tx).await.expect("attach");
    conn.subscribe().unwrap();
    Tab { conn, frames }
}

/// Find the first `AuditLogLoaded` payload anywhere in a batch frame.
fn audit_entries(v: &serde_json::Value) -> Option<Vec<AuditLogEntry>> {
    match v {
        serde_json::Value::Object(m) => {
            if m.get("type").and_then(|t| t.as_str()) == Some("AuditLogLoaded") {
                let entries = m.get("data")?.get("entries")?.clone();
                return serde_json::from_value(entries).ok();
            }
            m.values().find_map(audit_entries)
        }
        serde_json::Value::Array(a) => a.iter().find_map(audit_entries),
        _ => None,
    }
}

/// Ask the tab's backend for the audit log and wait for the reply.
async fn load_audit(tab: &mut Tab) -> Vec<AuditLogEntry> {
    tab.conn.send(BackendCmd::RefreshAuditLog).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let frame = tab.frames.recv().await.expect("frames closed");
            let v: serde_json::Value = serde_json::from_str(&frame).unwrap();
            if let Some(entries) = audit_entries(&v) {
                return entries;
            }
        }
    })
    .await
    .expect("AuditLogLoaded within 10s")
}

async fn wait_for<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    for _ in 0..200 {
        if let Some(v) = f() {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutations_are_attributed_to_the_real_user_and_filtered_per_viewer() {
    fathom::startup::install_crypto_provider();
    let (api, seen) = start_api().await;
    let idp = common::MockIdp::start().await;
    let state_dir =
        std::env::temp_dir().join(format!("fathom-audit-visibility-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&state_dir);
    let c = Arc::new(cfg(&idp.base, &state_dir));
    assert_eq!(c.access_model, AccessModel::Impersonation);
    let oidc = Arc::new(OidcClient::discover(&c).await.unwrap());
    let sessions = SessionStore::new(SessionConfig::default());
    // In-cluster impersonation: the base config carries only the service account's token.
    let mut base = kube::Config::new(api.parse().unwrap());
    base.auth_info.token = Some(SecretString::from("sa-token".to_owned()));
    let host = HostedTenantHost::with_base(c.clone(), oidc, sessions.clone(), Ok(base));

    // bob connects to the home cluster and attempts a delete the API server refuses.
    let bob_s = sessions
        .create(identity("bob", &["dev"]), tokens())
        .unwrap();
    let mut bob = open_tab(&host, &bob_s).await;
    bob.conn
        .send(BackendCmd::SetContext {
            context: "in-cluster".into(),
        })
        .unwrap();
    wait_for("the backend to reach the API server", || {
        (!seen.lock().unwrap().is_empty()).then_some(())
    })
    .await;
    bob.conn
        .send(BackendCmd::DeletePod {
            namespace: "team-a".into(),
            name: "web-0".into(),
        })
        .unwrap();

    let audit_path = state_dir.join("audit").join(AUDIT_FILE);
    let recorded = wait_for("bob's audit entry", || {
        let raw = std::fs::read_to_string(&audit_path).ok()?;
        raw.lines()
            .filter_map(|l| serde_json::from_str::<AuditLogEntry>(l).ok())
            .find(|e| e.target_name.as_deref() == Some("web-0"))
    })
    .await;

    // T049: the entry names the real user, never the service account, in impersonation mode.
    let actor = recorded
        .actor
        .as_ref()
        .expect("hosted entries carry an actor");
    assert_eq!(actor.username, "bob");
    assert_eq!(actor.sub, "sub-bob");
    assert_eq!(recorded.context, "in-cluster");
    assert_eq!(recorded.target_namespace.as_deref(), Some("team-a"));
    // Spec 026 outcome rules: a refused mutation is recorded as a failure with its reason.
    match &recorded.outcome {
        AuditOutcome::Failure { reason } => assert!(!reason.is_empty()),
        AuditOutcome::Success => panic!("a refused delete must not be recorded as success"),
    }
    // The API server saw the delete as the impersonated user, on the SA's own token.
    {
        let seen = seen.lock().unwrap();
        let (_, _, h) = seen
            .iter()
            .find(|(m, p, _)| m == Method::DELETE && p.ends_with("/pods/web-0"))
            .expect("the delete reached the API server");
        assert_eq!(h.get("impersonate-user").unwrap(), "oidc:bob");
        assert_eq!(h.get("authorization").unwrap(), "Bearer sa-token");
    }
    // No token ever lands in the audit file.
    let raw = std::fs::read_to_string(&audit_path).unwrap();
    for secret in ["sa-token", "user-id-token", "user-access-token"] {
        assert!(!raw.contains(secret), "audit log leaked {secret}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&audit_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    // T050: bob sees his own entry through the protocol's read path.
    let bobs = load_audit(&mut bob).await;
    assert!(
        bobs.iter()
            .any(|e| e.target_name.as_deref() == Some("web-0"))
    );
    assert!(
        bobs.iter()
            .all(|e| e.actor.as_ref().unwrap().sub == "sub-bob")
    );

    // alice (ordinary user) sees none of bob's entries.
    let alice_s = sessions
        .create(identity("alice", &["dev"]), tokens())
        .unwrap();
    let mut alice = open_tab(&host, &alice_s).await;
    let alices = load_audit(&mut alice).await;
    assert!(
        alices
            .iter()
            .all(|e| e.actor.as_ref().is_some_and(|a| a.sub == "sub-alice")),
        "alice must not see other users' entries: {alices:?}"
    );

    // carol is in an audit viewer group and sees bob's entry.
    let carol_s = sessions
        .create(identity("carol", &["auditors"]), tokens())
        .unwrap();
    let mut carol = open_tab(&host, &carol_s).await;
    let carols = load_audit(&mut carol).await;
    assert!(
        carols
            .iter()
            .any(|e| e.target_name.as_deref() == Some("web-0"))
    );

    drop((bob, alice, carol));
    let _ = std::fs::remove_dir_all(&state_dir);
}

/// FR-013: users cannot alter audit records. The only audit command in the protocol is the
/// read (`RefreshAuditLog`); no edit/delete/clear variant exists or deserializes.
#[test]
fn protocol_has_no_audit_edit_or_delete_operation() {
    const CMD_SRC: &str = include_str!("../../src/protocol/v1/cmd.rs");
    let start = CMD_SRC.find("pub enum BackendCmdV1").unwrap();
    let body = &CMD_SRC[start..];
    let end = body.find("\n}\n").unwrap();
    let audit_variants: Vec<&str> = body[..end]
        .lines()
        .map(str::trim)
        .filter(|l| l.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
        .map(|l| l.split(|c: char| !c.is_alphanumeric()).next().unwrap())
        .filter(|n| n.contains("Audit"))
        .collect();
    assert_eq!(audit_variants, ["RefreshAuditLog"]);

    for name in [
        "DeleteAuditLog",
        "DeleteAuditLogEntry",
        "ClearAuditLog",
        "EditAuditLogEntry",
        "UpdateAuditLogEntry",
        "PruneAuditLog",
        "SaveAuditLog",
    ] {
        for v in [
            serde_json::json!({ "type": name }),
            serde_json::json!({ "type": name, "data": { "id": "x" } }),
        ] {
            assert!(
                serde_json::from_value::<BackendCmd>(v).is_err(),
                "{name} must not exist"
            );
        }
    }
}
