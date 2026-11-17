//! Spec 068 T062: the runner re-checks a `choice` param against its own checkout of the target.
//! A value that is one of the choices runs; a well-formed value that is not (say, a `from:`
//! directory the server saw but the commit does not have) is refused `choice_mismatch` before
//! anything runs, and the refusal never echoes the value.

mod common;

use std::collections::BTreeMap;

use common::{Host, POLICY, assign, repo, run};
use keel_exec::frames::{Frame, Refusal};

const MANIFEST: &str = r#"
version: 1
profiles:
  debian: {}
inventories:
  prod: { path: inventories/prod/hosts.yaml }
jobs:
  web:
    playbook: playbooks/web.yaml
    profiles: [debian]
    inventory: prod
    params:
      stack: { kind: choice, from: composes }
      size: { kind: choice, values: [small, large], default: small }
"#;

const FILES: &[(&str, &str)] = &[
    ("composes/web/compose.yaml", "services: {}\n"),
    ("composes/db/compose.yaml", "services: {}\n"),
    ("composes/.hidden/compose.yaml", "services: {}\n"),
];

fn with_params(a: Frame, pairs: &[(&str, &str)]) -> Frame {
    let Frame::Assign {
        run_id,
        workspace,
        binding,
        job,
        base,
        target,
        deadline,
        pack_size,
        quorum,
        check,
        ..
    } = a
    else {
        unreachable!()
    };
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
        check,
        params: pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect::<BTreeMap<_, _>>(),
    }
}

async fn go(host: &Host, run_id: &str, params: &[(&str, &str)]) -> Vec<Frame> {
    let (commits, pack) = repo(&host.root, MANIFEST, FILES, None);
    run(
        host,
        with_params(assign(run_id, "web", None, &commits[0], &pack), params),
        &pack,
        vec![],
        false,
    )
    .await
}

fn refused(frames: &[Frame]) -> Option<Refusal> {
    frames.iter().find_map(|f| match f {
        Frame::Refused { reason, .. } => Some(*reason),
        _ => None,
    })
}

#[tokio::test]
async fn values_that_are_choices_at_the_commit_run() {
    let host = Host::new(POLICY);
    let frames = go(&host, "run-ch1", &[("stack", "db"), ("size", "large")]).await;
    assert!(
        matches!(frames.last(), Some(Frame::Finished { exit_code: 0, .. })),
        "{frames:?}"
    );
    assert!(!host.record().is_empty(), "the playbook did not run");
}

#[tokio::test]
async fn a_directory_missing_at_the_commit_is_refused_choice_mismatch() {
    let host = Host::new(POLICY);
    let frames = go(&host, "run-ch2", &[("stack", "cache-zz9")]).await;
    assert_eq!(
        refused(&frames),
        Some(Refusal::ChoiceMismatch),
        "{frames:?}"
    );
    assert!(host.record().is_empty(), "the playbook ran");
    assert!(
        !format!("{frames:?}").contains("cache-zz9"),
        "the value was echoed"
    );
}

#[tokio::test]
async fn a_hidden_directory_or_a_value_outside_the_list_is_refused() {
    let host = Host::new(POLICY);
    let frames = go(&host, "run-ch3", &[("size", "medium")]).await;
    assert_eq!(
        refused(&frames),
        Some(Refusal::ChoiceMismatch),
        "{frames:?}"
    );
    assert!(host.record().is_empty(), "the playbook ran");
    // `.hidden` is not even a well-formed choice value: the kind check refuses it first.
    let host = Host::new(POLICY);
    let frames = go(&host, "run-ch4", &[("stack", ".hidden")]).await;
    assert_eq!(
        refused(&frames),
        Some(Refusal::ParamsMismatch),
        "{frames:?}"
    );
    assert!(host.record().is_empty(), "the playbook ran");
}
