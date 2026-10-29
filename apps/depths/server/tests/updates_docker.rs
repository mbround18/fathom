//! Spec 062 T049, T063, T078: image updates against a real daemon and a local `distribution`
//! registry (quickstart §2).
//!
//! Opt-in: `DEPTHS_IT_DOCKER=1 cargo test -p depths-server --test updates_docker`. Without
//! `DEPTHS_IT_DOCKER=1` every test returns at once. Run by the main session only.
//!
//! Each test starts its own registry container and `depths-it-upd-<random>` stacks. On drop it
//! removes, by name, what it created: the stacks' containers, the registry, the images it built and
//! pushed, and the stacks' `depths-keep/…` tags. It never touches other containers, volumes or
//! images. Images build `FROM busybox:1.37`, which needs Docker Hub once.
//!
//! The check reaches the registry through `Client::with_base_url`, as in production but over plain
//! http: the adapter refuses loopback hosts (`PlainHttp`), so [`Loopback`] presents the registry
//! as `registry.test` to it. Docker pulls from `localhost:<port>` as usual.

mod support;

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use depths_core::ports::CancelToken;
use depths_core::updates::keep::{self, KeepKind};
use depths_core::updates::reference::ImageRef;
use depths_core::updates::registry::{Platform, Registry, RegistryFuture, Resolution};
use depths_protocol::{Actor, RunId, RunTrigger, StackOutcome, UpdateRun, UpdateState};
use depths_runtime_cli::CliRuntime;
use depths_server::ops::Ops;
use depths_server::updates::batch::{self, Only};
use depths_server::updates::inputs::{Inputs, RuntimeInputs};
use depths_server::updates::registry::{OciRegistry, user_agent};
use depths_server::updates::run::{Batch, Runner};
use depths_server::updates::{SystemClock, Updates, scheduler};
use oci_registry::{Client, RequestCounts};
use support::{App, LOOPBACK, OWNER, app_with_env};

fn enabled() -> bool {
    std::env::var("DEPTHS_IT_DOCKER").as_deref() == Ok("1")
}

fn docker(args: &[&str]) -> std::process::Output {
    Command::new("docker")
        .args(args)
        .output()
        .expect("docker runs")
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// `docker …`, which must succeed.
fn ok(args: &[&str]) -> String {
    let out = docker(args);
    assert!(
        out.status.success(),
        "docker {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout(&out)
}

/// The image's id, or `None` when it is gone.
fn image_id(reference: &str) -> Option<String> {
    let out = docker(&["image", "inspect", "--format", "{{.Id}}", reference]);
    out.status.success().then(|| stdout(&out))
}

/// The daemon uses the containerd image store.
fn containerd_store() -> bool {
    ok(&["info", "--format", "{{json .DriverStatus}}"]).contains("io.containerd.snapshotter.v1")
}

/// The Dockerfile of one test image. `user` is the image's `USER`; `healthy` its healthcheck.
fn dockerfile(version: &str, user: &str, healthy: bool) -> String {
    let check = if healthy { "true" } else { "false" };
    format!(
        "FROM busybox:1.37\n\
         LABEL org.opencontainers.image.version={version}\n\
         USER {user}\n\
         HEALTHCHECK --interval=2s --timeout=2s --start-period=1s --retries=2 CMD {check}\n\
         CMD [\"sleep\", \"86400\"]\n"
    )
}

/// What the adapter's calls cost, one entry per call.
type Counts = Arc<Mutex<Vec<RequestCounts>>>;

/// The production adapter over `Client::with_base_url`, shown the registry as a non-loopback host.
struct Loopback(OciRegistry);

fn outside(r: &ImageRef) -> ImageRef {
    let mut r = r.clone();
    r.registry = "registry.test".into();
    r
}

impl Registry for Loopback {
    fn head_digest<'a>(&'a self, r: &'a ImageRef) -> RegistryFuture<'a, String> {
        Box::pin(async move {
            let r = outside(r);
            self.0.head_digest(&r).await
        })
    }

    fn resolve<'a>(
        &'a self,
        r: &'a ImageRef,
        digest: &'a str,
        p: &'a Platform,
    ) -> RegistryFuture<'a, Resolution> {
        Box::pin(async move {
            let r = outside(r);
            self.0.resolve(&r, digest, p).await
        })
    }
}

/// A registry, the real runtime under Depths, and the stacks a test made; removed on drop.
struct Lab {
    app: App,
    rt: tokio::runtime::Runtime,
    inputs: Arc<dyn Inputs>,
    counts: Counts,
    prefix: String,
    /// `localhost:<port>`.
    host: String,
    stacks: Vec<String>,
    built: Mutex<Vec<String>>,
    /// `held-…` tags that keep a rebuilt tag's previous image named until an update keeps it.
    holds: Mutex<Vec<String>>,
    containers: Mutex<Vec<String>>,
}

impl Lab {
    fn new() -> Lab {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let prefix = format!("depths-it-upd-{}", &id[..8]);
        let registry = format!("{prefix}-registry");
        ok(&[
            "run",
            "-d",
            "--name",
            &registry,
            "-p",
            "127.0.0.1::5000",
            "registry:2",
        ]);
        let bound = ok(&["port", &registry, "5000/tcp"]);
        let port = bound
            .lines()
            .next()
            .and_then(|l| l.rsplit(':').next())
            .expect("a published port")
            .to_owned();
        let deadline = Instant::now() + Duration::from_secs(20);
        while std::net::TcpStream::connect(("127.0.0.1", port.parse::<u16>().unwrap())).is_err() {
            assert!(Instant::now() < deadline, "the registry did not start");
            std::thread::sleep(Duration::from_millis(200));
        }
        let base: url::Url = format!("http://127.0.0.1:{port}").parse().unwrap();
        let counts: Counts = Arc::default();
        let sink = counts.clone();
        let docker_config = tempfile::tempdir().unwrap().keep();
        let app = app_with_env(&[("DEPTHS_UPDATES", "on")], move |s| {
            s.runtime = Arc::new(CliRuntime::new("docker"));
            s.ops = Arc::new(Ops::new(
                s.store.clone(),
                s.runtime.clone(),
                s.audit.clone(),
                s.watch.clone(),
            ));
            let u = Updates::open(&s.cfg, Arc::new(SystemClock), move || {
                let client = Client::with_base_url(&user_agent(), base);
                Arc::new(Loopback(OciRegistry::with_client(
                    client,
                    docker_config,
                    Box::new(move |_, c| sink.lock().unwrap().push(c)),
                )))
            })
            .unwrap();
            *s = s.clone().with_updates(u);
        });
        let inputs: Arc<dyn Inputs> = Arc::new(RuntimeInputs {
            store: app.state.store.clone(),
            runtime: app.state.runtime.clone(),
            hostname: None,
        });
        let lab = Lab {
            app,
            rt: tokio::runtime::Runtime::new().unwrap(),
            inputs,
            counts,
            prefix,
            host: format!("localhost:{port}"),
            stacks: Vec::new(),
            built: Mutex::default(),
            holds: Mutex::default(),
            containers: Mutex::new(vec![registry]),
        };
        lab.settings(false);
        lab
    }

    fn u(&self) -> &Arc<Updates> {
        self.app.state.updates.as_ref().unwrap()
    }

    /// Checks on, the shortest health timeout, and `cleanup`.
    fn settings(&self, cleanup: bool) {
        self.u()
            .store()
            .update_decisions(|d| {
                d.settings.checks_enabled = true;
                d.settings.health_timeout_secs = 30;
                d.settings.cleanup = cleanup;
            })
            .unwrap();
    }

    /// `<host>/<prefix>/<repo>:<tag>`.
    fn reference(&self, repo: &str, tag: &str) -> String {
        format!("{}/{}/{repo}:{tag}", self.host, self.prefix)
    }

    /// Builds `dockerfile` as `repo:tag`, pushes it, and returns the image id.
    ///
    /// The build moves the local tag. A container may still run the image it pointed to, and on
    /// the containerd image store an image the build left without a name is gone (no inspect, no
    /// `docker tag` by id), where a real `docker pull` would have kept it. So the previous image is
    /// first tagged `<repo>:held-<id>`, which [`Lab::update`] releases once the run has kept it.
    fn push(&self, repo: &str, tag: &str, dockerfile: &str) -> String {
        let reference = self.reference(repo, tag);
        if let Some(old) = image_id(&reference) {
            let short = old.trim_start_matches("sha256:");
            let hold = self.reference(repo, &format!("held-{}", &short[..short.len().min(12)]));
            ok(&["tag", &old, &hold]);
            self.holds.lock().unwrap().push(hold);
        }
        let mut child = Command::new("docker")
            .args(["build", "-q", "-t", &reference, "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("docker build runs");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(dockerfile.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "build {reference}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let id = stdout(&out);
        self.built.lock().unwrap().push(id.clone());
        ok(&["push", "-q", &reference]);
        id
    }

    /// Writes the stack `<prefix>-<suffix>` and starts it with compose, healthy.
    fn stack(&mut self, suffix: &str, compose: &str) -> String {
        let name = format!("{}-{suffix}", self.prefix);
        self.app.add_stack(&name, compose, None);
        self.stacks.push(name.clone());
        let out = Command::new("docker")
            .args(["compose", "up", "-d", "--wait"])
            .current_dir(self.dir(&name))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "compose up {name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        name
    }

    fn dir(&self, stack: &str) -> PathBuf {
        self.app.stacks.join(stack)
    }

    fn compose_bytes(&self, stack: &str) -> Vec<u8> {
        std::fs::read(self.dir(stack).join("compose.yaml")).unwrap()
    }

    /// The id of the service's container.
    fn container(&self, stack: &str, service: &str) -> String {
        let out = Command::new("docker")
            .args(["compose", "ps", "-q", service])
            .current_dir(self.dir(stack))
            .output()
            .unwrap();
        stdout(&out)
    }

    /// One scheduled check pass over `stack`; returns the requests it made, per call.
    fn check(&self, stack: &str) -> Vec<RequestCounts> {
        self.counts.lock().unwrap().clear();
        self.rt.block_on(scheduler::pass(
            self.u(),
            self.inputs.clone(),
            vec![stack.to_owned()],
        ));
        self.counts.lock().unwrap().clone()
    }

    fn state(&self, stack: &str, service: &str) -> UpdateState {
        self.u().store().observed().services[&format!("{stack}/{service}")].state
    }

    /// Previews and runs the update of `stack`'s available services, as a confirmed manual run.
    fn update(&self, stack: &str) -> UpdateRun {
        let preview = batch::preview_stack(
            self.u(),
            &*self.app.state.store,
            &*self.inputs,
            self.app.state.ops.compose_policy(),
            stack,
            Only::Available,
        )
        .unwrap();
        let mut job = preview.job.expect("an update to run");
        job.acknowledged = true;
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let drain = std::thread::spawn(move || while rx.blocking_recv().is_some() {});
        let _context = self.rt.enter();
        let run = Runner {
            ops: self.app.state.ops.clone(),
            updates: self.u().clone(),
            sleep: Arc::new(std::thread::sleep),
        }
        .run(
            Batch {
                run_id: RunId(uuid::Uuid::new_v4()),
                trigger: RunTrigger::Manual { actor: owner() },
                ip: LOOPBACK,
                jobs: vec![job],
                cancel: CancelToken::new(),
                reply_to: None,
                reply_on_op: false,
            },
            tx,
        );
        drain.join().unwrap();
        self.release();
        run
    }

    /// Removes the `held-…` tags (the tag only: the image stays while another name or a container
    /// keeps it).
    fn release(&self) {
        for hold in self.holds.lock().unwrap().drain(..) {
            let _ = docker(&["image", "rm", &hold]);
        }
    }

    /// Nothing would run for `stack`.
    fn nothing_to_update(&self, stack: &str) -> bool {
        batch::preview_stack(
            self.u(),
            &*self.app.state.store,
            &*self.inputs,
            self.app.state.ops.compose_policy(),
            stack,
            Only::Available,
        )
        .map(|p| p.job.is_none())
        .unwrap_or(true)
    }

    /// The registry's access log lines since start.
    fn access_log(&self) -> Vec<String> {
        let out = docker(&["logs", &format!("{}-registry", self.prefix)]);
        let mut all = String::from_utf8_lossy(&out.stdout).into_owned();
        all.push_str(&String::from_utf8_lossy(&out.stderr));
        all.lines()
            .filter(|l| l.contains("/v2/"))
            .map(str::to_owned)
            .collect()
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        for stack in &self.stacks {
            let _ = Command::new("docker")
                .args(["compose", "down", "--timeout", "1"])
                .current_dir(self.dir(stack))
                .output();
        }
        for c in self.containers.lock().unwrap().iter() {
            let _ = docker(&["rm", "-f", c]);
        }
        self.release();
        let listed = docker(&["image", "ls", "--format", "{{.Repository}}:{{.Tag}}"]);
        let ours = format!("{}/{}/", self.host, self.prefix);
        let keeps: Vec<String> = self.stacks.iter().map(|s| keep::repository(s)).collect();
        for tag in stdout(&listed).lines() {
            if tag.starts_with(&ours) || keeps.iter().any(|k| tag.starts_with(&format!("{k}:"))) {
                let _ = docker(&["image", "rm", tag]);
            }
        }
        for id in self.built.lock().unwrap().iter() {
            let _ = docker(&["image", "rm", id]);
        }
    }
}

fn owner() -> Actor {
    serde_json::from_value(serde_json::json!({
        "provider": "discord", "subject": OWNER, "username": OWNER,
    }))
    .unwrap()
}

/// `app` from our registry, plus `side`, which no test updates.
fn two_services(lab: &Lab) -> String {
    format!(
        "services:\n  app:\n    image: {}\n  side:\n    image: {}\n",
        lab.reference("app", "1"),
        lab.reference("side", "1")
    )
}

fn sum(calls: &[RequestCounts]) -> RequestCounts {
    calls
        .iter()
        .fold(RequestCounts::default(), |a, c| RequestCounts {
            head: a.head + c.head,
            get: a.get + c.get,
            manifest_get: a.manifest_get + c.manifest_get,
        })
}

fn outcome(run: &UpdateRun) -> Option<StackOutcome> {
    run.stacks.first().and_then(|s| s.outcome.clone())
}

// --- US2 (T049) ----------------------------------------------------------------------------------

#[test]
fn an_unchanged_tag_is_head_only_and_a_pushed_image_is_resolved_once() {
    if !enabled() {
        return;
    }
    let mut lab = Lab::new();
    lab.push("app", "1", &dockerfile("1", "1000", true));
    lab.push("side", "1", &dockerfile("1", "1000", true));
    let stack = lab.stack("head", &two_services(&lab));
    lab.check(&stack);

    let before = lab.access_log().len();
    let calls = lab.check(&stack);
    let total = sum(&calls);
    assert!(total.head >= 1, "{calls:?}");
    assert_eq!(total.get, 0, "an unchanged tag is HEAD only: {calls:?}");
    assert_eq!(lab.state(&stack, "app"), UpdateState::UpToDate);
    let log = lab.access_log();
    assert!(
        log[before..].iter().all(|l| !l.contains("\"GET /v2/")),
        "no GET reached the registry: {:?}",
        &log[before..]
    );

    lab.push("app", "1", &dockerfile("2", "1000", true));
    let calls = lab.check(&stack);
    // The new digest is resolved once. On the containerd image store the running image's id is
    // the digest it was pulled by, so the checker also resolves that once to compare platform
    // manifests (check.rs `same_image`); both are cached from then on.
    let expected = if containerd_store() { 2 } else { 1 };
    assert_eq!(
        calls.iter().filter(|c| c.get > 0).count(),
        expected,
        "one resolution per digest never seen: {calls:?}"
    );
    assert_eq!(lab.state(&stack, "app"), UpdateState::Available);
    let calls = lab.check(&stack);
    assert_eq!(sum(&calls).get, 0, "resolved once, then cached: {calls:?}");
    let observed = lab.u().store().observed();
    let available = observed.services[&format!("{stack}/app")]
        .available
        .clone()
        .unwrap();
    assert_eq!(available.version.as_deref(), Some("2"));
    assert_eq!(lab.state(&stack, "side"), UpdateState::UpToDate);
}

#[test]
fn a_multi_arch_repush_of_the_same_image_is_not_an_update() {
    if !enabled() {
        return;
    }
    if !docker(&["buildx", "version"]).status.success() {
        eprintln!("skipped: docker buildx is not installed (imagetools makes the index)");
        return;
    }
    let mut lab = Lab::new();
    lab.push("app", "1", &dockerfile("1", "1000", true));
    lab.push("side", "1", &dockerfile("1", "1000", true));
    let stack = lab.stack("multi", &two_services(&lab));
    lab.check(&stack);
    // An index over the same host manifest: the tag's digest changes, the image does not.
    let reference = lab.reference("app", "1");
    ok(&[
        "buildx",
        "imagetools",
        "create",
        "-t",
        &reference,
        &reference,
    ]);
    lab.check(&stack);
    assert_eq!(lab.state(&stack, "app"), UpdateState::UpToDate);
    assert!(lab.nothing_to_update(&stack));
}

/// A newer image pulled by hand takes the tag, and the image the container runs is left without a
/// name (under the containerd store the daemon no longer inspects it). The service still shows
/// its container, by image id, with the new image waiting.
#[test]
fn a_hand_pulled_tag_leaves_the_running_image_unnamed_and_the_service_updatable() {
    if !enabled() {
        return;
    }
    let mut lab = Lab::new();
    lab.push("app", "1", &dockerfile("1", "1000", true));
    lab.push("side", "1", &dockerfile("1", "1000", true));
    let stack = lab.stack("unnamed", &two_services(&lab));
    let old = image_id(&lab.reference("app", "1")).unwrap();

    // As `docker pull` would leave it: the tag names the new image, the old one has no name.
    lab.push("app", "1", &dockerfile("2", "1000", true));
    lab.release();
    assert_ne!(image_id(&lab.reference("app", "1")), Some(old.clone()));
    lab.check(&stack);

    let observed = lab.u().store().observed();
    let app = &observed.services[&format!("{stack}/app")];
    assert_eq!(app.state, UpdateState::Available, "{:?}", app.reason);
    assert_eq!(app.reason, None);
    assert_eq!(
        app.running.as_ref().map(|r| r.image_id.as_str()),
        Some(old.as_str()),
        "matched by the container's image id"
    );
    assert!(!lab.nothing_to_update(&stack));
}

/// The update flow both store tests run.
fn update_recreates_only_the_changed_service(lab: &mut Lab, suffix: &str) {
    lab.push("app", "1", &dockerfile("1", "1000", true));
    lab.push("side", "1", &dockerfile("1", "1000", true));
    let stack = lab.stack(suffix, &two_services(lab));
    let compose = lab.compose_bytes(&stack);
    let (old_app, old_side) = (lab.container(&stack, "app"), lab.container(&stack, "side"));
    let old_image = image_id(&lab.reference("app", "1")).unwrap();

    let new_image = lab.push("app", "1", &dockerfile("2", "1000", true));
    // The build retagged the local image; point the tag back, so the runner's tag step shows.
    ok(&["tag", &old_image, &lab.reference("app", "1")]);
    lab.check(&stack);
    assert_eq!(lab.state(&stack, "app"), UpdateState::Available);
    // Research R3 below reads the app's container events. They are followed live through the
    // run: the daemon replays only its last 256 events, and the healthchecks of tests running in
    // parallel push the `create` out of that window before a later `docker events --until` asks.
    let since = chrono::Utc::now().timestamp().to_string();
    let mut follow = Command::new("docker")
        .args([
            "events",
            "--since",
            &since,
            "--filter",
            "type=container",
            "--filter",
            &format!("label=com.docker.compose.project={stack}"),
            "--filter",
            "label=com.docker.compose.service=app",
            "--format",
            "{{.Action}} {{.Actor.ID}}",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("docker events runs");
    let run = lab.update(&stack);
    std::thread::sleep(Duration::from_millis(500));
    let _ = follow.kill();
    let events = stdout(&follow.wait_with_output().unwrap());
    assert_eq!(outcome(&run), Some(StackOutcome::Updated), "{run:?}");

    let new_app = lab.container(&stack, "app");
    assert_ne!(new_app, old_app, "app recreated");
    assert_eq!(lab.container(&stack, "side"), old_side, "side untouched");
    assert_eq!(
        lab.compose_bytes(&stack),
        compose,
        "compose file byte-identical"
    );
    assert_eq!(image_id(&lab.reference("app", "1")), Some(new_image));
    let previous = keep::tag(&stack, "app", KeepKind::Previous);
    assert_eq!(
        image_id(&previous),
        Some(old_image),
        "the old image is kept"
    );

    // Research R3: compose creates the replacement before it stops the old container.
    let lines: Vec<(&str, &str)> = events.lines().filter_map(|l| l.split_once(' ')).collect();
    // `compose ps -q` and the events may print ids of different lengths.
    let same = |a: &str, b: &str| !a.is_empty() && (a.starts_with(b) || b.starts_with(a));
    let created = lines
        .iter()
        .position(|(action, id)| *action == "create" && same(id, &new_app))
        .unwrap_or_else(|| panic!("no create of the new container: {lines:?}"));
    let stopped = lines
        .iter()
        .position(|(action, id)| matches!(*action, "stop" | "kill") && same(id, &old_app))
        .unwrap_or_else(|| panic!("no stop of the old container: {lines:?}"));
    assert!(created < stopped, "created before stopped: {lines:?}");
}

#[test]
fn an_update_recreates_only_the_changed_service_and_leaves_compose_byte_identical() {
    if !enabled() {
        return;
    }
    let mut lab = Lab::new();
    update_recreates_only_the_changed_service(&mut lab, "upd");
}

/// SC-005.
#[test]
fn an_image_that_turns_root_is_held_with_tag_and_container_unchanged() {
    if !enabled() {
        return;
    }
    let mut lab = Lab::new();
    lab.push("app", "1", &dockerfile("1", "1000", true));
    lab.push("side", "1", &dockerfile("1", "1000", true));
    let stack = lab.stack("root", &two_services(&lab));
    let container = lab.container(&stack, "app");
    let image = image_id(&lab.reference("app", "1")).unwrap();

    lab.push("app", "1", &dockerfile("2", "root", true));
    // The push retagged the local image; point the tag back as a plain pull would have left it.
    ok(&["tag", &image, &lab.reference("app", "1")]);
    lab.check(&stack);
    if lab.nothing_to_update(&stack) {
        // Held at check time: nothing runs.
        assert_eq!(lab.state(&stack, "app"), UpdateState::Held);
    } else {
        let run = lab.update(&stack);
        assert_eq!(outcome(&run), Some(StackOutcome::Held), "{run:?}");
        assert!(image_id(&keep::tag(&stack, "app", KeepKind::Held)).is_some());
    }
    assert_eq!(
        lab.container(&stack, "app"),
        container,
        "container unchanged"
    );
    assert_eq!(
        image_id(&lab.reference("app", "1")),
        Some(image),
        "tag unchanged"
    );
}

#[test]
fn the_same_update_on_the_containerd_image_store() {
    if !enabled() {
        return;
    }
    let info = ok(&["info", "--format", "{{json .DriverStatus}}"]);
    if !info.contains("io.containerd.snapshotter.v1") {
        eprintln!("skipped: the daemon does not use the containerd image store ({info})");
        return;
    }
    let mut lab = Lab::new();
    update_recreates_only_the_changed_service(&mut lab, "ctrd");
}

// --- US3 (T063) ----------------------------------------------------------------------------------

/// SC-004.
#[test]
fn a_failing_healthcheck_is_rolled_back_in_time_and_skipped_next_pass() {
    if !enabled() {
        return;
    }
    let mut lab = Lab::new();
    lab.push("app", "1", &dockerfile("1", "1000", true));
    lab.push("side", "1", &dockerfile("1", "1000", true));
    let stack = lab.stack("rollback", &two_services(&lab));
    let old_image = image_id(&lab.reference("app", "1")).unwrap();

    lab.push("app", "1", &dockerfile("2", "1000", false));
    ok(&["tag", &old_image, &lab.reference("app", "1")]);
    lab.check(&stack);
    let digest = lab.u().store().observed().services[&format!("{stack}/app")]
        .available
        .clone()
        .unwrap()
        .digest;

    let started = Instant::now();
    let run = lab.update(&stack);
    let limit = Duration::from_secs(30 + 60);
    assert!(
        started.elapsed() <= limit,
        "rolled back in {:?}",
        started.elapsed()
    );
    assert_eq!(outcome(&run), Some(StackOutcome::RolledBack), "{run:?}");
    assert_eq!(
        image_id(&lab.reference("app", "1")),
        Some(old_image),
        "the tag points to the old image"
    );
    assert!(image_id(&keep::tag(&stack, "app", KeepKind::Held)).is_some());
    let decisions = lab.u().store().decisions().unwrap();
    assert!(
        decisions
            .rejected
            .iter()
            .any(|r| r.stack == stack && r.service == "app" && r.digest == digest),
        "{:?}",
        decisions.rejected
    );

    lab.check(&stack);
    assert_eq!(lab.state(&stack, "app"), UpdateState::Rejected);
    assert!(lab.nothing_to_update(&stack), "the next pass skips it");
}

// --- US5 (T078) ----------------------------------------------------------------------------------

#[test]
fn cleanup_removes_only_the_image_from_two_updates_ago_and_keeps_one_in_use() {
    if !enabled() {
        return;
    }
    let mut lab = Lab::new();
    lab.settings(true);
    let v1 = lab.push("app", "1", &dockerfile("1", "1000", true));
    lab.push("side", "1", &dockerfile("1", "1000", true));
    let stack = lab.stack("cleanup", &two_services(&lab));

    let v2 = lab.push("app", "1", &dockerfile("2", "1000", true));
    lab.check(&stack);
    assert_eq!(outcome(&lab.update(&stack)), Some(StackOutcome::Updated));
    assert!(image_id(&v1).is_some(), "v1 is the previous image: kept");

    let v3 = lab.push("app", "1", &dockerfile("3", "1000", true));
    lab.check(&stack);
    assert_eq!(outcome(&lab.update(&stack)), Some(StackOutcome::Updated));
    assert!(image_id(&v1).is_none(), "v1, two updates ago, removed");
    assert!(image_id(&v2).is_some(), "v2 is the previous image: kept");

    // Another container uses v2: the next cleanup keeps it.
    let pin = format!("{}-pin", lab.prefix);
    ok(&["run", "-d", "--name", &pin, &v2, "sleep", "86400"]);
    lab.containers.lock().unwrap().push(pin);
    lab.push("app", "1", &dockerfile("4", "1000", true));
    lab.check(&stack);
    assert_eq!(outcome(&lab.update(&stack)), Some(StackOutcome::Updated));
    assert!(image_id(&v2).is_some(), "in use by another container: kept");
    assert!(image_id(&v3).is_some(), "v3 is the previous image: kept");
}
