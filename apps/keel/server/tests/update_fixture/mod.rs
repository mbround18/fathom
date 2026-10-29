//! Shared support for the spec 061 server tests: a known running build, an offered update, and a
//! System workspace with (optionally) a host carrying the `keel-server` profile and a bound repo
//! whose `keel-server` job declares `params.keel_server_image_digest`. Nothing touches the
//! network: the update status is set directly, never checked against a registry.

#![allow(dead_code)]

use std::sync::Arc;

use keel_protocol::{Cmd, Event, HostProfiles, ProfileName, UpdateCheckState, UpdateOutcome};
use keel_server::build_info::BuildInfo;
use keel_server::updates::{ImageInfo, Updates};
use oci_registry::{Digest, Platform};
use serde_json::Value;
use uuid::Uuid;

use crate::repos::{Harness, Source};

pub const RUNNING: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const NEW: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
pub const OTHER: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

/// The `keel-server` job, declaring the digest param or not.
pub fn manifest(with_param: bool) -> String {
    let params = if with_param {
        "    params:\n      keel_server_image_digest: { kind: oci_digest }\n"
    } else {
        ""
    };
    format!(
        "version: 1\nprofiles: {{ keel-server: {{}} }}\njobs:\n  keel-server:\n    \
         playbook: playbooks/web.yaml\n    profiles: [keel-server]\n    inventory: prod\n{params}"
    )
}

pub fn build() -> BuildInfo {
    BuildInfo {
        version: Some("0.2.0".into()),
        revision: Some("abcdef1".into()),
        built_at: Some(time::OffsetDateTime::from_unix_timestamp(1_790_000_000).expect("time")),
        source: None,
        digest: Some(RUNNING.into()),
        platform: Platform::current(),
    }
}

/// Swaps in [`build`] as the running build, with checks on or off. Call before `spawn`.
pub fn running(h: &mut Harness, build: BuildInfo, check: bool) {
    let mut config = h.app.updates.config.clone();
    config.check = check;
    h.app.updates = Arc::new(Updates::new(build, config));
}

/// Sets the status as a finished check that found `digest` with `outcome`.
pub fn offer(h: &Harness, outcome: UpdateOutcome, digest: &str) {
    let digest = Digest::parse(digest).expect("digest");
    h.app.updates.status.send_modify(|s| {
        s.state = UpdateCheckState::Checked;
        s.outcome = Some(outcome);
        s.checked_at = Some(time::OffsetDateTime::now_utc());
        s.available = Some(ImageInfo {
            digest,
            version: Some("0.3.0".into()),
            revision: Some("fedcba9".into()),
            built_at: None,
            release_notes: None,
            release_notes_note: Some("no source label".into()),
        });
    });
}

fn profiles(names: &[&str]) -> HostProfiles {
    HostProfiles(
        names
            .iter()
            .map(|n| ProfileName::new(*n).expect("profile"))
            .collect(),
    )
}

/// What [`system`] builds.
pub struct System {
    pub workspace: Uuid,
    pub project: Uuid,
    pub binding: Option<Uuid>,
    pub host: Option<Uuid>,
    _src: Option<Source>,
}

/// How much of the update prerequisites to set up.
#[derive(Clone, Copy)]
pub struct Setup {
    /// A bound repo with the `keel-server` job.
    pub job: bool,
    /// The job declares the digest param.
    pub param: bool,
    /// An enrolled host with the `keel-server` profile.
    pub host: bool,
}

impl Setup {
    pub const FULL: Self = Self {
        job: true,
        param: true,
        host: true,
    };
}

/// Configures the System workspace (allowing `keel-server`) and sets up `setup`. Runs there need
/// no approval.
pub async fn system(h: &Harness, setup: Setup) -> System {
    let (_, out) = h
        .confirm(Cmd::PreviewSystemConfigure {
            allowed_profiles: profiles(&["keel-server"]),
            version: 0,
        })
        .await;
    let workspace = match out.as_slice() {
        [
            Event::Confirmed { .. },
            Event::System {
                workspace: Some(ws),
                ..
            },
        ] => *ws,
        other => panic!("configure: {other:?}"),
    };
    let owner = &h.db.owner;
    let project = Uuid::now_v7();
    sqlx::query(
        "insert into projects (id, workspace_id, slug, name) values ($1, $2, 'keel', 'Keel')",
    )
    .bind(project)
    .bind(workspace)
    .execute(owner)
    .await
    .expect("project");
    sqlx::query(
        "insert into approval_policies (id, workspace_id, priority, requires_approval) \
         values ($1, $2, 1, false)",
    )
    .bind(Uuid::now_v7())
    .bind(workspace)
    .execute(owner)
    .await
    .expect("policy");
    let inventory = Uuid::now_v7();
    sqlx::query(
        "insert into inventories (id, workspace_id, name, source) values ($1, $2, 'prod', 'managed')",
    )
    .bind(inventory)
    .bind(workspace)
    .execute(owner)
    .await
    .expect("inventory");
    let revision = Uuid::now_v7();
    sqlx::query(
        "insert into inventory_revisions (id, inventory_id, seq, content, content_hash, status) \
         values ($1, $2, 1, '{}', 'h1', 'valid')",
    )
    .bind(revision)
    .bind(inventory)
    .execute(owner)
    .await
    .expect("revision");
    sqlx::query(
        "insert into inventory_hosts (revision_id, host, groups) values ($1, 'keel-1', '{keel}')",
    )
    .bind(revision)
    .execute(owner)
    .await
    .expect("inventory host");

    let (binding, src) = if setup.job {
        let src = Source::new(&manifest(setup.param));
        let mut spec = crate::repos::spec(h, &src);
        spec.project = project;
        let (_, out) = h.confirm(Cmd::PreviewBindingCreate(spec)).await;
        let binding = match out.as_slice() {
            [Event::Confirmed { .. }, Event::Binding { binding, .. }] => binding.id,
            other => panic!("binding create: {other:?}"),
        };
        (Some(binding), Some(src))
    } else {
        (None, None)
    };

    let host = if setup.host {
        let id = Uuid::now_v7();
        sqlx::query(
            "insert into hosts (id, workspace_id, inventory_name, allowed_profiles) \
             values ($1, $2, 'keel-1', '{keel-server}')",
        )
        .bind(id)
        .bind(workspace)
        .execute(owner)
        .await
        .expect("host");
        // The profile row comes from the manifest; without a repo, declare it here.
        sqlx::query(
            "insert into profiles (id, workspace_id, name) values ($1, $2, 'keel-server') \
             on conflict (workspace_id, name) do nothing",
        )
        .bind(Uuid::now_v7())
        .bind(workspace)
        .execute(owner)
        .await
        .expect("profile");
        sqlx::query(
            "insert into host_profiles (host_id, profile_id) \
             select $1, id from profiles where workspace_id = $2 and name = 'keel-server'",
        )
        .bind(id)
        .bind(workspace)
        .execute(owner)
        .await
        .expect("host profile");
        Some(id)
    } else {
        None
    };
    System {
        workspace,
        project,
        binding,
        host,
        _src: src,
    }
}

/// Audit rows for `action`, oldest first, as JSON (without the hash chain).
pub async fn audited(h: &Harness, action: &str) -> Vec<Value> {
    let rows: Vec<(Value,)> = sqlx::query_as(
        "select to_jsonb(a) - 'hash' - 'prev_hash' from audit_event a where action = $1 \
         order by seq",
    )
    .bind(action)
    .fetch_all(&h.db.owner)
    .await
    .expect("audit");
    rows.into_iter().map(|r| r.0).collect()
}

/// The reasons of every `keel_update_refused`, oldest first.
pub async fn refusals(h: &Harness) -> Vec<String> {
    audited(h, "keel_update_refused")
        .await
        .iter()
        .map(|r| r["reason"].as_str().unwrap_or_default().to_owned())
        .collect()
}
