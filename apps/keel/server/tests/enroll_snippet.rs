//! The enrolment snippet (spec 060 US1, T010): a confirmed `PreviewHostEnroll` replies the
//! instance template rendered with the real token, plus the instructions. With no settings row
//! the default template applies; when the settings can't be read, the defaults are used and
//! `instructions_unavailable` says so.
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`); skips without it.

mod common;
mod repos;

use keel_core::enroll_template::DEFAULT_TEMPLATE;
use keel_protocol::{Cmd, Event, HostName, HostProfiles, ProfileName};
use keel_server::hosts;
use repos::Harness;
use uuid::Uuid;

async fn harness(test: &str) -> Option<Harness> {
    let h = Harness::new(test).await?;
    hosts::bootstrap(&h.app).await.expect("bootstrap");
    for name in ["debian", "web"] {
        sqlx::query("insert into profiles (id, workspace_id, name) values ($1, $2, $3)")
            .bind(Uuid::now_v7())
            .bind(h.workspace)
            .bind(name)
            .execute(&h.db.owner)
            .await
            .expect("profile");
    }
    Some(h)
}

struct Minted {
    token: String,
    expires_at: String,
    snippet: String,
    instructions: String,
    unavailable: bool,
}

async fn mint(h: &Harness, name: &str) -> Minted {
    let cmd = Cmd::PreviewHostEnroll {
        workspace: h.workspace,
        inventory_name: HostName::new(name).expect("name"),
        profiles: HostProfiles(vec![
            ProfileName::new("debian").expect("profile"),
            ProfileName::new("web").expect("profile"),
        ]),
    };
    let (_, out) = h.confirm(cmd).await;
    match out.as_slice() {
        [
            Event::Confirmed { .. },
            Event::HostEnrollment {
                token,
                expires_at,
                snippet,
                instructions,
                instructions_unavailable,
                ..
            },
        ] => Minted {
            token: token.0.clone(),
            expires_at: expires_at.clone(),
            snippet: snippet.clone(),
            instructions: instructions.clone(),
            unavailable: *instructions_unavailable,
        },
        other => panic!("enrol: {other:?}"),
    }
}

#[tokio::test]
async fn no_settings_row_renders_the_default_template() {
    let Some(h) = harness("snippet_default").await else {
        return;
    };
    let m = mint(&h, "web-1").await;
    assert!(DEFAULT_TEMPLATE.contains("%t"));
    assert_eq!(m.snippet, DEFAULT_TEMPLATE.replace("%t", &m.token));
    assert_eq!(m.instructions, "");
    assert!(!m.unavailable);
    // The token reaches the reply only: not the audit log.
    assert!(!h.audit_text().await.contains(&m.token));
    h.finish().await;
}

#[tokio::test]
async fn a_saved_template_is_rendered_with_every_placeholder() {
    let Some(h) = harness("snippet_saved").await else {
        return;
    };
    sqlx::query(
        "insert into instance_settings (enroll_template, enroll_instructions) values ($1, $2)",
    )
    .bind("t=%t n=%n p=%p w=%w s=%s u=%u e=%e %%t")
    .bind("## Before you start\n\nRun it as **root**.")
    .execute(&h.db.owner)
    .await
    .expect("settings");
    let m = mint(&h, "web-2").await;
    assert_eq!(
        m.snippet,
        format!(
            "t={} n=web-2 p=debian,web w=ops s=keel.example.com:7831 u=https://keel.example.com \
             e={} %t",
            m.token, m.expires_at
        )
    );
    assert_eq!(m.instructions, "## Before you start\n\nRun it as **root**.");
    assert!(!m.unavailable);
    h.finish().await;
}

#[tokio::test]
async fn unreadable_settings_fall_back_to_the_defaults() {
    let Some(h) = harness("snippet_unavailable").await else {
        return;
    };
    sqlx::query(
        "insert into instance_settings (enroll_template, enroll_instructions) values ($1, $2)",
    )
    .bind("custom %t")
    .bind("custom instructions")
    .execute(&h.db.owner)
    .await
    .expect("settings");
    // Make the read fail for the serving role.
    sqlx::query("revoke select on instance_settings from keel_app")
        .execute(&h.db.owner)
        .await
        .expect("revoke");
    let m = mint(&h, "web-3").await;
    assert_eq!(m.snippet, DEFAULT_TEMPLATE.replace("%t", &m.token));
    assert_eq!(m.instructions, "");
    assert!(m.unavailable);
    h.finish().await;
}
