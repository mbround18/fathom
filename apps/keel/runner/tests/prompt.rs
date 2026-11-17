//! Spec 068 T063 (FR-015): `ansible-playbook` runs in a new session with stdin at `/dev/null`,
//! so it has no controlling terminal and a play that prompts fails at once instead of hanging
//! the run until its deadline.

mod common;

use std::time::Duration;

use common::{Host, POLICY, assign, field, repo, run};
use keel_exec::frames::Frame;

const MANIFEST: &str = r#"
version: 1
profiles:
  debian: {}
inventories:
  prod: { path: inventories/prod/hosts.yaml }
jobs:
  web: { playbook: playbooks/web.yaml, profiles: [debian], inventory: prod }
"#;

#[tokio::test]
async fn a_prompting_play_fails_fast_with_no_terminal() {
    let host = Host::new(POLICY);
    let (commits, pack) = repo(&host.root, MANIFEST, &[("prompt.flag", "")], None);
    let frames = tokio::time::timeout(
        Duration::from_secs(60),
        run(
            &host,
            assign("run-p1", "web", None, &commits[0], &pack),
            &pack,
            vec![],
            false,
        ),
    )
    .await
    .expect("the run hung on a prompt");
    assert!(
        matches!(frames.last(), Some(Frame::Finished { exit_code, .. }) if *exit_code != 0),
        "{frames:?}"
    );
    let record = host.record();
    let pid = field(&record, "pid").expect("pid");
    assert_eq!(
        field(&record, "sid"),
        Some(pid),
        "not a session leader: {record}"
    );
    assert_eq!(
        field(&record, "pgrp"),
        Some(pid),
        "not a group leader: {record}"
    );
    assert_eq!(field(&record, "stdin"), Some("/dev/null"), "{record}");
}
