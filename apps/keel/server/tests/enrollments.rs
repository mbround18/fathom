//! Enrolment tokens for admins (spec 060 US3, T035): every token in every workspace is listed
//! with its state (pending, used, revoked, burned, expired) and filtered; a pending token can be
//! revoked, and a revoked token is then refused at enrolment like a burned one; a token that is
//! not used can be rolled into a new one for the same host and profiles, revoking the old one
//! while it was pending. A used token can be neither. Nothing secret reaches the audit log.
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`); skips without it.

mod common;
mod repos;

use keel_exec::ca::{CaKind, SshCa};
use keel_protocol::ProfileName;
use keel_protocol::{Cmd, EnrollmentInfo, EnrollmentState, Event, HostName, HostProfiles};
use keel_server::hosts;
use keel_server::runner_listener::{EnrollRefusal, EnrollRequest, enroll_key};
use repos::{Harness, op_error, preview, sign_in};
use russh::keys::ssh_key::public::KeyData;
use serde_json::Value;
use uuid::Uuid;

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

fn enroll_cmd(h: &Harness, name: &str) -> Cmd {
    Cmd::PreviewHostEnroll {
        workspace: h.workspace,
        inventory_name: HostName::new(name).expect("name"),
        profiles: HostProfiles(vec![ProfileName::new("debian").expect("profile")]),
    }
}

/// The new token's id and secret, from a `HostEnrollment` reply.
fn minted(out: &[Event]) -> (Uuid, String, String) {
    match out {
        [
            Event::Confirmed { .. },
            Event::HostEnrollment {
                id, token, snippet, ..
            },
        ] => (*id, token.0.clone(), snippet.clone()),
        other => panic!("enrol: {other:?}"),
    }
}

async fn mint(h: &Harness, name: &str) -> (Uuid, String) {
    let (_, out) = h.confirm(enroll_cmd(h, name)).await;
    let (id, token, _) = minted(&out);
    (id, token)
}

fn request<'a>(token: &'a str, name: &'a str, allowed: &'a [String]) -> EnrollRequest<'a> {
    EnrollRequest {
        token,
        inventory_name: name,
        allowed_profiles: allowed,
        require_signed: false,
    }
}

fn runner_key() -> KeyData {
    SshCa::generate(CaKind::Runner)
        .expect("key")
        .key_data()
        .clone()
}

fn items(out: &[Event]) -> Vec<EnrollmentInfo> {
    match out.iter().find(|e| matches!(e, Event::Enrollments { .. })) {
        Some(Event::Enrollments { items }) => items.clone(),
        _ => panic!("expected enrolments, got {out:?}"),
    }
}

async fn list(h: &Harness, state: Option<EnrollmentState>) -> Vec<EnrollmentInfo> {
    items(
        &h.cmd(Cmd::ListEnrollments {
            state,
            workspace: None,
        })
        .await,
    )
}

fn state_of(items: &[EnrollmentInfo], id: Uuid) -> EnrollmentState {
    items
        .iter()
        .find(|i| i.id == id)
        .unwrap_or_else(|| panic!("{id} not listed"))
        .state
}

async fn contexts(h: &Harness, action: &str) -> Vec<Value> {
    let rows: Vec<(Value,)> = sqlx::query_as(
        "select context from audit_event where action = $1 and decision = 'allowed' order by seq",
    )
    .bind(action)
    .fetch_all(&h.db.owner)
    .await
    .expect("audit");
    rows.into_iter().map(|r| r.0).collect()
}

#[tokio::test]
async fn every_state_is_listed_and_filtered() {
    let Some(h) = harness("enrollments_states").await else {
        return;
    };
    let allowed = vec!["debian".to_owned()];
    let (pending, _) = mint(&h, "web-1").await;
    let (used, used_token) = mint(&h, "web-2").await;
    enroll_key(
        &h.app,
        &runner_key(),
        &request(&used_token, "web-2", &allowed),
    )
    .await
    .expect("enrol");
    let (revoked, _) = mint(&h, "web-3").await;
    let (burned, burned_token) = mint(&h, "web-4").await;
    // A refusal burns the token.
    let refused = enroll_key(
        &h.app,
        &runner_key(),
        &request(&burned_token, "other", &allowed),
    )
    .await
    .expect_err("name mismatch");
    assert!(matches!(refused, Ok(EnrollRefusal::InventoryNameMismatch)));
    let (expired, _) = mint(&h, "web-5").await;
    sqlx::query(
        "update host_enrollments set expires_at = now() - interval '1 second' where id = $1",
    )
    .bind(expired)
    .execute(&h.db.owner)
    .await
    .expect("expire");
    h.confirm(Cmd::PreviewEnrollmentRevoke {
        enrollment: revoked,
    })
    .await;

    let all = list(&h, None).await;
    assert_eq!(all.len(), 5);
    // Newest first.
    assert_eq!(all[0].id, expired);
    assert_eq!(state_of(&all, pending), EnrollmentState::Pending);
    assert_eq!(state_of(&all, used), EnrollmentState::Used);
    assert_eq!(state_of(&all, revoked), EnrollmentState::Revoked);
    assert_eq!(state_of(&all, burned), EnrollmentState::Burned);
    assert_eq!(state_of(&all, expired), EnrollmentState::Expired);
    let u = all.iter().find(|i| i.id == used).expect("used");
    assert!(u.host.is_some());
    assert_eq!(u.workspace, h.workspace);
    assert_eq!(u.workspace_slug, "ops");
    assert_eq!(u.inventory_name, "web-2");
    assert_eq!(u.profiles, vec!["debian".to_owned()]);
    assert!(u.created_by.is_some());

    for (state, id) in [
        (EnrollmentState::Pending, pending),
        (EnrollmentState::Used, used),
        (EnrollmentState::Revoked, revoked),
        (EnrollmentState::Burned, burned),
        (EnrollmentState::Expired, expired),
    ] {
        let only = list(&h, Some(state)).await;
        assert_eq!(only.iter().map(|i| i.id).collect::<Vec<_>>(), vec![id]);
    }
    let other = items(
        &h.cmd(Cmd::ListEnrollments {
            state: None,
            workspace: Some(Uuid::now_v7()),
        })
        .await,
    );
    assert!(other.is_empty());
    h.finish().await;
}

#[tokio::test]
async fn a_revoked_token_is_refused_like_a_burned_one() {
    let Some(h) = harness("enrollments_revoke").await else {
        return;
    };
    let (id, token) = mint(&h, "web-1").await;
    let (p, out) = h
        .confirm(Cmd::PreviewEnrollmentRevoke { enrollment: id })
        .await;
    assert!(p.step_up);
    assert_eq!(p.action, "host_enrollment_revoked");
    assert_eq!(state_of(&items(&out), id), EnrollmentState::Revoked);

    let allowed = vec!["debian".to_owned()];
    match enroll_key(&h.app, &runner_key(), &request(&token, "web-1", &allowed)).await {
        Err(Ok(r)) => assert_eq!(r, EnrollRefusal::TokenReused),
        Ok(_) => panic!("a revoked token enrolled"),
        Err(Err(e)) => panic!("enrol failed: {e}"),
    }

    // Only a pending token can be revoked.
    let out = h.cmd(Cmd::PreviewEnrollmentRevoke { enrollment: id }).await;
    assert_eq!(op_error(&out), "enrollment_not_pending");
    let out = h
        .cmd(Cmd::PreviewEnrollmentRevoke {
            enrollment: Uuid::now_v7(),
        })
        .await;
    assert_eq!(op_error(&out), "not_found");

    let ctx = contexts(&h, "host_enrollment_revoked").await;
    assert_eq!(ctx.len(), 1);
    assert_eq!(ctx[0]["workspace"], "ops");
    assert_eq!(ctx[0]["by_admin"], false);
    assert!(!h.audit_text().await.contains(&token));
    h.finish().await;
}

#[tokio::test]
async fn a_used_token_can_be_neither_revoked_nor_rolled() {
    let Some(h) = harness("enrollments_used").await else {
        return;
    };
    let (id, token) = mint(&h, "web-1").await;
    let allowed = vec!["debian".to_owned()];
    enroll_key(&h.app, &runner_key(), &request(&token, "web-1", &allowed))
        .await
        .expect("enrol");
    for cmd in [
        Cmd::PreviewEnrollmentRevoke { enrollment: id },
        Cmd::PreviewEnrollmentRoll { enrollment: id },
    ] {
        assert_eq!(op_error(&h.cmd(cmd).await), "enrollment_used");
    }
    h.finish().await;
}

#[tokio::test]
async fn rolling_replaces_the_token_and_rechecks_the_name() {
    let Some(h) = harness("enrollments_roll").await else {
        return;
    };
    let (old, old_token) = mint(&h, "web-1").await;
    let (p, out) = h
        .confirm(Cmd::PreviewEnrollmentRoll { enrollment: old })
        .await;
    assert!(p.step_up);
    assert_eq!(p.action, "host_enrollment_roll");
    let (new, new_token, snippet) = minted(&out);
    assert_ne!(new, old);
    assert_ne!(new_token, old_token);
    assert!(snippet.contains(&new_token));

    let all = list(&h, None).await;
    assert_eq!(state_of(&all, old), EnrollmentState::Revoked);
    let n = all.iter().find(|i| i.id == new).expect("new");
    assert_eq!(n.state, EnrollmentState::Pending);
    assert_eq!(n.rolled_from, Some(old));
    assert_eq!(n.inventory_name, "web-1");
    assert_eq!(n.profiles, vec!["debian".to_owned()]);

    let revoked = contexts(&h, "host_enrollment_revoked").await;
    assert_eq!(revoked.len(), 1);
    assert_eq!(revoked[0]["rolled_to"], new.to_string());
    let created = contexts(&h, "host_enrollment_created").await;
    assert_eq!(created.len(), 2);
    assert_eq!(created[1]["rolled_from"], old.to_string());
    let audit = h.audit_text().await;
    assert!(!audit.contains(&old_token) && !audit.contains(&new_token));

    // An expired token can be rolled too; it is left as it was.
    sqlx::query(
        "update host_enrollments set expires_at = now() - interval '1 second' where id = $1",
    )
    .bind(new)
    .execute(&h.db.owner)
    .await
    .expect("expire");
    let (_, out) = h
        .confirm(Cmd::PreviewEnrollmentRoll { enrollment: new })
        .await;
    let (newer, _, _) = minted(&out);
    let all = list(&h, None).await;
    assert_eq!(state_of(&all, new), EnrollmentState::Expired);
    assert_eq!(state_of(&all, newer), EnrollmentState::Pending);

    // Once a host holds the name, rolling is refused.
    let allowed = vec!["debian".to_owned()];
    let (spare, _) = mint(&h, "web-2").await;
    let (_, token) = mint(&h, "web-2").await;
    enroll_key(&h.app, &runner_key(), &request(&token, "web-2", &allowed))
        .await
        .expect("enrol");
    let out = h
        .cmd(Cmd::PreviewEnrollmentRoll { enrollment: spare })
        .await;
    assert_eq!(op_error(&out), "host_exists");
    h.finish().await;
}

#[tokio::test]
async fn users_are_denied() {
    let Some(h) = harness("enrollments_denied").await else {
        return;
    };
    let (id, _) = mint(&h, "web-1").await;
    let bob = sign_in(&h.app, "bob", "bob").await;
    for cmd in [
        Cmd::ListEnrollments {
            state: None,
            workspace: None,
        },
        Cmd::PreviewEnrollmentRevoke { enrollment: id },
        Cmd::PreviewEnrollmentRoll { enrollment: id },
    ] {
        let out = h.cmd_as(&bob, cmd).await;
        assert!(matches!(out.as_slice(), [Event::Denied { .. }]), "{out:?}");
    }
    // An admin revoking the owner's token is marked as such.
    sqlx::query("update users set instance_role = 'admin' where subject = 'bob'")
        .execute(&h.db.owner)
        .await
        .expect("promote");
    h.step_up(&bob).await;
    let p = preview(
        &h.cmd_as(&bob, Cmd::PreviewEnrollmentRevoke { enrollment: id })
            .await,
    );
    let out = h
        .cmd_as(
            &bob,
            Cmd::Confirm {
                preview_id: p.preview_id,
                step_up: None,
            },
        )
        .await;
    assert!(
        matches!(out.first(), Some(Event::Confirmed { .. })),
        "{out:?}"
    );
    let ctx = contexts(&h, "host_enrollment_revoked").await;
    assert_eq!(ctx[0]["by_admin"], true);
    h.finish().await;
}
