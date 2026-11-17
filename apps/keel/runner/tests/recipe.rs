//! Spec 068 T025: a manifest with recipes (`includes`, `jobs`), `groups`, `covers` and `env`
//! parses on the runner, and a converge step whose job is outside the host's ceiling is refused
//! `profile_not_allowed` even though a recipe the host may run includes it.

mod common;

use common::{Host, POLICY, assign, repo, run};
use keel_exec::frames::{Frame, Refusal};

const MANIFEST: &str = r#"
version: 1
profiles:
  debian: { jobs: [baseline] }
  docker:
    includes: [debian]
    jobs: [docker-install]
inventories:
  prod: { path: inventories/prod/hosts.yaml }
groups:
  web: [docker]
jobs:
  baseline: { playbook: playbooks/web.yaml, profiles: [debian], inventory: prod }
  docker-install:
    playbook: playbooks/web.yaml
    profiles: [docker]
    inventory: prod
    covers: [baseline]
    env: { DOCKER_TOKEN: docker-token }
"#;

#[tokio::test]
async fn a_recipe_manifest_runs_a_step_inside_the_ceiling() {
    let host = Host::new(POLICY);
    let (commits, pack) = repo(&host.root, MANIFEST, &[], None);
    let frames = run(
        &host,
        assign("run-1", "baseline", None, &commits[0], &pack),
        &pack,
        vec![],
        false,
    )
    .await;
    assert!(
        matches!(frames.last(), Some(Frame::Finished { exit_code: 0, .. })),
        "{frames:?}"
    );
    assert!(!host.record().is_empty(), "the playbook did not run");
}

#[tokio::test]
async fn a_step_outside_the_ceiling_is_refused() {
    let host = Host::new(POLICY);
    let (commits, pack) = repo(&host.root, MANIFEST, &[], None);
    let frames = run(
        &host,
        assign("run-2", "docker-install", None, &commits[0], &pack),
        &pack,
        vec![],
        false,
    )
    .await;
    assert!(
        frames.iter().any(|f| matches!(
            f,
            Frame::Refused {
                reason: Refusal::ProfileNotAllowed,
                ..
            }
        )),
        "{frames:?}"
    );
    assert!(host.record().is_empty(), "the playbook ran");
}
