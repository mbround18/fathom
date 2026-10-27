//! Database access (spec 055 T021). PostgreSQL through sqlx with runtime queries, so the build
//! never needs a database.
//!
//! Two kinds of connection:
//! - [`migrate`] runs as the configured user (the schema owner) and applies the embedded
//!   migrations.
//! - [`connect`] opens the serving pool. Every connection switches to the `keel_app` role, which
//!   may only read and insert `audit_event`. A connection that cannot switch is refused, so the
//!   server never serves with owner rights.

use std::str::FromStr;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use sqlx::Executor;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};

/// The embedded migrations (`apps/keel/server/migrations`).
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// The role the serving pool runs as.
pub const APP_ROLE: &str = "keel_app";

/// Parses a database URL. The error never repeats the URL (it may carry a password).
pub fn options(url: &SecretString) -> Result<PgConnectOptions, String> {
    PgConnectOptions::from_str(url.expose_secret())
        .map_err(|_| "the database URL does not parse (KEEL_DATABASE_URL)".to_owned())
}

/// Applies every pending migration with `opts` (the schema owner).
pub async fn migrate(opts: PgConnectOptions) -> Result<(), String> {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(opts)
        .await
        .map_err(|e| format!("cannot connect to the database: {e}"))?;
    let result = MIGRATOR
        .run(&pool)
        .await
        .map_err(|e| format!("migration failed: {e}"));
    pool.close().await;
    result
}

/// Opens the serving pool. Each connection runs `SET ROLE keel_app`.
pub async fn connect(opts: PgConnectOptions, max: u32) -> Result<PgPool, String> {
    PgPoolOptions::new()
        .max_connections(max)
        .acquire_timeout(Duration::from_secs(10))
        .after_connect(|conn, _meta| {
            Box::pin(async move { conn.execute("set role keel_app").await.map(|_| ()) })
        })
        .connect_with(opts)
        .await
        .map_err(|e| format!("cannot connect to the database as {APP_ROLE}: {e}"))
}
