//! Real-Docker harness for the image-update tests (spec 062): a local `distribution` registry,
//! the real runtime under Depths, and the stacks a test made, removed by name on drop.
//!
//! The check reaches the registry through `Client::with_base_url`, as in production but over plain
//! http: the adapter refuses loopback hosts (`PlainHttp`), so [`Loopback`] presents the registry
//! as `registry.test` to it. Docker pulls from `localhost:<port>` as usual.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use depths_core::ports::CancelToken;
use depths_core::updates::keep;
use depths_core::updates::reference::ImageRef;
use depths_core::updates::registry::{Platform, Registry, RegistryFuture, Resolution};
use depths_protocol::{Actor, RunId, RunTrigger, UpdateRun, UpdateState};
use depths_runtime_cli::CliRuntime;
use depths_server::ops::Ops;
use depths_server::updates::batch::{self, Only};
use depths_server::updates::inputs::{Inputs, RuntimeInputs};
use depths_server::updates::registry::{OciRegistry, user_agent};
use depths_server::updates::run::{Batch, Runner};
use depths_server::updates::{SystemClock, Updates, scheduler};
use oci_registry::{Client, RequestCounts};

use super::{App, LOOPBACK, OWNER, app_in};

/// `DEPTHS_IT_DOCKER=1`: the real-Docker tests run.
pub fn enabled() -> bool {
    std::env::var("DEPTHS_IT_DOCKER").as_deref() == Ok("1")
}

pub fn docker(args: &[&str]) -> std::process::Output {
    Command::new("docker")
        .args(args)
        .output()
        .expect("docker runs")
}

pub fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// `docker …`, which must succeed.
pub fn ok(args: &[&str]) -> String {
    let out = docker(args);
    assert!(
        out.status.success(),
        "docker {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout(&out)
}

/// The image's id, or `None` when it is gone.
pub fn image_id(reference: &str) -> Option<String> {
    let out = docker(&["image", "inspect", "--format", "{{.Id}}", reference]);
    out.status.success().then(|| stdout(&out))
}

/// The daemon uses the containerd image store.
pub fn containerd_store() -> bool {
    ok(&["info", "--format", "{{json .DriverStatus}}"]).contains("io.containerd.snapshotter.v1")
}

/// The Dockerfile of one test image. `user` is the image's `USER`; `healthy` its healthcheck.
pub fn dockerfile(version: &str, user: &str, healthy: bool) -> String {
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
pub type Counts = Arc<Mutex<Vec<RequestCounts>>>;

/// The production adapter over `Client::with_base_url`, shown the registry as a non-loopback host.
pub struct Loopback(pub OciRegistry);

pub fn outside(r: &ImageRef) -> ImageRef {
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
pub struct Lab {
    pub app: App,
    pub rt: tokio::runtime::Runtime,
    pub inputs: Arc<dyn Inputs>,
    pub counts: Counts,
    pub prefix: String,
    /// `localhost:<port>`.
    pub host: String,
    pub stacks: Vec<String>,
    pub built: Mutex<Vec<String>>,
    /// `held-…` tags that keep a rebuilt tag's previous image named until an update keeps it.
    pub holds: Mutex<Vec<String>>,
    pub containers: Mutex<Vec<String>>,
    /// The scheduler and Auto tasks of [`Lab::start_tasks`], aborted by [`Lab::restart`].
    pub tasks: Vec<tokio::task::JoinHandle<()>>,
    /// What [`Lab::restart`] builds the app again with.
    base: url::Url,
    docker_config: PathBuf,
    env: Vec<(String, String)>,
}

impl Lab {
    pub fn new() -> Lab {
        Lab::with("depths-it-upd", &[])
    }

    /// A lab whose names start `<root>-<random>`, with extra environment for Depths.
    pub fn with(root: &str, env: &[(&str, &str)]) -> Lab {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let prefix = format!("{root}-{}", &id[..8]);
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
        let docker_config = tempfile::tempdir().unwrap().keep();
        let mut env: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        env.insert(0, ("DEPTHS_UPDATES".into(), "on".into()));
        let app = build(
            Arc::new(tempfile::tempdir().unwrap()),
            &env,
            &base,
            &docker_config,
            &counts,
        );
        let inputs = inputs_of(&app);
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
            tasks: Vec::new(),
            base,
            docker_config,
            env,
        };
        lab.settings(false);
        lab
    }

    /// Starts the check scheduler and the Auto queue, as `main` does.
    pub fn start_tasks(&mut self) {
        use depths_server::updates::auto;
        let u = self.u().clone();
        let deps = Arc::new(auto::Deps {
            ops: self.app.state.ops.clone(),
            inputs: self.inputs.clone(),
            zone: Arc::new(auto::HostZone),
            is_admin: auto::admins(self.app.state.users.clone()),
        });
        let check = self
            .rt
            .spawn(scheduler::run(u.clone(), self.inputs.clone()));
        self.tasks.push(check);
        self.tasks.push(self.rt.spawn(auto::run(u, deps)));
    }

    /// Depths stops and starts again on the same stacks and data dir, as after a crash: the tasks
    /// are aborted, a new app (and `Updates`) is built, and the start-up pass closes the runs left
    /// open, as `main` does. Returns how many it closed. The tasks are not started again.
    pub fn restart(&mut self) -> usize {
        for t in self.tasks.drain(..) {
            t.abort();
        }
        let app = build(
            self.app.dir(),
            &self.env,
            &self.base,
            &self.docker_config,
            &self.counts,
        );
        self.inputs = inputs_of(&app);
        self.app = app;
        depths_server::updates::run::interrupt_open_runs(self.u(), &self.app.state.ops)
    }

    pub fn u(&self) -> &Arc<Updates> {
        self.app.state.updates.as_ref().unwrap()
    }

    /// Checks on, the shortest health timeout, and `cleanup`.
    pub fn settings(&self, cleanup: bool) {
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
    pub fn reference(&self, repo: &str, tag: &str) -> String {
        format!("{}/{}/{repo}:{tag}", self.host, self.prefix)
    }

    /// Builds `dockerfile` as `repo:tag`, pushes it, and returns the image id.
    ///
    /// The build moves the local tag. A container may still run the image it pointed to, and on
    /// the containerd image store an image the build left without a name is gone (no inspect, no
    /// `docker tag` by id), where a real `docker pull` would have kept it. So the previous image is
    /// first tagged `<repo>:held-<id>`, which [`Lab::update`] releases once the run has kept it.
    pub fn push(&self, repo: &str, tag: &str, dockerfile: &str) -> String {
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
    pub fn stack(&mut self, suffix: &str, compose: &str) -> String {
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

    pub fn dir(&self, stack: &str) -> PathBuf {
        self.app.stacks.join(stack)
    }

    pub fn compose_bytes(&self, stack: &str) -> Vec<u8> {
        std::fs::read(self.dir(stack).join("compose.yaml")).unwrap()
    }

    /// The id of the service's container.
    pub fn container(&self, stack: &str, service: &str) -> String {
        let out = Command::new("docker")
            .args(["compose", "ps", "-q", service])
            .current_dir(self.dir(stack))
            .output()
            .unwrap();
        stdout(&out)
    }

    /// One scheduled check pass over `stack`; returns the requests it made, per call.
    pub fn check(&self, stack: &str) -> Vec<RequestCounts> {
        self.counts.lock().unwrap().clear();
        self.rt.block_on(scheduler::pass(
            self.u(),
            self.inputs.clone(),
            vec![stack.to_owned()],
        ));
        self.counts.lock().unwrap().clone()
    }

    pub fn state(&self, stack: &str, service: &str) -> UpdateState {
        self.u().store().observed().services[&format!("{stack}/{service}")].state
    }

    /// Previews and runs the update of `stack`'s available services, as a confirmed manual run.
    pub fn update(&self, stack: &str) -> UpdateRun {
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
    pub fn release(&self) {
        for hold in self.holds.lock().unwrap().drain(..) {
            let _ = docker(&["image", "rm", &hold]);
        }
    }

    /// Nothing would run for `stack`.
    pub fn nothing_to_update(&self, stack: &str) -> bool {
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
    pub fn access_log(&self) -> Vec<String> {
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

/// The app over `dir`: the real runtime, and `Updates` reaching the lab's registry.
fn build(
    dir: Arc<tempfile::TempDir>,
    env: &[(String, String)],
    base: &url::Url,
    docker_config: &std::path::Path,
    counts: &Counts,
) -> App {
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let (base, docker_config, sink) = (base.clone(), docker_config.to_owned(), counts.clone());
    app_in(dir, &env, move |s| {
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
    })
}

fn inputs_of(app: &App) -> Arc<dyn Inputs> {
    Arc::new(RuntimeInputs {
        store: app.state.store.clone(),
        runtime: app.state.runtime.clone(),
        hostname: None,
    })
}

pub fn owner() -> Actor {
    serde_json::from_value(serde_json::json!({
        "provider": "discord", "subject": OWNER, "username": OWNER,
    }))
    .unwrap()
}
