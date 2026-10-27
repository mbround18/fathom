//! The audit log against PostgreSQL (spec 055 T013): the chain links, `audit_event` refuses
//! UPDATE/DELETE/TRUNCATE for the serving role and the owner, and `verify` finds an offline edit.

mod common;

use common::TestDb;
use keel_server::audit::{self, Actor, Decision, Entry, GENESIS};
use serde_json::json;
use uuid::Uuid;

fn entry<'a>(actor: &'a Actor, action: &'a str) -> Entry<'a> {
    Entry {
        actor,
        action,
        workspace: None,
        target: json!({"kind": "user", "id": Uuid::from_u128(9).to_string()}),
        decision: Decision::Allowed,
        reason: None,
        context: json!({"ip": "127.0.0.1"}),
    }
}

async fn record_n(db: &TestDb, n: usize) {
    let actor = Actor::User {
        id: Uuid::from_u128(1),
        issuer: "https://idp.example".into(),
        subject: "ada".into(),
        display_name: "Ada".into(),
    };
    for i in 0..n {
        let mut tx = db.app.begin().await.unwrap();
        audit::record(&mut tx, entry(&actor, &format!("test_{i}")))
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
}

#[tokio::test]
async fn chain_links_and_verifies() {
    let Some(db) = TestDb::new("chain_links_and_verifies").await else {
        return;
    };
    record_n(&db, 3).await;

    let rows: Vec<(i64, Vec<u8>, Vec<u8>)> =
        sqlx::query_as("select seq, prev_hash, hash from audit_event order by seq")
            .fetch_all(&db.app)
            .await
            .unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].1, GENESIS.to_vec());
    assert_eq!(rows[1].1, rows[0].2);
    assert_eq!(rows[2].1, rows[1].2);

    let mut conn = db.app.acquire().await.unwrap();
    let report = audit::verify(&mut conn).await.unwrap();
    assert_eq!(report.checked, 3);
    assert_eq!(report.broken, None);
    drop(conn);
    db.drop().await;
}

#[tokio::test]
async fn rolled_back_event_leaves_no_trace() {
    let Some(db) = TestDb::new("rolled_back_event_leaves_no_trace").await else {
        return;
    };
    let mut tx = db.app.begin().await.unwrap();
    audit::record(&mut tx, entry(&Actor::System, "abandoned"))
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    record_n(&db, 1).await;
    let mut conn = db.app.acquire().await.unwrap();
    let report = audit::verify(&mut conn).await.unwrap();
    assert_eq!((report.checked, report.broken), (1, None));
    drop(conn);
    db.drop().await;
}

#[tokio::test]
async fn serving_role_cannot_change_events() {
    let Some(db) = TestDb::new("serving_role_cannot_change_events").await else {
        return;
    };
    record_n(&db, 1).await;
    for sql in [
        "update audit_event set decision = 'denied'",
        "delete from audit_event",
        "truncate audit_event",
    ] {
        let err = sqlx::query(sql).execute(&db.app).await.unwrap_err();
        let code = err
            .as_database_error()
            .and_then(|e| e.code())
            .unwrap_or_default();
        assert_eq!(code, "42501", "{sql}: {err}");
    }
    db.drop().await;
}

#[tokio::test]
async fn owner_is_refused_by_the_trigger() {
    let Some(db) = TestDb::new("owner_is_refused_by_the_trigger").await else {
        return;
    };
    record_n(&db, 1).await;
    for sql in [
        "update audit_event set decision = 'denied'",
        "delete from audit_event",
        "truncate audit_event",
    ] {
        let err = sqlx::query(sql).execute(&db.owner).await.unwrap_err();
        assert!(err.to_string().contains("insert-only"), "{sql}: {err}");
    }
    db.drop().await;
}

#[tokio::test]
async fn verify_finds_an_offline_edit() {
    let Some(db) = TestDb::new("verify_finds_an_offline_edit").await else {
        return;
    };
    record_n(&db, 3).await;

    // Someone with the owner's rights switches the trigger off and rewrites history.
    let mut tx = db.owner.begin().await.unwrap();
    sqlx::query("alter table audit_event disable trigger audit_event_no_update_delete")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("update audit_event set decision = 'denied' where seq = (select min(seq) + 1 from audit_event)")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("alter table audit_event enable trigger audit_event_no_update_delete")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let (second,): (i64,) = sqlx::query_as("select min(seq) + 1 from audit_event")
        .fetch_one(&db.owner)
        .await
        .unwrap();
    let mut conn = db.app.acquire().await.unwrap();
    let report = audit::verify(&mut conn).await.unwrap();
    let broken = report.broken.expect("the edit is found");
    assert_eq!(report.checked, 1);
    assert_eq!(broken.seq, second);
    assert_eq!(broken.why, "hash does not match the event's contents");
    drop(conn);
    db.drop().await;
}
