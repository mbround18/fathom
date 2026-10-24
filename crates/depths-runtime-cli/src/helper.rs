//! The [`VolumeHelper`] port over `docker run -i`, and the pool of warm helpers (spec 053, R4).
//!
//! - The argv is fixed (contracts/helper.md): [`helper_argv`] builds it from a [`HelperSpec`] the
//!   server derived itself, and nothing a caller passes becomes a flag. Everything it would not
//!   mount is refused before the argv exists: a named volume with no [`VolumeExists`] proof
//!   (`--mount type=volume` would create it), a volume name Docker would not accept, and a bind
//!   source that is not absolute, not normalised, carries `,` or `=`, or that `bind_reason()`
//!   rates root or escape.
//! - Each helper is named `depths-helper-<random>`, so a protocol error can `docker rm -f` exactly
//!   that container.
//! - [`HelperPool`] keeps a warm helper per spec for [`HELPER_IDLE`], at most `max` at once, and
//!   closes the least recently used first. One op at a time per helper; transfers get one-shot
//!   helpers that count toward `max` but are never reused.
//!
//! Every call blocks; async callers run it on a blocking thread.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher as _;
use std::ops::{Deref, DerefMut};
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use container_runtime::runner::Runner as _;
use depths_core::plan::DOCKER;
use depths_core::ports::{
    ChannelError, HelperChannel, HelperProcess, HelperSource, HelperSpec, OpMode, RuntimeError,
    VolumeHelper,
};
use depths_core::security::{Severity, bind_reason, normalise_host_path, valid_container_arg};

use crate::{CliRuntime, Mapped};

/// Every helper container carries `depths.helper=1`; [`CliVolumeHelper::sweep`] removes them.
pub const HELPER_LABEL: &str = "depths.helper";
/// The stack a helper works for.
pub const HELPER_STACK_LABEL: &str = "depths.helper.stack";
/// The helper binary inside the server's own image.
pub const HELPER_ENTRYPOINT: &str = "/usr/local/bin/depths-volume-helper";
/// Where the helper finds its mount.
pub const HELPER_ROOT: &str = "/volume";
/// How long a warm helper waits for its next op.
pub const HELPER_IDLE: Duration = Duration::from_secs(90);
/// How long a closed helper has to exit before it is killed.
const CLOSE_GRACE: Duration = Duration::from_secs(5);
/// How much of `docker run`'s stderr a failed start carries back.
const STDERR_KEEP: usize = 4096;

// ---------------------------------------------------------------------------
// The argv
// ---------------------------------------------------------------------------

/// Why a helper argv was not built. Nothing ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelperRefusal {
    /// A named volume with no proof it exists, or a proof for another volume.
    VolumeNotProven(String),
    /// Fails Docker's volume name rule, `[a-zA-Z0-9][a-zA-Z0-9_.-]+`.
    InvalidVolumeName(String),
    BindNotUtf8,
    BindNotAbsolute(String),
    /// `..`, `.`, `//` or a trailing `/`.
    BindNotNormalised(String),
    /// `,` or `=`, which `--mount` would read as its own syntax.
    BindSyntax(String),
    /// `bind_reason()` rated it root or escape; its message.
    BindRisky(String),
    /// Not `sha256:<64 hex>` or 64 hex: the helper runs by image id only.
    InvalidImage(String),
    InvalidContainerName(String),
}

impl std::fmt::Display for HelperRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::VolumeNotProven(v) => {
                write!(f, "refused: volume {v:?} was not confirmed to exist")
            }
            Self::InvalidVolumeName(v) => write!(f, "refused: {v:?} is not a volume name"),
            Self::BindNotUtf8 => f.write_str("refused: the bind source is not UTF-8"),
            Self::BindNotAbsolute(p) => write!(f, "refused: bind source {p:?} is not absolute"),
            Self::BindNotNormalised(p) => {
                write!(f, "refused: bind source {p:?} is not normalised")
            }
            Self::BindSyntax(p) => {
                write!(f, "refused: bind source {p:?} contains ',' or '='")
            }
            Self::BindRisky(why) => write!(f, "refused: {why}"),
            Self::InvalidImage(i) => write!(f, "refused: {i:?} is not an image id"),
            Self::InvalidContainerName(n) => write!(f, "refused: {n:?} is not a container name"),
        }
    }
}

impl std::error::Error for HelperRefusal {}

impl From<HelperRefusal> for RuntimeError {
    fn from(r: HelperRefusal) -> Self {
        RuntimeError::Failed(r.to_string())
    }
}

/// Docker's volume name rule for the local driver: `[a-zA-Z0-9][a-zA-Z0-9_.-]+`.
pub fn valid_volume_name(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() >= 2
        && b.len() <= 255
        && b[0].is_ascii_alphanumeric()
        && b[1..]
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-'))
}

/// Proof that a named volume existed when `docker volume inspect` last looked. Only
/// [`CliVolumeHelper::volume_exists`] makes one, so the argv can never name a volume nobody
/// checked, and `docker run` never creates one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeExists(String);

impl VolumeExists {
    pub fn name(&self) -> &str {
        &self.0
    }

    #[cfg(test)]
    pub(crate) fn assume(name: &str) -> Self {
        Self(name.to_owned())
    }
}

/// An image id, `sha256:<64 hex>` or 64 hex: never a tag, which could resolve to something else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageId(String);

impl ImageId {
    pub fn parse(raw: &str) -> Result<Self, HelperRefusal> {
        let hex = raw.strip_prefix("sha256:").unwrap_or(raw);
        if hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            Ok(Self(raw.to_owned()))
        } else {
            Err(HelperRefusal::InvalidImage(raw.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The `--mount` value for `source`, or why it may not be mounted.
fn mount_arg(
    source: &HelperSource,
    proof: Option<&VolumeExists>,
    mode: OpMode,
) -> Result<String, HelperRefusal> {
    let (kind, src) = match source {
        HelperSource::Volume(name) => {
            if !valid_volume_name(name) {
                return Err(HelperRefusal::InvalidVolumeName(name.clone()));
            }
            if proof.map(VolumeExists::name) != Some(name.as_str()) {
                return Err(HelperRefusal::VolumeNotProven(name.clone()));
            }
            ("volume", name.as_str())
        }
        HelperSource::Bind(path) => ("bind", bind_source(path, mode)?),
    };
    let readonly = match mode {
        OpMode::ReadOnly => ",readonly",
        OpMode::ReadWrite => "",
    };
    Ok(format!("type={kind},src={src},dst={HELPER_ROOT}{readonly}"))
}

fn bind_source(path: &Path, mode: OpMode) -> Result<&str, HelperRefusal> {
    let s = path.to_str().ok_or(HelperRefusal::BindNotUtf8)?;
    if !s.starts_with('/') {
        return Err(HelperRefusal::BindNotAbsolute(s.to_owned()));
    }
    if normalise_host_path(s).as_deref() != Some(s) {
        return Err(HelperRefusal::BindNotNormalised(s.to_owned()));
    }
    if s.contains([',', '=']) {
        return Err(HelperRefusal::BindSyntax(s.to_owned()));
    }
    // The helper's own mount is rated: writable only when the op writes.
    if let Some(reason) = bind_reason(s, mode == OpMode::ReadWrite)
        && reason.severity != Severity::Info
    {
        return Err(HelperRefusal::BindRisky(reason.message));
    }
    Ok(s)
}

/// The helper's `docker run` argv, program first (contracts/helper.md). `proof` must be the
/// volume's when `spec.source` is a named volume; `name` is the container's name.
pub fn helper_argv(
    spec: &HelperSpec,
    proof: Option<&VolumeExists>,
    image: &ImageId,
    name: &str,
) -> Result<Vec<String>, HelperRefusal> {
    let mount = mount_arg(&spec.source, proof, spec.mode)?;
    if !valid_container_arg(name) {
        return Err(HelperRefusal::InvalidContainerName(name.to_owned()));
    }
    let fixed = [
        DOCKER,
        "run",
        "-i",
        "--rm",
        "--pull",
        "never",
        "--network",
        "none",
        "--read-only",
        "--cap-drop",
        "ALL",
        "--security-opt",
        "no-new-privileges",
    ];
    let mut argv: Vec<String> = fixed.iter().map(|s| (*s).to_owned()).collect();
    argv.extend(["--user".to_owned(), format!("{}:{}", spec.uid, spec.gid)]);
    for s in [
        "--memory",
        "256m",
        "--pids-limit",
        "32",
        "--cpus",
        "1",
        "--log-driver",
        "none",
        "--label",
    ] {
        argv.push(s.to_owned());
    }
    argv.push(format!("{HELPER_LABEL}=1"));
    argv.push("--label".to_owned());
    argv.push(format!("{HELPER_STACK_LABEL}={}", spec.stack.as_str()));
    argv.extend([
        "--name".to_owned(),
        name.to_owned(),
        "--entrypoint".to_owned(),
        HELPER_ENTRYPOINT.to_owned(),
        "--mount".to_owned(),
        mount,
        image.as_str().to_owned(),
    ]);
    Ok(argv)
}

/// A fresh container name: unique per process and start, not secret.
fn container_name() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let h = RandomState::new().hash_one((std::process::id(), n, nanos));
    format!("depths-helper-{h:016x}")
}

// ---------------------------------------------------------------------------
// The adapter
// ---------------------------------------------------------------------------

/// The docker CLI as a [`VolumeHelper`]: runs the server's own image, by id, as the helper.
#[derive(Debug, Clone)]
pub struct CliVolumeHelper {
    cli: CliRuntime,
    image: ImageId,
}

impl CliVolumeHelper {
    pub fn new(cli: CliRuntime, image: ImageId) -> Self {
        Self { cli, image }
    }

    /// `docker volume inspect` of `name`: the proof when it exists, `None` when it does not.
    pub fn volume_exists(&self, name: &str) -> Result<Option<VolumeExists>, RuntimeError> {
        if !valid_volume_name(name) {
            return Err(HelperRefusal::InvalidVolumeName(name.to_owned()).into());
        }
        let out = Mapped(&self.cli)
            .run(
                DOCKER,
                &["volume", "inspect", "--format", "{{.Name}}", "--", name],
            )
            .map_err(|e| self.cli.unavailable(e))?;
        if out.success && out.stdout.trim() == name {
            Ok(Some(VolumeExists(name.to_owned())))
        } else if out.stderr.contains("no such volume") || out.stderr.contains("No such volume") {
            Ok(None)
        } else if out.success {
            Err(RuntimeError::Failed(format!(
                "docker volume inspect: unexpected output {:?}",
                out.stdout.trim()
            )))
        } else {
            Err(RuntimeError::Unavailable(out.stderr.trim_end().to_owned()))
        }
    }
}

impl VolumeHelper for CliVolumeHelper {
    fn start(&self, spec: &HelperSpec) -> Result<HelperChannel, RuntimeError> {
        let proof =
            match &spec.source {
                HelperSource::Volume(name) => Some(self.volume_exists(name)?.ok_or_else(|| {
                    RuntimeError::Failed(format!("volume {name:?} does not exist"))
                })?),
                HelperSource::Bind(_) => None,
            };
        let name = container_name();
        let argv = helper_argv(spec, proof.as_ref(), &self.image, &name)?;
        let mut child = self
            .cli
            .command(&argv[1..], None)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| self.cli.spawn_error(&e))?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(RuntimeError::Failed(
                "the helper's pipes are missing".into(),
            ));
        };
        let tail = Arc::new(Mutex::new(Vec::new()));
        let reader = {
            let tail = tail.clone();
            std::thread::spawn(move || keep_tail(stderr, &tail))
        };
        let process = CliHelperProcess {
            child,
            name,
            cli: self.cli.clone(),
            stderr: Some(reader),
        };
        let chan = HelperChannel::open(Box::new(stdout), Box::new(stdin), Box::new(process))
            .map_err(|e| {
                // The process is killed and its stderr reader joined by now.
                let said = String::from_utf8_lossy(&tail.lock().unwrap_or_else(|p| p.into_inner()))
                    .trim_end()
                    .to_owned();
                if said.is_empty() {
                    RuntimeError::Failed(e.to_string())
                } else {
                    RuntimeError::Failed(format!("{e}: {said}"))
                }
            })?;
        let hello = *chan.hello();
        if (hello.uid, hello.gid) != (spec.uid, spec.gid) {
            chan.kill();
            return Err(RuntimeError::Failed(format!(
                "the helper runs as {}:{}, not {}:{}",
                hello.uid, hello.gid, spec.uid, spec.gid
            )));
        }
        Ok(chan)
    }

    fn sweep(&self) -> Result<usize, RuntimeError> {
        let label = format!("label={HELPER_LABEL}=1");
        let out = Mapped(&self.cli)
            .run(DOCKER, &["ps", "-aq", "--no-trunc", "--filter", &label])
            .map_err(|e| self.cli.unavailable(e))?;
        if !out.success {
            return Err(RuntimeError::Unavailable(out.stderr.trim_end().to_owned()));
        }
        let ids = leftover_ids(&out.stdout);
        if ids.is_empty() {
            return Ok(0);
        }
        let mut args = vec!["rm", "-f", "--"];
        args.extend(ids.iter().copied());
        let rm = Mapped(&self.cli)
            .run(DOCKER, &args)
            .map_err(|e| self.cli.unavailable(e))?;
        if !rm.success {
            return Err(RuntimeError::Failed(rm.stderr.trim_end().to_owned()));
        }
        Ok(ids.len())
    }
}

/// The container ids in `docker ps -q` output, skipping anything that is not one.
pub fn leftover_ids(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|id| !id.is_empty() && valid_container_arg(id))
        .collect()
}

/// Keeps the first [`STDERR_KEEP`] bytes of `from` and drains the rest, so the helper never
/// blocks on a full stderr pipe.
fn keep_tail(mut from: impl std::io::Read, into: &Mutex<Vec<u8>>) {
    let mut buf = [0u8; 1024];
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                let mut kept = into.lock().unwrap_or_else(|p| p.into_inner());
                let room = STDERR_KEEP.saturating_sub(kept.len());
                kept.extend_from_slice(&buf[..n.min(room)]);
            }
        }
    }
}

struct CliHelperProcess {
    child: Child,
    name: String,
    cli: CliRuntime,
    stderr: Option<JoinHandle<()>>,
}

impl CliHelperProcess {
    fn reap(&mut self) {
        let _ = self.child.wait();
        if let Some(r) = self.stderr.take() {
            let _ = r.join();
        }
    }
}

impl HelperProcess for CliHelperProcess {
    fn close(mut self: Box<Self>) {
        // stdin is closed, so the helper exits 0 and `--rm` removes it; a stuck one is killed.
        let deadline = Instant::now() + CLOSE_GRACE;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return self.reap(),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => return self.kill(),
            }
        }
    }

    fn kill(mut self: Box<Self>) {
        // The container first: killing only the CLI could leave it running.
        let _ = Mapped(&self.cli).run(DOCKER, &["rm", "-f", "--", &self.name]);
        let _ = self.child.kill();
        self.reap();
    }
}

// ---------------------------------------------------------------------------
// The pool
// ---------------------------------------------------------------------------

/// Where the pool reads the time, so tests can move it.
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Why a pooled op did not run to the end.
#[derive(Debug)]
pub enum PoolError {
    /// The helper did not start; nothing ran.
    Start(RuntimeError),
    /// The channel failed during the op; the helper was killed and removed.
    Channel(ChannelError),
}

impl std::fmt::Display for PoolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Start(e) => e.fmt(f),
            Self::Channel(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for PoolError {}

/// Warm helpers, one per [`HelperSpec`] (stack, mount, uid, gid, mode), at most `max` live,
/// one-shots included. Cheap to clone; clones share the pool.
#[derive(Clone)]
pub struct HelperPool {
    inner: Arc<Inner>,
}

struct Inner {
    helper: Arc<dyn VolumeHelper>,
    clock: Arc<dyn Clock>,
    max: usize,
    idle: Duration,
    state: Mutex<State>,
    /// Signalled whenever a helper is returned or a slot frees.
    freed: Condvar,
}

#[derive(Default)]
struct State {
    warm: Vec<Slot>,
    one_shots: usize,
    next_id: u64,
}

struct Slot {
    id: u64,
    spec: HelperSpec,
    /// `None` while it starts or an op runs.
    chan: Option<HelperChannel>,
    last_used: Instant,
}

impl State {
    fn live(&self) -> usize {
        self.warm.len() + self.one_shots
    }

    /// Takes out every idle helper unused for `idle`.
    fn reap(&mut self, now: Instant, idle: Duration, out: &mut Vec<HelperChannel>) {
        let mut i = 0;
        while i < self.warm.len() {
            let slot = &self.warm[i];
            if slot.chan.is_some() && now.duration_since(slot.last_used) >= idle {
                out.extend(self.warm.remove(i).chan);
            } else {
                i += 1;
            }
        }
    }

    /// Takes out the least recently used idle helper, if there is one.
    fn evict_lru(&mut self, out: &mut Vec<HelperChannel>) -> bool {
        let lru = self
            .warm
            .iter()
            .enumerate()
            .filter(|(_, s)| s.chan.is_some())
            .min_by_key(|(_, s)| s.last_used)
            .map(|(i, _)| i);
        match lru {
            Some(i) => {
                out.extend(self.warm.remove(i).chan);
                true
            }
            None => false,
        }
    }
}

enum Acquired {
    Warm(u64, HelperChannel),
    Cold(u64),
}

impl HelperPool {
    /// `max` is `files_helpers` (default 8); `idle` is normally [`HELPER_IDLE`].
    pub fn new(
        helper: Arc<dyn VolumeHelper>,
        clock: Arc<dyn Clock>,
        max: usize,
        idle: Duration,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                helper,
                clock,
                max: max.max(1),
                idle,
                state: Mutex::new(State::default()),
                freed: Condvar::new(),
            }),
        }
    }

    /// Runs `op` on the warm helper for `spec`, starting one if there is none. Ops on one helper
    /// run one at a time; a second caller waits. An error from `op` kills the helper.
    pub fn run<T>(
        &self,
        spec: &HelperSpec,
        op: impl FnOnce(&mut HelperChannel) -> Result<T, ChannelError>,
    ) -> Result<T, PoolError> {
        let (id, mut chan) = match self.acquire(spec) {
            Acquired::Warm(id, chan) => (id, chan),
            Acquired::Cold(id) => match self.inner.helper.start(spec) {
                Ok(chan) => (id, chan),
                Err(e) => {
                    self.forget(id);
                    return Err(PoolError::Start(e));
                }
            },
        };
        match op(&mut chan) {
            Ok(v) => {
                let mut st = self.lock();
                let now = self.inner.clock.now();
                if let Some(slot) = st.warm.iter_mut().find(|s| s.id == id) {
                    slot.chan = Some(chan);
                    slot.last_used = now;
                }
                drop(st);
                self.inner.freed.notify_all();
                Ok(v)
            }
            Err(e) => {
                self.forget(id);
                chan.kill();
                Err(PoolError::Channel(e))
            }
        }
    }

    /// A helper of its own for one transfer. It counts toward `max` until dropped, and is never
    /// reused: end it with [`OneShot::close`], or [`OneShot::kill`] after an error.
    pub fn one_shot(&self, spec: &HelperSpec) -> Result<OneShot, PoolError> {
        let mut closing = Vec::new();
        {
            let mut st = self.lock();
            loop {
                st.reap(self.inner.clock.now(), self.inner.idle, &mut closing);
                if st.live() < self.inner.max {
                    st.one_shots += 1;
                    break;
                }
                if !st.evict_lru(&mut closing) {
                    st = self.wait(st);
                }
            }
        }
        closing.into_iter().for_each(HelperChannel::close);
        match self.inner.helper.start(spec) {
            Ok(chan) => Ok(OneShot {
                chan: Some(chan),
                pool: self.inner.clone(),
            }),
            Err(e) => {
                self.lock().one_shots -= 1;
                self.inner.freed.notify_all();
                Err(PoolError::Start(e))
            }
        }
    }

    /// Closes every idle helper unused for the idle timeout. The pool also does this on every
    /// call; a server ticks it so an unused pool empties too.
    pub fn reap(&self) {
        let mut closing = Vec::new();
        self.lock()
            .reap(self.inner.clock.now(), self.inner.idle, &mut closing);
        closing.into_iter().for_each(HelperChannel::close);
    }

    /// Helpers alive now, warm and one-shot.
    pub fn live(&self) -> usize {
        self.lock().live()
    }

    fn acquire(&self, spec: &HelperSpec) -> Acquired {
        let mut closing = Vec::new();
        let got = {
            let mut st = self.lock();
            loop {
                let now = self.inner.clock.now();
                st.reap(now, self.inner.idle, &mut closing);
                if let Some(slot) = st.warm.iter_mut().find(|s| &s.spec == spec) {
                    match slot.chan.take() {
                        Some(chan) => {
                            slot.last_used = now;
                            break Acquired::Warm(slot.id, chan);
                        }
                        // Busy or starting: one op at a time.
                        None => {
                            st = self.wait(st);
                            continue;
                        }
                    }
                }
                if st.live() < self.inner.max {
                    let id = st.next_id;
                    st.next_id += 1;
                    st.warm.push(Slot {
                        id,
                        spec: spec.clone(),
                        chan: None,
                        last_used: now,
                    });
                    break Acquired::Cold(id);
                }
                if !st.evict_lru(&mut closing) {
                    st = self.wait(st);
                }
            }
        };
        closing.into_iter().for_each(HelperChannel::close);
        got
    }

    fn forget(&self, id: u64) {
        self.lock().warm.retain(|s| s.id != id);
        self.inner.freed.notify_all();
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn wait<'a>(&self, st: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        self.inner.freed.wait(st).unwrap_or_else(|p| p.into_inner())
    }
}

/// A one-shot helper from [`HelperPool::one_shot`]. Dropping it without [`OneShot::close`]
/// kills the helper.
pub struct OneShot {
    chan: Option<HelperChannel>,
    pool: Arc<Inner>,
}

impl OneShot {
    pub fn close(mut self) {
        if let Some(chan) = self.chan.take() {
            chan.close();
        }
    }

    pub fn kill(mut self) {
        if let Some(chan) = self.chan.take() {
            chan.kill();
        }
    }
}

impl Deref for OneShot {
    type Target = HelperChannel;
    fn deref(&self) -> &HelperChannel {
        self.chan
            .as_ref()
            .expect("a one-shot holds its channel until it ends")
    }
}

impl DerefMut for OneShot {
    fn deref_mut(&mut self) -> &mut HelperChannel {
        self.chan
            .as_mut()
            .expect("a one-shot holds its channel until it ends")
    }
}

impl Drop for OneShot {
    fn drop(&mut self) {
        drop(self.chan.take());
        let mut st = self.pool.state.lock().unwrap_or_else(|p| p.into_inner());
        st.one_shots -= 1;
        drop(st);
        self.pool.freed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;

    use depths_core::StackName;
    use depths_volumes::frame::{Frame, FrameKind};
    use depths_volumes::op::{Hello, PROTO};

    use super::*;

    const IMAGE: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn spec(source: HelperSource, mode: OpMode) -> HelperSpec {
        HelperSpec {
            stack: StackName::parse("game").unwrap(),
            source,
            uid: 1000,
            gid: 100,
            mode,
        }
    }

    fn volume(name: &str) -> HelperSource {
        HelperSource::Volume(name.into())
    }

    fn bind(path: &str) -> HelperSource {
        HelperSource::Bind(PathBuf::from(path))
    }

    fn argv(spec: &HelperSpec, proof: Option<&VolumeExists>) -> Result<String, HelperRefusal> {
        let image = ImageId::parse(IMAGE).unwrap();
        helper_argv(spec, proof, &image, "depths-helper-00ff").map(|a| a.join(" "))
    }

    fn expected(mount: &str) -> String {
        format!(
            "docker run -i --rm --pull never --network none --read-only --cap-drop ALL \
             --security-opt no-new-privileges --user 1000:100 \
             --memory 256m --pids-limit 32 --cpus 1 --log-driver none \
             --label depths.helper=1 --label depths.helper.stack=game \
             --name depths-helper-00ff \
             --entrypoint /usr/local/bin/depths-volume-helper \
             --mount {mount} {IMAGE}"
        )
    }

    #[test]
    fn a_volume_argv_is_the_contracts_read_only_unless_the_op_writes() {
        let proof = VolumeExists::assume("game_world");
        let ro = spec(volume("game_world"), OpMode::ReadOnly);
        assert_eq!(
            argv(&ro, Some(&proof)).unwrap(),
            expected("type=volume,src=game_world,dst=/volume,readonly")
        );
        let rw = spec(volume("game_world"), OpMode::ReadWrite);
        assert_eq!(
            argv(&rw, Some(&proof)).unwrap(),
            expected("type=volume,src=game_world,dst=/volume")
        );
    }

    #[test]
    fn a_bind_argv_is_the_contracts_read_only_unless_the_op_writes() {
        let ro = spec(bind("/srv/game/config"), OpMode::ReadOnly);
        assert_eq!(
            argv(&ro, None).unwrap(),
            expected("type=bind,src=/srv/game/config,dst=/volume,readonly")
        );
        let rw = spec(bind("/srv/game/config"), OpMode::ReadWrite);
        assert_eq!(
            argv(&rw, None).unwrap(),
            expected("type=bind,src=/srv/game/config,dst=/volume")
        );
        // An info-level rating (a read-only `/proc/sys`) is still mountable.
        let info = spec(bind("/proc/sys"), OpMode::ReadOnly);
        assert!(argv(&info, None).is_ok());
    }

    #[test]
    fn a_volume_needs_its_own_proof_and_a_valid_name() {
        let s = spec(volume("game_world"), OpMode::ReadOnly);
        assert_eq!(
            argv(&s, None),
            Err(HelperRefusal::VolumeNotProven("game_world".into()))
        );
        let other = VolumeExists::assume("other");
        assert_eq!(
            argv(&s, Some(&other)),
            Err(HelperRefusal::VolumeNotProven("game_world".into()))
        );
        for bad in ["", "a", "-rm", "_x", "a,b", "a=b", "a/b", "a b", "vol\n"] {
            let s = spec(volume(bad), OpMode::ReadOnly);
            let proof = VolumeExists::assume(bad);
            assert_eq!(
                argv(&s, Some(&proof)),
                Err(HelperRefusal::InvalidVolumeName(bad.into())),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_bind_must_be_absolute_normalised_plain_and_not_risky() {
        let refused = |path: &str, mode| argv(&spec(bind(path), mode), None).unwrap_err();
        assert_eq!(
            refused("srv/game", OpMode::ReadOnly),
            HelperRefusal::BindNotAbsolute("srv/game".into())
        );
        for p in ["/srv/../etc", "/srv/./game", "/srv//game", "/srv/game/"] {
            assert_eq!(
                refused(p, OpMode::ReadOnly),
                HelperRefusal::BindNotNormalised(p.into()),
                "{p}"
            );
        }
        for p in ["/srv/a,readonly=false", "/srv/a=b"] {
            assert_eq!(
                refused(p, OpMode::ReadOnly),
                HelperRefusal::BindSyntax(p.into())
            );
        }
        // Root and escape ratings from `bind_reason()`, read-only and writable.
        for (p, mode) in [
            ("/", OpMode::ReadOnly),
            ("/", OpMode::ReadWrite),
            ("/etc", OpMode::ReadOnly),
            ("/var/run/docker.sock", OpMode::ReadOnly),
            ("/proc/sys", OpMode::ReadWrite),
        ] {
            assert!(
                matches!(refused(p, mode), HelperRefusal::BindRisky(_)),
                "{p} {mode:?}"
            );
        }
        use std::os::unix::ffi::OsStrExt as _;
        let raw = PathBuf::from(std::ffi::OsStr::from_bytes(b"/srv/\xff"));
        assert_eq!(
            argv(&spec(HelperSource::Bind(raw), OpMode::ReadOnly), None),
            Err(HelperRefusal::BindNotUtf8)
        );
    }

    #[test]
    fn only_an_image_id_and_a_plain_container_name_are_accepted() {
        assert!(ImageId::parse(IMAGE).is_ok());
        assert!(ImageId::parse(&IMAGE[7..]).is_ok());
        for bad in ["depths:latest", "sha256:abc", "", &IMAGE.to_uppercase()] {
            assert!(ImageId::parse(bad).is_err(), "{bad}");
        }
        let s = spec(bind("/srv/game"), OpMode::ReadOnly);
        let image = ImageId::parse(IMAGE).unwrap();
        assert_eq!(
            helper_argv(&s, None, &image, "--privileged"),
            Err(HelperRefusal::InvalidContainerName("--privileged".into()))
        );
        let a = container_name();
        let b = container_name();
        assert_ne!(a, b);
        assert!(a.starts_with("depths-helper-") && valid_container_arg(&a));
    }

    #[test]
    fn leftover_ids_skip_blank_and_hostile_lines() {
        assert_eq!(
            leftover_ids("abc123\n\n  def456  \n--all\n"),
            ["abc123", "def456"]
        );
    }

    // -- The pool -----------------------------------------------------------------------------

    #[derive(Default)]
    struct FakeClock(Mutex<Duration>);

    static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

    impl FakeClock {
        fn advance(&self, by: Duration) {
            *self.0.lock().unwrap() += by;
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            *BASE.get_or_init(Instant::now) + *self.0.lock().unwrap()
        }
    }

    /// Starts in-memory helpers, numbered, and records how each ended.
    #[derive(Default)]
    struct FakeSpawner {
        started: Mutex<Vec<(u32, HelperSpec)>>,
        ended: Arc<Mutex<Vec<(u32, &'static str)>>>,
        fail: AtomicBool,
    }

    struct FakeProcess(u32, Arc<Mutex<Vec<(u32, &'static str)>>>);

    impl HelperProcess for FakeProcess {
        fn close(self: Box<Self>) {
            self.1.lock().unwrap().push((self.0, "close"));
        }
        fn kill(self: Box<Self>) {
            self.1.lock().unwrap().push((self.0, "kill"));
        }
    }

    impl VolumeHelper for FakeSpawner {
        fn start(&self, spec: &HelperSpec) -> Result<HelperChannel, RuntimeError> {
            if self.fail.load(Ordering::SeqCst) {
                return Err(RuntimeError::Failed("no such image".into()));
            }
            let mut started = self.started.lock().unwrap();
            let n = started.len() as u32;
            started.push((n, spec.clone()));
            let hello = Hello {
                proto: PROTO,
                openat2: true,
                uid: spec.uid,
                gid: spec.gid,
            };
            let mut bytes = Vec::new();
            Frame::json(FrameKind::Hello, &hello)
                .unwrap()
                .write_to(&mut bytes)
                .unwrap();
            HelperChannel::open(
                Box::new(std::io::Cursor::new(bytes)),
                Box::new(std::io::sink()),
                Box::new(FakeProcess(n, self.ended.clone())),
            )
            .map_err(|e| RuntimeError::Failed(e.to_string()))
        }
    }

    impl FakeSpawner {
        fn starts(&self) -> usize {
            self.started.lock().unwrap().len()
        }
        fn ended(&self) -> Vec<(u32, &'static str)> {
            std::mem::take(&mut self.ended.lock().unwrap())
        }
    }

    fn pool(max: usize) -> (HelperPool, Arc<FakeSpawner>, Arc<FakeClock>) {
        let spawner = Arc::new(FakeSpawner::default());
        let clock = Arc::new(FakeClock::default());
        let pool = HelperPool::new(spawner.clone(), clock.clone(), max, HELPER_IDLE);
        (pool, spawner, clock)
    }

    fn uid(u: u32) -> HelperSpec {
        HelperSpec {
            uid: u,
            gid: u,
            ..spec(volume("game_world"), OpMode::ReadOnly)
        }
    }

    fn noop(pool: &HelperPool, s: &HelperSpec) {
        pool.run(s, |chan| {
            assert_eq!(chan.hello().uid, s.uid);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_helper_is_reused_per_spec_and_a_new_spec_starts_another() {
        let (pool, spawner, _) = pool(8);
        noop(&pool, &uid(1));
        noop(&pool, &uid(1));
        assert_eq!(spawner.starts(), 1);
        noop(&pool, &uid(2));
        let rw = HelperSpec {
            mode: OpMode::ReadWrite,
            ..uid(1)
        };
        noop(&pool, &rw);
        assert_eq!(spawner.starts(), 3);
        assert_eq!(pool.live(), 3);
        assert!(spawner.ended().is_empty());
    }

    #[test]
    fn an_idle_helper_closes_after_ninety_seconds() {
        let (pool, spawner, clock) = pool(8);
        noop(&pool, &uid(1));
        clock.advance(Duration::from_secs(89));
        noop(&pool, &uid(1));
        assert_eq!(spawner.starts(), 1, "89 s idle is still warm");

        // Each use restarts the idle timer.
        clock.advance(Duration::from_secs(89));
        pool.reap();
        assert_eq!(pool.live(), 1);
        clock.advance(Duration::from_secs(1));
        pool.reap();
        assert_eq!(pool.live(), 0);
        assert_eq!(spawner.ended(), [(0, "close")]);

        noop(&pool, &uid(1));
        assert_eq!(spawner.starts(), 2, "a cold start after the timeout");
    }

    #[test]
    fn at_the_cap_the_least_recently_used_helper_closes_first() {
        let (pool, spawner, clock) = pool(2);
        noop(&pool, &uid(1)); // helper 0
        clock.advance(Duration::from_secs(1));
        noop(&pool, &uid(2)); // helper 1
        clock.advance(Duration::from_secs(1));
        noop(&pool, &uid(1)); // helper 0 is now the most recent
        clock.advance(Duration::from_secs(1));
        noop(&pool, &uid(3)); // helper 2 evicts helper 1
        assert_eq!(spawner.ended(), [(1, "close")]);
        assert_eq!(pool.live(), 2);
        noop(&pool, &uid(1));
        assert_eq!(spawner.starts(), 3, "helper 0 survived");
    }

    #[test]
    fn a_channel_error_kills_the_helper_and_the_next_op_starts_fresh() {
        let (pool, spawner, _) = pool(8);
        noop(&pool, &uid(1));
        let err = pool
            .run(&uid(1), |_| Err::<(), _>(ChannelError::Closed))
            .unwrap_err();
        assert!(matches!(err, PoolError::Channel(ChannelError::Closed)));
        assert_eq!(spawner.ended(), [(0, "kill")]);
        assert_eq!(pool.live(), 0);
        noop(&pool, &uid(1));
        assert_eq!(spawner.starts(), 2);
    }

    #[test]
    fn a_failed_start_frees_its_slot() {
        let (pool, spawner, _) = pool(1);
        spawner.fail.store(true, Ordering::SeqCst);
        assert!(matches!(
            pool.run(&uid(1), |_| Ok(())),
            Err(PoolError::Start(_))
        ));
        assert!(matches!(pool.one_shot(&uid(1)), Err(PoolError::Start(_))));
        assert_eq!(pool.live(), 0);
        spawner.fail.store(false, Ordering::SeqCst);
        noop(&pool, &uid(1));
        assert_eq!(pool.live(), 1);
    }

    #[test]
    fn a_one_shot_counts_toward_the_cap_and_is_never_reused() {
        let (pool, spawner, clock) = pool(2);
        noop(&pool, &uid(1)); // helper 0
        clock.advance(Duration::from_secs(1));
        noop(&pool, &uid(2)); // helper 1
        let shot = pool.one_shot(&uid(1)).unwrap(); // helper 2 evicts helper 0
        assert_eq!(spawner.ended(), [(0, "close")]);
        assert_eq!(shot.hello().uid, 1);
        assert_eq!(pool.live(), 2);
        shot.close();
        assert_eq!(spawner.ended(), [(2, "close")]);
        assert_eq!(pool.live(), 1);

        let shot = pool.one_shot(&uid(2)).unwrap();
        drop(shot);
        assert_eq!(
            spawner.ended(),
            [(3, "kill")],
            "a dropped one-shot is killed"
        );
        assert_eq!(pool.live(), 1);
        noop(&pool, &uid(2));
        assert_eq!(spawner.starts(), 4, "the warm helper 1 served it");
    }

    #[test]
    fn ops_on_one_helper_never_overlap() {
        let (pool, spawner, _) = pool(8);
        let busy = Arc::new(AtomicBool::new(false));
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let (pool, busy) = (pool.clone(), busy.clone());
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        pool.run(&uid(1), |_| {
                            assert!(!busy.swap(true, Ordering::SeqCst), "two ops at once");
                            std::thread::yield_now();
                            busy.store(false, Ordering::SeqCst);
                            Ok(())
                        })
                        .unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(spawner.starts(), 1);
    }
}
