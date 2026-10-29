//! Batch preview tests: what is pinned, what is left and why, and the steps shown.

use std::sync::Arc;

use depths_core::ports::{HostPlatform, ImageInfo};
use depths_protocol::{
    AvailableImage, EffectivePolicy, Exclusion, PolicySource, RunningImage, UpdatePolicy,
};

use super::*;
use crate::updates::SystemClock;
use crate::updates::store::Rejected;

struct Live(Vec<ServiceInput>);

impl Inputs for Live {
    fn stacks(&self) -> Vec<String> {
        vec!["web".into()]
    }
    fn host(&self) -> Option<HostPlatform> {
        None
    }
    fn services(&self, _: &str) -> Vec<ServiceInput> {
        self.0.clone()
    }
}

const COMPOSE: &str = "services:\n  app:\n    image: nginx:1\n  db:\n    image: postgres:17\n  side:\n    image: redis:7\n";

struct Fx {
    _dir: tempfile::TempDir,
    store: Arc<dyn StackStore>,
    updates: Arc<Updates>,
}

fn fx(compose: &str) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let cfg = crate::updates::tests::config(dir.path(), "on");
    std::fs::create_dir_all(&cfg.data_dir).unwrap();
    let stacks = dir.path().join("stacks");
    std::fs::create_dir_all(stacks.join("web")).unwrap();
    std::fs::write(stacks.join("web").join("compose.yaml"), compose).unwrap();
    let store: Arc<dyn StackStore> =
        Arc::new(depths_stacks_fs::FsStackStore::open(&stacks).unwrap());
    let updates = Updates::open(&cfg, Arc::new(SystemClock), || {
        Arc::new(crate::updates::tests::NoNetwork)
    })
    .unwrap()
    .unwrap();
    Fx {
        _dir: dir,
        store,
        updates,
    }
}

fn svc(service: &str, reference: &str, state: UpdateState) -> ServiceUpdate {
    ServiceUpdate {
        stack: "web".into(),
        service: service.into(),
        reference: reference.into(),
        running: Some(RunningImage {
            image_id: format!("sha256:old-{service}"),
            repo_digest: Some("sha256:olddigest".into()),
            version: Some("1.0".into()),
            revision: None,
        }),
        available: Some(AvailableImage {
            digest: format!("sha256:new-{service}"),
            platform_digest: "sha256:plat".into(),
            config_digest: format!("sha256:cfg-{service}"),
            version: Some("1.1".into()),
            revision: None,
            created: None,
            user: None,
        }),
        state,
        reason: None,
        effective: EffectivePolicy {
            policy: UpdatePolicy::Notify,
            source: PolicySource::Default,
            excluded: None,
            labels: Vec::new(),
        },
        last_checked: None,
        last_outcome: None,
        replicas: 2,
    }
}

impl Fx {
    fn seed(&self, list: Vec<ServiceUpdate>) {
        self.updates
            .store()
            .update_observed(|o| {
                for s in list {
                    o.services.insert(format!("web/{}", s.service), s);
                }
            })
            .unwrap();
    }

    fn preview(&self, live: Vec<ServiceInput>, only: Only<'_>) -> StackPreview {
        preview_stack(
            &self.updates,
            &*self.store,
            &Live(live),
            ComposePolicy::Warn,
            "web",
            only,
        )
        .unwrap()
    }
}

fn live(service: &str) -> ServiceInput {
    ServiceInput {
        service: service.into(),
        user: Some("1000".into()),
        platform: Some("linux/arm64".into()),
        running: Some(ImageInfo {
            user: Some("app".into()),
            ..ImageInfo::default()
        }),
        ..ServiceInput::default()
    }
}

#[test]
fn only_services_with_a_newer_image_are_pinned_and_the_rest_say_why() {
    let f = fx(COMPOSE);
    let mut excluded = svc("side", "redis:7", UpdateState::Excluded);
    excluded.effective.excluded = Some(Exclusion::LabelDisabled);
    excluded.reason = Some("a Watchtower label turns updates off".into());
    f.seed(vec![
        svc("app", "nginx:1", UpdateState::Available),
        svc("db", "postgres:17", UpdateState::UpToDate),
        excluded,
    ]);
    let p = f.preview(vec![live("app")], Only::Available);
    assert_eq!(p.plan.services.len(), 1);
    assert_eq!(p.plan.services[0].service, "app");
    assert_eq!(
        p.plan.skipped,
        vec![
            ("db".to_owned(), "up to date".to_owned()),
            (
                "side".to_owned(),
                "a Watchtower label turns updates off".to_owned()
            ),
        ]
    );
    let job = p.job.unwrap();
    assert_eq!(job.stack_hash.len(), 64, "bound to the content hash");
    assert!(!job.apply_held);
    let pin = &job.pins[0];
    assert_eq!(pin.pull, "docker.io/library/nginx@sha256:new-app");
    assert_eq!(pin.digest, "sha256:new-app");
    assert_eq!(pin.to_image.as_deref(), Some("sha256:cfg-app"));
    assert_eq!(pin.from_image, "sha256:old-app");
    assert_eq!(pin.compose_user.as_deref(), Some("1000"));
    assert_eq!(pin.old_user, "app");
    assert_eq!(pin.platform.as_deref(), Some("linux/arm64"));
    assert_eq!(pin.replicas, 2);

    let argv: Vec<String> = p.plan.argv.iter().map(|a| a.join(" ")).collect();
    assert_eq!(argv.len(), 4, "{argv:#?}");
    assert!(argv[0].starts_with("docker pull --platform linux/arm64 docker.io/library/nginx@"));
    assert_eq!(
        argv[1],
        "docker image tag sha256:old-app depths-keep/web:app-previous"
    );
    assert_eq!(argv[2], "docker image tag sha256:cfg-app nginx:1");
    assert!(
        argv[3].ends_with("up -d --no-deps --pull never app"),
        "{}",
        argv[3]
    );
    assert!(!p.plan.ack_required && !p.escape);
}

#[test]
fn a_rejected_digest_is_left_until_retried() {
    let f = fx(COMPOSE);
    f.seed(vec![svc("app", "nginx:1", UpdateState::Available)]);
    f.updates
        .store()
        .update_decisions(|d| {
            d.rejected.push(Rejected {
                stack: "web".into(),
                service: "app".into(),
                digest: "sha256:new-app".into(),
                at: "2026-10-01T00:00:00Z".into(),
                reason: "unhealthy".into(),
                run_id: None,
            })
        })
        .unwrap();
    let p = f.preview(vec![], Only::Available);
    assert!(p.job.is_none(), "nothing would change");
    assert!(p.plan.argv.is_empty());
    assert!(p.plan.skipped[0].1.starts_with("rejected"));
}

#[test]
fn escape_findings_need_an_acknowledgement() {
    let f = fx("services:\n  app:\n    image: nginx:1\n    privileged: true\n");
    f.seed(vec![svc("app", "nginx:1", UpdateState::Available)]);
    let p = f.preview(vec![], Only::Available);
    assert!(p.plan.ack_required);
    assert!(p.escape);
    assert!(p.plan.findings.iter().any(|w| w.code == "privileged"));
    // Without live data the pin still stands; `user:` unknown means the root check applies.
    let pin = &p.job.unwrap().pins[0];
    assert_eq!(pin.compose_user, None);
    assert_eq!(pin.platform, None);
}

#[test]
fn apply_held_pins_only_the_held_service_at_the_held_digest() {
    let f = fx(COMPOSE);
    f.seed(vec![
        svc("app", "nginx:1", UpdateState::Held),
        svc("db", "postgres:17", UpdateState::Available),
    ]);
    let p = f.preview(
        vec![],
        Only::Held {
            service: "app",
            digest: "sha256:new-app",
        },
    );
    let job = p.job.unwrap();
    assert!(job.apply_held);
    assert_eq!(job.pins.len(), 1);
    assert_eq!(job.pins[0].service, "app");

    let moved = f.preview(
        vec![],
        Only::Held {
            service: "app",
            digest: "sha256:other",
        },
    );
    assert!(moved.job.is_none());
    assert!(moved.plan.skipped[0].1.contains("preview again"));

    let not_held = f.preview(
        vec![],
        Only::Held {
            service: "db",
            digest: "sha256:new-db",
        },
    );
    assert!(not_held.job.is_none());
}

#[test]
fn a_stack_that_cannot_be_read_is_refused() {
    let f = fx(COMPOSE);
    let err = preview_stack(
        &f.updates,
        &*f.store,
        &Live(vec![]),
        ComposePolicy::Warn,
        "nope",
        Only::Available,
    )
    .unwrap_err();
    assert!(err.starts_with("nope"), "{err}");
}
