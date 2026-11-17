//! Spec 068 T052: an `env_var` secret item lands in `ansible-playbook`'s environment (which is
//! otherwise cleared), a reserved name or one the job's `env:` does not ask for is refused
//! `env_refused` before anything runs, and the value is never in the run's log.

mod common;

use common::{Host, POLICY, assign, logs, repo, run};
use keel_exec::frames::{Frame, Refusal, SecretItem, SecretString};

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
    env: { CF_TOKEN: cf-token }
"#;

const VALUE: &str = "canary-6f1c2a9e value";

fn env_item(name: &str, env_name: &str) -> SecretItem {
    SecretItem {
        name: name.to_owned(),
        kind: "env_var".to_owned(),
        value: SecretString::new(VALUE),
        env_name: Some(env_name.to_owned()),
    }
}

async fn go(host: &Host, run_id: &str, items: Vec<SecretItem>) -> Vec<Frame> {
    let (commits, pack) = repo(&host.root, MANIFEST, &[], None);
    run(
        host,
        assign(run_id, "web", None, &commits[0], &pack),
        &pack,
        items,
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
async fn an_env_var_item_is_set_for_the_playbook_and_never_logged() {
    let host = Host::new(POLICY);
    let frames = go(&host, "run-e1", vec![env_item("cf-token", "CF_TOKEN")]).await;
    assert!(
        matches!(frames.last(), Some(Frame::Finished { exit_code: 0, .. })),
        "{frames:?}"
    );
    let record = host.record();
    assert!(
        record.lines().any(|l| l == format!("env:CF_TOKEN={VALUE}")),
        "{record}"
    );
    // The runner's own environment is cleared first: nothing cargo set reaches the playbook.
    assert!(
        !record.lines().any(|l| l.starts_with("env:CARGO")),
        "{record}"
    );
    assert!(
        !logs(&frames).contains("canary-6f1c2a9e"),
        "the value was logged"
    );
    assert!(!format!("{frames:?}").contains("canary-6f1c2a9e"));
}

#[tokio::test]
async fn a_reserved_name_is_refused_before_anything_runs() {
    let host = Host::new(POLICY);
    let frames = go(&host, "run-e2", vec![env_item("cf-token", "LD_PRELOAD")]).await;
    assert_eq!(refused(&frames), Some(Refusal::EnvRefused), "{frames:?}");
    assert!(host.record().is_empty(), "the playbook ran");
    assert!(!format!("{frames:?}").contains("canary-6f1c2a9e"));
}

#[tokio::test]
async fn a_name_the_job_does_not_ask_for_is_refused() {
    let host = Host::new(POLICY);
    let frames = go(
        &host,
        "run-e3",
        vec![
            env_item("cf-token", "CF_TOKEN"),
            env_item("other", "OTHER_TOKEN"),
        ],
    )
    .await;
    assert_eq!(refused(&frames), Some(Refusal::EnvRefused), "{frames:?}");
    assert!(host.record().is_empty(), "the playbook ran");
}

#[tokio::test]
async fn an_env_credential_not_sent_fails_the_run() {
    let host = Host::new(POLICY);
    let frames = go(&host, "run-e4", vec![]).await;
    assert!(refused(&frames).is_none(), "{frames:?}");
    assert!(
        !matches!(frames.last(), Some(Frame::Finished { exit_code: 0, .. })),
        "{frames:?}"
    );
    assert!(host.record().is_empty(), "the playbook ran");
}
