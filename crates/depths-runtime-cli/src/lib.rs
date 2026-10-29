//! `depths-runtime-cli`: the [`ContainerRuntime`] port over the `docker` CLI.
//!
//! - Every command is an argv. A plan's argv names `docker` first; this adapter runs the
//!   configured binary (`--docker-bin`) in its place and refuses any other program, so a plan can
//!   never start something that is not the runtime CLI. Nothing here runs a shell (T095).
//! - `DOCKER_HOST` and the rest of the environment pass through unchanged (the socket proxy).
//! - Output is streamed line by line to the [`OutputSink`]; the last [`STDERR_TAIL_BYTES`] of
//!   stderr come back verbatim in a failure (FR-013).
//! - Cancel sends SIGINT, as a terminal's Ctrl-C would, then kills after [`CliRuntime::grace`].
//!
//! Every call blocks; async callers run it on a blocking thread.

use std::collections::VecDeque;
use std::io::{BufRead as _, BufReader, Read, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use container_runtime::cli::Cli;
use container_runtime::inventory::{ContainerRow, list_containers_labelled};
use container_runtime::runner::{CmdOutput, RunError, Runner};
use depths_core::plan::{DOCKER, RunStep};
use depths_core::ports::{
    CancelToken, ContainerHealth, ContainerRuntime, HealthStatus, HostPlatform, ImageInfo,
    ImageStore, OutputChunk, OutputSink, OutputStream, ProjectContainers, RuntimeError,
    RuntimeSnapshot, ShellSession, StepOutcome, StreamId, StreamSink, UsageSample, Validation,
    ValidationIssue,
};
use depths_core::security::{DaemonMode, Inspect, valid_container_arg};
use depths_core::updates::registry::Platform;
use serde_json::Value;

pub mod helper;
mod streams;
pub use helper::{CliVolumeHelper, HelperPool};
pub use streams::{
    BACKLOG, BATCH_BYTES, BATCH_LINES, FLUSH_INTERVAL, LINE_CAP, WINDOW_BYTES, WINDOW_CAP,
};

/// The compose label naming the directory a project was started from (R1, T048).
pub const WORKING_DIR_LABEL: &str = "com.docker.compose.project.working_dir";

/// How much of stderr a failure carries back.
pub const STDERR_TAIL_BYTES: usize = 64 * 1024;

const POLL: Duration = Duration::from_millis(50);

/// The docker CLI as a [`ContainerRuntime`].
#[derive(Debug, Clone)]
pub struct CliRuntime {
    docker_bin: PathBuf,
    grace: Duration,
}

impl CliRuntime {
    /// `docker_bin` is run wherever a plan says `docker`.
    pub fn new(docker_bin: impl Into<PathBuf>) -> Self {
        Self {
            docker_bin: docker_bin.into(),
            grace: Duration::from_secs(10),
        }
    }

    /// How long a cancelled command has after SIGINT before it is killed.
    pub fn with_grace(mut self, grace: Duration) -> Self {
        self.grace = grace;
        self
    }

    pub fn grace(&self) -> Duration {
        self.grace
    }

    pub fn docker_bin(&self) -> &Path {
        &self.docker_bin
    }

    /// The argv without its program, or a refusal when the program is not `docker`.
    fn args_of<'a>(&self, argv: &'a [String]) -> Result<&'a [String], RuntimeError> {
        match argv.split_first() {
            Some((program, rest)) if program == DOCKER => Ok(rest),
            Some((program, _)) => Err(RuntimeError::Failed(format!(
                "refused to run {program:?}: only {DOCKER} is run"
            ))),
            None => Err(RuntimeError::Failed("empty command".into())),
        }
    }

    fn command(&self, args: &[String], cwd: Option<&Path>) -> Command {
        let mut cmd = Command::new(&self.docker_bin);
        cmd.args(args);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        cmd
    }

    fn spawn_error(&self, e: &std::io::Error) -> RuntimeError {
        if e.kind() == std::io::ErrorKind::NotFound {
            RuntimeError::Unavailable(format!("{} not found", self.docker_bin.display()))
        } else {
            RuntimeError::Unavailable(format!("{}: {e}", self.docker_bin.display()))
        }
    }

    /// Spawns `args` with piped output (and stdin when given), and drives it to the end.
    fn drive(
        &self,
        args: &[String],
        cwd: Option<&Path>,
        stdin: Option<&[u8]>,
        on_line: &dyn Fn(OutputChunk),
        cancel: &CancelToken,
    ) -> Result<Finished, RuntimeError> {
        let mut cmd = self.command(args, cwd);
        cmd.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| self.spawn_error(&e))?;

        if let (Some(data), Some(mut pipe)) = (stdin, child.stdin.take()) {
            let data = data.to_vec();
            // A child that exits without reading gives EPIPE here, which is not our error.
            std::thread::spawn(move || {
                let _ = pipe.write_all(&data);
            });
        }
        let (tx, rx) = mpsc::channel::<OutputChunk>();
        let mut readers = Vec::new();
        if let Some(out) = child.stdout.take() {
            readers.push(pump(out, OutputStream::Stdout, tx.clone()));
        }
        if let Some(err) = child.stderr.take() {
            readers.push(pump(err, OutputStream::Stderr, tx.clone()));
        }
        drop(tx);

        let mut tail = Tail::default();
        let mut interrupted: Option<Instant> = None;
        loop {
            match rx.recv_timeout(POLL) {
                Ok(chunk) => {
                    if chunk.stream == OutputStream::Stderr {
                        tail.push(&chunk.text);
                    }
                    on_line(chunk);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if cancel.is_cancelled() {
                match interrupted {
                    None => {
                        interrupt(&child);
                        interrupted = Some(Instant::now());
                    }
                    Some(at) if at.elapsed() >= self.grace => {
                        let _ = child.kill();
                    }
                    Some(_) => {}
                }
            }
        }
        for r in readers {
            let _ = r.join();
        }
        let status = wait(&mut child)?;
        Ok(Finished {
            status,
            cancelled: interrupted.is_some() || cancel.is_cancelled(),
            stderr_tail: tail.into_string(),
        })
    }
}

struct Finished {
    status: ExitStatus,
    cancelled: bool,
    stderr_tail: String,
}

fn wait(child: &mut Child) -> Result<ExitStatus, RuntimeError> {
    child
        .wait()
        .map_err(|e| RuntimeError::Failed(format!("waiting for the command: {e}")))
}

fn interrupt(child: &Child) {
    let pid = i32::try_from(child.id())
        .ok()
        .and_then(rustix::process::Pid::from_raw);
    if let Some(pid) = pid {
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::INT);
    }
}

/// Reads `from` line by line (bytes, lossily decoded) into `tx` on its own thread.
fn pump(
    from: impl Read + Send + 'static,
    stream: OutputStream,
    tx: mpsc::Sender<OutputChunk>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(from);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    let text = String::from_utf8_lossy(&buf).into_owned();
                    if tx.send(OutputChunk { stream, text }).is_err() {
                        return;
                    }
                }
            }
        }
    })
}

/// The last [`STDERR_TAIL_BYTES`] of stderr, whole lines.
#[derive(Default)]
struct Tail {
    lines: VecDeque<String>,
    bytes: usize,
}

impl Tail {
    fn push(&mut self, line: &str) {
        self.bytes += line.len();
        self.lines.push_back(line.to_owned());
        while self.bytes > STDERR_TAIL_BYTES && self.lines.len() > 1 {
            if let Some(old) = self.lines.pop_front() {
                self.bytes -= old.len();
            }
        }
    }

    fn into_string(self) -> String {
        self.lines.into_iter().collect()
    }
}

/// Maps `docker` to the configured binary for the shared inventory code.
struct Mapped<'a>(&'a CliRuntime);

impl Runner for Mapped<'_> {
    fn run(&self, bin: &str, args: &[&str]) -> Result<CmdOutput, RunError> {
        if bin != DOCKER {
            return Err(RunError::Unsupported);
        }
        let out = Command::new(&self.0.docker_bin)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => RunError::NotFound,
                _ => RunError::Other(e.to_string()),
            })?;
        Ok(CmdOutput {
            success: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

impl CliRuntime {
    fn unavailable(&self, e: RunError) -> RuntimeError {
        match e {
            RunError::NotFound => {
                RuntimeError::Unavailable(format!("{} not found", self.docker_bin.display()))
            }
            RunError::Unsupported => RuntimeError::Unsupported,
            // Docker's own words (FR-010): the daemon is down, the proxy refused, and so on.
            RunError::Other(stderr) => RuntimeError::Unavailable(stderr.trim_end().to_owned()),
        }
    }
}

impl ContainerRuntime for CliRuntime {
    fn containers_by_project(&self) -> Result<RuntimeSnapshot, RuntimeError> {
        let rows = list_containers_labelled(Cli::docker(), &Mapped(self), &[WORKING_DIR_LABEL])
            .map_err(|e| self.unavailable(e))?;
        Ok(group(rows))
    }

    fn run_step(
        &self,
        step: &RunStep,
        stdin: Option<&[u8]>,
        sink: &dyn OutputSink,
        cancel: &CancelToken,
    ) -> Result<StepOutcome, RuntimeError> {
        let args = self.args_of(&step.argv)?;
        let done = self.drive(
            args,
            Some(&step.cwd),
            stdin,
            &|chunk| {
                // A sink whose reader left (a closed socket) does not stop the operation.
                let _ = sink.emit(chunk);
            },
            cancel,
        )?;
        Ok(if done.cancelled {
            StepOutcome::Cancelled
        } else if done.status.success() {
            StepOutcome::Succeeded
        } else {
            StepOutcome::Failed {
                exit_code: done.status.code(),
                stderr_tail: done.stderr_tail,
            }
        })
    }

    fn validate(&self, step: &RunStep, draft: &[u8]) -> Result<Validation, RuntimeError> {
        let args = self.args_of(&step.argv)?;
        let done = self.drive(
            args,
            Some(&step.cwd),
            Some(draft),
            &|_| {},
            &CancelToken::new(),
        )?;
        if done.status.success() {
            Ok(Validation::Valid)
        } else {
            Ok(Validation::Invalid(issues(&done.stderr_tail)))
        }
    }

    fn compose_config(&self, step: &RunStep) -> Result<String, RuntimeError> {
        let args = self.args_of(&step.argv)?;
        let out = std::sync::Mutex::new(String::new());
        let done = self.drive(
            args,
            Some(&step.cwd),
            None,
            &|chunk| {
                if chunk.stream == OutputStream::Stdout {
                    let mut o = out.lock().unwrap_or_else(|e| e.into_inner());
                    o.push_str(&chunk.text);
                    o.push('\n');
                }
            },
            &CancelToken::new(),
        )?;
        if done.status.success() {
            Ok(out.into_inner().unwrap_or_else(|e| e.into_inner()))
        } else {
            Err(RuntimeError::Failed(done.stderr_tail.trim_end().to_owned()))
        }
    }

    fn ping(&self) -> Result<(), RuntimeError> {
        let out = Mapped(self)
            .run(DOCKER, &["version", "--format", "{{.Server.Version}}"])
            .map_err(|e| self.unavailable(e))?;
        if out.success {
            Ok(())
        } else {
            Err(RuntimeError::Unavailable(out.stderr.trim_end().to_owned()))
        }
    }

    fn follow_events(
        &self,
        on_event: &(dyn Fn() + Sync),
        cancel: &CancelToken,
    ) -> Result<(), RuntimeError> {
        let args: Vec<String> = ["events", "--format", "{{json .}}"]
            .iter()
            .map(|a| (*a).to_owned())
            .collect();
        let done = self.drive(
            &args,
            None,
            None,
            &|chunk| {
                if chunk.stream == OutputStream::Stdout && !chunk.text.trim().is_empty() {
                    on_event();
                }
            },
            cancel,
        )?;
        if done.cancelled {
            Ok(())
        } else {
            Err(RuntimeError::Unavailable(
                done.stderr_tail.trim_end().to_owned(),
            ))
        }
    }

    fn follow_logs(
        &self,
        step: &RunStep,
        stream_id: StreamId,
        sink: &dyn StreamSink,
        cancel: &CancelToken,
    ) -> Result<(), RuntimeError> {
        self.follow(step, stream_id, sink, cancel)
    }

    fn open_shell(
        &self,
        container_id: &str,
        shell: &str,
        size: (u16, u16),
        stream_id: StreamId,
        sink: std::sync::Arc<dyn StreamSink>,
    ) -> Result<Box<dyn ShellSession>, RuntimeError> {
        self.shell(container_id, shell, size, stream_id, sink)
    }

    fn stats(&self, ids: &[String], at: &str) -> Result<Vec<UsageSample>, RuntimeError> {
        self.sample(ids, at)
    }

    fn inspect_security(&self, ids: &[String]) -> Result<Vec<Inspect>, RuntimeError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(bad) = ids.iter().find(|id| !valid_container_arg(id)) {
            return Err(RuntimeError::Failed(format!(
                "refused to inspect {bad:?}: not a container id"
            )));
        }
        let mut args = vec!["inspect", "--type", "container", "--"];
        args.extend(ids.iter().map(String::as_str));
        let out = Mapped(self)
            .run(DOCKER, &args)
            .map_err(|e| self.unavailable(e))?;
        parse_inspect(&out.stdout, out.success, &out.stderr)
    }

    fn daemon_mode(&self) -> Result<DaemonMode, RuntimeError> {
        let out = Mapped(self)
            .run(DOCKER, &["info", "--format", "{{json .SecurityOptions}}"])
            .map_err(|e| self.unavailable(e))?;
        if !out.success {
            return Err(RuntimeError::Unavailable(out.stderr.trim_end().to_owned()));
        }
        parse_security_options(&out.stdout)
    }

    fn image_inspect(&self, refs: &[String]) -> Result<Vec<ImageInfo>, RuntimeError> {
        if refs.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(bad) = refs.iter().find(|r| !valid_image_arg(r)) {
            return Err(RuntimeError::Failed(format!(
                "refused to inspect {bad:?}: not an image reference"
            )));
        }
        let mut args = vec!["image", "inspect", "--"];
        args.extend(refs.iter().map(String::as_str));
        let out = Mapped(self)
            .run(DOCKER, &args)
            .map_err(|e| self.unavailable(e))?;
        parse_image_inspect(&out.stdout, out.success, &out.stderr)
    }

    fn containers_health(&self, ids: &[String]) -> Result<Vec<ContainerHealth>, RuntimeError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(bad) = ids.iter().find(|id| !valid_container_arg(id)) {
            return Err(RuntimeError::Failed(format!(
                "refused to inspect {bad:?}: not a container id"
            )));
        }
        let mut args = vec!["inspect", "--type", "container", "--"];
        args.extend(ids.iter().map(String::as_str));
        let out = Mapped(self)
            .run(DOCKER, &args)
            .map_err(|e| self.unavailable(e))?;
        parse_containers_health(&out.stdout, out.success, &out.stderr)
    }

    fn host_platform(&self) -> Result<HostPlatform, RuntimeError> {
        let out = Mapped(self)
            .run(DOCKER, &["info", "--format", INFO_FORMAT])
            .map_err(|e| self.unavailable(e))?;
        if !out.success {
            return Err(RuntimeError::Unavailable(out.stderr.trim_end().to_owned()));
        }
        parse_docker_info(&out.stdout)
    }
}

/// `docker info` reduced to the three fields [`parse_docker_info`] reads.
const INFO_FORMAT: &str = r#"{"OSType":{{json .OSType}},"Architecture":{{json .Architecture}},"DriverStatus":{{json .DriverStatus}}}"#;

/// An image reference or id that cannot be read as an option: no leading `-`, no whitespace or
/// control characters, bounded.
fn valid_image_arg(r: &str) -> bool {
    !r.is_empty()
        && r.len() <= 512
        && !r.starts_with('-')
        && r.bytes().all(|b| b.is_ascii_graphic())
}

/// A `docker inspect`-style array, read whenever it parses: a missing object makes docker exit
/// non-zero but still print the others. Empty output is an error only when docker did not say
/// "No such".
fn inspect_array(
    what: &str,
    stdout: &str,
    success: bool,
    stderr: &str,
) -> Result<Vec<Value>, RuntimeError> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return if success || stderr.contains("No such") {
            Ok(Vec::new())
        } else {
            Err(RuntimeError::Unavailable(stderr.trim_end().to_owned()))
        };
    }
    serde_json::from_str::<Vec<Value>>(trimmed)
        .map_err(|e| RuntimeError::Failed(format!("{what}: unreadable output: {e}")))
}

fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn non_empty(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

fn strings(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// `docker image inspect`'s array, reduced to [`ImageInfo`] (spec 062).
pub fn parse_image_inspect(
    stdout: &str,
    success: bool,
    stderr: &str,
) -> Result<Vec<ImageInfo>, RuntimeError> {
    let items = inspect_array("docker image inspect", stdout, success, stderr)?;
    Ok(items
        .iter()
        .map(|i| {
            let config = i.get("Config").unwrap_or(&Value::Null);
            ImageInfo {
                id: text(i, "Id"),
                repo_tags: strings(i, "RepoTags"),
                repo_digests: strings(i, "RepoDigests"),
                os: text(i, "Os"),
                architecture: text(i, "Architecture"),
                variant: non_empty(text(i, "Variant")),
                user: non_empty(text(config, "User")),
                created: non_empty(text(i, "Created")),
                labels: config
                    .get("Labels")
                    .and_then(Value::as_object)
                    .map(|m| {
                        m.iter()
                            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                            .collect()
                    })
                    .unwrap_or_default(),
            }
        })
        .collect())
}

/// `docker inspect --type container`'s array, reduced to [`ContainerHealth`] (spec 062 R6).
pub fn parse_containers_health(
    stdout: &str,
    success: bool,
    stderr: &str,
) -> Result<Vec<ContainerHealth>, RuntimeError> {
    let items = inspect_array("docker inspect", stdout, success, stderr)?;
    Ok(items
        .iter()
        .map(|c| {
            let state = c.get("State").unwrap_or(&Value::Null);
            let health = match state
                .get("Health")
                .and_then(|h| h.get("Status"))
                .and_then(Value::as_str)
            {
                Some("healthy") => HealthStatus::Healthy,
                Some("unhealthy") => HealthStatus::Unhealthy,
                Some("starting") => HealthStatus::Starting,
                _ => HealthStatus::None,
            };
            ContainerHealth {
                id: text(c, "Id"),
                image: text(c, "Image"),
                running: state
                    .get("Running")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                status: text(state, "Status"),
                health,
                restart_count: c
                    .get("RestartCount")
                    .and_then(Value::as_u64)
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or(0),
            }
        })
        .collect())
}

/// The containerd snapshotter's `DriverStatus` entry.
const CONTAINERD_DRIVER: &str = "io.containerd.snapshotter.v1";

/// `docker info` (`OSType`, `Architecture`, `DriverStatus`): the host platform in OCI names and
/// the image store. `Architecture` is the kernel's (`uname -m`), so it is normalised as
/// containerd's `platforms` package does.
pub fn parse_docker_info(stdout: &str) -> Result<HostPlatform, RuntimeError> {
    let v: Value = serde_json::from_str(stdout.trim())
        .map_err(|e| RuntimeError::Failed(format!("docker info: unreadable output: {e}")))?;
    let os = text(&v, "OSType");
    let arch = text(&v, "Architecture");
    if os.is_empty() || arch.is_empty() {
        return Err(RuntimeError::Failed(
            "docker info: no OSType or Architecture".into(),
        ));
    }
    let (architecture, variant) = match arch.as_str() {
        "x86_64" | "x86-64" | "amd64" => ("amd64", None),
        "aarch64" | "arm64" => ("arm64", Some("v8")),
        "armv7l" | "armv7" | "armhf" => ("arm", Some("v7")),
        "armv6l" | "armv6" | "armel" => ("arm", Some("v6")),
        "i386" | "i486" | "i586" | "i686" | "386" => ("386", None),
        other => (other, None),
    };
    let containerd = v
        .get("DriverStatus")
        .and_then(Value::as_array)
        .is_some_and(|rows| {
            rows.iter().any(|row| {
                row.get(0).and_then(Value::as_str) == Some("driver-type")
                    && row.get(1).and_then(Value::as_str) == Some(CONTAINERD_DRIVER)
            })
        });
    Ok(HostPlatform {
        platform: Platform {
            os,
            architecture: architecture.to_owned(),
            variant: variant.map(str::to_owned),
        },
        store: if containerd {
            ImageStore::Containerd
        } else {
            ImageStore::Classic
        },
    })
}

/// `docker inspect`'s array. A missing container makes docker exit non-zero but still print the
/// others, so the output is read whenever it parses; an error only when it does not.
pub fn parse_inspect(
    stdout: &str,
    success: bool,
    stderr: &str,
) -> Result<Vec<Inspect>, RuntimeError> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return if success {
            Ok(Vec::new())
        } else if stderr.contains("No such") {
            // Every container named is gone.
            Ok(Vec::new())
        } else {
            Err(RuntimeError::Unavailable(stderr.trim_end().to_owned()))
        };
    }
    serde_json::from_str::<Vec<Inspect>>(trimmed)
        .map_err(|e| RuntimeError::Failed(format!("docker inspect: unreadable output: {e}")))
}

/// `docker info --format '{{json .SecurityOptions}}'`: a JSON array of strings, or `null`.
pub fn parse_security_options(stdout: &str) -> Result<DaemonMode, RuntimeError> {
    let opts: Option<Vec<String>> = serde_json::from_str(stdout.trim())
        .map_err(|e| RuntimeError::Failed(format!("docker info: unreadable output: {e}")))?;
    Ok(DaemonMode::from_security_options(&opts.unwrap_or_default()))
}

/// Groups `docker ps` rows into compose projects (with the directory each was started from)
/// and standalone containers.
pub fn group(rows: Vec<ContainerRow>) -> RuntimeSnapshot {
    let mut snap = RuntimeSnapshot::default();
    for row in rows {
        match row.container.project.clone() {
            Some(project) => {
                let entry: &mut ProjectContainers = snap.projects.entry(project).or_default();
                if entry.working_dir.is_none() {
                    entry.working_dir = row.labels.get(WORKING_DIR_LABEL).cloned();
                }
                entry.containers.push(row.container);
            }
            None => snap.standalone.push(row.container),
        }
    }
    snap
}

/// Compose's error lines as issues, with the line number where it gave one
/// (`yaml: line 3: mapping values are not allowed in this context`).
pub fn issues(stderr: &str) -> Vec<ValidationIssue> {
    let found: Vec<ValidationIssue> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| ValidationIssue {
            line: line_number(l),
            message: l.to_owned(),
        })
        .collect();
    if found.is_empty() {
        vec![ValidationIssue {
            line: None,
            message: "the compose file did not validate".into(),
        }]
    } else {
        found
    }
}

fn line_number(text: &str) -> Option<u32> {
    let at = text.find("line ")? + "line ".len();
    let digits: String = text[at..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use container_runtime::inventory::parse_containers_labelled;

    const SEP: char = '\u{1f}';

    #[test]
    fn inspect_output_is_read_even_when_one_container_is_gone() {
        let out = r#"[{"Id":"a","Config":{"User":"0"},"HostConfig":{"Privileged":true}}]"#;
        let got = parse_inspect(out, false, "Error: No such container: b").unwrap();
        assert_eq!(got.len(), 1);
        assert!(got[0].host_config.privileged);
        assert!(
            parse_inspect("", false, "Error: No such container: b")
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            parse_inspect("", false, "Cannot connect to the Docker daemon"),
            Err(RuntimeError::Unavailable(_))
        ));
        assert!(matches!(
            parse_inspect("not json", true, ""),
            Err(RuntimeError::Failed(_))
        ));
    }

    #[test]
    fn security_options_give_the_daemon_mode() {
        assert_eq!(
            parse_security_options("[\"name=seccomp,profile=builtin\",\"name=rootless\"]\n")
                .unwrap(),
            DaemonMode::Rootless
        );
        assert_eq!(parse_security_options("null").unwrap(), DaemonMode::Rootful);
        assert!(parse_security_options("<no value>").is_err());
    }

    #[test]
    fn inspect_refuses_an_id_that_could_be_an_option() {
        let rt = CliRuntime::new("/nonexistent/docker");
        assert!(matches!(
            rt.inspect_security(&["--format=x".into()]),
            Err(RuntimeError::Failed(_))
        ));
        assert!(rt.inspect_security(&[]).unwrap().is_empty());
    }

    #[test]
    fn a_shell_refuses_hostile_ids_and_paths_before_running_anything() {
        struct Nobody;
        impl StreamSink for Nobody {
            fn emit(&self, _: depths_core::ports::StreamEvent) -> bool {
                false
            }
        }
        let rt = CliRuntime::new("/nonexistent/docker");
        let sink: std::sync::Arc<dyn StreamSink> = std::sync::Arc::new(Nobody);
        for (id, shell) in [
            ("--privileged", "/bin/sh"),
            ("abc", "/bin/sh -c id"),
            ("abc", "sh"),
            ("abc", "/bin/sh;id"),
        ] {
            let r = rt.open_shell(id, shell, (80, 24), StreamId(1), sink.clone());
            assert!(matches!(r, Err(RuntimeError::Failed(_))), "{id} {shell}");
        }
    }

    fn row(fields: &[&str]) -> String {
        fields.join(&SEP.to_string())
    }

    #[test]
    fn rows_group_into_projects_with_their_working_dir_and_standalone() {
        let up = "Up 2 hours";
        let out = [
            row(&[
                "a1",
                "web-app-1",
                "nginx",
                "running",
                up,
                "",
                "",
                "",
                "web",
                "app",
                "",
                "/opt/stacks/web",
            ]),
            row(&[
                "a2",
                "web-db-1",
                "postgres",
                "running",
                up,
                "",
                "",
                "",
                "web",
                "db",
                "",
                "/opt/stacks/web",
            ]),
            row(&[
                "b1",
                "loner",
                "busybox",
                "exited",
                "Exited (0)",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
            ]),
        ]
        .join("\n");
        let snap = group(parse_containers_labelled(&out, &[WORKING_DIR_LABEL]));
        let web = &snap.projects["web"];
        assert_eq!(web.working_dir.as_deref(), Some("/opt/stacks/web"));
        assert_eq!(web.containers.len(), 2);
        assert_eq!(snap.standalone.len(), 1);
        assert_eq!(snap.standalone[0].name, "loner");
    }

    #[test]
    fn compose_line_numbers_are_read_from_its_errors() {
        let got = issues("yaml: line 3: mapping values are not allowed in this context\n");
        assert_eq!(got[0].line, Some(3));
        assert!(got[0].message.contains("mapping values"));
        assert_eq!(issues("services.web Additional property foo")[0].line, None);
        assert_eq!(issues("").len(), 1, "an empty stderr still says why");
    }

    #[test]
    fn the_tail_keeps_the_newest_lines() {
        let mut t = Tail::default();
        let line = format!("{}\n", "x".repeat(1023));
        for _ in 0..100 {
            t.push(&line);
        }
        t.push("last\n");
        let s = t.into_string();
        assert!(s.len() <= STDERR_TAIL_BYTES);
        assert!(s.ends_with("last\n"));
    }

    // --- spec 062: image updates --------------------------------------------------------

    /// Captured from `docker image inspect nginx:1.27` (fields trimmed to those read).
    const IMAGE_INSPECT: &str = r#"[{
        "Id": "sha256:aaaa",
        "RepoTags": ["nginx:1.27"],
        "RepoDigests": ["nginx@sha256:bbbb"],
        "Created": "2026-10-01T12:00:00.123Z",
        "Config": {"User": "", "Labels": {"org.opencontainers.image.version": "1.27.2"}},
        "Architecture": "arm64",
        "Variant": "v8",
        "Os": "linux"
    }, {
        "Id": "sha256:cccc",
        "RepoTags": null,
        "RepoDigests": null,
        "Config": {"User": "101:101", "Labels": null},
        "Architecture": "amd64",
        "Os": "linux"
    }]"#;

    #[test]
    fn image_inspect_reads_ids_digests_platform_and_config() {
        let got = parse_image_inspect(IMAGE_INSPECT, true, "").unwrap();
        assert_eq!(got.len(), 2);
        let a = &got[0];
        assert_eq!(a.id, "sha256:aaaa");
        assert_eq!(a.repo_tags, vec!["nginx:1.27".to_owned()]);
        assert_eq!(a.repo_digests, vec!["nginx@sha256:bbbb".to_owned()]);
        assert_eq!(
            (a.os.as_str(), a.architecture.as_str(), a.variant.as_deref()),
            ("linux", "arm64", Some("v8"))
        );
        assert_eq!(a.user, None, "an empty user is none");
        assert_eq!(a.created.as_deref(), Some("2026-10-01T12:00:00.123Z"));
        assert_eq!(
            a.labels
                .get("org.opencontainers.image.version")
                .map(String::as_str),
            Some("1.27.2")
        );
        let b = &got[1];
        assert!(b.repo_tags.is_empty() && b.repo_digests.is_empty() && b.labels.is_empty());
        assert_eq!(
            (b.user.as_deref(), b.variant.as_deref()),
            (Some("101:101"), None)
        );
    }

    #[test]
    fn image_inspect_leaves_out_a_missing_image() {
        // docker prints the images it found, then fails for the rest.
        let one =
            parse_image_inspect(IMAGE_INSPECT, false, "Error: No such image: gone:1").unwrap();
        assert_eq!(one.len(), 2);
        assert!(
            parse_image_inspect("[]", false, "Error: No such image: gone:1")
                .unwrap()
                .is_empty()
        );
        assert!(
            parse_image_inspect(
                "",
                false,
                "Error response from daemon: No such image: gone:1"
            )
            .unwrap()
            .is_empty()
        );
        assert!(matches!(
            parse_image_inspect("", false, "Cannot connect to the Docker daemon"),
            Err(RuntimeError::Unavailable(_))
        ));
        assert!(matches!(
            parse_image_inspect("nope", true, ""),
            Err(RuntimeError::Failed(_))
        ));
    }

    #[test]
    fn image_inspect_refuses_a_reference_that_could_be_an_option() {
        let rt = CliRuntime::new("/nonexistent/docker");
        for bad in ["--format=x", "-q", "", "a b", "a\nb"] {
            assert!(
                matches!(
                    rt.image_inspect(&[bad.into()]),
                    Err(RuntimeError::Failed(_))
                ),
                "{bad:?}"
            );
        }
        assert!(rt.image_inspect(&[]).unwrap().is_empty());
    }

    /// Captured from `docker inspect --type container`: one with a healthcheck, one without,
    /// one restarting.
    const CONTAINER_INSPECT: &str = r#"[{
        "Id": "c1", "Image": "sha256:aaaa", "RestartCount": 0,
        "State": {"Status": "running", "Running": true, "Health": {"Status": "healthy", "FailingStreak": 0}}
    }, {
        "Id": "c2", "Image": "sha256:cccc", "RestartCount": 3,
        "State": {"Status": "restarting", "Running": true}
    }, {
        "Id": "c3", "Image": "sha256:dddd", "RestartCount": 0,
        "State": {"Status": "exited", "Running": false, "Health": {"Status": "unhealthy"}}
    }, {
        "Id": "c4", "Image": "sha256:eeee",
        "State": {"Status": "running", "Running": true, "Health": {"Status": "starting"}}
    }]"#;

    #[test]
    fn container_health_reads_state_health_and_restarts() {
        let got = parse_containers_health(CONTAINER_INSPECT, true, "").unwrap();
        let want = [
            (
                "c1",
                "sha256:aaaa",
                true,
                "running",
                HealthStatus::Healthy,
                0,
            ),
            (
                "c2",
                "sha256:cccc",
                true,
                "restarting",
                HealthStatus::None,
                3,
            ),
            (
                "c3",
                "sha256:dddd",
                false,
                "exited",
                HealthStatus::Unhealthy,
                0,
            ),
            (
                "c4",
                "sha256:eeee",
                true,
                "running",
                HealthStatus::Starting,
                0,
            ),
        ];
        assert_eq!(got.len(), want.len());
        for (g, (id, image, running, status, health, restarts)) in got.iter().zip(want) {
            assert_eq!(
                (
                    g.id.as_str(),
                    g.image.as_str(),
                    g.running,
                    g.status.as_str(),
                    g.health,
                    g.restart_count
                ),
                (id, image, running, status, health, restarts)
            );
        }
        assert!(
            parse_containers_health("", false, "Error: No such container: c9")
                .unwrap()
                .is_empty()
        );
        let rt = CliRuntime::new("/nonexistent/docker");
        assert!(matches!(
            rt.containers_health(&["-x".into()]),
            Err(RuntimeError::Failed(_))
        ));
        assert!(rt.containers_health(&[]).unwrap().is_empty());
    }

    fn info(os: &str, arch: &str, driver_status: &str) -> String {
        format!(
            r#"{{"OSType":"{os}","Architecture":"{arch}","Driver":"overlay2","DriverStatus":{driver_status}}}"#
        )
    }

    #[test]
    fn docker_info_gives_the_oci_platform() {
        let classic = r#"[["Backing Filesystem","extfs"],["Supports d_type","true"]]"#;
        let cases = [
            ("x86_64", "amd64", None),
            ("amd64", "amd64", None),
            ("aarch64", "arm64", Some("v8")),
            ("arm64", "arm64", Some("v8")),
            ("armv7l", "arm", Some("v7")),
            ("armv6l", "arm", Some("v6")),
            ("i686", "386", None),
            ("s390x", "s390x", None),
            ("ppc64le", "ppc64le", None),
            ("riscv64", "riscv64", None),
        ];
        for (uname, arch, variant) in cases {
            let p = parse_docker_info(&info("linux", uname, classic)).unwrap();
            assert_eq!(
                (
                    p.platform.os.as_str(),
                    p.platform.architecture.as_str(),
                    p.platform.variant.as_deref()
                ),
                ("linux", arch, variant),
                "{uname}"
            );
            assert_eq!(p.store, ImageStore::Classic);
        }
    }

    #[test]
    fn docker_info_detects_the_containerd_store() {
        let containerd = r#"[["driver-type","io.containerd.snapshotter.v1"]]"#;
        let p = parse_docker_info(&info("linux", "x86_64", containerd)).unwrap();
        assert_eq!(p.store, ImageStore::Containerd);
        let p = parse_docker_info(&info("linux", "x86_64", "null")).unwrap();
        assert_eq!(p.store, ImageStore::Classic);
        assert!(parse_docker_info("<no value>").is_err());
        assert!(parse_docker_info(r#"{"OSType":"","Architecture":"x86_64"}"#).is_err());
    }
}
