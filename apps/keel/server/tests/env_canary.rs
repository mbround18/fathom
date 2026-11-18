//! Credentials as environment variables (spec 068 US6, T054, SC-005) against the fake runner:
//! a job's `env:` entry goes to the host as an `env_var` secret naming its variable; a missing
//! or wrong-kind credential fails the run with `credential_unavailable` before the host sees
//! it; previews name the credential but never its value; and a canary value appears nowhere
//! outside the sealed credential store: no log, event, audit row, or text/jsonb/bytea column.

mod common;
mod converge_support;
mod repos;

use converge_support::Fixture;
use keel_exec::fake::{FakeRunner, Script};
use keel_exec::frames::{Frame, SECRET_KIND_ENV_VAR};
use keel_protocol::{Cmd, CredentialKind, Event, ObjectName, Pattern, SecretValue};
use uuid::Uuid;

const CANARY: &str = "tskey-canary-5f0c2e19a7d84b3e";

async fn job(f: &Fixture, name: &str) -> Uuid {
    let (id,): (Uuid,) = sqlx::query_as("select id from jobs where binding_id = $1 and name = $2")
        .bind(f.binding)
        .bind(name)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("job");
    id
}

/// Creates a credential and grants it to the fixture's binding for every profile.
async fn credential(f: &Fixture, name: &str, kind: CredentialKind, value: &str) -> Uuid {
    let (_, out) =
        f.h.confirm(Cmd::PreviewCredentialCreate {
            workspace: f.h.workspace,
            name: ObjectName::new(name).expect("name"),
            kind,
            value: SecretValue::new(value).expect("value"),
        })
        .await;
    let id = match out.as_slice() {
        [_, Event::Credentials { items }] => {
            items
                .iter()
                .find(|c| c.name.as_str() == name)
                .expect("listed")
                .id
        }
        other => panic!("credential create: {other:?}"),
    };
    f.h.confirm(Cmd::PreviewCredentialGrant {
        credential: id,
        binding: f.binding,
        profiles: vec![],
    })
    .await;
    id
}

/// Starts the `tailscale` job (profile `debian`) on `node-1`; returns the preview's summary and
/// the run. A manual run matches hosts by their own profiles, so `node-1` gets `debian` too.
async fn start_tailscale(f: &Fixture) -> (String, Uuid) {
    sqlx::query(
        "insert into host_profiles (host_id, profile_id) \
         select $1, id from profiles where workspace_id = $2 and name = 'debian' \
         on conflict do nothing",
    )
    .bind(f.node)
    .bind(f.h.workspace)
    .execute(&f.h.db.owner)
    .await
    .expect("debian");
    let (p, out) =
        f.h.confirm(Cmd::PreviewRunStart {
            job: job(f, "tailscale").await,
            limit: Some(Pattern::new("node-1").expect("limit")),
            params: Default::default(),
            check: false,
        })
        .await;
    let run = match out.as_slice() {
        [_, Event::Run { run, .. }] => run.id,
        other => panic!("run start: {other:?}"),
    };
    (p.summary, run)
}

/// The run's `(state, refusal)` for `node-1`, once it ended.
async fn node_outcome(f: &Fixture, run: Uuid) -> (String, Option<String>) {
    f.ended(run).await;
    sqlx::query_as("select state, refusal from run_hosts where run_id = $1")
        .bind(run)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("run host")
}

#[tokio::test]
async fn a_missing_env_credential_fails_the_run_before_the_host_sees_it() {
    let Some(f) = Fixture::new("env_missing", false).await else {
        return;
    };
    let mut runners = f
        .attach(|_, id| FakeRunner::new(id.to_string()).default_script(Script::finish(0)))
        .await;
    let (summary, run) = start_tailscale(&f).await;
    assert!(
        summary.contains("tailscale-authkey (as $TS_AUTHKEY)"),
        "{summary}"
    );
    assert_eq!(node_outcome(&f, run).await.0, "failed");
    let (state, reason): (String, Option<String>) =
        sqlx::query_as("select state, state_reason from runs where id = $1")
            .bind(run)
            .fetch_one(&f.h.db.owner)
            .await
            .expect("run");
    assert_eq!(state, "failed", "{reason:?}");
    let (events,): (i64,) = sqlx::query_as(
        "select count(*) from run_events where run_id = $1 and kind = 'credential_unavailable'",
    )
    .bind(run)
    .fetch_one(&f.h.db.owner)
    .await
    .expect("run events");
    assert_eq!(events, 1);
    // Nothing of the run reached the host.
    while let Ok(frame) = runners[0].received.try_recv() {
        assert!(!matches!(frame, Frame::Assign { .. }), "{frame:?}");
    }
    f.h.finish().await;
}

#[tokio::test]
async fn an_env_credential_of_another_kind_is_not_sent() {
    let Some(f) = Fixture::new("env_wrong_kind", false).await else {
        return;
    };
    credential(
        &f,
        "tailscale-authkey",
        CredentialKind::VaultPassword,
        CANARY,
    )
    .await;
    let _runners = f
        .attach(|_, id| FakeRunner::new(id.to_string()).default_script(Script::finish(0)))
        .await;
    let (_, run) = start_tailscale(&f).await;
    assert_eq!(node_outcome(&f, run).await.0, "failed");
    f.h.finish().await;
}

#[tokio::test]
async fn a_canary_env_value_reaches_the_host_and_nothing_else() {
    let Some(f) = Fixture::new("env_canary", false).await else {
        return;
    };
    credential(&f, "tailscale-authkey", CredentialKind::EnvValue, CANARY).await;
    let mut runners = f
        .attach(|_, id| {
            FakeRunner::new(id.to_string())
                .default_script(Script::finish(0).event(r#"{"type":"keel.play.start"}"#))
        })
        .await;
    let (summary, run) = start_tailscale(&f).await;
    assert!(!summary.contains(CANARY));
    assert_eq!(node_outcome(&f, run).await, ("succeeded".into(), None));

    // The host got exactly one env_var item for TS_AUTHKEY.
    let mut sent = None;
    while let Ok(frame) = runners[0].received.try_recv() {
        if let Frame::Secrets { items, .. } = frame {
            sent = Some(items);
        }
    }
    let items = sent.expect("a Secrets frame");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "tailscale-authkey");
    assert_eq!(items[0].kind, SECRET_KIND_ENV_VAR);
    assert_eq!(items[0].env_name.as_deref(), Some("TS_AUTHKEY"));
    assert_eq!(items[0].value.expose(), CANARY);

    // What a user can read about the run never carries it.
    let read = serde_json::to_string(&f.h.cmd(Cmd::GetRun { run }).await).expect("json");
    assert!(!read.contains(CANARY));
    assert!(!f.h.audit_text().await.contains(CANARY));

    // Nor does any column of any table, as text or as bytes.
    let columns: Vec<(String, String, String)> = sqlx::query_as(
        "select table_name::text, column_name::text, data_type::text \
         from information_schema.columns c \
         where table_schema = current_schema() \
           and data_type in ('text', 'character varying', 'jsonb', 'json', 'bytea', 'ARRAY') \
           and exists (select 1 from information_schema.tables t \
                       where t.table_schema = c.table_schema and t.table_name = c.table_name \
                         and t.table_type = 'BASE TABLE')",
    )
    .fetch_all(&f.h.db.owner)
    .await
    .expect("columns");
    assert!(columns.len() > 20, "{columns:?}");
    for (table, column, ty) in &columns {
        let test = if ty == "bytea" {
            format!("position($1::bytea in \"{column}\") > 0")
        } else {
            format!("\"{column}\"::text like '%' || $2 || '%'")
        };
        let (hits,): (i64,) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select count(*) from \"{table}\" where {test}"
        )))
        .bind(CANARY.as_bytes())
        .bind(CANARY)
        .fetch_one(&f.h.db.owner)
        .await
        .unwrap_or_else(|e| panic!("{table}.{column}: {e}"));
        assert_eq!(hits, 0, "the canary is in {table}.{column}");
    }
    f.h.finish().await;
}
