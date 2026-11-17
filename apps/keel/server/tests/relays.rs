//! Relay revocation and quarantine release (spec 067 T027, T059): revoking closes the link and
//! every runner stream it carries at once, and the relay is never dialled again; releasing a
//! quarantine puts the relay back into service. Both are audited in the same transaction as the
//! row change.
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`); skips without it.

mod common;
mod relay_kit;
mod repos;

use keel_exec::codec::read_frame;
use keel_exec::relay::Frame;
use keel_server::audit::Actor;
use keel_server::relays::{
    self, RelayActionError, RelayState, RelaySupervisor, release_relay, revoke_relay,
};
use keel_server::runner_listener::RunnerHost;
use relay_kit::{
    TestRelay, WAIT, detached, harness, opts, register, runner_session, wait_live,
};
use uuid::Uuid;

async fn actions(h: &repos::Harness, action: &str) -> Vec<serde_json::Value> {
    let rows: Vec<(serde_json::Value,)> =
        sqlx::query_as("select context from audit_event where action = $1 order by seq")
            .bind(action)
            .fetch_all(&h.db.owner)
            .await
            .expect("audit");
    rows.into_iter().map(|r| r.0).collect()
}

#[tokio::test]
async fn revoking_a_relay_closes_its_link_and_every_carried_stream() {
    let Some((h, zone)) = harness("relays_revoke").await else {
        return;
    };
    let mut relay = TestRelay::start(&h).await;
    let id = register(
        &h,
        zone,
        "edge-1",
        &relay.addr.to_string(),
        &relay.host_key_data(),
    )
    .await;
    let sup = RelaySupervisor::new(h.app.clone(), RunnerHost::new().expect("host"), opts());
    sup.sync().await.expect("sync");
    let mut side = relay.next_link().await.answer(relay.fingerprint());
    wait_live(&h, id, |l| l.state == RelayState::Connected).await;
    let (host_id, _ssh, mut stream) = runner_session(&h, &side.handle, "web-1", 40001).await;
    assert_eq!(h.app.runs.path(host_id).and_then(|p| p.relay()), Some(id));

    // Not this workspace's relay: nothing happens.
    assert!(matches!(
        revoke_relay(&h.app, &Actor::System, Uuid::now_v7(), id).await,
        Err(RelayActionError::NotFound)
    ));
    assert!(h.app.runs.path(host_id).is_some());

    revoke_relay(&h.app, &Actor::System, h.workspace, id)
        .await
        .expect("revoke");

    // The relay is told why, …
    let close = tokio::time::timeout(WAIT, side.seen.recv())
        .await
        .expect("no Close")
        .expect("control channel ended without Close");
    assert!(
        matches!(&close, Frame::Close { reason } if reason == "revoked"),
        "{close:?}"
    );
    // … the runner's stream over it ends (its session follows the lost-runner path) …
    let end = tokio::time::timeout(WAIT, async {
        loop {
            match read_frame(&mut stream).await {
                Ok(Some(_)) => {}
                other => return other.map(|_| ()),
            }
        }
    })
    .await;
    assert!(end.is_ok(), "the carried stream stayed open");
    detached(&h, host_id).await;

    // … the row and its audit landed together, …
    let (revoked,): (bool,) =
        sqlx::query_as("select revoked_at is not null from relays where id = $1")
            .bind(id)
            .fetch_one(&h.db.owner)
            .await
            .expect("relay");
    assert!(revoked);
    let rows = actions(&h, relays::ACTION_REVOKE).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["quarantine_reason"], serde_json::Value::Null);

    // … and the relay is never dialled again.
    sup.sync().await.expect("sync");
    assert!(h.app.relay_live.get(id).is_none());
    relay.no_link().await;

    // Revocation is final.
    assert!(matches!(
        revoke_relay(&h.app, &Actor::System, h.workspace, id).await,
        Err(RelayActionError::Revoked)
    ));
    assert!(matches!(
        release_relay(&h.app, &Actor::System, h.workspace, id).await,
        Err(RelayActionError::Revoked)
    ));
    assert_eq!(actions(&h, relays::ACTION_REVOKE).await.len(), 1);
    sup.shutdown();
    h.finish().await;
}

#[tokio::test]
async fn releasing_a_quarantined_relay_puts_it_back_in_service() {
    let Some((h, zone)) = harness("relays_release").await else {
        return;
    };
    let mut relay = TestRelay::start(&h).await;
    let id = register(
        &h,
        zone,
        "edge-1",
        &relay.addr.to_string(),
        &relay.host_key_data(),
    )
    .await;
    let sup = RelaySupervisor::new(h.app.clone(), RunnerHost::new().expect("host"), opts());

    // Not quarantined: nothing to release.
    assert!(matches!(
        release_relay(&h.app, &Actor::System, h.workspace, id).await,
        Err(RelayActionError::NotQuarantined)
    ));

    sqlx::query(
        "update relays set quarantined_at = now(), quarantine_reason = 'bad_frame' \
         where id = $1",
    )
    .bind(id)
    .execute(&h.db.owner)
    .await
    .expect("quarantine");
    sup.sync().await.expect("sync");
    wait_live(&h, id, |l| l.state == RelayState::Quarantined).await;
    relay.no_link().await;

    assert!(matches!(
        release_relay(&h.app, &Actor::System, Uuid::now_v7(), id).await,
        Err(RelayActionError::NotFound)
    ));
    release_relay(&h.app, &Actor::System, h.workspace, id)
        .await
        .expect("release");
    let (reason, at): (Option<String>, Option<time::OffsetDateTime>) =
        sqlx::query_as("select quarantine_reason, quarantined_at from relays where id = $1")
            .bind(id)
            .fetch_one(&h.db.owner)
            .await
            .expect("relay");
    assert_eq!((reason, at), (None, None));
    let rows = actions(&h, relays::ACTION_RELEASE).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["quarantine_reason"], "bad_frame");

    // Dialled again.
    sup.sync().await.expect("sync");
    let _side = relay.next_link().await.answer(relay.fingerprint());
    wait_live(&h, id, |l| l.state == RelayState::Connected).await;
    assert!(matches!(
        release_relay(&h.app, &Actor::System, h.workspace, id).await,
        Err(RelayActionError::NotQuarantined)
    ));
    sup.shutdown();
    h.finish().await;
}
