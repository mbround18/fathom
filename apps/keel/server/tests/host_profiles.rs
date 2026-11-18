//! Spec 068 US3 (T037): editing a host's profiles behind step-up, the re-issued certificate and
//! its audit, and the suggestions a sync derives from the manifest's `groups:`.
//!
//! Needs Postgres (see `scripts/keel-test-db.sh`); skipped when it is not running.

mod common;
mod converge_support;
mod repos;

use converge_support::Fixture;
use keel_protocol::{
    Cmd, Event, HostProfilesChange, HostProfilesPreview, PreviewId, ProfileSuggestion,
    SuggestionAction,
};
use keel_server::sync::{Trigger, sync_binding};
use repos::op_error;
use uuid::Uuid;

fn change(host: Uuid, add: &[&str], remove: &[&str]) -> Cmd {
    Cmd::PreviewHostProfiles(HostProfilesChange {
        host_id: host,
        add: add.iter().map(|s| (*s).to_owned()).collect(),
        remove: remove.iter().map(|s| (*s).to_owned()).collect(),
    })
}

async fn preview(f: &Fixture, cmd: Cmd) -> (PreviewId, HostProfilesPreview) {
    let out = f.h.cmd(cmd).await;
    match out.as_slice() {
        [
            Event::HostProfilesPreview {
                preview_token,
                preview,
            },
            Event::Preview(p),
        ] => {
            assert_eq!(*preview_token, p.preview_id);
            assert!(p.step_up, "a profile change needs step-up");
            (*preview_token, preview.clone())
        }
        other => panic!("expected a profiles preview, got {other:?}"),
    }
}

async fn confirm(f: &Fixture, token: PreviewId) -> Vec<Event> {
    f.h.cmd(Cmd::Confirm {
        preview_id: token,
        step_up: None,
    })
    .await
}

async fn assigned(f: &Fixture, host: Uuid) -> Vec<(String, Option<Uuid>)> {
    sqlx::query_as(
        "select p.name, hp.assigned_by from host_profiles hp join profiles p \
         on p.id = hp.profile_id where hp.host_id = $1 order by p.name",
    )
    .bind(host)
    .fetch_all(&f.h.db.owner)
    .await
    .expect("host profiles")
}

/// Enrolls `node-2` (ceiling debian, keel-node; assigned keel-node) through the real
/// `enroll_key`, so it has a key and a certificate.
async fn enrolled(f: &Fixture) -> Uuid {
    use keel_exec::ca::{CaKind, SshCa};
    use keel_protocol::{HostName, HostProfiles, ProfileName};
    use keel_server::runner_listener::{EnrollRequest, enroll_key};

    keel_server::hosts::bootstrap(&f.h.app)
        .await
        .expect("bootstrap");
    let (_, out) =
        f.h.confirm(Cmd::PreviewHostEnroll {
            workspace: f.h.workspace,
            inventory_name: HostName::new("node-2").expect("name"),
            profiles: HostProfiles(vec![ProfileName::new("keel-node").expect("profile")]),
            auto_converge: Some(false),
        })
        .await;
    let token = match out.as_slice() {
        [Event::Confirmed { .. }, Event::HostEnrollment { token, .. }] => token.0.clone(),
        other => panic!("enroll: {other:?}"),
    };
    let key = SshCa::generate(CaKind::Runner)
        .expect("key")
        .key_data()
        .clone();
    let allowed = vec!["debian".to_owned(), "keel-node".to_owned()];
    let (host, _) = enroll_key(
        &f.h.app,
        &key,
        &EnrollRequest {
            token: &token,
            inventory_name: "node-2",
            allowed_profiles: &allowed,
            require_signed: false,
        },
    )
    .await
    .unwrap_or_else(|e| panic!("enroll: {e:?}"));
    host
}

/// The principals of `host`'s newest certificate, sorted.
async fn principals(f: &Fixture, host: Uuid) -> Vec<String> {
    let (mut p,): (Vec<String>,) = sqlx::query_as(
        "select principals from ssh_certs where host_id = $1 order by id desc limit 1",
    )
    .bind(host)
    .fetch_one(&f.h.db.owner)
    .await
    .expect("certificate");
    p.sort();
    p
}

#[tokio::test]
async fn profiles_change_behind_step_up_and_reissue_the_certificate() {
    let Some(f) = Fixture::new("host_profiles_set", false).await else {
        return;
    };
    let host = enrolled(&f).await;
    let certs_before: i64 = sqlx::query_scalar("select count(*) from ssh_certs where host_id = $1")
        .bind(host)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("certs");
    let cmd = || change(host, &["debian", "k3s-master"], &["keel-node"]);

    let (token, p) = preview(&f, cmd()).await;
    assert_eq!(p.change.add, vec!["debian", "k3s-master"]);
    assert_eq!(p.change.remove, vec!["keel-node"]);
    assert_eq!(
        p.outside_ceiling,
        vec!["k3s-master"],
        "outside the host's own ceiling"
    );
    assert_eq!(p.principals_after, vec!["host:node-2", "profile:debian"]);
    let jobs: Vec<&str> = p.plan_after.iter().map(|s| s.job.as_str()).collect();
    assert_eq!(jobs, vec!["debian", "k3s-master"], "{:?}", p.plan_after);

    // A stale session cannot confirm, and nothing changes.
    sqlx::query("update sessions set verified_at = null where id = $1")
        .bind(&f.h.owner.session)
        .execute(&f.h.db.owner)
        .await
        .expect("stale");
    // The owner has no passkey yet, so the step-up refusal asks for one.
    let refused = confirm(&f, token).await;
    assert!(
        matches!(refused.as_slice(), [Event::PasskeyEnrolmentRequired]),
        "{refused:?}"
    );
    assert_eq!(assigned(&f, host).await.len(), 1);

    f.h.step_up(&f.h.owner).await;
    let (token, _) = preview(&f, cmd()).await;
    match confirm(&f, token).await.as_slice() {
        [Event::Confirmed { .. }, Event::Host(d)] => {
            assert_eq!(d.host.profiles, vec!["debian", "k3s-master"])
        }
        other => panic!("confirm: {other:?}"),
    }
    let rows = assigned(&f, host).await;
    let names: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(names, vec!["debian", "k3s-master"]);
    assert!(
        rows.iter().all(|r| r.1.is_some()),
        "assigned_by recorded: {rows:?}"
    );

    // Re-issued: only the profiles inside the ceiling become principals.
    let certs_after: i64 = sqlx::query_scalar("select count(*) from ssh_certs where host_id = $1")
        .bind(host)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("certs");
    assert_eq!(certs_after, certs_before + 1);
    assert_eq!(
        principals(&f, host).await,
        vec!["host:node-2", "profile:debian"]
    );
    let (actor, context): (String, serde_json::Value) = sqlx::query_as(
        "select actor, context from audit_event where action = 'host.profiles_set' \
         and target->>'host' = $1",
    )
    .bind(host.to_string())
    .fetch_one(&f.h.db.owner)
    .await
    .expect("audited");
    assert!(actor.starts_with("user:"), "{actor}");
    assert_eq!(
        context["added"],
        serde_json::json!(["debian", "k3s-master"])
    );
    assert_eq!(context["removed"], serde_json::json!(["keel-node"]));
    assert!(context["serial"].is_string(), "{context}");

    // Removing it again drops the principal.
    let (token, _) = preview(&f, change(host, &[], &["debian"])).await;
    assert!(matches!(
        confirm(&f, token).await.first(),
        Some(Event::Confirmed { .. })
    ));
    assert_eq!(principals(&f, host).await, vec!["host:node-2"]);
    f.h.finish().await;
}

#[tokio::test]
async fn profile_change_refusals() {
    let Some(f) = Fixture::new("host_profiles_refused", false).await else {
        return;
    };
    for (cmd, code) in [
        (change(f.node, &["keel-node"], &[]), "no_change"),
        (change(f.node, &[], &["k3s-master"]), "no_change"),
        (change(f.node, &["nope"], &[]), "unknown_profile"),
        (change(f.node, &["debian"], &["debian"]), "invalid_change"),
        (change(Uuid::now_v7(), &["debian"], &[]), "not_found"),
    ] {
        assert_eq!(op_error(&f.h.cmd(cmd).await), code);
    }
    // A host with no key (never connected) still changes; no certificate to re-issue.
    let (token, p) = preview(&f, change(f.node, &["debian"], &[])).await;
    assert!(p.outside_ceiling.is_empty());
    assert!(matches!(
        confirm(&f, token).await.first(),
        Some(Event::Confirmed { .. })
    ));
    sqlx::query("update hosts set state = 'revoked' where id = $1")
        .bind(f.node)
        .execute(&f.h.db.owner)
        .await
        .expect("revoke");
    assert_eq!(
        op_error(&f.h.cmd(change(f.node, &[], &["debian"])).await),
        "host_revoked"
    );
    f.h.finish().await;
}

async fn set_groups(f: &Fixture, host: &str, groups: &[&str]) {
    sqlx::query("update inventory_hosts set groups = $2 where host = $1")
        .bind(host)
        .bind(groups.iter().map(|g| (*g).to_owned()).collect::<Vec<_>>())
        .execute(&f.h.db.owner)
        .await
        .expect("groups");
}

async fn sync(f: &Fixture) {
    sync_binding(&f.h.app, f.binding, Trigger::Poll)
        .await
        .unwrap_or_else(|e| panic!("sync: {e:?}"));
}

async fn listed(f: &Fixture, host: Option<Uuid>) -> Vec<ProfileSuggestion> {
    match f
        .h
        .cmd(Cmd::ListProfileSuggestions {
            workspace: f.h.workspace,
            host_id: host,
        })
        .await
        .as_slice()
    {
        [Event::ProfileSuggestions { items }] => items.clone(),
        other => panic!("list: {other:?}"),
    }
}

fn brief(items: &[ProfileSuggestion]) -> Vec<(String, SuggestionAction, Vec<String>)> {
    items
        .iter()
        .map(|s| (s.profile.clone(), s.action, s.groups.clone()))
        .collect()
}

#[tokio::test]
async fn suggestions_come_from_groups_and_are_never_assigned() {
    let Some(f) = Fixture::new("host_profiles_suggest", false).await else {
        return;
    };
    // master-1 moves from k3s_servers to keel_nodes; node-1 is where its profile says.
    set_groups(&f, "master-1", &["keel_nodes"]).await;
    set_groups(&f, "node-1", &["keel_nodes"]).await;
    sync(&f).await;
    let items = listed(&f, None).await;
    assert!(items.iter().all(|s| s.host_id == f.master), "{items:?}");
    assert_eq!(
        brief(&items),
        vec![
            (
                "keel-node".to_owned(),
                SuggestionAction::Add,
                vec!["keel_nodes".to_owned()]
            ),
            (
                "k3s-master".to_owned(),
                SuggestionAction::Remove,
                vec!["k3s_servers".to_owned()]
            ),
        ]
    );
    // Nothing assigned or removed on its own.
    let names: Vec<String> = assigned(&f, f.master)
        .await
        .into_iter()
        .map(|r| r.0)
        .collect();
    assert_eq!(names, vec!["k3s-master"]);
    let hosts =
        f.h.cmd(Cmd::ListHosts {
            workspace: f.h.workspace,
            profile: None,
            drift: None,
            changed_outside: None,
        })
        .await;
    match hosts.as_slice() {
        [Event::Hosts { items }] => {
            let count = |id| items.iter().find(|h| h.id == id).map(|h| h.suggestions);
            assert_eq!(count(f.master), Some(2));
            assert_eq!(count(f.node), Some(0));
        }
        other => panic!("hosts: {other:?}"),
    }

    // Dismissed: gone from the list, and a sync with the same groups keeps it gone.
    let out =
        f.h.cmd(Cmd::DismissProfileSuggestion {
            host_id: f.master,
            profile: "k3s-master".to_owned(),
            action: SuggestionAction::Remove,
        })
        .await;
    match out.as_slice() {
        [Event::ProfileSuggestions { items }] => assert_eq!(items.len(), 1, "{items:?}"),
        other => panic!("dismiss: {other:?}"),
    }
    sync(&f).await;
    assert_eq!(listed(&f, Some(f.master)).await.len(), 1);
    let (by,): (Option<Uuid>,) = sqlx::query_as(
        "select dismissed_by from profile_suggestions where host_id = $1 and action = 'remove'",
    )
    .bind(f.master)
    .fetch_one(&f.h.db.owner)
    .await
    .expect("dismissed");
    assert!(by.is_some());
    assert_eq!(
        op_error(
            &f.h.cmd(Cmd::DismissProfileSuggestion {
                host_id: f.master,
                profile: "debian".to_owned(),
                action: SuggestionAction::Add,
            })
            .await
        ),
        "not_found"
    );

    // Back in k3s_servers: the remove no longer applies. Out again: it returns, undismissed.
    set_groups(&f, "master-1", &["keel_nodes", "k3s_servers"]).await;
    sync(&f).await;
    assert_eq!(listed(&f, Some(f.master)).await.len(), 1);
    set_groups(&f, "master-1", &["keel_nodes"]).await;
    sync(&f).await;
    assert_eq!(listed(&f, Some(f.master)).await.len(), 2);

    // Accepting through the profile preview settles the add.
    let (token, _) = preview(&f, change(f.master, &["keel-node"], &[])).await;
    assert!(matches!(
        confirm(&f, token).await.first(),
        Some(Event::Confirmed { .. })
    ));
    let left = brief(&listed(&f, Some(f.master)).await);
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0].1, SuggestionAction::Remove);
    f.h.finish().await;
}
