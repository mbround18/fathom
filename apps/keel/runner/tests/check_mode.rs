//! Spec 068 T047: a check-mode run (`Assign.check`) runs `ansible-playbook --check --diff`; in
//! signed mode its ledger `end` line is `checked`, the job's base does not move, and its run id
//! is still spent.

mod common;

use common::{Host, POLICY, args, assign, repo, run, seeded_ledger, signing_key};
use keel_exec::frames::Frame;
use keel_trust::{CommitId, Ledger};

const MANIFEST: &str = r#"
version: 1
profiles:
  debian: {}
inventories:
  prod: { path: inventories/prod/hosts.yaml }
jobs:
  web: { playbook: playbooks/web.yaml, profiles: [debian], inventory: prod }
"#;

fn checked(a: Frame) -> Frame {
    match a {
        Frame::Assign {
            run_id,
            workspace,
            binding,
            job,
            base,
            target,
            deadline,
            pack_size,
            quorum,
            params,
            ..
        } => Frame::Assign {
            run_id,
            workspace,
            binding,
            job,
            base,
            target,
            deadline,
            pack_size,
            quorum,
            params,
            check: true,
        },
        other => other,
    }
}

#[tokio::test]
async fn a_check_run_adds_check_and_diff_before_the_playbook() {
    let host = Host::new(POLICY);
    let (commits, pack) = repo(&host.root, MANIFEST, &[], None);
    let frames = run(
        &host,
        checked(assign("run-c1", "web", None, &commits[0], &pack)),
        &pack,
        vec![],
        false,
    )
    .await;
    assert!(
        matches!(frames.last(), Some(Frame::Finished { exit_code: 0, .. })),
        "{frames:?}"
    );
    let record = host.record();
    let argv = args(&record);
    let n = argv.len();
    assert!(n >= 3, "{argv:?}");
    assert_eq!(&argv[n - 3..], ["--check", "--diff", "playbooks/web.yaml"]);
}

#[tokio::test]
async fn a_normal_run_has_no_check_flags() {
    let host = Host::new(POLICY);
    let (commits, pack) = repo(&host.root, MANIFEST, &[], None);
    run(
        &host,
        assign("run-c2", "web", None, &commits[0], &pack),
        &pack,
        vec![],
        false,
    )
    .await;
    let record = host.record();
    let argv = args(&record);
    assert!(
        !argv.contains(&"--check") && !argv.contains(&"--diff"),
        "{argv:?}"
    );
}

#[tokio::test]
async fn a_signed_check_run_ends_checked_and_leaves_the_base() {
    let host = Host::new(&format!("require_signed = true\n{POLICY}"));
    let (key, signer) = signing_key(&host.root);
    std::fs::write(host.cfg.trust_dir.join("signers"), format!("{signer}\n")).unwrap();
    std::fs::write(host.cfg.trust_dir.join("approvers.json"), "[]").unwrap();
    let (commits, pack) = repo(&host.root, MANIFEST, &[], Some(&key));
    let (base, target) = (commits[0].clone(), commits[1].clone());
    let seed = seeded_ledger("web", &base);
    std::fs::write(host.cfg.state_dir.join("ledger.jsonl"), &seed).unwrap();

    let frames = run(
        &host,
        checked(assign("run-c3", "web", Some(&base), &target, &pack)),
        &pack,
        vec![],
        true,
    )
    .await;
    let Some(Frame::Finished {
        exit_code: 0,
        ledger_line,
        ..
    }) = frames.last()
    else {
        panic!("{frames:?}");
    };
    assert!(
        ledger_line.contains(r#""outcome":"checked""#),
        "{ledger_line}"
    );
    let text = host.ledger();
    assert!(text.starts_with(&seed), "the ledger was rewritten");
    let ledger = Ledger::parse(&text).unwrap();
    assert_eq!(
        ledger.base("web"),
        Some(CommitId::parse(&base).unwrap()),
        "a check run moved the base"
    );
    assert!(ledger.has_start("run-c3"));

    // The same run id again, now as a real run: a replay, refused, and the ledger is unchanged.
    let frames = run(
        &host,
        assign("run-c3", "web", Some(&base), &target, &pack),
        &pack,
        vec![],
        true,
    )
    .await;
    assert!(
        frames.iter().any(|f| matches!(f, Frame::Refused { .. })),
        "{frames:?}"
    );
    assert_eq!(host.ledger(), text);
}
