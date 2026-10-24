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
    CancelToken, ContainerRuntime, OutputChunk, OutputSink, OutputStream, ProjectContainers,
    RuntimeError, RuntimeSnapshot, ShellSession, StepOutcome, StreamId, StreamSink, UsageSample,
    Validation, ValidationIssue,
};
use depths_core::security::{DaemonMode, Inspect, valid_container_arg};

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
}
