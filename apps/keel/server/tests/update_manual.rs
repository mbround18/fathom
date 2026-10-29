//! The manual update path (spec 061 US5, T078): when an update run can't be made, the owner's
//! status says what is missing and how to update by hand, for the digest on offer.
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`); skips without it. Nothing calls a registry.

mod common;
mod repos;
mod update_fixture;

use keel_protocol::{Cmd, Event, UpdateOutcome, UpdatePath};
use repos::Harness;
use update_fixture::{NEW, Setup, build, offer, running, system};

async fn path(h: &Harness) -> UpdatePath {
    match h.cmd(Cmd::GetUpdateStatus).await.as_slice() {
        [Event::UpdateStatus(s)] => s.update_path.clone(),
        other => panic!("{other:?}"),
    }
}

fn assert_manual(path: &UpdatePath, want: &str) {
    let UpdatePath::Manual {
        missing,
        digest_ref,
        steps,
    } = path
    else {
        panic!("expected the manual path for {want}, got {path:?}");
    };
    assert_eq!(missing, want);
    // The default watched image is docker.io/mbround18/keel:latest.
    assert_eq!(digest_ref, &format!("mbround18/keel@{NEW}"));
    assert_eq!(
        steps,
        &[
            format!("With the Ansible repository: make keel-server KEEL_DIGEST={NEW}"),
            format!("Or with plain compose: set the keel service's image: to mbround18/keel@{NEW}"),
            "docker compose pull".to_owned(),
            "docker compose up -d".to_owned(),
        ]
    );
}

#[tokio::test]
async fn each_missing_prerequisite_gives_the_manual_path() {
    let Some(mut h) = Harness::new("upd_manual").await else {
        return;
    };
    running(&mut h, build(), false);
    offer(&h, UpdateOutcome::UpdateAvailable, NEW);
    assert_manual(&path(&h).await, "no_system_workspace");

    system(
        &h,
        Setup {
            host: false,
            ..Setup::FULL
        },
    )
    .await;
    assert_manual(&path(&h).await, "no_keel_host");
    h.finish().await;
}

#[tokio::test]
async fn a_disallowed_profile_gives_the_manual_path() {
    let Some(mut h) = Harness::new("upd_manual_profile").await else {
        return;
    };
    running(&mut h, build(), false);
    offer(&h, UpdateOutcome::UpdateAvailable, NEW);
    system(&h, Setup::FULL).await;
    assert_eq!(path(&h).await, UpdatePath::Run);
    sqlx::query("update system_config set allowed_profiles = '{debian}'")
        .execute(&h.db.owner)
        .await
        .expect("allowed");
    assert_manual(&path(&h).await, "profile_not_allowed");
    h.finish().await;
}

#[tokio::test]
async fn a_job_without_the_param_gives_the_manual_path() {
    let Some(mut h) = Harness::new("upd_manual_param").await else {
        return;
    };
    running(&mut h, build(), false);
    offer(&h, UpdateOutcome::UpdateAvailable, NEW);
    system(
        &h,
        Setup {
            param: false,
            ..Setup::FULL
        },
    )
    .await;
    assert_manual(&path(&h).await, "job_missing_param");
    h.finish().await;
}
