//! Typed host queries (spec 067 US5, T042): `AskHost` sends a `Query` to the host's runner and
//! settles the stored row with the runner's answer or refusal, a timeout, `no_capability` or
//! `disconnected`; every settled query is audited once, without its answer. Directly and through
//! a relay.
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`); skips without it.

mod common;
mod relay_kit;
mod repos;

use std::collections::BTreeMap;
use std::time::Duration;

use keel_exec::codec::{read_frame, write_frame};
use keel_exec::frames::{CAP_QUERY_V1, Frame, MAX_QUERY_ANSWER, QueryRefusal};
use keel_protocol::{Cmd, Event, HostQuery, QueryKind, QueryOutcome};
use keel_server::relays::{RelayState, RelaySupervisor};
use keel_server::runner_listener::RunnerHost;
use keel_server::runs::{RunSettings, Runs};
use relay_kit::{TestRelay, WAIT, harness, opts, register, runner_session_with, wait_live};
use repos::{Harness, op_error, sign_in};
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

/// A sentinel in answers that must never reach the audit log.
const SECRET: &str = "answer-content-7f3a";

fn settings() -> RunSettings {
    RunSettings {
        query_timeout: Duration::from_millis(400),
        ..RunSettings::default()
    }
}

async fn setup(test: &str) -> Option<Harness> {
    let mut h = Harness::new(test).await?;
    h.app.runs = Runs::new(settings());
    Some(h)
}

async fn add_host(h: &Harness, name: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query("insert into hosts (id, workspace_id, inventory_name) values ($1, $2, $3)")
        .bind(id)
        .bind(h.workspace)
        .bind(name)
        .execute(&h.db.owner)
        .await
        .expect("host");
    id
}

/// A runner on a duplex stream: Keel's `Query` frames arrive on `queries`, frames sent on `out`
/// are written back. Pings are answered.
struct QueryRunner {
    queries: mpsc::UnboundedReceiver<Frame>,
    out: mpsc::UnboundedSender<Frame>,
    task: tokio::task::JoinHandle<()>,
}

impl QueryRunner {
    async fn attach(h: &Harness, host: Uuid, capabilities: Vec<String>) -> Self {
        let (server_end, runner_end) = tokio::io::duplex(1 << 20);
        let (mut rd, mut wr) = tokio::io::split(runner_end);
        let (out, mut outgoing) = mpsc::unbounded_channel::<Frame>();
        let (queries_tx, queries) = mpsc::unbounded_channel();
        let hello = Frame::Hello {
            host_id: host.to_string(),
            version: "0.0.0".into(),
            ansible_core: "2.21.0".into(),
            python: "3.12.3".into(),
            allowed_profiles: vec![],
            require_signed: false,
            override_quorum: 1,
            ledger_head: None,
            bases: Vec::new(),
            age_recipient: None,
            capabilities,
        };
        out.send(hello).expect("hello");
        let pong = out.clone();
        let task = tokio::spawn(async move {
            let writer = tokio::spawn(async move {
                while let Some(f) = outgoing.recv().await {
                    if write_frame(&mut wr, &f).await.is_err() {
                        return;
                    }
                }
            });
            while let Ok(Some(frame)) = read_frame(&mut rd).await {
                match frame {
                    Frame::Ping => {
                        let _ = pong.send(Frame::Pong);
                    }
                    q @ Frame::Query { .. } => {
                        let _ = queries_tx.send(q);
                    }
                    _ => {}
                }
            }
            writer.abort();
        });
        keel_server::queue::attach(h.app.clone(), host, server_end)
            .await
            .expect("attach");
        Self {
            queries,
            out,
            task,
        }
    }

    async fn next_query(&mut self) -> (String, String, BTreeMap<String, String>, String) {
        match tokio::time::timeout(WAIT, self.queries.recv()).await {
            Ok(Some(Frame::Query {
                query_id,
                kind,
                params,
                asked_by,
            })) => (query_id, kind, params, asked_by),
            other => panic!("expected a query, got {other:?}"),
        }
    }

    fn reply(&self, frame: Frame) {
        self.out.send(frame).expect("reply");
    }
}

impl Drop for QueryRunner {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn ask(host: Uuid, kind: QueryKind, params: &[(&str, &str)]) -> Cmd {
    Cmd::AskHost {
        host,
        kind,
        params: params
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
    }
}

fn asked(events: &[Event]) -> HostQuery {
    match events {
        [Event::HostQuery(q)] => q.clone(),
        other => panic!("expected a HostQuery, got {other:?}"),
    }
}

/// The next `HostQueryAnswered` for `id`.
async fn answered(events: &mut broadcast::Receiver<(keel_core::WorkspaceId, Event)>, id: Uuid) -> HostQuery {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let got = tokio::time::timeout_at(deadline, events.recv())
            .await
            .expect("no HostQueryAnswered")
            .expect("events");
        if let (_, Event::HostQueryAnswered(q)) = got {
            if q.id == id {
                return q;
            }
        }
    }
}

async fn list(h: &Harness, host: Uuid) -> (Vec<HostQuery>, Option<bool>) {
    match h
        .cmd(Cmd::ListHostQueries { host, limit: None })
        .await
        .as_slice()
    {
        [Event::HostQueries {
            items, query_v1, ..
        }] => (items.clone(), *query_v1),
        other => panic!("expected HostQueries, got {other:?}"),
    }
}

/// The `context` of every `host.query` audit row, oldest first.
async fn audits(h: &Harness) -> Vec<Value> {
    let rows: Vec<(Value,)> = sqlx::query_as(
        "select context from audit_event where action = 'host.query' order by seq",
    )
    .fetch_all(&h.db.owner)
    .await
    .expect("audit");
    rows.into_iter().map(|r| r.0).collect()
}

#[tokio::test]
async fn an_answered_query_is_stored_pushed_listed_and_audited_without_its_answer() {
    let Some(h) = setup("hq_answered").await else {
        return;
    };
    let host = add_host(&h, "web-1").await;
    let mut runner = QueryRunner::attach(&h, host, vec![CAP_QUERY_V1.to_owned()]).await;
    let mut events = h.app.repos.events.subscribe();

    let q = asked(
        &h.cmd(ask(host, QueryKind::UnitState, &[("unit", "nginx.service")]))
            .await,
    );
    assert_eq!(q.outcome, None);
    assert_eq!(q.asked_by, "owner");
    let (query_id, kind, params, asked_by) = runner.next_query().await;
    assert_eq!(query_id, q.id.to_string());
    assert_eq!(kind, "unit_state");
    assert_eq!(params.get("unit").map(String::as_str), Some("nginx.service"));
    assert_eq!(asked_by, "owner");

    runner.reply(Frame::QueryAnswer {
        query_id,
        answer: json!({ "active_state": "active", "note": SECRET }),
    });
    let done = answered(&mut events, q.id).await;
    assert_eq!(done.outcome, Some(QueryOutcome::Answered));
    assert_eq!(done.answer.as_ref().expect("answer")["active_state"], "active");
    assert!(done.answered_at.is_some());

    let (items, query_v1) = list(&h, host).await;
    assert_eq!(query_v1, Some(true));
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].outcome, Some(QueryOutcome::Answered));

    // A late duplicate does not settle it again.
    runner.reply(Frame::QueryAnswer {
        query_id: q.id.to_string(),
        answer: json!({ "active_state": "failed" }),
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let a = audits(&h).await;
    assert_eq!(a.len(), 1, "{a:?}");
    assert_eq!(a[0]["outcome"], "answered");
    assert_eq!(a[0]["kind"], "unit_state");
    assert!(!h.audit_text().await.contains(SECRET));
    assert!(!h.audit_text().await.contains("active_state"));
    let (items, _) = list(&h, host).await;
    assert_eq!(items[0].answer.as_ref().expect("answer")["active_state"], "active");
    h.finish().await;
}

#[tokio::test]
async fn refusals_contract_breaches_and_foreign_replies() {
    let Some(h) = setup("hq_refused").await else {
        return;
    };
    let host = add_host(&h, "web-1").await;
    let other = add_host(&h, "web-2").await;
    let mut runner = QueryRunner::attach(&h, host, vec![CAP_QUERY_V1.to_owned()]).await;
    let mut events = h.app.repos.events.subscribe();

    // Refused by the runner.
    let q = asked(&h.cmd(ask(host, QueryKind::PathFree, &[("path", "/srv")])).await);
    let (id, ..) = runner.next_query().await;
    runner.reply(Frame::QueryRefused {
        query_id: id,
        reason: QueryRefusal::NotAllowed,
    });
    let done = answered(&mut events, q.id).await;
    assert_eq!(done.outcome, Some(QueryOutcome::Refused));
    assert_eq!(done.refusal.as_deref(), Some("not_allowed"));
    assert_eq!(done.answer, None);

    // A reply naming this host's query but arriving from another host is ignored.
    let q = asked(&h.cmd(ask(host, QueryKind::Facts, &[])).await);
    let (id, ..) = runner.next_query().await;
    keel_server::host_queries::on_reply(
        &h.app,
        other,
        Frame::QueryAnswer {
            query_id: id.clone(),
            answer: json!({ "forged": true }),
        },
    )
    .await;
    let (items, _) = list(&h, host).await;
    assert_eq!(items[0].id, q.id);
    assert_eq!(items[0].outcome, None, "a foreign reply settled the query");
    // An answer that is not an object breaks the contract: refused, not stored.
    runner.reply(Frame::QueryAnswer {
        query_id: id,
        answer: json!(["not", "an", "object"]),
    });
    let done = answered(&mut events, q.id).await;
    assert_eq!(done.outcome, Some(QueryOutcome::Refused));
    assert_eq!(done.refusal.as_deref(), Some("invalid_answer"));
    assert_eq!(done.answer, None);

    // Over the cap: refused as too large.
    let q = asked(&h.cmd(ask(host, QueryKind::Facts, &[])).await);
    let (id, ..) = runner.next_query().await;
    runner.reply(Frame::QueryAnswer {
        query_id: id,
        answer: json!({ "pad": "x".repeat(MAX_QUERY_ANSWER) }),
    });
    let done = answered(&mut events, q.id).await;
    assert_eq!(done.refusal.as_deref(), Some("too_large"));
    assert_eq!(done.answer, None);
    h.finish().await;
}

#[tokio::test]
async fn unanswered_queries_time_out_and_a_lost_session_disconnects_the_rest() {
    let Some(h) = setup("hq_timeout").await else {
        return;
    };
    let host = add_host(&h, "web-1").await;
    let mut runner = QueryRunner::attach(&h, host, vec![CAP_QUERY_V1.to_owned()]).await;
    let mut events = h.app.repos.events.subscribe();

    let q = asked(&h.cmd(ask(host, QueryKind::DiskHealth, &[])).await);
    runner.next_query().await;
    let done = answered(&mut events, q.id).await;
    assert_eq!(done.outcome, Some(QueryOutcome::Timeout));
    let a = audits(&h).await;
    assert_eq!(a.last().expect("audit")["outcome"], "timeout");

    // Pending when the runner goes away: disconnected, not left hanging.
    let q = asked(&h.cmd(ask(host, QueryKind::AppliedRevision, &[])).await);
    runner.next_query().await;
    drop(runner);
    let done = answered(&mut events, q.id).await;
    assert_eq!(done.outcome, Some(QueryOutcome::Disconnected));
    h.finish().await;
}

#[tokio::test]
async fn no_session_or_an_old_runner_settles_at_once() {
    let Some(h) = setup("hq_early").await else {
        return;
    };
    let host = add_host(&h, "web-1").await;

    let q = asked(&h.cmd(ask(host, QueryKind::Facts, &[])).await);
    assert_eq!(q.outcome, Some(QueryOutcome::Disconnected));
    assert!(q.answered_at.is_some());
    assert_eq!(list(&h, host).await.1, None);

    let _old = QueryRunner::attach(&h, host, Vec::new()).await;
    let q = asked(&h.cmd(ask(host, QueryKind::Facts, &[])).await);
    assert_eq!(q.outcome, Some(QueryOutcome::NoCapability));
    let (items, query_v1) = list(&h, host).await;
    assert_eq!(query_v1, Some(false));
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, q.id, "newest first");

    let outcomes: Vec<Value> = audits(&h).await.iter().map(|c| c["outcome"].clone()).collect();
    assert_eq!(outcomes, vec![json!("disconnected"), json!("no_capability")]);
    h.finish().await;
}

#[tokio::test]
async fn bad_parameters_are_refused_before_anything_is_sent() {
    let Some(h) = setup("hq_params").await else {
        return;
    };
    let host = add_host(&h, "web-1").await;
    let mut runner = QueryRunner::attach(&h, host, vec![CAP_QUERY_V1.to_owned()]).await;
    for cmd in [
        ask(host, QueryKind::UnitState, &[]),
        ask(host, QueryKind::UnitState, &[("unit", "-H evil")]),
        ask(host, QueryKind::UnitState, &[("unit", "a b")]),
        ask(host, QueryKind::PathFree, &[("path", "relative")]),
        ask(host, QueryKind::PathFree, &[("path", "/srv/../etc/shadow")]),
        ask(host, QueryKind::Facts, &[("unit", "x")]),
    ] {
        assert_eq!(op_error(&h.cmd(cmd).await), "bad_params");
    }
    assert!(list(&h, host).await.0.is_empty());
    assert!(runner.queries.try_recv().is_err(), "a bad query reached the runner");
    assert_eq!(op_error(&h.cmd(ask(Uuid::now_v7(), QueryKind::Facts, &[])).await), "not_found");
    h.finish().await;
}

#[tokio::test]
async fn viewers_list_but_only_operators_ask() {
    let Some(h) = setup("hq_authz").await else {
        return;
    };
    let host = add_host(&h, "web-1").await;
    let vic = sign_in(&h.app, "21", "vic").await;
    let team = Uuid::now_v7();
    sqlx::query("insert into teams (id, name) values ($1, 'team-vic')")
        .bind(team)
        .execute(&h.db.owner)
        .await
        .expect("team");
    sqlx::query("insert into team_members (team_id, user_id) values ($1, $2)")
        .bind(team)
        .bind(vic.user.id)
        .execute(&h.db.owner)
        .await
        .expect("member");
    sqlx::query("insert into grants (id, team_id, workspace_id, role) values ($1, $2, $3, 'viewer')")
        .bind(Uuid::now_v7())
        .bind(team)
        .bind(h.workspace)
        .execute(&h.db.owner)
        .await
        .expect("grant");

    let out = h.cmd_as(&vic, ask(host, QueryKind::Facts, &[])).await;
    assert!(matches!(out.as_slice(), [Event::Denied { .. }]), "{out:?}");
    let out = h
        .cmd_as(&vic, Cmd::ListHostQueries { host, limit: None })
        .await;
    assert!(matches!(out.as_slice(), [Event::HostQueries { .. }]), "{out:?}");
    h.finish().await;
}

#[tokio::test]
async fn each_host_keeps_its_newest_two_hundred_queries() {
    let Some(h) = setup("hq_prune").await else {
        return;
    };
    let host = add_host(&h, "web-1").await;
    sqlx::query(
        "insert into host_queries (id, workspace_id, host_id, kind, params, asked_by, asked_at, \
         outcome, answered_at) \
         select gen_random_uuid(), $1, $2, 'facts', '{}'::jsonb, $3, \
                now() - make_interval(secs => g), 'disconnected', now() - make_interval(secs => g) \
           from generate_series(1, 205) g",
    )
    .bind(h.workspace)
    .bind(host)
    .bind(h.owner.user.id)
    .execute(&h.db.owner)
    .await
    .expect("seed");
    let q = asked(&h.cmd(ask(host, QueryKind::Facts, &[])).await);
    let (n,): (i64,) = sqlx::query_as("select count(*) from host_queries where host_id = $1")
        .bind(host)
        .fetch_one(&h.db.owner)
        .await
        .expect("count");
    assert_eq!(n, 200);
    let items = match h
        .cmd(Cmd::ListHostQueries {
            host,
            limit: Some(1000),
        })
        .await
        .as_slice()
    {
        [Event::HostQueries { items, .. }] => items.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(items.len(), 200);
    assert_eq!(items[0].id, q.id, "the newest survives");
    h.finish().await;
}

#[tokio::test]
async fn a_query_through_a_relay_reaches_the_runner_and_comes_back() {
    let Some((mut h, zone)) = harness("hq_relay").await else {
        return;
    };
    h.app.runs = Runs::new(settings());
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
    let side = relay.next_link().await.answer(relay.fingerprint());
    wait_live(&h, id, |l| l.state == RelayState::Connected).await;
    let (host, _ssh, mut stream) =
        runner_session_with(&h, &side.handle, "web-1", 40011, vec![CAP_QUERY_V1.to_owned()])
            .await;
    let mut events = h.app.repos.events.subscribe();

    let q = asked(&h.cmd(ask(host, QueryKind::PathFree, &[("path", "/var")])).await);
    let query_id = loop {
        match tokio::time::timeout(WAIT, read_frame(&mut stream))
            .await
            .expect("no query")
            .expect("read")
            .expect("open")
        {
            Frame::Query {
                query_id, kind, ..
            } => {
                assert_eq!(kind, "path_free");
                break query_id;
            }
            Frame::Ping => write_frame(&mut stream, &Frame::Pong).await.expect("pong"),
            _ => {}
        }
    };
    write_frame(
        &mut stream,
        &Frame::QueryAnswer {
            query_id,
            answer: json!({ "free_bytes": 1024, "total_bytes": 4096, "note": SECRET }),
        },
    )
    .await
    .expect("answer");
    let done = answered(&mut events, q.id).await;
    assert_eq!(done.outcome, Some(QueryOutcome::Answered));
    assert_eq!(done.answer.as_ref().expect("answer")["free_bytes"], 1024);
    assert!(!h.audit_text().await.contains(SECRET));
    sup.shutdown();
    h.finish().await;
}
