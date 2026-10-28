//! Runs outside Keel (spec 058 amendment A2, T038): a runner's `OutsideRuns` batch is stored
//! once per entry and acknowledged with its offset; an entry without `--check` sets the host's
//! `changed_outside` and pushes `HostChangedOutside`; a `--check` entry does not; a malformed
//! entry is counted, not stored; the table is insert-only; each batch is audited; `GetHost` shows
//! the newest entries and `ListHosts` filters on the flag. Clearing it is covered where the runs
//! that clear it are: `runs.rs` (every assigned job ran since) and `signed.rs` (a baseline).

mod common;
mod repos;

use std::time::Duration;

use keel_exec::codec::{read_frame, write_frame};
use keel_exec::frames::{Frame, OutsideRun};
use keel_protocol::{Cmd, Event};
use repos::Harness;
use tokio::io::DuplexStream;
use uuid::Uuid;

fn entry(at: &str, playbook: &str, check: bool) -> OutsideRun {
    OutsideRun {
        at: at.into(),
        user: "root".into(),
        playbook: playbook.into(),
        commit: Some("a".repeat(40)),
        dirty: false,
        check,
    }
}

async fn host(h: &Harness, name: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into hosts (id, workspace_id, inventory_name, allowed_profiles) \
         values ($1, $2, $3, '{}')",
    )
    .bind(id)
    .bind(h.workspace)
    .bind(name)
    .execute(&h.db.owner)
    .await
    .expect("host");
    id
}

/// Attaches a hand-driven runner for `id`; returns its end of the stream.
async fn attach(h: &Harness, id: Uuid) -> DuplexStream {
    let (ours, theirs) = tokio::io::duplex(1 << 16);
    let mut ours = ours;
    write_frame(
        &mut ours,
        &Frame::Hello {
            host_id: id.to_string(),
            version: "0.1.0".into(),
            ansible_core: "2.21.4".into(),
            python: "3.13".into(),
            allowed_profiles: vec![],
            require_signed: false,
            override_quorum: 1,
            ledger_head: None,
            bases: vec![],
        },
    )
    .await
    .expect("hello");
    keel_server::queue::attach(h.app.clone(), id, theirs)
        .await
        .expect("attach");
    ours
}

/// Sends a batch and waits for its acknowledgement, skipping any other frame.
async fn send(s: &mut DuplexStream, entries: Vec<OutsideRun>, skipped: u32, through: u64) {
    write_frame(
        s,
        &Frame::OutsideRuns {
            entries,
            skipped,
            through,
        },
    )
    .await
    .expect("send");
    let ack = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match read_frame(s).await.expect("read") {
                Some(Frame::OutsideRunsAck { through }) => return through,
                Some(_) => {}
                None => panic!("the server closed the session"),
            }
        }
    })
    .await
    .expect("an acknowledgement");
    assert_eq!(ack, through);
}

async fn flagged(h: &Harness, id: Uuid) -> bool {
    let (f,): (bool,) = sqlx::query_as("select changed_outside from hosts where id = $1")
        .bind(id)
        .fetch_one(&h.db.owner)
        .await
        .expect("host");
    f
}

async fn stored(h: &Harness, id: Uuid) -> i64 {
    let (n,): (i64,) = sqlx::query_as("select count(*) from host_outside_runs where host_id = $1")
        .bind(id)
        .fetch_one(&h.db.owner)
        .await
        .expect("count");
    n
}

#[tokio::test]
async fn a_batch_is_stored_once_acknowledged_and_flags_the_host() {
    let Some(h) = Harness::new("outside_runs_flag").await else {
        return;
    };
    let web = host(&h, "web-1").await;
    let db = host(&h, "db-1").await;
    let mut events = h.app.repos.events.subscribe();
    let mut s = attach(&h, web).await;

    // A `--check` run changes nothing on the host: stored, not flagged.
    send(
        &mut s,
        vec![entry("2026-10-27T10:00:00Z", "site.yaml", true)],
        0,
        120,
    )
    .await;
    assert_eq!(stored(&h, web).await, 1);
    assert!(!flagged(&h, web).await);

    // A real run flags it; a malformed entry is counted, not stored.
    let mut bad = entry("not a time", "site.yaml", false);
    bad.user = "x".repeat(65);
    send(
        &mut s,
        vec![
            entry("2026-10-27T11:00:00Z", "playbooks/debian.yaml", false),
            bad,
        ],
        2,
        480,
    )
    .await;
    assert_eq!(stored(&h, web).await, 2);
    assert!(flagged(&h, web).await);
    assert!(!flagged(&h, db).await);
    let pushed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok((_, Event::HostChangedOutside { host })) = events.recv().await {
                return host;
            }
        }
    })
    .await
    .expect("HostChangedOutside");
    assert_eq!(pushed, web);

    // A re-sent batch (the runner re-read its record) stores nothing new and is not audited.
    let audited = h.audit_actions().await.len();
    send(
        &mut s,
        vec![entry(
            "2026-10-27T11:00:00Z",
            "playbooks/debian.yaml",
            false,
        )],
        0,
        480,
    )
    .await;
    assert_eq!(stored(&h, web).await, 2);
    assert_eq!(h.audit_actions().await.len(), audited);
    let reports: Vec<_> = h
        .audit_actions()
        .await
        .into_iter()
        .filter(|a| a.0 == "host_outside_runs_reported")
        .collect();
    assert_eq!(reports.len(), 2, "{reports:?}");
    let audit = h.audit_text().await;
    assert!(audit.contains("\"skipped\":3"), "{audit}");

    // GetHost shows the host's claims, newest first; ListHosts filters on the flag.
    let out = h.cmd(Cmd::GetHost { host: web }).await;
    let [Event::Host(d)] = out.as_slice() else {
        panic!("host: {out:?}");
    };
    assert!(d.host.changed_outside);
    let books: Vec<_> = d.outside_runs.iter().map(|r| r.playbook.as_str()).collect();
    assert_eq!(books, ["playbooks/debian.yaml", "site.yaml"]);
    assert!(!d.outside_runs[0].check && d.outside_runs[1].check);
    for (want, names) in [
        (Some(true), vec!["web-1"]),
        (Some(false), vec!["db-1"]),
        (None, vec!["db-1", "web-1"]),
    ] {
        let out = h
            .cmd(Cmd::ListHosts {
                workspace: h.workspace,
                profile: None,
                drift: None,
                changed_outside: want,
            })
            .await;
        let [Event::Hosts { items }] = out.as_slice() else {
            panic!("hosts: {out:?}");
        };
        let mut got: Vec<_> = items.iter().map(|i| i.inventory_name.as_str()).collect();
        got.sort_unstable();
        assert_eq!(got, names, "{want:?}");
    }
    h.finish().await;
}

#[tokio::test]
async fn host_outside_runs_is_insert_only() {
    let Some(h) = Harness::new("outside_runs_insert_only").await else {
        return;
    };
    let web = host(&h, "web-1").await;
    keel_server::outside::ingest(
        &h.app,
        web,
        &[entry("2026-10-27T11:00:00Z", "site.yaml", false)],
        0,
    )
    .await
    .expect("ingest");
    for sql in [
        "update host_outside_runs set dirty = true",
        "delete from host_outside_runs",
        "truncate host_outside_runs",
    ] {
        let err = sqlx::query(sql).execute(&h.db.app).await.unwrap_err();
        let code = err
            .as_database_error()
            .and_then(|e| e.code())
            .unwrap_or_default();
        assert_eq!(code, "42501", "{sql}: {err}");
        let err = sqlx::query(sql).execute(&h.db.owner).await.unwrap_err();
        assert!(err.to_string().contains("insert-only"), "{sql}: {err}");
    }
    // The database holds the same bounds as the frame.
    let err = sqlx::query(
        "insert into host_outside_runs (host_id, at, \"user\", playbook, commit, dirty, \"check\") \
         values ($1, now(), 'root', 'site.yaml', 'ABC', false, false)",
    )
    .bind(web)
    .execute(&h.db.app)
    .await
    .unwrap_err();
    assert!(err.to_string().contains("check"), "{err}");
    h.finish().await;
}
