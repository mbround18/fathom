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

#[tokio::test]
async fn run_approval_columns_and_insert_only_answers() {
    let Some(t) = TestDb::new("run_approval_columns").await else {
        return;
    };
    // A held run must carry its terms (spec 064).
    let (checks, index): (i64, i64) = sqlx::query_as(
        "select (select count(*) from pg_constraint \
           where conrelid = 'runs'::regclass and conname = 'runs_held_terms'), \
         (select count(*) from pg_indexes \
           where schemaname = current_schema() and indexname = 'runs_pending_approval')",
    )
    .fetch_one(&t.owner)
    .await
    .unwrap();
    assert_eq!((checks, index), (1, 1));
    let cols: Vec<(String,)> = sqlx::query_as(
        "select column_name::text from information_schema.columns \
         where table_schema = current_schema() and table_name = 'runs' \
           and column_name in ('approvals_needed', 'four_eyes', 'approval_expires_at') \
         order by 1",
    )
    .fetch_all(&t.owner)
    .await
    .unwrap();
    assert_eq!(cols.len(), 3, "{cols:?}");
    // Answers record the approver's role, and keel_app can only add them.
    let (role_col, upd, del, ins): (i64, bool, bool, bool) = sqlx::query_as(
        "select (select count(*) from information_schema.columns \
           where table_schema = current_schema() and table_name = 'approvals' \
             and column_name = 'approver_role'), \
         has_table_privilege('keel_app', 'approvals', 'UPDATE'), \
         has_table_privilege('keel_app', 'approvals', 'DELETE'), \
         has_table_privilege('keel_app', 'approvals', 'INSERT')",
    )
    .fetch_one(&t.owner)
    .await
    .unwrap();
    assert_eq!((role_col, upd, del, ins), (1, false, false, true));
    t.drop().await;
}

/// Spec 067 T004: zones, relays, the relay-link key and host queries.
#[tokio::test]
async fn relay_tables_constraints_and_grants() {
    let Some(t) = TestDb::new("relay_tables").await else {
        return;
    };
    let exec = |sql: &'static str| {
        let app = t.app.clone();
        async move { sqlx::query(sql).execute(&app).await }
    };
    sqlx::raw_sql(
        "insert into workspaces (id, slug, name) values \
           ('00000000-0000-0000-0000-0000000000a1', 'one', 'One'), \
           ('00000000-0000-0000-0000-0000000000a2', 'two', 'Two'); \
         insert into users (id, issuer, subject, display_name, instance_role) values \
           ('00000000-0000-0000-0000-0000000000b1', 'i', 's', 'Ada', 'owner'); \
         insert into hosts (id, workspace_id, inventory_name) values \
           ('00000000-0000-0000-0000-0000000000c1', '00000000-0000-0000-0000-0000000000a1', 'web1')",
    )
    .execute(&t.owner)
    .await
    .expect("fixtures");

    // Zone names: the slug rule, `direct` reserved, unique per workspace.
    exec(
        "insert into zones (id, workspace_id, name) values \
         ('00000000-0000-0000-0000-0000000000d1', '00000000-0000-0000-0000-0000000000a1', 'dmz')",
    )
    .await
    .expect("zone");
    for bad in [
        "insert into zones (id, workspace_id, name) values \
         (gen_random_uuid(), '00000000-0000-0000-0000-0000000000a1', 'direct')",
        "insert into zones (id, workspace_id, name) values \
         (gen_random_uuid(), '00000000-0000-0000-0000-0000000000a1', 'Bad_Name')",
        "insert into zones (id, workspace_id, name) values \
         (gen_random_uuid(), '00000000-0000-0000-0000-0000000000a1', 'dmz')",
    ] {
        assert!(exec(bad).await.is_err(), "{bad}");
    }

    // A host's zone and a relay's zone must be in the same workspace.
    exec(
        "update hosts set zone_id = '00000000-0000-0000-0000-0000000000d1', \
          last_path = '{\"kind\":\"direct\",\"peer\":\"203.0.113.4:51234\"}'",
    )
    .await
    .expect("host zone");
    let relay = "insert into relays (id, workspace_id, zone_id, name, dial_address, host_key, \
                 host_key_fingerprint) values (gen_random_uuid(), $1, \
                 '00000000-0000-0000-0000-0000000000d1', $2, 'relay.dmz:7832', \
                 'ssh-ed25519 AAAA', $3)";
    let add_relay = |ws: &'static str, name: &'static str, fp: &'static str| {
        let app = t.app.clone();
        async move {
            sqlx::query(relay)
                .bind(uuid::Uuid::parse_str(ws).unwrap())
                .bind(name)
                .bind(fp)
                .execute(&app)
                .await
        }
    };
    add_relay("00000000-0000-0000-0000-0000000000a1", "r1", "SHA256:one")
        .await
        .expect("relay");
    assert!(
        add_relay("00000000-0000-0000-0000-0000000000a2", "r2", "SHA256:two")
            .await
            .is_err(),
        "zone of another workspace"
    );
    assert!(
        add_relay("00000000-0000-0000-0000-0000000000a1", "r3", "SHA256:one")
            .await
            .is_err(),
        "two relays never share a host key"
    );
    assert!(
        add_relay(
            "00000000-0000-0000-0000-0000000000a1",
            "direct",
            "SHA256:three"
        )
        .await
        .is_err(),
        "direct is reserved"
    );
    // A quarantine always says why.
    assert!(
        exec("update relays set quarantined_at = now()")
            .await
            .is_err()
    );
    exec("update relays set quarantined_at = now(), quarantine_reason = 'forbidden_channel'")
        .await
        .expect("quarantine");

    // One active relay-link key.
    for (id, fp) in [("e1", "SHA256:k1"), ("e2", "SHA256:k2")] {
        let r = sqlx::query(
            "insert into relay_link_keys (id, public_key, fingerprint, sealed_private_key, state) \
             values ($1, 'ssh-ed25519 AAAA', $2, '\\x00', 'active')",
        )
        .bind(uuid::Uuid::parse_str(&format!("00000000-0000-0000-0000-0000000000{id}")).unwrap())
        .bind(fp)
        .execute(&t.app)
        .await;
        assert_eq!(r.is_ok(), id == "e1", "{id}: {r:?}");
    }

    // Host queries: keel_app may delete (pruning), and only there.
    exec(
        "insert into host_queries (id, workspace_id, host_id, kind, asked_by) values \
         (gen_random_uuid(), '00000000-0000-0000-0000-0000000000a1', \
          '00000000-0000-0000-0000-0000000000c1', 'facts', \
          '00000000-0000-0000-0000-0000000000b1')",
    )
    .await
    .expect("query");
    assert!(
        exec(
            "insert into host_queries (id, workspace_id, host_id, kind, asked_by) values \
              (gen_random_uuid(), '00000000-0000-0000-0000-0000000000a1', \
               '00000000-0000-0000-0000-0000000000c1', 'shell', \
               '00000000-0000-0000-0000-0000000000b1')"
        )
        .await
        .is_err(),
        "unknown kind"
    );
    exec("delete from host_queries").await.expect("prune");
    // 0020: keel_app may remove a zone, but never one a host or relay still points at.
    let err = exec("delete from zones").await.unwrap_err();
    assert!(err.to_string().contains("foreign key"), "{err}");
    for sql in [
        "delete from relays",
        "delete from relay_link_keys",
        "truncate host_queries",
    ] {
        let err = exec(sql).await.unwrap_err();
        assert!(
            err.to_string().contains("permission denied"),
            "{sql}: {err}"
        );
    }
    t.drop().await;
}
