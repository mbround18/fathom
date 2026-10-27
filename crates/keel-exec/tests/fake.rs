//! T067: the `fake` runner answers Assign with its script.
#![cfg(feature = "fake")]

use keel_exec::codec::{read_frame, write_frame};
use keel_exec::events::HostStats;
use keel_exec::fake::{CANCELLED_EXIT_CODE, FakeRunner, Script};
use keel_exec::frames::{Frame, LogStream, Refusal};
use tokio::io::DuplexStream;

fn assign(run_id: &str, job: &str, pack_size: u64) -> Frame {
    Frame::Assign {
        run_id: run_id.into(),
        job: job.into(),
        base: None,
        target: "a".repeat(40),
        deadline: 1_900_000_000,
        pack_size,
    }
}

async fn next(s: &mut DuplexStream) -> Frame {
    read_frame(s).await.unwrap().expect("a frame")
}

#[tokio::test]
async fn hello_then_a_scripted_run() {
    let stats = HostStats {
        ok: 2,
        changed: 1,
        ..Default::default()
    };
    let fake = FakeRunner::new("h-1").script(
        "site",
        Script::finish(0)
            .event(r#"{"v":1,"event":"playbook_start","playbook":"site.yml"}"#)
            .log(LogStream::Stdout, b"PLAY [all]\n".to_vec())
            .stats("web-1", stats)
            .ledger_line(r#"{"seq":1}"#),
    );
    let (mut s, mut handle) = fake.spawn();

    let Frame::Hello { host_id, .. } = next(&mut s).await else {
        panic!("Hello first")
    };
    assert_eq!(host_id, "h-1");

    write_frame(&mut s, &assign("r-1", "site", 3))
        .await
        .unwrap();
    write_frame(
        &mut s,
        &Frame::Pack {
            run_id: "r-1".into(),
            chunk: vec![1, 2],
            last: false,
            sha256: None,
        },
    )
    .await
    .unwrap();
    write_frame(
        &mut s,
        &Frame::Pack {
            run_id: "r-1".into(),
            chunk: vec![3],
            last: true,
            sha256: Some("x".into()),
        },
    )
    .await
    .unwrap();

    assert_eq!(
        next(&mut s).await,
        Frame::Started {
            run_id: "r-1".into()
        }
    );
    let Frame::Event { seq: 1, json, .. } = next(&mut s).await else {
        panic!("event")
    };
    assert!(json.contains("playbook_start"));
    let Frame::Log {
        seq: 2,
        stream: LogStream::Stdout,
        ..
    } = next(&mut s).await
    else {
        panic!("log")
    };
    let Frame::Finished {
        exit_code: 0,
        stats: got,
        ledger_line,
        ..
    } = next(&mut s).await
    else {
        panic!("finished")
    };
    assert_eq!(got["web-1"], stats);
    assert_eq!(ledger_line, r#"{"seq":1}"#);

    drop(s);
    handle.task.await.unwrap().unwrap();
    let mut names = Vec::new();
    while let Ok(f) = handle.received.try_recv() {
        names.push(f.name());
    }
    assert_eq!(names, ["Assign", "Pack", "Pack"]);
}

#[tokio::test]
async fn refusal_hang_cancel_and_pings() {
    let fake = FakeRunner::new("h-2")
        .script("signed", Script::refuse(Refusal::Unsigned, "no signature"))
        .script("slow", Script::hang());
    let (mut s, _handle) = fake.spawn();
    let _hello = next(&mut s).await;

    write_frame(&mut s, &assign("r-1", "signed", 0))
        .await
        .unwrap();
    let Frame::Refused {
        reason: Refusal::Unsigned,
        ..
    } = next(&mut s).await
    else {
        panic!("refused")
    };

    write_frame(&mut s, &assign("r-2", "slow", 0))
        .await
        .unwrap();
    assert_eq!(
        next(&mut s).await,
        Frame::Started {
            run_id: "r-2".into()
        }
    );
    write_frame(&mut s, &Frame::Ping).await.unwrap();
    assert_eq!(next(&mut s).await, Frame::Pong);
    write_frame(
        &mut s,
        &Frame::Cancel {
            run_id: "r-2".into(),
        },
    )
    .await
    .unwrap();
    let Frame::Finished { exit_code, .. } = next(&mut s).await else {
        panic!("finished")
    };
    assert_eq!(exit_code, CANCELLED_EXIT_CODE);

    // Unscripted jobs finish with 0.
    write_frame(&mut s, &assign("r-3", "other", 0))
        .await
        .unwrap();
    assert_eq!(
        next(&mut s).await,
        Frame::Started {
            run_id: "r-3".into()
        }
    );
    let Frame::Finished { exit_code: 0, .. } = next(&mut s).await else {
        panic!("finished")
    };
}

#[tokio::test]
async fn a_lost_host_does_not_pong() {
    let (mut s, _handle) = FakeRunner::new("h-3").answer_pings(false).spawn();
    let _hello = next(&mut s).await;
    write_frame(&mut s, &Frame::Ping).await.unwrap();
    let r = tokio::time::timeout(std::time::Duration::from_millis(100), read_frame(&mut s)).await;
    assert!(r.is_err(), "no answer expected");
}
