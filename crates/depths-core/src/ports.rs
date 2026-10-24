//! The two ports Depths reaches the world through, and the domain types they carry.
//!
//! - [`StackStore`]: the stacks root on disk. `depths-stacks-fs` implements it, confining every
//!   path to `root/<name>` (FR-005).
//! - [`ContainerRuntime`]: the container runtime. The server implements it over
//!   `container-runtime`'s `Runner` (argv only, never a shell).
//! - [`VolumeHelper`]: starts a volume helper container on a mount and hands back its frame
//!   [`HelperChannel`] (spec 053). `depths-runtime-cli` implements it with `docker run -i`.
//!
//! All are synchronous, like `container_runtime::runner::Runner`: the server calls them from a
//! blocking task. Names reach them only as [`StackName`]s, and commands only as the plan's
//! [`RunStep`]s, so neither port ever sees a path or a command line a client wrote.
//!
//! [`execute`] runs a confirmed plan through both, in order.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

pub use container_runtime::sink::{StreamEvent, StreamSink};
use container_runtime::wire::Container;
pub use container_runtime::wire::{StreamId, UsageSample};
use depths_volumes::frame::{Frame, FrameError, FrameKind, read_frame};
pub use depths_volumes::mount::{HelperSource, OpMode};
use depths_volumes::op::{Hello, PROTO};
use serde::{Deserialize, Serialize};

use crate::discovery::ComposeFile;
use crate::hash::ContentHash;
use crate::name::StackName;
use crate::plan::{Plan, RunStep, Step, StepStdin};
use crate::status::{Definition, ServiceSpec, StackStatus, derive_status};

// ---------------------------------------------------------------------------
// What the store knows
// ---------------------------------------------------------------------------

/// What reading a stack's compose file produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "detail", rename_all = "snake_case")]
pub enum DefinitionState {
    Parsed(Vec<ServiceSpec>),
    /// Not YAML, or not a compose mapping. The stack is `invalid`.
    Unparseable(String),
    /// Not read: too large, unreadable, or not followed.
    NotLoaded,
}

/// One immediate sub-directory of the stacks root that holds a compose file, as the store sees
/// it: files and metadata, no containers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackEntry {
    /// The directory name, as it is on disk, valid or not.
    pub name: String,
    /// `valid_name(name)`. An invalid stack is listed and never acted on.
    pub valid_name: bool,
    /// `None` only when the directory could not be read (then `read_error` says why).
    pub compose_file: Option<ComposeFile>,
    pub shadowed_files: Vec<ComposeFile>,
    pub has_env: bool,
    /// `None` when the files could not be read.
    pub content_hash: Option<ContentHash>,
    /// The latest mtime of the compose file and `.env`.
    pub modified_at: Option<SystemTime>,
    pub definition: DefinitionState,
    /// Why the directory or its files cannot be read. Listed, never edited.
    pub read_error: Option<String>,
    /// Why Depths cannot write the files (permissions, or a symlinked stack directory).
    pub read_only_reason: Option<String>,
    /// A compose or `.env` file is over the size limit: shown, not loaded into the editor.
    pub too_large: bool,
}

impl StackEntry {
    /// The validated name, when there is one.
    pub fn stack_name(&self) -> Option<StackName> {
        StackName::parse(&self.name).ok()
    }

    /// Whether lifecycle operations may run on this stack.
    pub fn actionable(&self) -> bool {
        self.valid_name
            && self.read_error.is_none()
            && !matches!(self.definition, DefinitionState::Unparseable(_))
    }

    /// Whether the editor may load and save this stack.
    pub fn editable(&self) -> bool {
        self.actionable()
            && self.read_only_reason.is_none()
            && !self.too_large
            && self.compose_file.is_some()
    }
}

/// Everything under the stacks root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackListing {
    pub root: PathBuf,
    /// `<root>/global.env` exists.
    pub global_env: bool,
    pub stacks: Vec<StackEntry>,
}

/// A stack's files as the editor loads them: exact text, byte for byte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackFiles {
    pub compose_file: ComposeFile,
    pub compose: String,
    /// `None` when there is no `.env`.
    pub env: Option<String>,
    pub content_hash: ContentHash,
}

/// An edit, for a save or a create (data-model.md "StackDraft").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackDraft {
    /// An existing stack for a save; a new name for a create.
    pub stack: StackName,
    /// The hash the editor loaded; `None` for a create.
    pub base_hash: Option<ContentHash>,
    /// Written byte for byte.
    pub compose: String,
    /// `None` leaves `.env` as it is. An empty string where there is no `.env` creates nothing.
    pub env: Option<String>,
}

impl StackDraft {
    /// Whether `.env` exists once this draft is written, given whether it exists now.
    pub fn env_exists_after(&self, has_env: bool) -> bool {
        has_env || self.env.as_deref().is_some_and(|e| !e.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum StoreError {
    /// No such stack directory.
    NotFound,
    /// The directory holds no compose file.
    NotAStack,
    /// Create found the directory already there.
    AlreadyExists,
    /// The path would leave the stack directory (`..`, an absolute name, a symlink; FR-005).
    Escapes {
        reason: String,
    },
    ReadOnly {
        reason: String,
    },
    TooLarge {
        file: String,
        bytes: u64,
        limit: u64,
    },
    /// The file is not UTF-8, so the editor cannot show it exactly.
    NotText {
        file: String,
    },
    /// `base_hash` is not the current hash: someone else changed the files (FR-019).
    Conflict {
        current: Box<StackFiles>,
    },
    Io {
        message: String,
    },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("no such stack"),
            Self::NotAStack => f.write_str("the directory holds no compose file"),
            Self::AlreadyExists => f.write_str("a stack with that name already exists"),
            Self::Escapes { reason } => write!(f, "refused: {reason}"),
            Self::ReadOnly { reason } => write!(f, "read-only: {reason}"),
            Self::TooLarge { file, bytes, limit } => {
                write!(f, "{file} is {bytes} bytes, over the {limit}-byte limit")
            }
            Self::NotText { file } => write!(f, "{file} is not UTF-8 text"),
            Self::Conflict { .. } => {
                f.write_str("the files changed on disk since they were loaded")
            }
            Self::Io { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for StoreError {}

/// The stacks root.
pub trait StackStore: Send + Sync {
    /// The canonical stacks root.
    fn root(&self) -> &Path;

    /// Whether `<root>/global.env` exists now.
    fn global_env(&self) -> bool;

    /// Every stack directory. Reads only; writes nothing (FR-003).
    fn list(&self) -> Result<StackListing, StoreError>;

    /// One stack's entry, as `list` would give it.
    fn entry(&self, name: &StackName) -> Result<StackEntry, StoreError>;

    /// One stack's files for the editor. Refuses a file over the size limit (`TooLarge`).
    fn read(&self, name: &StackName) -> Result<StackFiles, StoreError>;

    /// Writes the compose file (and `.env` when `env` is `Some`) exactly as given, each by temp
    /// file and rename in the same directory, keeping mode and owner. Refuses with `Conflict`
    /// when the current hash is not `base_hash`. Returns the new hash.
    fn write_atomic(
        &self,
        name: &StackName,
        compose: &str,
        env: Option<&str>,
        base_hash: &ContentHash,
    ) -> Result<ContentHash, StoreError>;

    /// Creates `root/<name>` with `compose.yaml` (and `.env` when `env` is non-empty). Refuses
    /// with `AlreadyExists` when the directory is there.
    fn create(
        &self,
        name: &StackName,
        compose: &str,
        env: Option<&str>,
    ) -> Result<ContentHash, StoreError>;

    /// Removes `root/<name>` and everything in it, never following a symlink.
    fn delete_dir(&self, name: &StackName) -> Result<(), StoreError>;
}

// ---------------------------------------------------------------------------
// What the runtime knows
// ---------------------------------------------------------------------------

/// One compose project's containers.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProjectContainers {
    /// The `com.docker.compose.project.working_dir` label, when the containers carry one.
    pub working_dir: Option<String>,
    pub containers: Vec<Container>,
}

/// Every container on the daemon, grouped by compose project.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RuntimeSnapshot {
    pub projects: BTreeMap<String, ProjectContainers>,
    /// Containers with no project label.
    pub standalone: Vec<Container>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", content = "detail", rename_all = "snake_case")]
pub enum RuntimeError {
    /// The daemon or the CLI cannot be reached (FR-010).
    Unavailable(String),
    /// This adapter cannot do what was asked.
    Unsupported,
    Failed(String),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(r) => write!(f, "container runtime unavailable: {r}"),
            Self::Unsupported => f.write_str("not supported by this runtime"),
            Self::Failed(r) => f.write_str(r),
        }
    }
}

impl std::error::Error for RuntimeError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputStream {
    Stdout,
    Stderr,
}

/// A piece of a running command's output, verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputChunk {
    pub stream: OutputStream,
    pub text: String,
}

/// Where a running operation's output goes (the server forwards it as `OpOutput`).
pub trait OutputSink: Send + Sync {
    /// `false` means nobody is listening; the command keeps running regardless.
    fn emit(&self, chunk: OutputChunk) -> bool;
}

/// Asks a running operation to stop (`Cancel{operation_id}`): the adapter sends SIGINT to the
/// compose process, and [`execute`] runs no further step.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum StepOutcome {
    Succeeded,
    /// Compose's stderr, verbatim, for the operator (FR-013).
    Failed {
        exit_code: Option<i32>,
        stderr_tail: String,
    },
    Cancelled,
}

/// One problem `docker compose config` found in a draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    /// When Compose names one.
    pub line: Option<u32>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", content = "issues", rename_all = "snake_case")]
pub enum Validation {
    Valid,
    Invalid(Vec<ValidationIssue>),
}

/// A live terminal opened by [`ContainerRuntime::open_shell`]. Dropping it without
/// [`ShellSession::close`] is a bug in the caller; adapters still try to end the process.
pub trait ShellSession: Send {
    /// Keystrokes or a paste, verbatim. An error means the shell is gone.
    fn write(&mut self, data: &[u8]) -> Result<(), RuntimeError>;
    /// The terminal's new size, columns then rows. Best effort.
    fn resize(&self, cols: u16, rows: u16);
    /// Ends the session, leaving no process behind.
    fn close(self: Box<Self>);
    /// Kills the session's process from another thread, for when the owner is stuck writing to a
    /// terminal nobody reads; the owner's write then fails and it calls [`ShellSession::close`].
    /// Harmless after the close.
    fn stopper(&self) -> Arc<dyn Fn() + Send + Sync> {
        Arc::new(|| {})
    }
}

/// The container runtime.
pub trait ContainerRuntime: Send + Sync {
    /// One reading of every container (`docker ps -a`), grouped by compose project.
    fn containers_by_project(&self) -> Result<RuntimeSnapshot, RuntimeError>;

    /// Runs one command, streaming its output to `sink`, until it exits or `cancel` fires.
    /// `stdin` is given exactly when the step's `stdin` is `Draft`.
    fn run_step(
        &self,
        step: &RunStep,
        stdin: Option<&[u8]>,
        sink: &dyn OutputSink,
        cancel: &CancelToken,
    ) -> Result<StepOutcome, RuntimeError>;

    /// Runs a validate step (`docker compose ... -f - config --quiet`) on `draft`, with the
    /// errors Compose gave, by line where it named one.
    fn validate(&self, step: &RunStep, draft: &[u8]) -> Result<Validation, RuntimeError>;

    /// Whether the runtime answers at all (`/readyz`). The default asks for the inventory;
    /// adapters with a cheaper probe override it.
    fn ping(&self) -> Result<(), RuntimeError> {
        self.containers_by_project().map(|_| ())
    }

    /// Follows the runtime's event stream, calling `on_event` once per event, until `cancel` is
    /// set (`Ok`) or the stream fails (`Err`). Blocking: callers run it on a blocking thread.
    /// The default has no stream, so watchers fall back to the filesystem alone.
    fn follow_events(
        &self,
        on_event: &(dyn Fn() + Sync),
        cancel: &CancelToken,
    ) -> Result<(), RuntimeError> {
        let _ = (on_event, cancel);
        Err(RuntimeError::Unsupported)
    }

    /// Follows a log step ([`crate::plan::stack_logs`], [`crate::plan::container_logs`]),
    /// sending `ContainerLogBatch`es to `sink`, until the command ends or `cancel` is set.
    /// Blocking. `Ok` is a clean end or a cancel; `Err(Failed)` carries the runtime's own words.
    /// Adapters bound what they buffer: a reader that falls behind loses lines (`truncated`).
    fn follow_logs(
        &self,
        step: &RunStep,
        stream_id: StreamId,
        sink: &dyn StreamSink,
        cancel: &CancelToken,
    ) -> Result<(), RuntimeError> {
        let _ = (step, stream_id, sink, cancel);
        Err(RuntimeError::Unsupported)
    }

    /// Opens an interactive terminal running `shell` in the container `container_id` (a full
    /// id). Output goes to `sink` as `ContainerSessionOutput`, then one `ContainerSessionClosed`.
    /// Whether a shell may be opened at all is the caller's decision.
    fn open_shell(
        &self,
        container_id: &str,
        shell: &str,
        size: (u16, u16),
        stream_id: StreamId,
        sink: Arc<dyn StreamSink>,
    ) -> Result<Box<dyn ShellSession>, RuntimeError> {
        let _ = (container_id, shell, size, stream_id, sink);
        Err(RuntimeError::Unsupported)
    }

    /// One `docker inspect` of these containers (full ids), in one call, reduced to the fields
    /// that decide containment. A container that is gone is left out, not an error. Empty `ids`
    /// runs nothing.
    fn inspect_security(
        &self,
        ids: &[String],
    ) -> Result<Vec<crate::security::Inspect>, RuntimeError> {
        let _ = ids;
        Err(RuntimeError::Unsupported)
    }

    /// The daemon's mode, from `docker info`'s `SecurityOptions`.
    fn daemon_mode(&self) -> Result<crate::security::DaemonMode, RuntimeError> {
        Err(RuntimeError::Unsupported)
    }

    /// One `docker stats --no-stream` reading of exactly these containers (full ids), stamped
    /// `at`. Empty `ids` runs nothing and returns nothing.
    fn stats(&self, ids: &[String], at: &str) -> Result<Vec<UsageSample>, RuntimeError> {
        let _ = (ids, at);
        Err(RuntimeError::Unsupported)
    }

    /// Runs commands in order, stopping at the first that does not succeed.
    fn run_steps(
        &self,
        steps: &[RunStep],
        draft: Option<&[u8]>,
        sink: &dyn OutputSink,
        cancel: &CancelToken,
    ) -> Result<StepOutcome, RuntimeError> {
        for step in steps {
            if cancel.is_cancelled() {
                return Ok(StepOutcome::Cancelled);
            }
            let stdin = stdin_for(step, draft)?;
            let outcome = self.run_step(step, stdin, sink, cancel)?;
            if outcome != StepOutcome::Succeeded {
                return Ok(outcome);
            }
        }
        Ok(StepOutcome::Succeeded)
    }
}

fn stdin_for<'a>(
    step: &RunStep,
    draft: Option<&'a [u8]>,
) -> Result<Option<&'a [u8]>, RuntimeError> {
    match step.stdin {
        StepStdin::None => Ok(None),
        StepStdin::Draft => draft.map(Some).ok_or_else(|| {
            RuntimeError::Failed("the step reads a draft and none was given".into())
        }),
    }
}

// ---------------------------------------------------------------------------
// The volume helper (spec 053)
// ---------------------------------------------------------------------------

/// Which helper to start: the stack it works for (its `depths.helper.stack` label), what it
/// mounts at `/volume`, the numeric uid:gid it runs as (R5), and whether the mount is writable.
/// The server derives `source` itself; a client never supplies one (R10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperSpec {
    pub stack: StackName,
    pub source: HelperSource,
    pub uid: u32,
    pub gid: u32,
    pub mode: OpMode,
}

/// Why a helper's channel failed. Every one ends that helper: the caller kills it with
/// [`HelperChannel::kill`] and never sends it another op.
#[derive(Debug)]
pub enum ChannelError {
    /// The helper's stdout ended: it exited, or its container is gone.
    Closed,
    /// A frame broke the protocol, or a pipe failed.
    Protocol(FrameError),
    /// The first frame was not a `Hello` this server speaks.
    Handshake(String),
}

impl fmt::Display for ChannelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => f.write_str("the volume helper exited"),
            Self::Protocol(e) => write!(f, "volume helper protocol error: {e}"),
            Self::Handshake(why) => write!(f, "volume helper handshake failed: {why}"),
        }
    }
}

impl std::error::Error for ChannelError {}

impl From<FrameError> for ChannelError {
    fn from(e: FrameError) -> Self {
        Self::Protocol(e)
    }
}

/// The process behind a [`HelperChannel`], ended the way its adapter knows how.
pub trait HelperProcess: Send {
    /// A clean end. The channel has already closed the helper's stdin, so the helper is exiting
    /// 0; wait for it.
    fn close(self: Box<Self>);
    /// A forced end, after a protocol error: kill the helper and remove its container.
    fn kill(self: Box<Self>);
}

/// A started helper's frame channel: its stdout to read, its stdin to write, and its process.
///
/// Blocking, like [`ShellSession`]: the server drives it from a blocking task. One op at a time;
/// the pool serialises. Generic over any byte pipes, so an in-process helper on a thread serves
/// the same channel a container does. Dropping it without [`HelperChannel::close`] kills the
/// helper.
pub struct HelperChannel {
    hello: Hello,
    reader: Box<dyn Read + Send>,
    writer: Option<Box<dyn Write + Send>>,
    process: Option<Box<dyn HelperProcess>>,
}

impl fmt::Debug for HelperChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HelperChannel")
            .field("hello", &self.hello)
            .finish_non_exhaustive()
    }
}

impl HelperChannel {
    /// Reads the helper's `Hello` from `reader`. A first frame that is not a `Hello` of
    /// protocol [`PROTO`] kills `process` and fails. `Hello.openat2 == false` is not an error
    /// here: the caller reports `helper_unavailable` from [`HelperChannel::hello`].
    pub fn open(
        mut reader: Box<dyn Read + Send>,
        writer: Box<dyn Write + Send>,
        process: Box<dyn HelperProcess>,
    ) -> Result<Self, ChannelError> {
        let hello = match handshake(&mut reader) {
            Ok(hello) => hello,
            Err(e) => {
                drop(writer);
                process.kill();
                return Err(e);
            }
        };
        Ok(Self {
            hello,
            reader,
            writer: Some(writer),
            process: Some(process),
        })
    }

    pub fn hello(&self) -> &Hello {
        &self.hello
    }

    /// Writes one frame and flushes it.
    pub fn send(&mut self, frame: &Frame) -> Result<(), ChannelError> {
        let writer = self.writer.as_mut().ok_or(ChannelError::Closed)?;
        frame.write_to(writer)?;
        writer
            .flush()
            .map_err(|e| ChannelError::Protocol(FrameError::Io(e)))
    }

    /// Reads the next frame. A clean end of the helper's stdout is [`ChannelError::Closed`].
    pub fn recv(&mut self) -> Result<Frame, ChannelError> {
        read_frame(&mut self.reader)?.ok_or(ChannelError::Closed)
    }

    /// Closes the helper's stdin, so it exits 0, and waits for it.
    pub fn close(mut self) {
        drop(self.writer.take());
        if let Some(process) = self.process.take() {
            process.close();
        }
    }

    /// Kills the helper and removes its container, after a protocol error.
    pub fn kill(mut self) {
        self.end_by_force();
    }

    fn end_by_force(&mut self) {
        drop(self.writer.take());
        if let Some(process) = self.process.take() {
            process.kill();
        }
    }
}

impl Drop for HelperChannel {
    fn drop(&mut self) {
        self.end_by_force();
    }
}

fn handshake(reader: &mut impl Read) -> Result<Hello, ChannelError> {
    let frame = read_frame(reader)?.ok_or(ChannelError::Closed)?;
    let hello: Hello = frame
        .parse(FrameKind::Hello)
        .map_err(|e| ChannelError::Handshake(e.to_string()))?;
    if hello.proto != PROTO {
        return Err(ChannelError::Handshake(format!(
            "the helper speaks protocol {}, this server {PROTO}",
            hello.proto
        )));
    }
    Ok(hello)
}

/// Starts volume helpers (contracts/helper.md).
pub trait VolumeHelper: Send + Sync {
    /// Starts a helper for `spec` and completes its handshake. Blocking: a cold start is a
    /// `docker run` (0.3–0.8 s). A named volume that does not exist is refused, never created,
    /// and a `Hello` for another uid:gid than `spec`'s is refused.
    fn start(&self, spec: &HelperSpec) -> Result<HelperChannel, RuntimeError>;

    /// Removes helpers an earlier run left behind (labelled `depths.helper=1`), at server start.
    /// How many it removed. The default has none to remove.
    fn sweep(&self) -> Result<usize, RuntimeError> {
        Ok(0)
    }
}

// ---------------------------------------------------------------------------
// Running a plan
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecError {
    Store(StoreError),
    Runtime(RuntimeError),
    /// The plan writes a draft and none was given.
    MissingDraft,
}

impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(e) => e.fmt(f),
            Self::Runtime(e) => e.fmt(f),
            Self::MissingDraft => f.write_str("the plan writes a draft and none was given"),
        }
    }
}

impl std::error::Error for ExecError {}

/// Runs a confirmed plan's steps in order: commands through `runtime`, file steps through
/// `store`. Stops at the first command that does not succeed, at the first error, or when
/// `cancel` fires between steps.
///
/// `WriteDraft` is a save when the draft carries a `base_hash` and a create when it does not.
pub fn execute(
    plan: &Plan,
    draft: Option<&StackDraft>,
    stack: &StackName,
    store: &dyn StackStore,
    runtime: &dyn ContainerRuntime,
    sink: &dyn OutputSink,
    cancel: &CancelToken,
) -> Result<StepOutcome, ExecError> {
    for step in &plan.steps {
        if cancel.is_cancelled() {
            return Ok(StepOutcome::Cancelled);
        }
        match step {
            Step::Run(run) => {
                let stdin = stdin_for(run, draft.map(|d| d.compose.as_bytes()))
                    .map_err(|_| ExecError::MissingDraft)?;
                let outcome = runtime
                    .run_step(run, stdin, sink, cancel)
                    .map_err(ExecError::Runtime)?;
                if outcome != StepOutcome::Succeeded {
                    return Ok(outcome);
                }
            }
            Step::WriteDraft => {
                let d = draft.ok_or(ExecError::MissingDraft)?;
                match &d.base_hash {
                    Some(base) => store.write_atomic(stack, &d.compose, d.env.as_deref(), base),
                    None => store.create(stack, &d.compose, d.env.as_deref()),
                }
                .map_err(ExecError::Store)?;
            }
            Step::RemoveDir => store.delete_dir(stack).map_err(ExecError::Store)?,
        }
    }
    Ok(StepOutcome::Succeeded)
}

// ---------------------------------------------------------------------------
// The joined view
// ---------------------------------------------------------------------------

/// One service of a stack, with its containers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceSummary {
    pub name: String,
    pub image: Option<String>,
    /// `false` for a container whose service the compose file does not name (an orphan).
    pub defined: bool,
    pub containers: Vec<Container>,
}

/// A stack as the list shows it: the store's entry joined with the runtime's containers
/// (data-model.md "Stack").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stack {
    pub name: String,
    pub compose_file: Option<ComposeFile>,
    pub shadowed_files: Vec<ComposeFile>,
    pub has_env: bool,
    pub content_hash: Option<ContentHash>,
    pub status: StackStatus,
    pub unhealthy: bool,
    pub services: Vec<ServiceSummary>,
    pub running_services: u32,
    pub total_services: u32,
    pub modified_at: Option<SystemTime>,
    /// Always true for a stack; foreign projects are [`ForeignProject`]s.
    pub managed: bool,
    pub read_error: Option<String>,
    pub read_only_reason: Option<String>,
    pub too_large: bool,
}

impl Stack {
    /// Joins an entry with its project's containers (those labelled with the stack's name).
    pub fn assemble(entry: StackEntry, containers: &[Container]) -> Self {
        let definition = if !entry.valid_name {
            Definition::Invalid
        } else {
            match &entry.definition {
                DefinitionState::Parsed(s) => Definition::Services(s),
                DefinitionState::Unparseable(_) => Definition::Invalid,
                DefinitionState::NotLoaded => Definition::Unknown,
            }
        };
        let summary = derive_status(definition, containers);

        let defined: &[ServiceSpec] = match &entry.definition {
            DefinitionState::Parsed(s) => s,
            _ => &[],
        };
        let service_of = |c: &Container| c.service.clone().unwrap_or_else(|| c.name.clone());
        let mut services: Vec<ServiceSummary> = defined
            .iter()
            .map(|s| ServiceSummary {
                name: s.name.clone(),
                image: s.image.clone(),
                defined: true,
                containers: containers
                    .iter()
                    .filter(|c| service_of(c) == s.name)
                    .cloned()
                    .collect(),
            })
            .collect();
        for c in containers {
            let name = service_of(c);
            match services.iter_mut().find(|s| s.name == name) {
                Some(s) if !s.defined => s.containers.push(c.clone()),
                Some(_) => {}
                None => services.push(ServiceSummary {
                    name,
                    image: Some(c.image.clone()),
                    defined: false,
                    containers: vec![c.clone()],
                }),
            }
        }

        Self {
            name: entry.name,
            compose_file: entry.compose_file,
            shadowed_files: entry.shadowed_files,
            has_env: entry.has_env,
            content_hash: entry.content_hash,
            status: summary.status,
            unhealthy: summary.unhealthy,
            services,
            running_services: summary.running_services,
            total_services: summary.total_services,
            modified_at: entry.modified_at,
            managed: true,
            read_error: entry.read_error,
            read_only_reason: entry.read_only_reason,
            too_large: entry.too_large,
        }
    }
}

/// A compose project with no stack directory, or whose working directory is outside the root.
/// Read-only for every role, listed to admins only (FR-008).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignProject {
    pub name: String,
    pub working_dir: Option<String>,
    pub containers: Vec<Container>,
    /// Set when `working_dir` is this stack's directory under a different project name
    /// (started by hand with `-p`): the stack shows "running elsewhere".
    pub elsewhere_of: Option<StackName>,
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use container_runtime::wire::{AdoptionState, RuntimeContainerState};

    use super::*;
    use crate::hash::content_hash_of;
    use crate::plan::{Operation, StackContext, plan};
    use crate::status::RestartPolicy;

    fn name(s: &str) -> StackName {
        StackName::parse(s).unwrap()
    }

    fn container(service: &str, state: RuntimeContainerState, status: &str) -> Container {
        Container {
            id: format!("{service}-id"),
            name: format!("web-{service}-1"),
            image: "busybox".into(),
            state,
            health: None,
            status: status.into(),
            created_at: String::new(),
            command: String::new(),
            ports: vec![],
            project: Some("web".into()),
            service: Some(service.into()),
            cluster: None,
            adoption: AdoptionState::NotAdopted,
        }
    }

    fn entry(definition: DefinitionState) -> StackEntry {
        StackEntry {
            name: "web".into(),
            valid_name: true,
            compose_file: Some(ComposeFile::ComposeYaml),
            shadowed_files: vec![],
            has_env: false,
            content_hash: Some(content_hash_of(ComposeFile::ComposeYaml, b"x", None)),
            modified_at: None,
            definition,
            read_error: None,
            read_only_reason: None,
            too_large: false,
        }
    }

    fn spec(n: &str) -> ServiceSpec {
        ServiceSpec {
            name: n.into(),
            image: Some(format!("{n}:1")),
            restart: RestartPolicy::Always,
        }
    }

    #[test]
    fn a_stack_joins_its_services_with_their_containers() {
        let e = entry(DefinitionState::Parsed(vec![spec("app"), spec("db")]));
        let up = container("app", RuntimeContainerState::Running, "Up 1 hour");
        let orphan = container("old", RuntimeContainerState::Running, "Up 1 hour");
        let s = Stack::assemble(e, &[up.clone(), orphan.clone()]);
        assert_eq!(s.status, StackStatus::Partial);
        assert_eq!((s.running_services, s.total_services), (2, 3));
        let names: Vec<(&str, bool, usize)> = s
            .services
            .iter()
            .map(|v| (v.name.as_str(), v.defined, v.containers.len()))
            .collect();
        assert_eq!(
            names,
            vec![("app", true, 1), ("db", true, 0), ("old", false, 1)]
        );
        assert_eq!(s.services[0].image.as_deref(), Some("app:1"));
        assert!(s.managed);
    }

    #[test]
    fn an_invalid_name_or_unparseable_file_is_invalid() {
        let mut e = entry(DefinitionState::Parsed(vec![spec("app")]));
        e.valid_name = false;
        assert_eq!(Stack::assemble(e, &[]).status, StackStatus::Invalid);
        let e = entry(DefinitionState::Unparseable("bad".into()));
        assert!(!e.actionable());
        assert_eq!(Stack::assemble(e, &[]).status, StackStatus::Invalid);
    }

    #[test]
    fn editability_follows_the_entrys_reasons() {
        let ok = entry(DefinitionState::NotLoaded);
        assert!(ok.actionable() && ok.editable());
        let mut big = ok.clone();
        big.too_large = true;
        assert!(big.actionable() && !big.editable());
        let mut ro = ok.clone();
        ro.read_only_reason = Some("symlink".into());
        assert!(ro.actionable() && !ro.editable());
        let mut unread = ok.clone();
        unread.read_error = Some("permission denied".into());
        assert!(!unread.actionable() && !unread.editable());
    }

    #[test]
    fn a_draft_creates_env_only_when_it_has_content() {
        let d = |env: Option<&str>| StackDraft {
            stack: name("web"),
            base_hash: None,
            compose: String::new(),
            env: env.map(str::to_owned),
        };
        assert!(!d(None).env_exists_after(false));
        assert!(d(None).env_exists_after(true));
        assert!(!d(Some("")).env_exists_after(false));
        assert!(d(Some("")).env_exists_after(true));
        assert!(d(Some("A=1")).env_exists_after(false));
    }

    #[test]
    fn the_cancel_token_is_shared_by_its_clones() {
        let t = CancelToken::new();
        let other = t.clone();
        assert!(!other.is_cancelled());
        t.cancel();
        assert!(other.is_cancelled());
    }

    // -- execute, over fakes --------------------------------------------------------------

    #[derive(Default)]
    struct Log(Mutex<Vec<String>>);

    impl Log {
        fn push(&self, s: String) {
            self.0.lock().unwrap().push(s);
        }
        fn take(&self) -> Vec<String> {
            std::mem::take(&mut self.0.lock().unwrap())
        }
    }

    struct FakeStore<'a>(&'a Log);

    impl StackStore for FakeStore<'_> {
        fn root(&self) -> &Path {
            Path::new("/opt/stacks")
        }
        fn global_env(&self) -> bool {
            false
        }
        fn list(&self) -> Result<StackListing, StoreError> {
            unreachable!()
        }
        fn entry(&self, _: &StackName) -> Result<StackEntry, StoreError> {
            unreachable!()
        }
        fn read(&self, _: &StackName) -> Result<StackFiles, StoreError> {
            unreachable!()
        }
        fn write_atomic(
            &self,
            n: &StackName,
            compose: &str,
            env: Option<&str>,
            _: &ContentHash,
        ) -> Result<ContentHash, StoreError> {
            self.0.push(format!("write {n} {compose} {env:?}"));
            Ok(content_hash_of(
                ComposeFile::ComposeYaml,
                compose.as_bytes(),
                None,
            ))
        }
        fn create(
            &self,
            n: &StackName,
            compose: &str,
            env: Option<&str>,
        ) -> Result<ContentHash, StoreError> {
            self.0.push(format!("create {n} {compose} {env:?}"));
            Ok(content_hash_of(
                ComposeFile::ComposeYaml,
                compose.as_bytes(),
                None,
            ))
        }
        fn delete_dir(&self, n: &StackName) -> Result<(), StoreError> {
            self.0.push(format!("rmdir {n}"));
            Ok(())
        }
    }

    /// Fails any command whose argv contains `fail_on`.
    struct FakeRuntime<'a> {
        log: &'a Log,
        fail_on: Option<&'static str>,
        cancel_after_first: Option<CancelToken>,
    }

    impl ContainerRuntime for FakeRuntime<'_> {
        fn containers_by_project(&self) -> Result<RuntimeSnapshot, RuntimeError> {
            Ok(RuntimeSnapshot::default())
        }
        fn run_step(
            &self,
            step: &RunStep,
            stdin: Option<&[u8]>,
            sink: &dyn OutputSink,
            _: &CancelToken,
        ) -> Result<StepOutcome, RuntimeError> {
            let line = step.argv.join(" ");
            let stdin = stdin.map(|b| String::from_utf8_lossy(b).into_owned());
            self.log.push(format!("run {line} stdin={stdin:?}"));
            sink.emit(OutputChunk {
                stream: OutputStream::Stderr,
                text: format!("ran {line}"),
            });
            if let Some(t) = &self.cancel_after_first {
                t.cancel();
            }
            if self.fail_on.is_some_and(|f| line.contains(f)) {
                return Ok(StepOutcome::Failed {
                    exit_code: Some(1),
                    stderr_tail: "boom".into(),
                });
            }
            Ok(StepOutcome::Succeeded)
        }
        fn validate(&self, _: &RunStep, _: &[u8]) -> Result<Validation, RuntimeError> {
            Ok(Validation::Valid)
        }
    }

    struct Collect(Mutex<Vec<OutputChunk>>);

    impl OutputSink for Collect {
        fn emit(&self, chunk: OutputChunk) -> bool {
            self.0.lock().unwrap().push(chunk);
            true
        }
    }

    fn planned(op: Operation) -> Plan {
        let n = name("web");
        plan(
            &StackContext {
                stacks_root: Path::new("/opt/stacks"),
                stack: &n,
                global_env: false,
                has_env: false,
            },
            &op,
        )
    }

    fn draft(base: bool) -> StackDraft {
        StackDraft {
            stack: name("web"),
            base_hash: base.then(|| content_hash_of(ComposeFile::ComposeYaml, b"old", None)),
            compose: "services: {}\n".into(),
            env: Some("A=1".into()),
        }
    }

    fn run(
        op: Operation,
        d: Option<&StackDraft>,
        fail_on: Option<&'static str>,
        cancel_after_first: bool,
    ) -> (Result<StepOutcome, ExecError>, Vec<String>, usize) {
        let log = Log::default();
        let cancel = CancelToken::new();
        let rt = FakeRuntime {
            log: &log,
            fail_on,
            cancel_after_first: cancel_after_first.then(|| cancel.clone()),
        };
        let sink = Collect(Mutex::new(vec![]));
        let out = execute(
            &planned(op),
            d,
            &name("web"),
            &FakeStore(&log),
            &rt,
            &sink,
            &cancel,
        );
        let chunks = sink.0.lock().unwrap().len();
        (out, log.take(), chunks)
    }

    #[test]
    fn a_save_with_deploy_validates_writes_then_ups_in_order() {
        let d = draft(true);
        let (out, log, chunks) = run(
            Operation::Save {
                deploy: true,
                env_after_write: true,
            },
            Some(&d),
            None,
            false,
        );
        assert_eq!(out, Ok(StepOutcome::Succeeded));
        assert_eq!(
            log,
            vec![
                "run docker compose -f - config --quiet stdin=Some(\"services: {}\\n\")",
                "write web services: {}\n Some(\"A=1\")",
                "run docker compose up -d --remove-orphans stdin=None",
            ]
        );
        assert_eq!(chunks, 2, "output reaches the sink");
    }

    #[test]
    fn a_draft_with_no_base_hash_is_a_create() {
        let d = draft(false);
        let (out, log, _) = run(
            Operation::Create {
                deploy: false,
                env_after_write: true,
            },
            Some(&d),
            None,
            false,
        );
        assert_eq!(out, Ok(StepOutcome::Succeeded));
        assert_eq!(log[1], "create web services: {}\n Some(\"A=1\")");
    }

    #[test]
    fn a_failed_validation_writes_nothing() {
        let d = draft(true);
        let (out, log, _) = run(
            Operation::Save {
                deploy: true,
                env_after_write: false,
            },
            Some(&d),
            Some("config"),
            false,
        );
        assert!(matches!(
            out,
            Ok(StepOutcome::Failed {
                exit_code: Some(1),
                ..
            })
        ));
        assert_eq!(log.len(), 1, "stopped after the failing step: {log:?}");
    }

    #[test]
    fn delete_removes_the_directory_only_after_down_succeeds() {
        let (out, log, _) = run(
            Operation::Delete {
                remove_directory: true,
            },
            None,
            None,
            false,
        );
        assert_eq!(out, Ok(StepOutcome::Succeeded));
        assert_eq!(log.last().map(String::as_str), Some("rmdir web"));

        let (_, log, _) = run(
            Operation::Delete {
                remove_directory: true,
            },
            None,
            Some("down"),
            false,
        );
        assert!(!log.iter().any(|l| l.starts_with("rmdir")));
    }

    #[test]
    fn a_cancel_stops_before_the_next_step() {
        let (out, log, _) = run(Operation::Update, None, None, true);
        assert_eq!(out, Ok(StepOutcome::Cancelled));
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn a_plan_that_writes_needs_a_draft() {
        let (out, log, _) = run(
            Operation::Save {
                deploy: false,
                env_after_write: false,
            },
            None,
            None,
            false,
        );
        assert_eq!(out, Err(ExecError::MissingDraft));
        assert!(log.is_empty());
    }

    #[test]
    fn run_steps_stops_at_the_first_failure() {
        let log = Log::default();
        let rt = FakeRuntime {
            log: &log,
            fail_on: Some("pull"),
            cancel_after_first: None,
        };
        let p = planned(Operation::Update);
        let steps: Vec<RunStep> = p.commands().cloned().collect();
        let out = rt
            .run_steps(
                &steps,
                None,
                &Collect(Mutex::new(vec![])),
                &CancelToken::new(),
            )
            .unwrap();
        assert!(matches!(out, StepOutcome::Failed { .. }));
        assert_eq!(log.take().len(), 1);
    }

    // -- The volume helper channel ------------------------------------------------------------

    /// Records how the channel ended its process.
    #[derive(Clone, Default)]
    struct Ends(Arc<Mutex<Vec<&'static str>>>);

    impl HelperProcess for Ends {
        fn close(self: Box<Self>) {
            self.0.lock().unwrap().push("close");
        }
        fn kill(self: Box<Self>) {
            self.0.lock().unwrap().push("kill");
        }
    }

    impl Ends {
        fn take(&self) -> Vec<&'static str> {
            std::mem::take(&mut self.0.lock().unwrap())
        }
    }

    fn hello(proto: u32) -> Frame {
        Frame::json(
            FrameKind::Hello,
            &Hello {
                proto,
                openat2: true,
                uid: 1000,
                gid: 1000,
            },
        )
        .unwrap()
    }

    /// A channel over real pipes to a "helper" thread that sends `first`, then echoes every
    /// frame back until its stdin ends, and reports how many it echoed.
    fn echo_helper(first: Frame) -> (Result<HelperChannel, ChannelError>, Ends, ThreadCount) {
        let (mut from_server, to_helper) = std::io::pipe().unwrap();
        let (from_helper, mut to_server) = std::io::pipe().unwrap();
        let thread = std::thread::spawn(move || {
            first.write_to(&mut to_server).unwrap();
            let mut n = 0;
            while let Ok(Some(frame)) = read_frame(&mut from_server) {
                frame.write_to(&mut to_server).unwrap();
                n += 1;
            }
            n
        });
        let ends = Ends::default();
        let chan = HelperChannel::open(
            Box::new(from_helper),
            Box::new(to_helper),
            Box::new(ends.clone()),
        );
        (chan, ends, thread)
    }

    type ThreadCount = std::thread::JoinHandle<usize>;

    #[test]
    fn a_channel_handshakes_then_carries_frames_and_closes_cleanly() {
        let (chan, ends, thread) = echo_helper(hello(PROTO));
        let mut chan = chan.unwrap();
        assert_eq!(chan.hello().uid, 1000);
        assert!(chan.hello().openat2);
        for body in [b"one".to_vec(), Vec::new()] {
            chan.send(&Frame::data(body.clone())).unwrap();
            assert_eq!(chan.recv().unwrap(), Frame::data(body));
        }
        chan.close();
        // Closing ended the helper's stdin, so it saw EOF after two frames.
        assert_eq!(thread.join().unwrap(), 2);
        assert_eq!(ends.take(), ["close"]);
    }

    #[test]
    fn a_wrong_first_frame_or_protocol_fails_the_handshake_and_kills() {
        for first in [hello(PROTO + 1), Frame::end(), Frame::data(b"{}".to_vec())] {
            let (chan, ends, thread) = echo_helper(first);
            let err = chan.unwrap_err();
            assert!(matches!(err, ChannelError::Handshake(_)), "{err}");
            assert_eq!(ends.take(), ["kill"]);
            assert_eq!(thread.join().unwrap(), 0);
        }
    }

    #[test]
    fn a_dropped_channel_kills_and_a_closed_stdout_is_closed() {
        let (chan, ends, thread) = echo_helper(hello(PROTO));
        drop(chan.unwrap());
        assert_eq!(ends.take(), ["kill"]);
        thread.join().unwrap();

        // A helper that sends nothing at all.
        let (reader, writer) = std::io::pipe().unwrap();
        drop(writer);
        let (_, to_helper) = std::io::pipe().unwrap();
        let ends = Ends::default();
        let err = HelperChannel::open(
            Box::new(reader),
            Box::new(to_helper),
            Box::new(ends.clone()),
        )
        .unwrap_err();
        assert!(matches!(err, ChannelError::Closed), "{err}");
        assert_eq!(ends.take(), ["kill"]);
    }
}
