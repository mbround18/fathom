//! Instance settings (spec 060 US2, T026): admins read and save the enrolment snippet template
//! and instructions with step-up; a stale version is `settings_conflict`; invalid templates are
//! refused with their code; the save is audited with the text before and after.
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`); skips without it.

mod common;
mod repos;

use keel_core::enroll_template::DEFAULT_TEMPLATE;
use keel_protocol::{Cmd, Event};
use repos::{Harness, op_error, preview, sign_in};
use serde_json::Value;

struct Got {
    template: String,
    instructions: String,
    version: i64,
    is_default: bool,
    runner_address: Option<String>,
    public_url: String,
}

fn settings(events: &[Event]) -> Got {
    match events.iter().find(|e| matches!(e, Event::Settings { .. })) {
        Some(Event::Settings {
            template,
            instructions,
            version,
            is_default,
            runner_address,
            public_url,
            role_mapping,
            warnings,
        }) => {
            assert!(role_mapping.is_none());
            assert!(warnings.is_empty());
            Got {
                template: template.clone(),
                instructions: instructions.clone(),
                version: *version,
                is_default: *is_default,
                runner_address: runner_address.clone(),
                public_url: public_url.clone(),
            }
        }
        _ => panic!("expected settings, got {events:?}"),
    }
}

fn update(template: &str, instructions: &str, version: i64) -> Cmd {
    Cmd::PreviewSettingsUpdate {
        template: template.to_owned(),
        instructions: instructions.to_owned(),
        version,
    }
}

async fn last_context(h: &Harness, action: &str) -> Value {
    let (v,): (Value,) = sqlx::query_as(
        "select context from audit_event where action = $1 and decision = 'allowed' \
         order by seq desc limit 1",
    )
    .bind(action)
    .fetch_one(&h.db.owner)
    .await
    .expect("audit row");
    v
}

#[tokio::test]
async fn defaults_then_first_save_then_update() {
    let Some(h) = Harness::new("settings_save").await else {
        return;
    };
    let s = settings(&h.cmd(Cmd::GetSettings).await);
    assert_eq!(s.template, DEFAULT_TEMPLATE);
    assert!(s.is_default);
    assert_eq!(s.instructions, "");
    assert_eq!(s.version, 0);
    assert_eq!(s.runner_address.as_deref(), Some("keel.example.com:7831"));
    assert_eq!(s.public_url, "https://keel.example.com");

    // The first save inserts the row, at version 1, with step-up.
    let (p, out) = h
        .confirm(update("curl -s %u | sh -s %t", "Run as root.", 0))
        .await;
    assert!(p.step_up);
    assert_eq!(p.action, "settings_update");
    let s = settings(&out);
    assert_eq!(s.template, "curl -s %u | sh -s %t");
    assert!(!s.is_default);
    assert_eq!(s.instructions, "Run as root.");
    assert_eq!(s.version, 1);

    let ctx = last_context(&h, "settings_update").await;
    assert_eq!(ctx["before"]["template"], DEFAULT_TEMPLATE);
    assert_eq!(ctx["before"]["instructions"], "");
    assert_eq!(ctx["after"]["template"], "curl -s %u | sh -s %t");
    assert_eq!(ctx["after"]["instructions"], "Run as root.");
    assert_eq!(ctx["version"], 1);

    // The next save updates it.
    let (_, out) = h.confirm(update("%t", "", 1)).await;
    let s = settings(&out);
    assert_eq!((s.template.as_str(), s.version), ("%t", 2));
    let ctx = last_context(&h, "settings_update").await;
    assert_eq!(ctx["before"]["template"], "curl -s %u | sh -s %t");
    assert_eq!(ctx["after"]["template"], "%t");
    h.finish().await;
}

#[tokio::test]
async fn a_stale_version_is_a_conflict() {
    let Some(h) = Harness::new("settings_conflict").await else {
        return;
    };
    h.confirm(update("%t", "", 0)).await;
    // Previewed against version 0 again: someone saved first.
    for stale in [0, 5] {
        let p = preview(&h.cmd(update("echo %t", "", stale)).await);
        let out = h
            .cmd(Cmd::Confirm {
                preview_id: p.preview_id,
                step_up: None,
            })
            .await;
        assert!(
            out.iter()
                .any(|e| matches!(e, Event::OpError { code, .. } if code == "settings_conflict")),
            "{out:?}"
        );
    }
    let s = settings(&h.cmd(Cmd::GetSettings).await);
    assert_eq!((s.template.as_str(), s.version), ("%t", 1));
    h.finish().await;
}

#[tokio::test]
async fn invalid_templates_and_instructions_are_refused() {
    let Some(h) = Harness::new("settings_invalid").await else {
        return;
    };
    assert_eq!(
        op_error(&h.cmd(update("no token", "", 0)).await),
        "missing_token"
    );
    assert_eq!(
        op_error(&h.cmd(update("%t %x", "", 0)).await),
        "unknown_placeholder"
    );
    let long = format!("%t{}", "a".repeat(4096));
    assert_eq!(op_error(&h.cmd(update(&long, "", 0)).await), "too_long");
    let notes = "a".repeat(20 * 1024 + 1);
    assert_eq!(
        op_error(&h.cmd(update("%t", &notes, 0)).await),
        "instructions_too_long"
    );
    // Exactly at the cap is fine.
    preview(&h.cmd(update("%t", &"a".repeat(20 * 1024), 0)).await);
    h.finish().await;
}

#[tokio::test]
async fn users_are_denied() {
    let Some(h) = Harness::new("settings_denied").await else {
        return;
    };
    let bob = sign_in(&h.app, "bob", "bob").await;
    for cmd in [Cmd::GetSettings, update("%t", "", 0)] {
        let out = h.cmd_as(&bob, cmd).await;
        assert!(matches!(out.as_slice(), [Event::Denied { .. }]), "{out:?}");
    }
    // An admin may.
    sqlx::query("update users set instance_role = 'admin' where subject = 'bob'")
        .execute(&h.db.owner)
        .await
        .expect("promote");
    settings(&h.cmd_as(&bob, Cmd::GetSettings).await);
    let (count,): (i64,) =
        sqlx::query_as("select count(*) from audit_event where action = 'get_settings'")
            .fetch_one(&h.db.owner)
            .await
            .expect("audit");
    assert_eq!(count, 1, "the refused read is audited");
    h.finish().await;
}
