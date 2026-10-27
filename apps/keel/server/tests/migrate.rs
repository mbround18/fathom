//! Migrations (spec 055 T021): they apply, re-running is a no-op, and the serving pool really
//! runs as `keel_app`.

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
