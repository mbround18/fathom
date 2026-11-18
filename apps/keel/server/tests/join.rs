//! Join tokens and join requests (spec 055 T141, T145; contracts/join.md): a workspace has no
//! join token until one is issued; a host that presents one becomes a pending request, asking
//! again does not make a second; accepting creates the host and its next attempt gets a
//! certificate; rejecting holds for 30 days; rotating rejects pending requests and refuses the
//! old token; known and revoked keys are refused; the queue and the rate are bounded; the token
//! never reaches the audit log.

mod common;
mod repos;

use std::sync::Arc;
use std::time::Duration;

use keel_exec::ca::{CaKind, HostCertVerifier, SshCa, TrustedCa};
use keel_exec::client_config::{KeelSshClient, client_config, open_subsystem};
use keel_exec::codec::{read_frame, write_frame};
use keel_exec::frames::{Frame, SecretString, Subsystem};
use keel_exec::join_token;
use keel_protocol::{
    Cmd, Event, HostName, HostProfiles, JoinComment, JoinRequestInfo, JoinRequestState,
    JoinTokenInfo, ProfileName,
};
use keel_server::hosts;
use keel_server::join::{self, JoinAttempt, JoinOutcome};
use keel_server::runner_listener::{self, RunnerPath};
use repos::{Harness, ORIGIN, op_error, preview};
use russh::client::{self, AuthResult, Handle};
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, Certificate, PrivateKey, PrivateKeyWithHashAlg};
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;

const WAIT: Duration = Duration::from_secs(10);

/// A harness with the CAs and the join signing key loaded and two profiles, `debian` and `web`.
async fn harness(test: &str) -> Option<Harness> {
    let h = Harness::new(test).await?;
    hosts::bootstrap(&h.app).await.expect("ssh bootstrap");
    join::bootstrap(&h.app).await.expect("join bootstrap");
    for name in ["debian", "web"] {
        sqlx::query("insert into profiles (id, workspace_id, name) values ($1, $2, $3)")
            .bind(Uuid::now_v7())
            .bind(h.workspace)
            .bind(name)
            .execute(&h.db.owner)
            .await
            .expect("profile");
    }
    Some(h)
}

fn runner_key() -> KeyData {
    SshCa::generate(CaKind::Runner)
        .expect("key")
        .key_data()
        .clone()
}

fn names(n: &[&str]) -> Vec<String> {
    n.iter().map(|&s| s.to_owned()).collect()
}

fn host_profiles(n: &[&str]) -> HostProfiles {
    HostProfiles(
        n.iter()
            .map(|p| ProfileName::new(*p).expect("profile"))
            .collect(),
    )
}

fn peer(n: u8) -> RunnerPath {
    RunnerPath::Direct {
        peer: Some(format!("203.0.113.{n}:40000").parse().expect("addr")),
    }
}

async fn issue(h: &Harness) -> (JoinTokenInfo, String) {
    let (_, out) = h
        .confirm(Cmd::PreviewJoinTokenIssue {
            workspace: h.workspace,
            expires_in_days: 30,
            inventory_name: None,
            profiles: Some(host_profiles(&["debian"])),
        })
        .await;
    match out.as_slice() {
        [
            Event::Confirmed { .. },
            Event::JoinTokenSecret {
                info,
                token,
                rejected: 0,
            },
        ] => (info.clone(), token.0.clone()),
        other => panic!("issue: {other:?}"),
    }
}

async fn attempt(
    h: &Harness,
    key: &KeyData,
    token: &str,
    name: &str,
    path: &RunnerPath,
) -> JoinOutcome {
    let allowed = names(&["debian", "web"]);
    let a = JoinAttempt {
        token,
        inventory_name: name,
        allowed_profiles: &allowed,
        require_signed: false,
        hostname: name,
        os: "Debian GNU/Linux 13 (trixie)",
        arch: "x86_64",
        runner_version: "0.1.0",
    };
    join::join_key(&h.app, key, &a, path).await.expect("join")
}

async fn requests(h: &Harness) -> Vec<JoinRequestInfo> {
    match h
        .cmd(Cmd::ListJoinRequests {
            workspace: h.workspace,
            state: None,
        })
        .await
        .as_slice()
    {
        [Event::JoinRequests { items }] => items.clone(),
        other => panic!("list: {other:?}"),
    }
}

async fn accept(h: &Harness, request: Uuid, name: &str, profiles: &[&str]) -> Vec<Event> {
    let out = h
        .cmd(Cmd::PreviewJoinAccept {
            request,
            inventory_name: HostName::new(name).expect("name"),
            profiles: host_profiles(profiles),
            auto_converge: None,
        })
        .await;
    let Some(Event::Preview(p)) = out.first() else {
        return out;
    };
    assert!(p.step_up, "accepting needs step-up");
    h.cmd(Cmd::Confirm {
        preview_id: p.preview_id,
        step_up: None,
    })
    .await
}

#[tokio::test]
async fn without_a_token_nothing_joins() {
    let Some(h) = harness("join_no_token").await else {
        return;
    };
    let out = h
        .cmd(Cmd::GetJoinToken {
            workspace: h.workspace,
        })
        .await;
    assert!(
        matches!(out.as_slice(), [Event::JoinToken(i)] if !i.active && i.jti.is_none()),
        "{out:?}"
    );
    // A token signed by some other key does not verify: closed, audited, nothing stored.
    let other = join_token::JoinSigningKey::generate();
    let forged = other.sign(&join_token::JoinClaims {
        iss: ORIGIN.to_owned(),
        aud: join_token::AUDIENCE.to_owned(),
        sub: h.workspace.to_string(),
        jti: join_token::new_jti(),
        iat: 0,
        exp: i64::MAX / 2,
        server: "keel.example.com:2222".into(),
        host_ca: String::new(),
        origin: ORIGIN.into(),
        rp_id: "keel.example.com".into(),
        inventory_name: None,
        profiles: None,
    });
    let key = runner_key();
    assert_eq!(
        attempt(&h, &key, &forged, "web-1", &peer(1)).await,
        JoinOutcome::Closed("unknown_key")
    );
    assert!(requests(&h).await.is_empty());
    assert!(
        h.audit_actions()
            .await
            .contains(&("join_refused".into(), "denied".into()))
    );
    h.finish().await;
}

#[tokio::test]
async fn a_request_waits_is_accepted_and_enrols_on_the_next_attempt() {
    let Some(h) = harness("join_accept").await else {
        return;
    };
    let (info, token) = issue(&h).await;
    assert!(info.active);
    // Stored by jti and hash only.
    let (jti, hash): (String, Vec<u8>) =
        sqlx::query_as("select jti, token_hash from join_tokens where workspace_id = $1")
            .bind(h.workspace)
            .fetch_one(&h.db.owner)
            .await
            .expect("token row");
    assert_eq!(Some(jti.clone()), info.jti);
    assert_eq!(hash, join_token::token_hash(&token).to_vec());
    // The claims carry what the host needs.
    let (_, claims) = join_token::decode_unverified(&token).expect("claims");
    assert_eq!(claims.sub, h.workspace.to_string());
    assert_eq!(claims.iss, ORIGIN);
    assert_eq!(claims.rp_id, "keel.example.com");
    assert!(claims.host_ca.starts_with("@cert-authority "));
    assert_eq!(claims.profiles, Some(names(&["debian"])));

    let key = runner_key();
    let JoinOutcome::Pending(id) = attempt(&h, &key, &token, "web-1", &peer(1)).await else {
        panic!("expected pending");
    };
    // Asking again is the same request.
    assert_eq!(
        attempt(&h, &key, &token, "web-1", &peer(1)).await,
        JoinOutcome::Pending(id)
    );
    let items = requests(&h).await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].state, JoinRequestState::Pending);
    assert_eq!(items[0].suggested_profiles, names(&["debian"]));
    assert_eq!(items[0].source, "203.0.113.1");

    let out = accept(&h, id, "web-1", &["web", "debian"]).await;
    let host = match out.as_slice() {
        [
            Event::Confirmed { .. },
            Event::JoinRequestDecided { request },
        ] => {
            assert_eq!(request.state, JoinRequestState::Accepted);
            request.host.expect("host")
        }
        other => panic!("accept: {other:?}"),
    };
    let (state, fp): (String, String) =
        sqlx::query_as("select state, ssh_key_fingerprint from hosts where id = $1")
            .bind(host)
            .fetch_one(&h.db.owner)
            .await
            .expect("host");
    assert_eq!(state, "offline");
    assert_eq!(fp, keel_exec::ca::fingerprint(&key));
    assert!(
        join::awaiting_cert(&h.db.owner, host)
            .await
            .expect("awaiting")
    );

    // The runner's next attempt collects its certificate.
    match attempt(&h, &key, &token, "web-1", &peer(1)).await {
        JoinOutcome::Enrolled { host: got, cert } => {
            assert_eq!(got, host);
            assert!(cert.starts_with("ssh-ed25519-cert-v01@openssh.com "));
        }
        other => panic!("expected enrolled, got {other:?}"),
    }
    assert!(
        !join::awaiting_cert(&h.db.owner, host)
            .await
            .expect("awaiting")
    );
    let actions = h.audit_actions().await;
    for a in [
        "join_signing_key_created",
        "join_token_issued",
        "join_requested",
        "join_accepted",
        "host_enrolled",
    ] {
        assert!(actions.iter().any(|(x, _)| x == a), "{a} not audited");
    }
    // The token is nowhere in the audit log, nor its signature.
    let audit = h.audit_text().await;
    let sig = token.rsplit('.').next().expect("sig");
    assert!(!audit.contains(&token) && !audit.contains(sig));
    h.finish().await;
}

#[tokio::test]
async fn profiles_outside_the_allowed_list_are_refused() {
    let Some(h) = harness("join_profiles").await else {
        return;
    };
    let (_, token) = issue(&h).await;
    let key = runner_key();
    let allowed = names(&["debian"]);
    let a = JoinAttempt {
        token: &token,
        inventory_name: "db-1",
        allowed_profiles: &allowed,
        require_signed: false,
        hostname: "db-1",
        os: "",
        arch: "aarch64",
        runner_version: "0.1.0",
    };
    let JoinOutcome::Pending(id) = join::join_key(&h.app, &key, &a, &peer(2))
        .await
        .expect("join")
    else {
        panic!("pending");
    };
    let out = accept(&h, id, "db-1", &["web"]).await;
    assert_eq!(op_error(&out), "profile_not_allowed");
    h.finish().await;
}

#[tokio::test]
async fn a_rejection_holds() {
    let Some(h) = harness("join_reject").await else {
        return;
    };
    let (_, token) = issue(&h).await;
    let key = runner_key();
    let JoinOutcome::Pending(id) = attempt(&h, &key, &token, "web-1", &peer(1)).await else {
        panic!("pending");
    };
    let out = h
        .cmd(Cmd::PreviewJoinReject {
            request: id,
            comment: Some(JoinComment::new("not ours").expect("comment")),
        })
        .await;
    let p = preview(&out);
    assert!(!p.step_up);
    let out = h
        .cmd(Cmd::Confirm {
            preview_id: p.preview_id,
            step_up: None,
        })
        .await;
    match out.as_slice() {
        [
            Event::Confirmed { .. },
            Event::JoinRequestDecided { request },
        ] => {
            assert_eq!(request.state, JoinRequestState::Rejected);
            assert_eq!(request.reason.as_deref(), Some("rejected"));
            assert_eq!(request.comment.as_deref(), Some("not ours"));
        }
        other => panic!("reject: {other:?}"),
    }
    assert_eq!(
        attempt(&h, &key, &token, "web-1", &peer(1)).await,
        JoinOutcome::Rejected("rejected".into())
    );
    // Deciding twice is refused.
    let out = accept(&h, id, "web-1", &["debian"]).await;
    assert_eq!(op_error(&out), "join_request_not_pending");
    h.finish().await;
}

#[tokio::test]
async fn rotating_rejects_pending_requests_and_refuses_the_old_token() {
    let Some(h) = harness("join_rotate").await else {
        return;
    };
    let (_, old) = issue(&h).await;
    // A second issue is refused while one is active.
    let out = h
        .cmd(Cmd::PreviewJoinTokenIssue {
            workspace: h.workspace,
            expires_in_days: 30,
            inventory_name: None,
            profiles: None,
        })
        .await;
    assert_eq!(op_error(&out), "join_token_active");

    let key = runner_key();
    let JoinOutcome::Pending(id) = attempt(&h, &key, &old, "web-1", &peer(1)).await else {
        panic!("pending");
    };
    let (p, out) = h
        .confirm(Cmd::PreviewJoinTokenRotate {
            workspace: h.workspace,
            expires_in_days: 7,
        })
        .await;
    assert_eq!(p.affected.len(), 1);
    let new = match out.as_slice() {
        [
            Event::Confirmed { .. },
            Event::JoinTokenSecret {
                token,
                rejected: 1,
                info,
            },
        ] => {
            assert_eq!(info.profiles, names(&["debian"]), "inherited");
            token.0.clone()
        }
        other => panic!("rotate: {other:?}"),
    };
    let items = requests(&h).await;
    assert_eq!(items[0].id, id);
    assert_eq!(items[0].reason.as_deref(), Some("token_rotated"));
    // The old token: the same answer again. Another host with it: the token is retired.
    assert_eq!(
        attempt(&h, &key, &old, "web-1", &peer(1)).await,
        JoinOutcome::Rejected("token_rotated".into())
    );
    assert_eq!(
        attempt(&h, &runner_key(), &old, "web-2", &peer(1)).await,
        JoinOutcome::Rejected("token_retired".into())
    );
    // The new token starts the request over.
    assert_eq!(
        attempt(&h, &key, &new, "web-1", &peer(1)).await,
        JoinOutcome::Pending(id)
    );

    // Disabling rejects it again and leaves no token.
    let (_, out) = h
        .confirm(Cmd::PreviewJoinTokenDisable {
            workspace: h.workspace,
        })
        .await;
    assert!(
        matches!(out.as_slice(), [Event::Confirmed { .. }, Event::JoinToken(i)] if !i.active),
        "{out:?}"
    );
    assert_eq!(
        attempt(&h, &runner_key(), &new, "web-3", &peer(1)).await,
        JoinOutcome::Rejected("token_retired".into())
    );

    // Rotating the signing key: every token stops verifying.
    let (_, token) = issue(&h).await;
    let (_, out) = h.confirm(Cmd::PreviewJoinSigningKeyRotate).await;
    assert!(
        matches!(
            out.as_slice(),
            [
                Event::Confirmed { .. },
                Event::JoinSigningKey {
                    retired_tokens: 1,
                    ..
                }
            ]
        ),
        "{out:?}"
    );
    assert_eq!(
        attempt(&h, &runner_key(), &token, "web-4", &peer(1)).await,
        JoinOutcome::Closed("unknown_key")
    );
    let audit = h.audit_text().await;
    for t in [&old, &new, &token] {
        assert!(!audit.contains(t.as_str()));
    }
    h.finish().await;
}

#[tokio::test]
async fn known_and_revoked_keys_are_refused() {
    let Some(h) = harness("join_known").await else {
        return;
    };
    let (_, token) = issue(&h).await;
    let key = runner_key();
    let JoinOutcome::Pending(id) = attempt(&h, &key, &token, "web-1", &peer(1)).await else {
        panic!("pending");
    };
    accept(&h, id, "web-1", &["debian"]).await;
    // Another workspace's token cannot reuse this key: it belongs to a host.
    let other_ws = Uuid::now_v7();
    sqlx::query("insert into workspaces (id, slug, name) values ($1, 'dev', 'Dev')")
        .bind(other_ws)
        .execute(&h.db.owner)
        .await
        .expect("workspace");
    let (_, out) = h
        .confirm(Cmd::PreviewJoinTokenIssue {
            workspace: other_ws,
            expires_in_days: 1,
            inventory_name: None,
            profiles: None,
        })
        .await;
    let Some(Event::JoinTokenSecret { token: t2, .. }) = out.get(1) else {
        panic!("issue: {out:?}");
    };
    assert_eq!(
        attempt(&h, &key, &t2.0, "web-1", &peer(1)).await,
        JoinOutcome::Rejected("known_key".into())
    );
    // Revoked: the accepted request no longer enrols.
    sqlx::query("update hosts set state = 'revoked' where ssh_key_fingerprint = $1")
        .bind(keel_exec::ca::fingerprint(&key))
        .execute(&h.db.owner)
        .await
        .expect("revoke");
    assert_eq!(
        attempt(&h, &key, &token, "web-1", &peer(1)).await,
        JoinOutcome::Rejected("known_key".into())
    );
    h.finish().await;
}

#[tokio::test]
async fn the_queue_and_the_rate_are_bounded() {
    let Some(mut h) = harness("join_bounds").await else {
        return;
    };
    let mut cfg = (*h.app.cfg).clone();
    cfg.join_pending_max = 2;
    cfg.join_rate = 3;
    h.app.cfg = Arc::new(cfg);
    let (_, token) = issue(&h).await;
    for n in 1..=2 {
        assert!(matches!(
            attempt(&h, &runner_key(), &token, "web", &peer(n)).await,
            JoinOutcome::Pending(_)
        ));
    }
    assert_eq!(
        attempt(&h, &runner_key(), &token, "web", &peer(3)).await,
        JoinOutcome::Rejected("queue_full".into())
    );
    assert_eq!(requests(&h).await.len(), 2, "queue_full is not stored");
    // A fourth attempt from one source in a minute is closed before the database.
    let key = runner_key();
    for _ in 0..3 {
        attempt(&h, &key, &token, "web", &peer(9)).await;
    }
    let before = h.audit_actions().await.len();
    assert_eq!(
        attempt(&h, &key, &token, "web", &peer(9)).await,
        JoinOutcome::Closed("rate_limited")
    );
    assert_eq!(h.audit_actions().await.len(), before);
    // A malformed frame is closed.
    let allowed = names(&["Not A Profile"]);
    let bad = JoinAttempt {
        token: &token,
        inventory_name: "web",
        allowed_profiles: &allowed,
        require_signed: false,
        hostname: "web",
        os: "",
        arch: "x86_64",
        runner_version: "0.1.0",
    };
    assert_eq!(
        join::join_key(&h.app, &key, &bad, &peer(10))
            .await
            .expect("join"),
        JoinOutcome::Closed("malformed_frame")
    );
    h.finish().await;
}

#[tokio::test]
async fn a_host_joins_over_ssh() {
    let Some(h) = harness("join_ssh").await else {
        return;
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let task = runner_listener::spawn(h.app.clone(), listener).expect("spawn");
    let (_, token) = issue(&h).await;
    let runner = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("key");

    let ask = |token: String| {
        let (runner, h) = (runner.clone(), &h);
        async move {
            let mut handle = ssh_connect(h, addr).await;
            let key = PrivateKeyWithHashAlg::new(Arc::new(runner), None);
            assert!(matches!(
                handle
                    .authenticate_publickey("runner", key)
                    .await
                    .expect("auth"),
                AuthResult::Success
            ));
            let mut stream = open_subsystem(&handle, Subsystem::Join)
                .await
                .expect("join subsystem");
            write_frame(
                &mut stream,
                &Frame::JoinRequest {
                    token: SecretString::new(token),
                    inventory_name: "web-1".into(),
                    allowed_profiles: names(&["debian"]),
                    require_signed: false,
                    hostname: "web-1".into(),
                    os: "Debian".into(),
                    arch: "x86_64".into(),
                    runner_version: "0.1.0".into(),
                },
            )
            .await
            .expect("write");
            tokio::time::timeout(WAIT, read_frame(&mut stream))
                .await
                .expect("no reply")
                .expect("read")
        }
    };
    let Some(Frame::JoinPending { request_id }) = ask(token.clone()).await else {
        panic!("expected JoinPending");
    };
    let id: Uuid = request_id.parse().expect("id");
    accept(&h, id, "web-1", &["debian"]).await;
    // Accepted and uncertified: the bare key reaches keel-join-v1 again for its certificate.
    let Some(Frame::Enrolled { cert, .. }) = ask(token).await else {
        panic!("expected Enrolled");
    };
    let cert = Certificate::from_openssh(&cert).expect("cert");
    let mut handle = ssh_connect(&h, addr).await;
    assert!(matches!(
        handle
            .authenticate_openssh_cert("runner", Arc::new(runner.clone()), cert)
            .await
            .expect("auth"),
        AuthResult::Success
    ));
    drop(handle);
    task.abort();
    h.finish().await;
}

async fn ssh_connect(h: &Harness, addr: std::net::SocketAddr) -> Handle<KeelSshClient> {
    let cas = h.app.ssh.get().expect("cas");
    let client = KeelSshClient::new(HostCertVerifier::new(
        vec![TrustedCa::active(cas.host.key_data().clone())],
        "keel.example.com",
    ));
    let stream = TcpStream::connect(addr).await.expect("connect");
    tokio::time::timeout(
        WAIT,
        client::connect_stream(Arc::new(client_config()), stream, client),
    )
    .await
    .expect("handshake hung")
    .expect("handshake")
}
