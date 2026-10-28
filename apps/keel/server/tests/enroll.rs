//! Host enrolment through the base profile (spec 055 US7, T075): an enrolment token is
//! `keel-enroll-v1.<secret>.<host CA fingerprint>`, single use, expires after 15 minutes and is
//! stored only as a hash; a reused token is refused and audited; the runner's `inventory_name`
//! must match the token's; profiles outside the host's `allowed_profiles` are refused; a refusal
//! burns the token; nothing secret reaches the audit log; revoking a host puts its certificate
//! on the KRL.

mod common;
mod repos;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use keel_exec::ca::{CaKind, CertRefusal, HostCertVerifier, SshCa, TrustedCa};
use keel_exec::client_config::{KeelSshClient, SubsystemError, client_config, open_subsystem};
use keel_exec::codec::{read_frame, write_frame};
use keel_exec::frames::{Frame, SecretString, Subsystem};
use keel_protocol::{Cmd, Event, HostName, HostProfiles, HostStatus, ProfileName};
use keel_server::hosts::{self, TOKEN_PREFIX, TOKEN_TTL};
use keel_server::runner_listener;
use keel_server::runner_listener::{EnrollRefusal, EnrollRequest, enroll_key};
use repos::{Harness, op_error, preview};
use russh::client::{self, AuthResult, Handle};
use russh::keys::Certificate;
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, PrivateKey, PrivateKeyWithHashAlg};
use time::OffsetDateTime;
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;

/// A harness with both CAs loaded and two workspace profiles, `debian` and `web`.
async fn harness(test: &str) -> Option<Harness> {
    let h = Harness::new(test).await?;
    hosts::bootstrap(&h.app).await.expect("bootstrap");
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

/// A fresh Ed25519 key, standing in for a runner's.
fn runner_key() -> KeyData {
    SshCa::generate(CaKind::Runner)
        .expect("key")
        .key_data()
        .clone()
}

fn host_profiles(names: &[String]) -> HostProfiles {
    HostProfiles(
        names
            .iter()
            .map(|n| ProfileName::new(n.clone()).expect("profile name"))
            .collect(),
    )
}

fn profiles(names: &[&str]) -> Vec<String> {
    names.iter().map(|&n| n.to_owned()).collect()
}

/// Mints a token for `name` with `profiles` through the preview/confirm flow.
async fn mint(h: &Harness, name: &str, profiles: &[&str]) -> (Uuid, String) {
    let cmd = Cmd::PreviewHostEnroll {
        workspace: h.workspace,
        inventory_name: HostName::new(name).expect("name"),
        profiles: host_profiles(&self::profiles(profiles)),
    };
    let (_, out) = h.confirm(cmd).await;
    match out.as_slice() {
        [
            Event::Confirmed { .. },
            Event::HostEnrollment { id, token, .. },
        ] => (*id, token.0.clone()),
        other => panic!("enrol: {other:?}"),
    }
}

fn request<'a>(token: &'a str, name: &'a str, allowed: &'a [String]) -> EnrollRequest<'a> {
    EnrollRequest {
        token,
        inventory_name: name,
        allowed_profiles: allowed,
        require_signed: false,
    }
}

async fn refusal(h: &Harness, key: &KeyData, req: &EnrollRequest<'_>) -> EnrollRefusal {
    match enroll_key(&h.app, key, req).await {
        Err(Ok(reason)) => reason,
        Ok(_) => panic!("enrolled, expected a refusal"),
        Err(Err(e)) => panic!("enrol failed: {e}"),
    }
}

async fn burned(h: &Harness, enrollment: Uuid) -> Option<String> {
    let (reason,): (Option<String>,) =
        sqlx::query_as("select burned_reason from host_enrollments where id = $1")
            .bind(enrollment)
            .fetch_one(&h.db.owner)
            .await
            .expect("enrollment");
    reason
}

fn now() -> SystemTime {
    SystemTime::now()
}

#[tokio::test]
async fn a_token_enrols_once_and_is_stored_hashed() {
    let Some(h) = harness("enroll_once").await else {
        return;
    };
    let (enrollment, token) = mint(&h, "web-1", &["debian"]).await;
    let cas = h.app.ssh.get().expect("cas");

    // keel-enroll-v1.<64 hex>.<host CA fingerprint>
    let rest = token
        .strip_prefix(&format!("{TOKEN_PREFIX}."))
        .expect("prefix");
    let (secret, fp) = rest.split_once('.').expect("three parts");
    assert_eq!(secret.len(), 64);
    assert!(secret.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(fp, cas.host.fingerprint());

    // Stored as a hash, expiring in 15 minutes; the secret is nowhere in the row.
    let (hash, expires, row): (Vec<u8>, OffsetDateTime, String) = sqlx::query_as(
        "select token_hash, expires_at, to_jsonb(e)::text from host_enrollments e where id = $1",
    )
    .bind(enrollment)
    .fetch_one(&h.db.owner)
    .await
    .expect("enrollment");
    assert_eq!(hash, hosts::token_hash(&token));
    assert!(!row.contains(secret));
    let ttl = expires - OffsetDateTime::now_utc();
    assert!(
        ttl <= TOKEN_TTL && ttl > TOKEN_TTL - time::Duration::minutes(1),
        "{ttl}"
    );

    let key = runner_key();
    let allowed = profiles(&["debian", "web"]);
    let (host, cert) = enroll_key(&h.app, &key, &request(&token, "web-1", &allowed))
        .await
        .unwrap_or_else(|e| panic!("enrol: {e:?}"));

    // The certificate is the runner's key, signed by the runner CA, naming the host and the
    // token's profiles (not every allowed one).
    let cert = Certificate::from_openssh(&cert).expect("cert");
    assert_eq!(cert.public_key(), &key);
    let id = cas.verifier.verify(&cert, now()).expect("verifies");
    assert_eq!(id.inventory_name, "web-1");
    assert_eq!(id.profiles, vec!["debian".to_owned()]);

    let (name, allowed_db, state): (String, Vec<String>, String) =
        sqlx::query_as("select inventory_name, allowed_profiles, state from hosts where id = $1")
            .bind(host)
            .fetch_one(&h.db.owner)
            .await
            .expect("host");
    assert_eq!(name, "web-1");
    assert_eq!(allowed_db, allowed);
    assert_eq!(state, "offline");
    let (assigned,): (i64,) =
        sqlx::query_as("select count(*) from host_profiles where host_id = $1")
            .bind(host)
            .fetch_one(&h.db.owner)
            .await
            .expect("host_profiles");
    assert_eq!(assigned, 1);

    // The same token again, even with another key, is refused and audited.
    let again = refusal(&h, &runner_key(), &request(&token, "web-1", &allowed)).await;
    assert_eq!(again, EnrollRefusal::TokenReused);

    let actions = h.audit_actions().await;
    for want in [
        ("host_enrollment_created", "allowed"),
        ("host_enrolled", "allowed"),
        ("host_enroll_refused", "denied"),
    ] {
        assert!(
            actions.iter().any(|(a, d)| a == want.0 && d == want.1),
            "{want:?} missing from {actions:?}"
        );
    }
    let audit = h.audit_text().await;
    assert!(!audit.contains(secret), "the token reached the audit log");
    assert!(!audit.contains(&token));

    // The host is listed.
    let out = h
        .cmd(Cmd::ListHosts {
            workspace: h.workspace,
            profile: None,
            drift: None,
        })
        .await;
    match out.as_slice() {
        [Event::Hosts { items }] => {
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].id, host);
            assert_eq!(items[0].state, HostStatus::Offline);
        }
        other => panic!("list: {other:?}"),
    }
    h.finish().await;
}

#[tokio::test]
async fn the_inventory_name_must_match_and_a_mismatch_burns_the_token() {
    let Some(h) = harness("enroll_name").await else {
        return;
    };
    let (enrollment, token) = mint(&h, "web-1", &["debian"]).await;
    let allowed = profiles(&["debian"]);
    let key = runner_key();
    let r = refusal(&h, &key, &request(&token, "web-2", &allowed)).await;
    assert_eq!(r, EnrollRefusal::InventoryNameMismatch);
    assert_eq!(
        burned(&h, enrollment).await.as_deref(),
        Some("inventory_name_mismatch")
    );
    // Burned: the right name no longer helps.
    let r = refusal(&h, &key, &request(&token, "web-1", &allowed)).await;
    assert_eq!(r, EnrollRefusal::TokenReused);
    let (hosts,): (i64,) = sqlx::query_as("select count(*) from hosts")
        .fetch_one(&h.db.owner)
        .await
        .expect("hosts");
    assert_eq!(hosts, 0);
    h.finish().await;
}

#[tokio::test]
async fn a_profile_outside_allowed_profiles_is_refused() {
    let Some(h) = harness("enroll_profiles").await else {
        return;
    };
    let (enrollment, token) = mint(&h, "web-1", &["debian", "web"]).await;
    let allowed = profiles(&["debian"]);
    let r = refusal(&h, &runner_key(), &request(&token, "web-1", &allowed)).await;
    assert_eq!(r, EnrollRefusal::ProfileNotAllowed);
    assert_eq!(
        burned(&h, enrollment).await.as_deref(),
        Some("profile_not_allowed")
    );

    // A malformed allowed_profiles list is refused too.
    let (_, token) = mint(&h, "web-2", &["debian"]).await;
    let bad = profiles(&["Debian!"]);
    let r = refusal(&h, &runner_key(), &request(&token, "web-2", &bad)).await;
    assert_eq!(r, EnrollRefusal::BadAllowedProfiles);

    // And the preview refuses a profile the workspace does not have.
    let out = h
        .cmd(Cmd::PreviewHostEnroll {
            workspace: h.workspace,
            inventory_name: HostName::new("web-3").expect("name"),
            profiles: host_profiles(&profiles(&["nope"])),
        })
        .await;
    assert_eq!(op_error(&out), "unknown_profile");
    h.finish().await;
}

#[tokio::test]
async fn an_expired_unknown_or_malformed_token_is_refused() {
    let Some(h) = harness("enroll_expired").await else {
        return;
    };
    let (enrollment, token) = mint(&h, "web-1", &["debian"]).await;
    sqlx::query(
        "update host_enrollments set expires_at = now() - interval '1 second' where id = $1",
    )
    .bind(enrollment)
    .execute(&h.db.owner)
    .await
    .expect("expire");
    let allowed = profiles(&["debian"]);
    let r = refusal(&h, &runner_key(), &request(&token, "web-1", &allowed)).await;
    assert_eq!(r, EnrollRefusal::Expired);
    assert_eq!(burned(&h, enrollment).await.as_deref(), Some("expired"));

    // A token naming another CA hashes to nothing on record.
    let fp = h.app.ssh.get().expect("cas").host.fingerprint();
    let (_, token) = mint(&h, "web-2", &["debian"]).await;
    let forged = token.replace(&fp, "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    let r = refusal(&h, &runner_key(), &request(&forged, "web-2", &allowed)).await;
    assert_eq!(r, EnrollRefusal::UnknownToken);

    for bad in [
        "",
        "keel-enroll-v1",
        "keel-enroll-v2.abc.SHA256:x",
        "not a token",
    ] {
        let r = refusal(&h, &runner_key(), &request(bad, "web-2", &allowed)).await;
        assert_eq!(r, EnrollRefusal::MalformedToken, "{bad:?}");
    }
    // The untouched web-2 token still works.
    enroll_key(&h.app, &runner_key(), &request(&token, "web-2", &allowed))
        .await
        .unwrap_or_else(|e| panic!("enrol: {e:?}"));
    h.finish().await;
}

#[tokio::test]
async fn a_stranger_cannot_mint_a_token_and_one_key_enrols_one_host() {
    let Some(h) = harness("enroll_key_once").await else {
        return;
    };
    let allowed = profiles(&["debian"]);
    let key = runner_key();
    let (_, t1) = mint(&h, "web-1", &["debian"]).await;
    enroll_key(&h.app, &key, &request(&t1, "web-1", &allowed))
        .await
        .unwrap_or_else(|e| panic!("enrol: {e:?}"));
    let (_, t2) = mint(&h, "web-2", &["debian"]).await;
    let r = refusal(&h, &key, &request(&t2, "web-2", &allowed)).await;
    assert_eq!(r, EnrollRefusal::KeyEnrolled);

    // An enrolled name cannot be minted again.
    let out = h
        .cmd(Cmd::PreviewHostEnroll {
            workspace: h.workspace,
            inventory_name: HostName::new("web-1").expect("name"),
            profiles: host_profiles(&allowed),
        })
        .await;
    assert_eq!(op_error(&out), "host_exists");

    // Someone with no grant in the workspace cannot mint a token.
    let stranger = repos::sign_in(&h.app, "2", "stranger").await;
    let out = h
        .cmd_as(
            &stranger,
            Cmd::PreviewHostEnroll {
                workspace: h.workspace,
                inventory_name: HostName::new("web-9").expect("name"),
                profiles: host_profiles(&allowed),
            },
        )
        .await;
    assert!(
        matches!(out.as_slice(), [Event::Denied { reason, .. }] if reason == "no_grant"),
        "stranger: {out:?}"
    );
    h.finish().await;
}

#[tokio::test]
async fn revoking_a_host_puts_its_certificate_on_the_krl() {
    let Some(h) = harness("enroll_revoke").await else {
        return;
    };
    let (_, token) = mint(&h, "web-1", &["debian"]).await;
    let allowed = profiles(&["debian"]);
    let (host, cert) = enroll_key(&h.app, &runner_key(), &request(&token, "web-1", &allowed))
        .await
        .unwrap_or_else(|e| panic!("enrol: {e:?}"));
    let cert = Certificate::from_openssh(&cert).expect("cert");
    h.app
        .ssh
        .get()
        .expect("cas")
        .verifier
        .verify(&cert, now())
        .expect("valid before revoke");

    let p = preview(&h.cmd(Cmd::PreviewHostRevoke { host }).await);
    assert!(p.step_up, "revoking a host needs a fresh step-up");
    let out = h
        .cmd(Cmd::Confirm {
            preview_id: p.preview_id,
            step_up: None,
        })
        .await;
    assert!(
        matches!(out.first(), Some(Event::Confirmed { .. })),
        "{out:?}"
    );

    let cas = h.app.ssh.get().expect("cas");
    assert!(cas.krl.is_certificate_revoked(&cert));
    assert_eq!(cas.verifier.verify(&cert, now()), Err(CertRefusal::Revoked));
    let (state, revoked): (String, i64) = sqlx::query_as(
        "select h.state, (select count(*) from ssh_certs c where c.host_id = h.id \
         and c.revoked_at is not null) from hosts h where h.id = $1",
    )
    .bind(host)
    .fetch_one(&h.db.owner)
    .await
    .expect("host");
    assert_eq!(state, "revoked");
    assert_eq!(revoked, 1);
    assert!(
        h.audit_actions()
            .await
            .iter()
            .any(|(a, d)| a == "host_revoked" && d == "allowed")
    );

    // The CAs survive a reload with the revocation intact (as after a restart).
    hosts::reload(&h.app).await.expect("reload");
    assert!(
        h.app
            .ssh
            .get()
            .expect("cas")
            .krl
            .is_certificate_revoked(&cert)
    );
    h.finish().await;
}

#[tokio::test]
async fn the_cas_are_created_once_and_sealed() {
    let Some(h) = harness("enroll_cas").await else {
        return;
    };
    let before = h.app.ssh.get().expect("cas");
    hosts::bootstrap(&h.app).await.expect("second bootstrap");
    let after = h.app.ssh.get().expect("cas");
    assert_eq!(before.host.fingerprint(), after.host.fingerprint());
    assert_eq!(before.runner.fingerprint(), after.runner.fingerprint());
    assert_ne!(after.host.fingerprint(), after.runner.fingerprint());

    let rows: Vec<(String, String)> =
        sqlx::query_as("select kind, to_jsonb(c)::text from ssh_cas c order by kind")
            .fetch_all(&h.db.owner)
            .await
            .expect("ssh_cas");
    assert_eq!(rows.len(), 2);
    for (_, row) in &rows {
        assert!(
            !row.contains("OPENSSH PRIVATE KEY"),
            "a CA key is not sealed"
        );
    }

    let out = h.cmd(Cmd::GetSshCa).await;
    match out.as_slice() {
        [
            Event::SshCa {
                cas,
                cert_authority,
            },
        ] => {
            assert_eq!(cas.len(), 2);
            assert!(cert_authority.starts_with("@cert-authority * ssh-ed25519 "));
        }
        other => panic!("ssh ca: {other:?}"),
    }
    h.finish().await;
}

const WAIT: Duration = Duration::from_secs(10);

/// Connects to the runner listener, pinning the host CA for the public URL's host name.
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

#[tokio::test]
async fn a_runner_enrols_over_ssh_then_authenticates_with_its_certificate() {
    let Some(h) = harness("enroll_ssh").await else {
        return;
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let task = runner_listener::spawn(h.app.clone(), listener).expect("spawn");
    let (_, token) = mint(&h, "web-1", &["debian"]).await;
    let runner = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("key");

    // An unknown bare key may open only keel-enroll-v1.
    let mut handle = ssh_connect(&h, addr).await;
    let key = PrivateKeyWithHashAlg::new(Arc::new(runner.clone()), None);
    assert!(matches!(
        handle
            .authenticate_publickey("runner", key)
            .await
            .expect("auth"),
        AuthResult::Success
    ));
    let err = open_subsystem(&handle, Subsystem::Runner).await.err();
    assert!(
        matches!(
            err,
            Some(SubsystemError::Refused(_) | SubsystemError::Closed)
        ),
        "{err:?}"
    );
    drop(handle);

    let mut handle = ssh_connect(&h, addr).await;
    let key = PrivateKeyWithHashAlg::new(Arc::new(runner.clone()), None);
    handle
        .authenticate_publickey("runner", key)
        .await
        .expect("auth");
    let mut stream = open_subsystem(&handle, Subsystem::Enroll)
        .await
        .expect("enroll subsystem");
    write_frame(
        &mut stream,
        &Frame::Enroll {
            token: SecretString::new(token.clone()),
            inventory_name: "web-1".into(),
            allowed_profiles: profiles(&["debian"]),
            require_signed: false,
        },
    )
    .await
    .expect("write");
    let reply = tokio::time::timeout(WAIT, read_frame(&mut stream))
        .await
        .expect("no reply")
        .expect("read");
    let Some(Frame::Enrolled { host_id, cert }) = reply else {
        panic!("expected Enrolled, got {reply:?}");
    };
    let host: Uuid = host_id.parse().expect("host id");
    drop(stream);
    drop(handle);

    // The certificate now authenticates the runner, and the host is enrolled.
    let cert = Certificate::from_openssh(&cert).expect("cert");
    let mut handle = ssh_connect(&h, addr).await;
    assert!(matches!(
        handle
            .authenticate_openssh_cert("runner", Arc::new(runner.clone()), cert)
            .await
            .expect("auth"),
        AuthResult::Success
    ));
    // ...and may no longer enrol.
    let err = open_subsystem(&handle, Subsystem::Enroll).await.err();
    assert!(err.is_some(), "a certified runner opened keel-enroll-v1");
    let (state,): (String,) = sqlx::query_as("select state from hosts where id = $1")
        .bind(host)
        .fetch_one(&h.db.owner)
        .await
        .expect("host");
    assert_ne!(state, "revoked");
    task.abort();
    h.finish().await;
}
