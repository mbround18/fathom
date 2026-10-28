//! API tokens for admins (spec 060 US4, T040): admins list every holder's tokens with their
//! state, filtered by holder and state; users can't. Revoking someone else's token asks for
//! step-up, is audited with `by_admin` and replies with the admin list; revoking your own
//! token is unchanged.
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`); skips without it.

mod common;
mod repos;

use keel_protocol::{AdminApiTokenInfo, ApiTokenState, Cmd, Event};
use repos::{Harness, preview, sign_in};
use serde_json::Value;
use uuid::Uuid;

/// Inserts a token for `holder`, `days` from expiry (negative: expired).
async fn token(h: &Harness, holder: Uuid, name: &str, days: i32) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into api_tokens (id, name, owner_user_id, workspace_id, actions, hash, \
         expires_at) values ($1, $2, $3, $4, array['list_workspaces'], $5, \
         now() + make_interval(days => $6))",
    )
    .bind(id)
    .bind(name)
    .bind(holder)
    .bind(h.workspace)
    .bind(id.as_bytes().to_vec())
    .bind(days)
    .execute(&h.db.owner)
    .await
    .expect("token");
    id
}

fn all(out: &[Event]) -> Vec<AdminApiTokenInfo> {
    match out.iter().find(|e| matches!(e, Event::AllApiTokens { .. })) {
        Some(Event::AllApiTokens { items }) => items.clone(),
        _ => panic!("expected all tokens, got {out:?}"),
    }
}

async fn list(h: &Harness, holder: Option<Uuid>, state: Option<ApiTokenState>) -> Vec<Uuid> {
    all(&h.cmd(Cmd::ListAllApiTokens { holder, state }).await)
        .iter()
        .map(|t| t.token.id)
        .collect()
}

async fn revoke_context(h: &Harness) -> Value {
    let (v,): (Value,) = sqlx::query_as(
        "select context from audit_event where action = 'api_token_revoke' \
         and decision = 'allowed' order by seq desc limit 1",
    )
    .fetch_one(&h.db.owner)
    .await
    .expect("audit row");
    v
}

#[tokio::test]
async fn admins_see_every_holder_and_filter() {
    let Some(h) = Harness::new("admin_tokens_list").await else {
        return;
    };
    let bob = sign_in(&h.app, "bob", "bob").await;
    let mine = token(&h, h.owner.user.id, "mine", 7).await;
    let active = token(&h, bob.user.id, "bob-ci", 7).await;
    let expired = token(&h, bob.user.id, "bob-old", -1).await;
    let revoked = token(&h, bob.user.id, "bob-gone", 7).await;
    sqlx::query("update api_tokens set revoked_at = now() where id = $1")
        .bind(revoked)
        .execute(&h.db.owner)
        .await
        .expect("revoke");

    let items = all(&h
        .cmd(Cmd::ListAllApiTokens {
            holder: None,
            state: None,
        })
        .await);
    assert_eq!(items.len(), 4);
    let b = items.iter().find(|t| t.token.id == active).expect("bob's");
    assert_eq!(b.holder, bob.user.id);
    assert_eq!(b.holder_name, "bob");
    assert_eq!(b.workspace_slug, "ops");
    assert_eq!(b.state, ApiTokenState::Active);

    let mut bobs = list(&h, Some(bob.user.id), None).await;
    bobs.sort();
    let mut want = vec![active, expired, revoked];
    want.sort();
    assert_eq!(bobs, want);
    assert_eq!(
        list(&h, None, Some(ApiTokenState::Expired)).await,
        vec![expired]
    );
    assert_eq!(
        list(&h, None, Some(ApiTokenState::Revoked)).await,
        vec![revoked]
    );
    let mut act = list(&h, None, Some(ApiTokenState::Active)).await;
    act.sort();
    let mut want = vec![mine, active];
    want.sort();
    assert_eq!(act, want);

    // Users can't.
    let out = h
        .cmd_as(
            &bob,
            Cmd::ListAllApiTokens {
                holder: None,
                state: None,
            },
        )
        .await;
    assert!(matches!(out.as_slice(), [Event::Denied { .. }]), "{out:?}");
    h.finish().await;
}

#[tokio::test]
async fn revoking_someone_elses_token_is_marked_and_needs_step_up() {
    let Some(h) = Harness::new("admin_tokens_revoke").await else {
        return;
    };
    let bob = sign_in(&h.app, "bob", "bob").await;
    let theirs = token(&h, bob.user.id, "bob-ci", 7).await;
    let mine = token(&h, h.owner.user.id, "mine", 7).await;

    let (p, out) = h
        .confirm(Cmd::PreviewApiTokenRevoke { token: theirs })
        .await;
    assert!(p.step_up);
    assert_eq!(p.affected, vec!["bob".to_owned()]);
    let items = all(&out);
    let t = items.iter().find(|t| t.token.id == theirs).expect("listed");
    assert_eq!(t.state, ApiTokenState::Revoked);
    assert_eq!(revoke_context(&h).await["by_admin"], true);

    // Your own: no step-up, no mark, your own list back.
    let p = preview(&h.cmd(Cmd::PreviewApiTokenRevoke { token: mine }).await);
    assert!(!p.step_up);
    let out = h
        .cmd(Cmd::Confirm {
            preview_id: p.preview_id,
            step_up: None,
        })
        .await;
    assert!(
        matches!(
            out.as_slice(),
            [Event::Confirmed { .. }, Event::ApiTokens { .. }]
        ),
        "{out:?}"
    );
    assert!(revoke_context(&h).await.get("by_admin").is_none());

    // A user still can't revoke someone else's.
    let other = token(&h, h.owner.user.id, "other", 7).await;
    let out = h
        .cmd_as(&bob, Cmd::PreviewApiTokenRevoke { token: other })
        .await;
    assert!(matches!(out.as_slice(), [Event::Denied { .. }]), "{out:?}");
    h.finish().await;
}
