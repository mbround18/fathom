//! Shared test support: one throwaway PostgreSQL schema per test.
//!
//! Start the shared cluster with `eval "$(scripts/keel-test-db.sh start)"`. When
//! `KEEL_TEST_DATABASE_URL` is unset, [`TestDb::new`] returns `None` and the test returns early
//! with a note, so `cargo test` passes on a machine without the cluster.

#![allow(dead_code)]

use std::str::FromStr;

use keel_server::db;
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};

pub struct TestDb {
    pub schema: String,
    /// The schema owner (the migrating user).
    pub owner: PgPool,
    /// The serving pool, as `keel_app`.
    pub app: PgPool,
    pub opts: PgConnectOptions,
    admin: PgPool,
}

impl TestDb {
    /// Creates a fresh schema, migrates it and opens both pools.
    pub async fn new(test: &str) -> Option<Self> {
        let Ok(url) = std::env::var("KEEL_TEST_DATABASE_URL") else {
            eprintln!(
                "skipping {test}: KEEL_TEST_DATABASE_URL is unset (scripts/keel-test-db.sh start)"
            );
            return None;
        };
        let base = PgConnectOptions::from_str(&url).expect("KEEL_TEST_DATABASE_URL parses");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(base.clone())
            .await
            .expect("connect to the test cluster");
        let schema = format!("keel_t_{}", uuid::Uuid::now_v7().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("create schema {schema}")))
            .execute(&admin)
            .await
            .expect("create schema");
        let opts = base.options([("search_path", schema.as_str())]);
        db::migrate(opts.clone()).await.expect("migrate");
        let owner = PgPoolOptions::new()
            .max_connections(2)
            .connect_with(opts.clone())
            .await
            .expect("owner pool");
        let app = db::connect(opts.clone(), 4).await.expect("app pool");
        Some(Self {
            schema,
            owner,
            app,
            opts,
            admin,
        })
    }

    /// Closes the pools and drops the schema.
    pub async fn drop(self) {
        self.app.close().await;
        self.owner.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "drop schema {} cascade",
            self.schema
        )))
        .execute(&self.admin)
        .await
        .expect("drop schema");
        self.admin.close().await;
    }
}
