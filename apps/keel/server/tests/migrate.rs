//! Migrations (spec 055 T021): they apply, re-running is a no-op, and the serving pool really
//! runs as `keel_app`. Spec 061 T016: the update columns and `keel_build_seen` exist, and
//! `keel_app` can't delete the build Keel last started as.

mod common;

use common::TestDb;
use keel_server::db;

#[tokio::test]
async fn migrations_apply_and_rerun_cleanly() {
    let Some(t) = TestDb::new("migrations_apply_and_rerun_cleanly").await else {
        return;
    };
    db::migrate(t.opts.clone())
        .await
        .expect("second run is a no-op");
    let tables: Vec<(String,)> = sqlx::query_as(
        "select table_name::text from information_schema.tables \
         where table_schema = current_schema() order by 1",
    )
    .fetch_all(&t.owner)
    .await
    .unwrap();
    let names: Vec<&str> = tables.iter().map(|(n,)| n.as_str()).collect();
    for want in [
        "api_tokens",
        "audit_event",
        "grants",
        "keel_build_seen",
        "passkeys",
        "sessions",
        "teams",
        "users",
        "workspaces",
    ] {
        assert!(names.contains(&want), "missing {want}: {names:?}");
    }
    t.drop().await;
}

#[tokio::test]
async fn serving_pool_runs_as_keel_app() {
    let Some(t) = TestDb::new("serving_pool_runs_as_keel_app").await else {
        return;
    };
    let (role,): (String,) = sqlx::query_as("select current_user::text")
        .fetch_one(&t.app)
        .await
        .unwrap();
    assert_eq!(role, db::APP_ROLE);
    let err = sqlx::query("select * from _sqlx_migrations")
        .fetch_all(&t.app)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("permission denied"), "{err}");
    t.drop().await;
}

#[tokio::test]
async fn update_columns_and_build_seen() {
    let Some(t) = TestDb::new("update_columns_and_build_seen").await else {
        return;
    };
    let digest = format!("sha256:{}", "a".repeat(64));
    // keel_app records the build it starts as, and updates it.
    sqlx::query(
        "insert into keel_build_seen (digest, revision, version) values ($1, 'abc1234', '0.2.0')",
    )
    .bind(&digest)
    .execute(&t.app)
    .await
    .expect("insert");
    sqlx::query("update keel_build_seen set version = '0.3.0'")
        .execute(&t.app)
        .await
        .expect("update");
    // At most one row, and only a well-formed digest.
    let err = sqlx::query("insert into keel_build_seen (id) values (true)")
        .execute(&t.app)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("duplicate key"), "{err}");
    let err = sqlx::query("update keel_build_seen set digest = 'sha256:XYZ'")
        .execute(&t.app)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("check constraint"), "{err}");
    // keel_app can neither delete nor truncate it.
    for sql in ["delete from keel_build_seen", "truncate keel_build_seen"] {
        let err = sqlx::query(sql).execute(&t.app).await.unwrap_err();
        assert!(
            err.to_string().contains("permission denied"),
            "{sql}: {err}"
        );
    }

    // Runs default to empty params, and the column only holds an object.
    let (default, checks): (String, i64) = sqlx::query_as(
        "select (select column_default::text from information_schema.columns \
           where table_schema = current_schema() and table_name = 'runs' \
             and column_name = 'params'), \
         (select count(*) from pg_constraint \
           where conrelid = 'runs'::regclass and contype = 'c' \
             and pg_get_constraintdef(oid) like '%jsonb_typeof(params)%')",
    )
    .fetch_one(&t.owner)
    .await
    .unwrap();
    assert!(default.contains("'{}'"), "{default}");
    assert_eq!(checks, 1);

    // The dismissed dot is a digest or nothing.
    let err = sqlx::query(
        "insert into users (id, issuer, subject, display_name, instance_role, \
         update_dot_dismissed) values (gen_random_uuid(), 'i', 's', 'Ada', 'owner', 'latest')",
    )
    .execute(&t.owner)
    .await
    .unwrap_err();
    assert!(err.to_string().contains("check constraint"), "{err}");
    t.drop().await;
}
