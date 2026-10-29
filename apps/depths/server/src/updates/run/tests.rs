//! Runner tests (T047, T060, T076) against a scripted runtime: images are a map that `image tag`
//! copies and `image rm` deletes, so the keep tags, the reference and cleanup can be checked.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use depths_core::plan::RunStep;
use depths_core::ports::{
    ContainerHealth, ImageInfo, OutputStream, ProjectContainers, RuntimeError, RuntimeSnapshot,
    StackStore, Validation,
};
use depths_protocol::{AdoptionState, Container, RuntimeContainerState};
use depths_protocol::{AuditEntry, AuditFilter};

use super::*;
use crate::audit::Audit;
use crate::updates::SystemClock;
use crate::watch::Watch;

type Hook = Box<dyn Fn(&str) + Send + Sync>;

#[derive(Default)]
struct Fake {
    ran: Mutex<Vec<String>>,
    /// `(argv prefix, stderr)`: the first step that starts with the prefix fails, once.
    fails: Mutex<Vec<(String, String)>>,
    images: Mutex<BTreeMap<String, ImageInfo>>,
    in_use: Mutex<BTreeSet<String>>,
    /// Per stack: `(service, container id)`.
    containers: Mutex<BTreeMap<String, Vec<(String, String)>>>,
    health: Mutex<VecDeque<Vec<ContainerHealth>>>,
    health_reads: AtomicUsize,
    daemon: Mutex<Option<DaemonMode>>,
    /// Called with each argv before it runs.
    hook: Mutex<Option<Hook>>,
}

impl ContainerRuntime for Fake {
    fn containers_by_project(&self) -> Result<RuntimeSnapshot, RuntimeError> {
        let mut snap = RuntimeSnapshot::default();
        for (stack, cs) in self.containers.lock().unwrap().iter() {
            snap.projects.insert(
                stack.clone(),
                ProjectContainers {
                    working_dir: None,
                    containers: cs.iter().map(|(s, id)| container(stack, s, id)).collect(),
                },
            );
        }
        Ok(snap)
    }
    fn run_step(
        &self,
        step: &RunStep,
        _: Option<&[u8]>,
        sink: &dyn OutputSink,
        _: &CancelToken,
    ) -> Result<StepOutcome, RuntimeError> {
        let argv = step.argv.join(" ");
        if let Some(h) = self.hook.lock().unwrap().as_ref() {
            h(&argv);
        }
        self.ran.lock().unwrap().push(argv.clone());
        let fail = {
            let mut f = self.fails.lock().unwrap();
            f.iter()
                .position(|(p, _)| argv.starts_with(p.as_str()))
                .map(|i| f.remove(i).1)
        };
        if let Some(stderr_tail) = fail {
            return Ok(StepOutcome::Failed {
                exit_code: Some(1),
                stderr_tail,
            });
        }
        let a = &step.argv;
        if a.get(1..3) == Some(&["image".into(), "tag".into()]) {
            let mut images = self.images.lock().unwrap();
            if let Some(i) = images.get(&a[3]).cloned() {
                images.insert(a[4].clone(), i);
            }
        }
        if a.get(1..3) == Some(&["image".into(), "rm".into()]) {
            if self.in_use.lock().unwrap().contains(&a[3]) {
                return Ok(StepOutcome::Failed {
                    exit_code: Some(1),
                    stderr_tail: "conflict: image is being used by a running container".into(),
                });
            }
            self.images
                .lock()
                .unwrap()
                .retain(|k, i| k != &a[3] && i.id != a[3]);
        }
        sink.emit(OutputChunk {
            stream: OutputStream::Stdout,
            text: format!("ran {argv}\n"),
        });
        Ok(StepOutcome::Succeeded)
    }
    fn validate(&self, _: &RunStep, _: &[u8]) -> Result<Validation, RuntimeError> {
        Ok(Validation::Valid)
    }
    fn follow_events(&self, _: &(dyn Fn() + Sync), _: &CancelToken) -> Result<(), RuntimeError> {
        Err(RuntimeError::Unsupported)
    }
    fn image_inspect(&self, refs: &[String]) -> Result<Vec<ImageInfo>, RuntimeError> {
        let images = self.images.lock().unwrap();
        Ok(refs.iter().filter_map(|r| images.get(r).cloned()).collect())
    }
    fn containers_health(&self, ids: &[String]) -> Result<Vec<ContainerHealth>, RuntimeError> {
        self.health_reads.fetch_add(1, Ordering::SeqCst);
        let mut seq = self.health.lock().unwrap();
        let now = if seq.len() > 1 {
            seq.pop_front().unwrap_or_default()
        } else {
            seq.front().cloned().unwrap_or_default()
        };
        Ok(now.into_iter().filter(|c| ids.contains(&c.id)).collect())
    }
    fn daemon_mode(&self) -> Result<DaemonMode, RuntimeError> {
        (*self.daemon.lock().unwrap()).ok_or(RuntimeError::Unsupported)
    }
}

fn container(stack: &str, service: &str, id: &str) -> Container {
    Container {
        id: id.into(),
        name: format!("{stack}-{service}-1"),
        image: "nginx:1".into(),
        state: RuntimeContainerState::Running,
        health: None,
        status: "Up".into(),
        created_at: String::new(),
        command: String::new(),
        ports: vec![],
        project: Some(stack.into()),
        service: Some(service.into()),
        cluster: None,
        adoption: AdoptionState::NotAdopted,
    }
}

fn health(id: &str, h: HealthStatus, restarts: u32) -> ContainerHealth {
    ContainerHealth {
        id: id.into(),
        image: "sha256:cfgnew".into(),
        running: true,
        status: "running".into(),
        health: h,
        restart_count: restarts,
    }
}

const STACKS: [&str; 4] = ["a", "b", "c", "d"];
const PLAIN: &str = "services:\n  app:\n    image: nginx:1\n";

struct Fx {
    _dir: tempfile::TempDir,
    fake: Arc<Fake>,
    audit: Arc<Audit>,
    ops: Arc<Ops>,
    updates: Arc<Updates>,
    runner: Runner,
}

fn fx() -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let cfg = crate::updates::tests::config(dir.path(), "on");
    std::fs::create_dir_all(&cfg.data_dir).unwrap();
    let stacks = dir.path().join("stacks");
    for s in STACKS {
        std::fs::create_dir_all(stacks.join(s)).unwrap();
        std::fs::write(stacks.join(s).join("compose.yaml"), PLAIN).unwrap();
    }
    let store: Arc<dyn StackStore> =
        Arc::new(depths_stacks_fs::FsStackStore::open(&stacks).unwrap());
    let fake = Arc::new(Fake::default());
    {
        let mut images = fake.images.lock().unwrap();
        images.insert(
            "nginx@sha256:new".into(),
            ImageInfo {
                id: "sha256:cfgnew".into(),
                repo_digests: vec!["nginx@sha256:new".into()],
                user: Some("1000".into()),
                ..ImageInfo::default()
            },
        );
        images.insert(
            "sha256:old".into(),
            ImageInfo {
                id: "sha256:old".into(),
                repo_digests: vec!["nginx@sha256:olddigest".into()],
                ..ImageInfo::default()
            },
        );
    }
    *fake.containers.lock().unwrap() = STACKS
        .iter()
        .map(|s| ((*s).to_owned(), vec![("app".to_owned(), format!("c-{s}"))]))
        .collect();
    *fake.health.lock().unwrap() = vec![
        STACKS
            .iter()
            .map(|s| health(&format!("c-{s}"), HealthStatus::Healthy, 0))
            .collect(),
    ]
    .into();
    *fake.daemon.lock().unwrap() = Some(DaemonMode::Rootful);
    let audit = Arc::new(Audit::open(dir.path().join("audit"), 1 << 20).unwrap());
    let watch = Watch::new(store.clone(), fake.clone());
    let ops = Arc::new(Ops::with_ttl(
        store,
        fake.clone(),
        audit.clone(),
        watch,
        Duration::from_secs(300),
    ));
    let updates = Updates::open(&cfg, Arc::new(SystemClock), || {
        Arc::new(crate::updates::tests::NoNetwork)
    })
    .unwrap()
    .unwrap();
    let runner = Runner {
        ops: ops.clone(),
        updates: updates.clone(),
        sleep: Arc::new(|_| {}),
    };
    Fx {
        _dir: dir,
        fake,
        audit,
        ops,
        updates,
        runner,
    }
}

fn admin() -> Actor {
    Actor {
        provider: Provider::Discord,
        subject: "1".into(),
        username: "admin".into(),
    }
}

impl Fx {
    fn hash(&self, stack: &str) -> String {
        self.ops
            .stack_store()
            .entry(&StackName::parse(stack).unwrap())
            .unwrap()
            .content_hash
            .unwrap()
            .as_str()
            .to_owned()
    }

    fn compose(&self, stack: &str) -> Vec<u8> {
        std::fs::read(
            self.ops
                .stack_store()
                .root()
                .join(stack)
                .join("compose.yaml"),
        )
        .unwrap()
    }

    fn write_compose(&self, stack: &str, text: &str) {
        std::fs::write(
            self.ops
                .stack_store()
                .root()
                .join(stack)
                .join("compose.yaml"),
            text,
        )
        .unwrap();
    }

    fn job(&self, stack: &str) -> StackJob {
        StackJob {
            stack: stack.into(),
            stack_hash: self.hash(stack),
            pins: vec![pin()],
            acknowledged: false,
            apply_held: false,
        }
    }

    fn fail(&self, prefix: &str, stderr: &str) {
        self.fake
            .fails
            .lock()
            .unwrap()
            .push((prefix.into(), stderr.into()));
    }

    fn health(&self, seq: Vec<Vec<ContainerHealth>>) {
        *self.fake.health.lock().unwrap() = seq.into();
    }

    fn ran(&self) -> Vec<String> {
        self.fake.ran.lock().unwrap().clone()
    }

    fn run(&self, trigger: RunTrigger, jobs: Vec<StackJob>) -> (UpdateRun, Vec<Event>) {
        self.run_with(trigger, jobs, CancelToken::new())
    }

    fn run_with(
        &self,
        trigger: RunTrigger,
        jobs: Vec<StackJob>,
        cancel: CancelToken,
    ) -> (UpdateRun, Vec<Event>) {
        let (tx, mut rx) = mpsc::channel(10_000);
        let run = self.runner.run(
            Batch {
                run_id: RunId(uuid::Uuid::new_v4()),
                trigger,
                ip: "203.0.113.9".parse().unwrap(),
                jobs,
                cancel,
            },
            tx,
        );
        let mut evs = Vec::new();
        while let Ok(e) = rx.try_recv() {
            evs.push(e.event);
        }
        (run, evs)
    }

    fn manual(&self, jobs: Vec<StackJob>) -> (UpdateRun, Vec<Event>) {
        self.run(RunTrigger::Manual { actor: admin() }, jobs)
    }

    fn entries(&self) -> Vec<AuditEntry> {
        let mut v = self.audit.page(&AuditFilter::default(), None).unwrap().0;
        v.reverse();
        v
    }

    fn cleanup(&self, on: bool) {
        self.updates
            .store()
            .update_decisions(|d| d.settings.cleanup = on)
            .unwrap();
    }
}

fn pin() -> ServicePin {
    ServicePin {
        service: "app".into(),
        reference: "nginx:1".into(),
        pull: "nginx@sha256:new".into(),
        digest: "sha256:new".into(),
        platform: None,
        to_image: Some("sha256:cfgnew".into()),
        from_image: "sha256:old".into(),
        from_digest: Some("sha256:olddigest".into()),
        version: Some("1.27.1".into()),
        compose_user: None,
        old_user: "1000".into(),
        replicas: 1,
    }
}

fn outcomes(run: &UpdateRun) -> Vec<(String, StackOutcome)> {
    run.stacks
        .iter()
        .map(|s| (s.stack.clone(), s.outcome.clone().unwrap()))
        .collect()
}

fn skipped(r: &str) -> StackOutcome {
    StackOutcome::Skipped { reason: r.into() }
}

// --- order, audit, events ---------------------------------------------------------------------

#[test]
fn stacks_run_in_order_one_at_a_time_each_audited_image_update() {
    let f = fx();
    let before = f.compose("a");
    let (run, evs) = f.manual(vec![f.job("a"), f.job("b")]);
    assert_eq!(
        outcomes(&run),
        vec![
            ("a".into(), StackOutcome::Updated),
            ("b".into(), StackOutcome::Updated)
        ]
    );
    let ran = f.ran();
    let pos = |s: &str| {
        ran.iter()
            .position(|r| r == s)
            .unwrap_or_else(|| panic!("{s}: {ran:#?}"))
    };
    let a_up = "docker compose up -d --no-deps --pull never app";
    assert!(ran.iter().filter(|r| *r == a_up).count() == 2, "{ran:#?}");
    assert!(
        pos("docker pull nginx@sha256:new")
            < pos("docker image tag sha256:old depths-keep/a:app-previous")
    );
    assert!(
        pos("docker image tag sha256:old depths-keep/a:app-previous")
            < pos("docker image tag sha256:cfgnew nginx:1")
    );
    assert!(
        !ran.iter()
            .any(|r| r.contains("--remove-orphans") || r.contains("prune"))
    );
    assert_eq!(
        f.compose("a"),
        before,
        "the compose file is byte-identical (FR-009)"
    );

    let e = f.entries();
    let updates: Vec<&AuditEntry> = e
        .iter()
        .filter(|e| e.action == AuditAction::ImageUpdate)
        .collect();
    assert_eq!(updates.len(), 2);
    assert_eq!(updates[0].stack.as_deref(), Some("a"));
    assert_eq!(updates[1].stack.as_deref(), Some("b"));
    let u = updates[0].update.as_ref().unwrap();
    assert_eq!(u.run_id, run.run_id);
    assert_eq!(u.services[0].to_digest, "sha256:new");
    assert_eq!(u.services[0].from_image, "sha256:old");
    assert_eq!(updates[0].outcome, AuditOutcome::Succeeded);

    // Events: started, per stack OpStarted .. UpdateStackDone, finished.
    assert!(
        matches!(evs.first(), Some(Event::UpdateRunStarted { stacks, .. }) if stacks == &["a", "b"])
    );
    assert!(
        matches!(evs.last(), Some(Event::UpdateRunFinished { summary, .. }) if summary.len() == 2)
    );
    let done: Vec<&str> = evs
        .iter()
        .filter_map(|e| match e {
            Event::UpdateStackDone {
                stack, audit_at, ..
            } => {
                assert!(!audit_at.is_empty());
                Some(stack.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(done, vec!["a", "b"]);
    assert!(evs.iter().any(|e| matches!(e, Event::OpOutput { .. })));
    assert_eq!(
        evs.iter()
            .filter(|e| matches!(
                e,
                Event::OpFinished {
                    outcome: OpOutcome::Succeeded,
                    ..
                }
            ))
            .count(),
        2
    );

    // The keep tag is recorded with the image id, and the locks are released.
    let keep = f.updates.store().observed().keep;
    assert!(
        keep.iter()
            .any(|k| k.tag == "depths-keep/a:app-previous" && k.image_id == "sha256:old")
    );
    assert!(!f.ops.is_busy("a") && !f.ops.is_busy("b"));
    // The journal holds the finished run.
    let j = f.updates.store().observed().journal;
    assert!(
        j.iter()
            .any(|r| r.run_id == run.run_id && r.finished_at.is_some())
    );
}

#[test]
fn only_services_with_a_newer_image_are_recreated() {
    let f = fx();
    f.write_compose(
        "a",
        "services:\n  app:\n    image: nginx:1\n  db:\n    image: postgres:17\n",
    );
    let (run, _) = f.manual(vec![f.job("a")]);
    assert_eq!(outcomes(&run)[0].1, StackOutcome::Updated);
    let ran = f.ran();
    assert!(ran.contains(&"docker compose up -d --no-deps --pull never app".to_owned()));
    assert!(!ran.iter().any(|r| r.contains("db")), "{ran:#?}");
}

#[test]
fn busy_and_stale_are_skipped_and_a_failure_does_not_stop_the_rest() {
    let f = fx();
    let alice = Actor {
        provider: Provider::Discord,
        subject: "7".into(),
        username: "alice".into(),
    };
    f.ops
        .claim_update("b", alice, "discord:7".into(), CancelToken::new())
        .unwrap();
    f.fail("docker pull", "manifest unknown");
    let mut stale = f.job("c");
    stale.stack_hash = "0".repeat(64);
    let (run, _) = f.manual(vec![f.job("a"), f.job("b"), stale, f.job("d")]);
    assert_eq!(
        outcomes(&run),
        vec![
            ("a".into(), StackOutcome::Failed),
            ("b".into(), skipped("busy: alice update")),
            ("c".into(), skipped("stale: preview again")),
            ("d".into(), StackOutcome::Updated),
        ]
    );
    // A failed pull changed nothing on the host.
    assert!(!f.ran().iter().any(|r| r.contains("depths-keep/a")));
    assert!(f.ops.is_busy("b"), "the other operation keeps its lock");
}

#[test]
fn cancel_skips_the_rest() {
    let f = fx();
    let cancel = CancelToken::new();
    let c = cancel.clone();
    *f.fake.hook.lock().unwrap() = Some(Box::new(move |argv| {
        if argv.starts_with("docker compose") {
            c.cancel();
        }
    }));
    let (run, _) = f.run_with(
        RunTrigger::Manual { actor: admin() },
        vec![f.job("a"), f.job("b"), f.job("c")],
        cancel,
    );
    let o = outcomes(&run);
    assert_eq!(o[1].1, skipped("cancelled"));
    assert_eq!(o[2].1, skipped("cancelled"));
}

// --- holds -------------------------------------------------------------------------------------

#[test]
fn an_image_that_turns_root_is_held_with_the_tag_and_container_unchanged() {
    let f = fx();
    f.fake
        .images
        .lock()
        .unwrap()
        .get_mut("nginx@sha256:new")
        .unwrap()
        .user = Some("root".into());
    let (run, _) = f.manual(vec![f.job("a")]);
    assert_eq!(outcomes(&run)[0].1, StackOutcome::Held);
    let ran = f.ran();
    assert!(
        !ran.iter()
            .any(|r| r == "docker image tag sha256:cfgnew nginx:1"),
        "{ran:#?}"
    );
    assert!(
        !ran.iter().any(|r| r.starts_with("docker compose")),
        "container unchanged"
    );
    assert!(ran.contains(&"docker image tag sha256:cfgnew depths-keep/a:app-held".to_owned()));
    let e = f.entries();
    let h = e
        .iter()
        .find(|e| e.action == AuditAction::ImageHold)
        .unwrap();
    assert_eq!(
        h.update.as_ref().unwrap().hold.as_deref(),
        Some("root_user")
    );

    // On a rootless daemon it is shown, not held.
    let g = fx();
    *g.fake.daemon.lock().unwrap() = Some(DaemonMode::Rootless);
    g.fake
        .images
        .lock()
        .unwrap()
        .get_mut("nginx@sha256:new")
        .unwrap()
        .user = Some("root".into());
    assert_eq!(
        outcomes(&g.manual(vec![g.job("a")]).0)[0].1,
        StackOutcome::Updated
    );

    // A service with `user:` set is never held; ApplyHeld overrides the hold.
    let mut set = g.job("b");
    set.pins[0].compose_user = Some("1000".into());
    *g.fake.daemon.lock().unwrap() = Some(DaemonMode::Rootful);
    assert_eq!(outcomes(&g.manual(vec![set]).0)[0].1, StackOutcome::Updated);
    let mut apply = g.job("c");
    apply.apply_held = true;
    assert_eq!(
        outcomes(&g.manual(vec![apply]).0)[0].1,
        StackOutcome::Updated
    );
}

#[test]
fn compose_findings_hold_a_scheduled_run_need_an_ack_and_refuse_under_enforce() {
    let f = fx();
    f.write_compose(
        "a",
        "services:\n  app:\n    image: nginx:1\n    privileged: true\n",
    );
    let scheduled = RunTrigger::Scheduled {
        on_behalf_of: [("a".to_owned(), admin())].into(),
    };
    assert_eq!(
        outcomes(&f.run(scheduled, vec![f.job("a")]).0)[0].1,
        StackOutcome::Held
    );
    let o = outcomes(&f.manual(vec![f.job("a")]).0);
    assert!(
        matches!(&o[0].1, StackOutcome::Skipped { reason } if reason.starts_with("acknowledge")),
        "{o:?}"
    );
    let mut acked = f.job("a");
    acked.acknowledged = true;
    assert_eq!(
        outcomes(&f.manual(vec![acked.clone()]).0)[0].1,
        StackOutcome::Updated
    );
    f.ops.set_compose_policy(ComposePolicy::Enforce);
    let o = outcomes(&f.manual(vec![acked]).0);
    assert!(
        matches!(&o[0].1, StackOutcome::Skipped { reason } if reason.contains("compose_blocked") && reason.contains("app:privileged")),
        "{o:?}"
    );
}

#[test]
fn a_guard_refusal_from_up_with_the_old_container_running_is_held() {
    let f = fx();
    f.fail(
        "docker compose",
        "Error: refused by depths-guard: privileged",
    );
    let (run, _) = f.manual(vec![f.job("a")]);
    assert_eq!(outcomes(&run)[0].1, StackOutcome::Held);
    let ran = f.ran();
    assert_eq!(
        ran.last().unwrap(),
        "docker image tag sha256:cfgnew depths-keep/a:app-held"
    );
    assert!(
        ran.contains(&"docker image tag sha256:old nginx:1".to_owned()),
        "the reference is put back"
    );
    let e = f.entries();
    assert!(e.iter().any(|e| e.action == AuditAction::ImageHold
        && e.update.as_ref().unwrap().hold.as_deref() == Some("guard")));
}

// --- health (research R3) ----------------------------------------------------------------------

fn wait(f: &Fx, expect: usize, timeout: u32) -> Result<(), String> {
    f.runner.wait(
        &StackName::parse("a").unwrap(),
        &["app".into()],
        expect,
        timeout,
    )
}

#[test]
fn health_wait() {
    let f = fx();
    let c = |h, r| vec![health("c-a", h, r)];

    f.health(vec![
        c(HealthStatus::Starting, 0),
        c(HealthStatus::Healthy, 0),
    ]);
    assert_eq!(wait(&f, 1, 10), Ok(()));

    f.fake.health_reads.store(0, Ordering::SeqCst);
    f.health(vec![c(HealthStatus::Unhealthy, 0)]);
    assert!(wait(&f, 1, 10).unwrap_err().contains("unhealthy"));
    assert_eq!(f.fake.health_reads.load(Ordering::SeqCst), 1, "fails early");

    // No healthcheck: running with an unchanged restart count for the whole timeout.
    f.fake.health_reads.store(0, Ordering::SeqCst);
    f.health(vec![c(HealthStatus::None, 3)]);
    assert_eq!(wait(&f, 1, 10), Ok(()));
    assert_eq!(
        f.fake.health_reads.load(Ordering::SeqCst),
        6,
        "10s at 2s: six reads"
    );

    f.health(vec![c(HealthStatus::None, 0), c(HealthStatus::None, 1)]);
    assert!(wait(&f, 1, 10).unwrap_err().contains("restarting"));

    f.health(vec![c(HealthStatus::Healthy, 0)]);
    assert!(
        wait(&f, 2, 10).unwrap_err().contains("1 of 2"),
        "missing replica"
    );

    f.health(vec![c(HealthStatus::Starting, 0)]);
    assert!(
        wait(&f, 1, 10)
            .unwrap_err()
            .contains("not healthy after 10s")
    );

    let mut exited = health("c-a", HealthStatus::None, 0);
    exited.running = false;
    exited.status = "exited (1)".into();
    f.health(vec![vec![exited]]);
    assert!(wait(&f, 1, 10).unwrap_err().contains("exited"));
}

// --- rollback ----------------------------------------------------------------------------------

#[test]
fn a_failed_wait_rolls_back_rejects_the_digest_and_keeps_the_image_held() {
    let f = fx();
    f.health(vec![
        vec![health("c-a", HealthStatus::Unhealthy, 0)],
        vec![health("c-a", HealthStatus::Healthy, 0)],
    ]);
    let (run, _) = f.manual(vec![f.job("a")]);
    assert_eq!(outcomes(&run)[0].1, StackOutcome::RolledBack);
    let ran = f.ran();
    let up = "docker compose up -d --no-deps --pull never app";
    let ups: Vec<usize> = ran
        .iter()
        .enumerate()
        .filter(|(_, r)| *r == up)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(ups.len(), 2);
    let back = ran
        .iter()
        .position(|r| r == "docker image tag sha256:old nginx:1")
        .unwrap();
    assert!(ups[0] < back && back < ups[1]);
    assert!(ran.contains(&"docker image tag sha256:cfgnew depths-keep/a:app-held".to_owned()));
    let d = f.updates.store().decisions().unwrap();
    assert!(
        d.rejected
            .iter()
            .any(|r| r.stack == "a" && r.digest == "sha256:new" && r.run_id == Some(run.run_id))
    );
    let e = f.entries();
    let rb = e
        .iter()
        .find(|e| e.action == AuditAction::ImageRollback)
        .unwrap();
    assert_eq!(
        rb.update.as_ref().unwrap().rejected_digest.as_deref(),
        Some("sha256:new")
    );
}

#[test]
fn failing_again_is_rollback_failed_with_no_third_attempt() {
    let f = fx();
    f.health(vec![vec![health("c-a", HealthStatus::Unhealthy, 0)]]);
    let (run, _) = f.manual(vec![f.job("a")]);
    assert_eq!(outcomes(&run)[0].1, StackOutcome::RollbackFailed);
    let ups = f
        .ran()
        .iter()
        .filter(|r| r.starts_with("docker compose"))
        .count();
    assert_eq!(ups, 2);
}

// --- journal (FR-018) --------------------------------------------------------------------------

#[test]
fn the_phase_is_on_disk_before_each_mutating_step() {
    let f = fx();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (s, u) = (seen.clone(), f.updates.clone());
    *f.fake.hook.lock().unwrap() = Some(Box::new(move |argv| {
        let phase = u
            .store()
            .observed()
            .open_runs()
            .next()
            .map(|r| r.stacks[0].phase);
        s.lock().unwrap().push((argv.to_owned(), phase));
    }));
    f.manual(vec![f.job("a")]);
    let seen = seen.lock().unwrap();
    for (argv, phase) in seen.iter() {
        let want = if argv.starts_with("docker pull") {
            RunPhase::Pulling
        } else {
            RunPhase::Recreating
        };
        assert_eq!(*phase, Some(want), "{argv}");
    }
}

#[test]
fn an_open_run_found_at_start_is_interrupted_and_nothing_resumes() {
    let f = fx();
    let run_id = RunId(uuid::Uuid::new_v4());
    let stack_run = |stack: &str, phase| StackRun {
        stack: stack.into(),
        services: Vec::new(),
        operation_id: None,
        phase,
        outcome: None,
    };
    f.updates
        .store()
        .update_observed(|o| {
            o.upsert_run(UpdateRun {
                run_id,
                trigger: RunTrigger::Scheduled {
                    on_behalf_of: [("a".to_owned(), admin())].into(),
                },
                stacks: vec![
                    stack_run("a", RunPhase::Recreating),
                    stack_run("b", RunPhase::Pending),
                ],
                started_at: "2026-10-01T03:00:00Z".into(),
                finished_at: None,
            })
        })
        .unwrap();
    assert_eq!(interrupt_open_runs(&f.updates, &f.ops), 1);
    assert!(f.ran().is_empty(), "nothing resumed or rolled back");
    let j = f.updates.store().observed().journal;
    let r = j.iter().find(|r| r.run_id == run_id).unwrap();
    assert!(r.finished_at.is_some());
    assert_eq!(r.stacks[0].outcome, Some(StackOutcome::Interrupted));
    assert_eq!(r.stacks[1].outcome, Some(skipped("interrupted")));
    let e = f.entries();
    let i = e
        .iter()
        .find(|e| e.action == AuditAction::UpdateInterrupted)
        .unwrap();
    assert_eq!(i.actor.username, "Depths (scheduled)");
    assert_eq!(i.on_behalf_of.as_ref().unwrap().username, "admin");
    assert!(
        i.diff_summary.as_deref().unwrap().contains("app=nginx:1"),
        "the real images"
    );
    assert_eq!(interrupt_open_runs(&f.updates, &f.ops), 0);
}

// --- scheduled runs (T060) ---------------------------------------------------------------------

#[test]
fn a_scheduled_run_acts_as_depths_on_behalf_of_the_policy_author() {
    let f = fx();
    let seen = Arc::new(Mutex::new(None));
    let (s, ops) = (seen.clone(), f.ops.clone());
    *f.fake.hook.lock().unwrap() = Some(Box::new(move |_| {
        if let Err(Refusal::Busy { actor, .. }) =
            ops.claim_update("a", admin(), "x".into(), CancelToken::new())
        {
            *s.lock().unwrap() = Some(actor.username);
        }
    }));
    let (run, _) = f.run(
        RunTrigger::Scheduled {
            on_behalf_of: [("a".to_owned(), admin())].into(),
        },
        vec![f.job("a")],
    );
    assert_eq!(outcomes(&run)[0].1, StackOutcome::Updated);
    assert_eq!(
        seen.lock().unwrap().as_deref(),
        Some("Depths (scheduled)"),
        "a busy refusal names it"
    );
    let e = f.entries();
    let u = e
        .iter()
        .find(|e| e.action == AuditAction::ImageUpdate)
        .unwrap();
    assert_eq!(u.actor, scheduled_actor());
    assert_eq!(u.on_behalf_of, Some(admin()));
}

#[test]
fn a_held_auto_stack_is_never_applied_and_rollback_failed_waits() {
    let f = fx();
    f.fake
        .images
        .lock()
        .unwrap()
        .get_mut("nginx@sha256:new")
        .unwrap()
        .user = Some("root".into());
    let scheduled = || RunTrigger::Scheduled {
        on_behalf_of: [("a".to_owned(), admin())].into(),
    };
    let mut job = f.job("a");
    job.apply_held = false;
    assert_eq!(
        outcomes(&f.run(scheduled(), vec![job]).0)[0].1,
        StackOutcome::Held
    );
    assert!(!f.ran().iter().any(|r| r.starts_with("docker compose")));
}

// --- cleanup (T076) ----------------------------------------------------------------------------

fn older_previous(f: &Fx) {
    f.fake.images.lock().unwrap().insert(
        "sha256:older".into(),
        ImageInfo {
            id: "sha256:older".into(),
            repo_digests: vec!["nginx@sha256:olderdigest".into()],
            ..ImageInfo::default()
        },
    );
    f.updates
        .store()
        .update_observed(|o| {
            o.keep.push(KeepRecord {
                stack: "a".into(),
                service: "app".into(),
                tag: "depths-keep/a:app-previous".into(),
                image_id: "sha256:older".into(),
                at: "2026-10-01T00:00:00Z".into(),
            })
        })
        .unwrap();
}

#[test]
fn cleanup_off_removes_nothing() {
    let f = fx();
    older_previous(&f);
    f.manual(vec![f.job("a")]);
    assert!(
        !f.ran()
            .iter()
            .any(|r| r.contains(" rm ") || r.contains("prune"))
    );
}

#[test]
fn cleanup_on_removes_the_image_the_previous_tag_held_once_healthy() {
    let f = fx();
    older_previous(&f);
    f.cleanup(true);
    let (run, _) = f.manual(vec![f.job("a")]);
    assert_eq!(outcomes(&run)[0].1, StackOutcome::Updated);
    let ran = f.ran();
    assert_eq!(ran.last().unwrap(), "docker image rm sha256:older");
    assert!(!ran.iter().any(|r| r.contains(" -f") || r.contains("prune")));
    let e = f.entries();
    let rm = e
        .iter()
        .find(|e| e.action == AuditAction::ImageRemove)
        .unwrap();
    let summary = rm.diff_summary.as_deref().unwrap();
    assert!(summary.contains("sha256:older") && summary.contains("nginx@sha256:olderdigest"));
    assert_eq!(rm.actor, admin());
    // The current rollback target is never removed.
    assert!(!ran.contains(&"docker image rm sha256:old".to_owned()));
}

#[test]
fn an_image_in_use_is_kept_with_a_note() {
    let f = fx();
    older_previous(&f);
    f.cleanup(true);
    f.fake.in_use.lock().unwrap().insert("sha256:older".into());
    let (run, evs) = f.manual(vec![f.job("a")]);
    assert_eq!(outcomes(&run)[0].1, StackOutcome::Updated, "not a failure");
    assert!(
        evs.iter().any(
            |e| matches!(e, Event::OpOutput { chunk, .. } if chunk.starts_with("kept: in use"))
        )
    );
    assert!(
        !f.entries()
            .iter()
            .any(|e| e.action == AuditAction::ImageRemove)
    );
}

#[test]
fn cleanup_never_removes_an_image_another_keep_tag_records_or_after_a_rollback() {
    let f = fx();
    older_previous(&f);
    f.cleanup(true);
    f.updates
        .store()
        .update_observed(|o| {
            o.keep.push(KeepRecord {
                stack: "b".into(),
                service: "app".into(),
                tag: "depths-keep/b:app-held".into(),
                image_id: "sha256:older".into(),
                at: "2026-10-01T00:00:00Z".into(),
            })
        })
        .unwrap();
    f.manual(vec![f.job("a")]);
    assert!(!f.ran().iter().any(|r| r.contains("image rm")));

    let g = fx();
    older_previous(&g);
    g.cleanup(true);
    g.health(vec![
        vec![health("c-a", HealthStatus::Unhealthy, 0)],
        vec![health("c-a", HealthStatus::Healthy, 0)],
    ]);
    assert_eq!(
        outcomes(&g.manual(vec![g.job("a")]).0)[0].1,
        StackOutcome::RolledBack
    );
    assert!(
        !g.ran().iter().any(|r| r.contains("image rm")),
        "a rollback cleans nothing"
    );
}
