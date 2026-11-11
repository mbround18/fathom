//! Spec 066: workspaces, the primary repository, the catalog, forge connections and host add.
//!
//! The harness for the stories' tests (T033, T040 to T046). Foundation (T005) checks only that
//! migration 0014 applied as data-model.md describes.

mod common;

use common::TestDb;

#[tokio::test]
async fn migration_0014_adds_the_spec_066_schema() {
    let Some(db) = TestDb::new("repo_schema").await else {
        return;
    };
    for (table, column) in [
        ("workspaces", "primary_binding"),
        ("workspaces", "primary_set_by"),
        ("workspaces", "primary_set_at"),
        ("hosts", "age_recipient"),
        ("forge_connections", "api_base"),
        ("binding_catalogs", "catalog"),
    ] {
        let (n,): (i64,) = sqlx::query_as(
            "select count(*) from information_schema.columns \
             where table_schema = current_schema() and table_name = $1 and column_name = $2",
        )
        .bind(table)
        .bind(column)
        .fetch_one(&db.owner)
        .await
        .unwrap();
        assert_eq!(n, 1, "{table}.{column}");
    }
    // Only an age public key fits `hosts.age_recipient`.
    let (n,): (i64,) = sqlx::query_as(
        "select count(*) from pg_constraint where conrelid = 'hosts'::regclass \
         and pg_get_constraintdef(oid) like '%age_recipient%age1%'",
    )
    .fetch_one(&db.owner)
    .await
    .unwrap();
    assert_eq!(n, 1);
    // `forge_token` is a credential kind.
    let (def,): (String,) = sqlx::query_as(
        "select pg_get_constraintdef(oid) from pg_constraint where conname = 'credentials_kind_check' \
         and connamespace = current_schema()::regnamespace",
    )
    .fetch_one(&db.owner)
    .await
    .unwrap();
    assert!(def.contains("forge_token"), "{def}");
    // The serving role may use the new tables.
    for table in ["forge_connections", "binding_catalogs"] {
        let (ok,): (bool,) =
            sqlx::query_as("select has_table_privilege('keel_app', $1, 'insert')")
                .bind(table)
                .fetch_one(&db.owner)
                .await
                .unwrap();
        assert!(ok, "{table}");
    }
    db.drop().await;
}
