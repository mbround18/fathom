//! Typed host queries (spec 067 US5, contracts/relay.md "Runner additions" 3–5).
//!
//! Keel asks `Query { kind, params }`; the runner answers `QueryAnswer` or `QueryRefused`. There
//! is no shell and nothing is built from a parameter but an argv element or a path that has
//! already been checked:
//!
//! - `/etc/keel-runner/trust/queries.toml` lists the kinds, units and paths this host answers. It
//!   is read fresh for every query. A missing or invalid file refuses everything `not_allowed`.
//! - A query is checked against that file and its own parameters before anything on the host is
//!   touched ([`check`]). A unit must match `^[A-Za-z0-9@._:-]{1,256}$` and an allowed name (exact,
//!   or a trailing `*` on a literal prefix). A path must be absolute, normalised, without `..`,
//!   at most 4096 bytes, and equal to or below an allowed path; it is then resolved and must still
//!   be below an allowed path once symlinks are followed, so a link cannot point a query outside.
//! - `unit_state` runs `systemctl show --property=… -- <unit>` as an argv array with an empty
//!   environment ([`unit_state_command`]); `path_free` is `statvfs`; `disk_health` reads
//!   `/proc/mounts` and `/sys`, and asks the executor for SMART (`ReadSmart`), which picks the
//!   disks itself. No query reads a file named by a parameter.
//! - Answers over [`MAX_QUERY_ANSWER`] bytes are refused `too_large`; a fifth query in flight is
//!   refused `busy` ([`InFlight`]).
//! - Every query is appended to `/var/lib/keel-runner/queries.log` as one JSON line with who
//!   asked and the outcome, never the answer ([`QueryLog`]), rotated at 1 MiB keeping 3.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{CString, OsString};
use std::fs::OpenOptions;
use std::io::{Read as _, Write as _};
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use keel_exec::codec::{read_frame, write_frame};
use keel_exec::frames::{
    Base, Frame, MAX_QUERIES_IN_FLIGHT, MAX_QUERY_ANSWER, QueryRefusal, SmartDisk,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::UnixStream;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::policy::read_trust_file;

/// The trust file under the trust directory.
pub const QUERIES_FILE: &str = "queries.toml";
/// The host's query log, under the runner's state directory.
pub const QUERY_LOG_FILE: &str = "queries.log";
/// The log is rotated past this size...
pub const QUERY_LOG_MAX: u64 = 1024 * 1024;
/// ...keeping this many old files (`queries.log.1` … `.3`).
pub const QUERY_LOG_KEEP: u32 = 3;
/// The longest unit name.
pub const MAX_UNIT: usize = 256;
/// The longest path, in bytes.
pub const MAX_PATH: usize = 4096;
/// How long a single host read (systemctl, the executor) may take.
const STEP_TIMEOUT: Duration = Duration::from_secs(10);
/// The most read from any one host file or command's output.
const MAX_READ: u64 = 1024 * 1024;

/// The query kinds of contracts/relay.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Facts,
    AppliedRevision,
    UnitState,
    DiskHealth,
    PathFree,
}

impl Kind {
    pub const ALL: [Kind; 5] = [
        Kind::Facts,
        Kind::AppliedRevision,
        Kind::UnitState,
        Kind::DiskHealth,
        Kind::PathFree,
    ];

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == name)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Kind::Facts => "facts",
            Kind::AppliedRevision => "applied_revision",
            Kind::UnitState => "unit_state",
            Kind::DiskHealth => "disk_health",
            Kind::PathFree => "path_free",
        }
    }

    /// The one parameter this kind takes, if any.
    const fn param(self) -> Option<&'static str> {
        match self {
            Kind::UnitState => Some("unit"),
            Kind::PathFree => Some("path"),
            _ => None,
        }
    }
}

/// `queries.toml`, as written.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTrust {
    #[serde(default)]
    kinds: Vec<String>,
    #[serde(default)]
    units: Vec<String>,
    #[serde(default)]
    paths: Vec<String>,
}

/// One `units` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
enum UnitPattern {
    Exact(String),
    Prefix(String),
}

/// What this host answers: a parsed, checked `queries.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryTrust {
    kinds: BTreeSet<Kind>,
    units: Vec<UnitPattern>,
    paths: Vec<PathBuf>,
}

impl QueryTrust {
    /// Parses `queries.toml`. Unknown keys, an unknown kind, a unit pattern that is not a valid
    /// name or a literal prefix with one trailing `*`, and a path that is not absolute and
    /// normalised all fail the whole file.
    pub fn parse(text: &str) -> Result<Self, String> {
        let raw: RawTrust = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut kinds = BTreeSet::new();
        for k in &raw.kinds {
            kinds.insert(Kind::parse(k).ok_or_else(|| format!("unknown kind {k:?}"))?);
        }
        let mut units = Vec::new();
        for u in &raw.units {
            let pattern = match u.strip_suffix('*') {
                Some(prefix) if !prefix.is_empty() && valid_unit(prefix) => {
                    UnitPattern::Prefix(prefix.to_owned())
                }
                None if valid_unit(u) => UnitPattern::Exact(u.clone()),
                _ => {
                    return Err(format!(
                        "unit {u:?} is not a unit name or a literal prefix followed by *"
                    ));
                }
            };
            units.push(pattern);
        }
        let mut paths = Vec::new();
        for p in &raw.paths {
            if !valid_path(p) {
                return Err(format!("path {p:?} is not absolute and normalised"));
            }
            paths.push(PathBuf::from(p));
        }
        Ok(Self {
            kinds,
            units,
            paths,
        })
    }

    /// Reads `<trust_dir>/queries.toml`. `None` when it is missing, unreadable or invalid: every
    /// query is then refused. Anything but a missing file is reported.
    pub fn load(trust_dir: &Path) -> Option<Self> {
        let path = trust_dir.join(QUERIES_FILE);
        if !path.exists() {
            return None;
        }
        match read_trust_file(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| Self::parse(&t))
        {
            Ok(trust) => Some(trust),
            Err(e) => {
                eprintln!(
                    "keel-runner: {} is invalid, refusing every query: {e}",
                    path.display()
                );
                None
            }
        }
    }

    pub fn allows_kind(&self, kind: Kind) -> bool {
        self.kinds.contains(&kind)
    }

    /// Whether a (valid) unit name is listed, exactly or by prefix.
    pub fn allows_unit(&self, unit: &str) -> bool {
        valid_unit(unit)
            && self.units.iter().any(|p| match p {
                UnitPattern::Exact(e) => e == unit,
                UnitPattern::Prefix(prefix) => unit.starts_with(prefix.as_str()),
            })
    }

    /// Whether a (valid) path is an allowed path or below one. Purely lexical; see
    /// [`resolve_allowed`] for the check after symlinks.
    pub fn allows_path(&self, path: &str) -> bool {
        valid_path(path)
            && self
                .paths
                .iter()
                .any(|a| is_at_or_below(Path::new(path), a))
    }
}

/// A unit name: `^[A-Za-z0-9@._:-]{1,256}$`.
pub fn valid_unit(unit: &str) -> bool {
    (1..=MAX_UNIT).contains(&unit.len())
        && unit
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'@' | b'.' | b'_' | b':' | b'-'))
}

/// A query path: absolute, at most 4096 bytes, no NUL, and normalised: no `.` or `..`
/// component, no empty component (`//`) and no trailing `/` (except `/` itself).
pub fn valid_path(path: &str) -> bool {
    if path.is_empty() || path.len() > MAX_PATH || !path.starts_with('/') || path.contains('\0') {
        return false;
    }
    if path == "/" {
        return true;
    }
    path[1..]
        .split('/')
        .all(|c| !c.is_empty() && c != "." && c != "..")
}

/// `path` is `allowed` or below it, component-wise (`/var/lib2` is not below `/var/lib`).
fn is_at_or_below(path: &Path, allowed: &Path) -> bool {
    path.starts_with(allowed)
}

/// Resolves a lexically allowed path, following symlinks, and checks that it is still at or
/// below an allowed path (itself resolved). `Err(NotAllowed)` when a link leads out,
/// `Err(Unavailable)` when the path does not exist.
pub fn resolve_allowed(trust: &QueryTrust, path: &str) -> Result<PathBuf, QueryRefusal> {
    if !trust.allows_path(path) {
        return Err(QueryRefusal::NotAllowed);
    }
    let real = std::fs::canonicalize(path).map_err(|_| QueryRefusal::Unavailable)?;
    let inside = trust.paths.iter().any(|a| {
        std::fs::canonicalize(a).is_ok_and(|ra| is_at_or_below(&real, &ra))
            || is_at_or_below(&real, a)
    });
    if inside {
        Ok(real)
    } else {
        Err(QueryRefusal::NotAllowed)
    }
}

/// A query that passed [`check`]: its kind and its one checked parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checked {
    Facts,
    AppliedRevision,
    UnitState(String),
    DiskHealth,
    PathFree(String),
}

/// Checks a query against the trust file and its parameters, touching nothing on the host.
///
/// - No (valid) trust file, a kind the runner does not know or one the file does not list, and a
///   unit or path the file does not allow: `not_allowed`.
/// - A missing, extra or malformed parameter: `bad_params`.
pub fn check(
    trust: Option<&QueryTrust>,
    kind: &str,
    params: &BTreeMap<String, String>,
) -> Result<Checked, QueryRefusal> {
    let trust = trust.ok_or(QueryRefusal::NotAllowed)?;
    let kind = Kind::parse(kind).ok_or(QueryRefusal::NotAllowed)?;
    if !trust.allows_kind(kind) {
        return Err(QueryRefusal::NotAllowed);
    }
    let value = match kind.param() {
        None if params.is_empty() => None,
        Some(name) if params.len() == 1 => {
            Some(params.get(name).ok_or(QueryRefusal::BadParams)?.as_str())
        }
        _ => return Err(QueryRefusal::BadParams),
    };
    Ok(match (kind, value) {
        (Kind::Facts, None) => Checked::Facts,
        (Kind::AppliedRevision, None) => Checked::AppliedRevision,
        (Kind::DiskHealth, None) => Checked::DiskHealth,
        (Kind::UnitState, Some(unit)) => {
            if !valid_unit(unit) {
                return Err(QueryRefusal::BadParams);
            }
            if !trust.allows_unit(unit) {
                return Err(QueryRefusal::NotAllowed);
            }
            Checked::UnitState(unit.to_owned())
        }
        (Kind::PathFree, Some(path)) => {
            if !valid_path(path) {
                return Err(QueryRefusal::BadParams);
            }
            if !trust.allows_path(path) {
                return Err(QueryRefusal::NotAllowed);
            }
            Checked::PathFree(path.to_owned())
        }
        _ => return Err(QueryRefusal::BadParams),
    })
}

/// The `systemctl` properties `unit_state` reads.
pub const UNIT_PROPERTIES: &str = "--property=LoadState,ActiveState,SubState,ActiveEnterTimestamp";

/// `systemctl show --property=… -- <unit>`: an argv array, `--` before the unit, an empty
/// environment, no stdin. The unit must already have passed [`check`].
pub fn unit_state_command(systemctl: &Path, unit: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(systemctl);
    cmd.args([
        OsString::from("show"),
        OsString::from(UNIT_PROPERTIES),
        OsString::from("--"),
        OsString::from(unit),
    ])
    .env_clear()
    .env("LANG", "C")
    .env("SYSTEMD_PAGER", "")
    .env("SYSTEMD_COLORS", "0")
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    cmd
}

/// The `unit_state` answer from `systemctl show` output.
pub fn parse_unit_state(output: &str) -> Value {
    let mut props = BTreeMap::new();
    for line in output.lines() {
        if let Some((k, v)) = line.split_once('=') {
            props.insert(k, v);
        }
    }
    let get = |k: &str| props.get(k).map(|v| (*v).to_owned());
    json!({
        "load": get("LoadState"),
        "active": get("ActiveState"),
        "sub": get("SubState"),
        "since": get("ActiveEnterTimestamp").filter(|s| !s.is_empty()),
    })
}

/// One `/proc/mounts` line of a block device, for `disk_health`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MountHealth {
    pub mount: String,
    pub device: String,
    pub fs: String,
    pub read_only: bool,
    /// ext4's `errors_count` from `/sys/fs/ext4/<dev>/`; `None` where the file system has none.
    pub errors: Option<u64>,
}

/// The block-device mounts in `/proc/mounts` text. `sys_root` is `/sys` (a temp dir in tests).
pub fn parse_mounts(text: &str, sys_root: &Path) -> Vec<MountHealth> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut f = line.split_ascii_whitespace();
        let (Some(device), Some(mount), Some(fs), Some(opts)) =
            (f.next(), f.next(), f.next(), f.next())
        else {
            continue;
        };
        if !device.starts_with("/dev/") {
            continue;
        }
        let errors = if fs == "ext4" {
            ext4_errors(device, sys_root)
        } else {
            None
        };
        out.push(MountHealth {
            mount: mount.to_owned(),
            device: device.to_owned(),
            fs: fs.to_owned(),
            read_only: opts.split(',').any(|o| o == "ro"),
            errors,
        });
        if out.len() == 512 {
            break;
        }
    }
    out
}

/// A kernel device name: one path component of safe characters.
fn kernel_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn ext4_errors(device: &str, sys_root: &Path) -> Option<u64> {
    // /dev/mapper/x is a link to /dev/dm-N, which is the name under /sys/fs/ext4.
    let real = std::fs::canonicalize(device).unwrap_or_else(|_| PathBuf::from(device));
    let name = real.file_name()?.to_str()?;
    if !kernel_name(name) {
        return None;
    }
    let text = read_bounded(
        &sys_root.join("fs/ext4").join(name).join("errors_count"),
        64,
    )?;
    text.trim().parse().ok()
}

/// Reads at most `max` bytes of a fixed host file as UTF-8.
fn read_bounded(path: &Path, max: u64) -> Option<String> {
    let mut buf = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(max)
        .read_to_end(&mut buf)
        .ok()?;
    String::from_utf8(buf).ok()
}

/// `statvfs` on an already resolved path.
fn statvfs(path: &Path) -> Option<Value> {
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: `c` is a valid NUL-terminated string and `st` a properly sized out-parameter.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &raw mut st) } != 0 {
        return None;
    }
    #[allow(clippy::unnecessary_cast)] // the field types differ between targets
    let frsize = st.f_frsize as u64;
    #[allow(clippy::unnecessary_cast)]
    Some(json!({
        "total_bytes": (st.f_blocks as u64).saturating_mul(frsize),
        "free_bytes": (st.f_bfree as u64).saturating_mul(frsize),
        "avail_bytes": (st.f_bavail as u64).saturating_mul(frsize),
        "inodes_free": st.f_ffree as u64,
    }))
}

fn uname_field(f: impl Fn(&libc::utsname) -> &[libc::c_char]) -> Option<String> {
    // SAFETY: `uts` is a properly sized out-parameter; the fields are NUL-terminated.
    let mut uts: libc::utsname = unsafe { std::mem::zeroed() };
    if unsafe { libc::uname(&raw mut uts) } != 0 {
        return None;
    }
    let field = f(&uts);
    #[allow(clippy::cast_sign_loss)] // c_char is i8 on some targets
    let bytes: Vec<u8> = field
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8)
        .collect();
    String::from_utf8(bytes).ok()
}

fn os_pretty_name(os_release: &Path) -> Option<String> {
    let text = read_bounded(os_release, 16 * 1024)?;
    text.lines().find_map(|l| {
        l.strip_prefix("PRETTY_NAME=")
            .map(|v| v.trim_matches('"').to_owned())
    })
}

/// Bounds the number of queries in flight in one session.
#[derive(Clone)]
pub struct InFlight(Arc<Semaphore>);

impl Default for InFlight {
    fn default() -> Self {
        Self(Arc::new(Semaphore::new(MAX_QUERIES_IN_FLIGHT)))
    }
}

impl InFlight {
    /// A slot, or `None` when [`MAX_QUERIES_IN_FLIGHT`] are already taken (`busy`). The slot is
    /// freed when the permit drops.
    pub fn try_start(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.0).try_acquire_owned().ok()
    }
}

/// The query log: JSON lines `{ at, query_id, kind, params, asked_by, outcome }`, rotated at
/// [`QUERY_LOG_MAX`] keeping [`QUERY_LOG_KEEP`]. Never the answer.
#[derive(Debug, Clone)]
pub struct QueryLog {
    path: PathBuf,
    lock: Arc<Mutex<()>>,
}

#[derive(Serialize)]
struct LogLine<'a> {
    at: String,
    query_id: &'a str,
    kind: &'a str,
    params: BTreeMap<&'a str, &'a str>,
    asked_by: &'a str,
    outcome: &'a str,
}

/// Cuts a Keel-supplied string for the log, at a char boundary.
fn cut(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

impl QueryLog {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends one line; `outcome` is `answered` or the refusal reason. Failures are reported,
    /// never fatal. Keel-supplied strings are cut to bounded lengths.
    pub fn append(
        &self,
        query_id: &str,
        kind: &str,
        params: &BTreeMap<String, String>,
        asked_by: &str,
        outcome: &str,
    ) {
        let line = LogLine {
            at: rfc3339(super::unix_now()),
            query_id: cut(query_id, 64),
            kind: cut(kind, 32),
            params: params
                .iter()
                .take(4)
                .map(|(k, v)| (cut(k, 32), cut(v, MAX_PATH)))
                .collect(),
            asked_by: cut(asked_by, 256),
            outcome,
        };
        let Ok(mut text) = serde_json::to_string(&line) else {
            return;
        };
        text.push('\n');
        let _guard = self
            .lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Err(e) = self.rotate_if_full().and_then(|()| {
            OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(&self.path)?
                .write_all(text.as_bytes())
        }) {
            eprintln!("keel-runner: writing {}: {e}", self.path.display());
        }
    }

    fn rotated(&self, n: u32) -> PathBuf {
        let mut p = self.path.as_os_str().to_owned();
        p.push(format!(".{n}"));
        PathBuf::from(p)
    }

    fn rotate_if_full(&self) -> std::io::Result<()> {
        match std::fs::metadata(&self.path) {
            Ok(m) if m.len() >= QUERY_LOG_MAX => {}
            _ => return Ok(()),
        }
        for n in (1..QUERY_LOG_KEEP).rev() {
            let from = self.rotated(n);
            if from.exists() {
                std::fs::rename(&from, self.rotated(n + 1))?;
            }
        }
        std::fs::rename(&self.path, self.rotated(1))
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ` for Unix seconds.
pub fn rfc3339(unix: u64) -> String {
    let days = i64::try_from(unix / 86_400).unwrap_or(0);
    let secs = unix % 86_400;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

/// Where the answers come from. Defaults are the production layout.
#[derive(Debug, Clone)]
pub struct QueryConfig {
    /// `/etc/keel-runner/trust`, for `queries.toml`.
    pub trust_dir: PathBuf,
    /// The executor socket (`ReadLedger`, `ReadSmart`).
    pub executor_socket: PathBuf,
    /// The Ansible venv, for `facts.ansible_core`.
    pub venv: PathBuf,
    /// `systemctl`, by absolute path.
    pub systemctl: PathBuf,
    /// `/proc/mounts`.
    pub proc_mounts: PathBuf,
    /// `/sys`.
    pub sys_root: PathBuf,
    /// `/proc/uptime`.
    pub proc_uptime: PathBuf,
    /// `/etc/os-release`.
    pub os_release: PathBuf,
}

impl QueryConfig {
    pub fn new(trust_dir: PathBuf, executor_socket: PathBuf, venv: PathBuf) -> Self {
        let systemctl = ["/usr/bin/systemctl", "/bin/systemctl"]
            .into_iter()
            .map(PathBuf::from)
            .find(|p| p.exists())
            .unwrap_or_else(|| PathBuf::from("/usr/bin/systemctl"));
        Self {
            trust_dir,
            executor_socket,
            venv,
            systemctl,
            proc_mounts: PathBuf::from("/proc/mounts"),
            sys_root: PathBuf::from("/sys"),
            proc_uptime: PathBuf::from("/proc/uptime"),
            os_release: PathBuf::from("/etc/os-release"),
        }
    }
}

/// Answers queries for one runner.
#[derive(Debug, Clone)]
pub struct Answerer {
    cfg: QueryConfig,
    log: QueryLog,
}

impl Answerer {
    pub fn new(cfg: QueryConfig, log: QueryLog) -> Self {
        Self { cfg, log }
    }

    pub fn config(&self) -> &QueryConfig {
        &self.cfg
    }

    pub fn log(&self) -> &QueryLog {
        &self.log
    }

    /// Logs and returns a `busy` refusal, for a query over the in-flight limit.
    pub fn busy(&self, query: &Frame) -> Option<Frame> {
        let Frame::Query {
            query_id,
            kind,
            params,
            asked_by,
        } = query
        else {
            return None;
        };
        self.log.append(
            query_id,
            kind,
            params,
            asked_by,
            QueryRefusal::Busy.as_str(),
        );
        Some(Frame::QueryRefused {
            query_id: query_id.clone(),
            reason: QueryRefusal::Busy,
        })
    }

    /// Answers one `Query` frame with `QueryAnswer` or `QueryRefused`, and logs it. `None` for
    /// any other frame.
    pub async fn answer(&self, query: Frame) -> Option<Frame> {
        let Frame::Query {
            query_id,
            kind,
            params,
            asked_by,
        } = query
        else {
            return None;
        };
        let trust = QueryTrust::load(&self.cfg.trust_dir);
        let result = match check(trust.as_ref(), &kind, &params) {
            Ok(checked) => self.compute(trust.as_ref(), checked).await,
            Err(r) => Err(r),
        };
        let result = result.and_then(|answer| {
            if serde_json::to_vec(&answer).map_or(usize::MAX, |b| b.len()) > MAX_QUERY_ANSWER {
                Err(QueryRefusal::TooLarge)
            } else {
                Ok(answer)
            }
        });
        let outcome = match &result {
            Ok(_) => "answered",
            Err(r) => r.as_str(),
        };
        self.log
            .append(&query_id, &kind, &params, &asked_by, outcome);
        Some(match result {
            Ok(answer) => Frame::QueryAnswer { query_id, answer },
            Err(reason) => Frame::QueryRefused { query_id, reason },
        })
    }

    async fn compute(
        &self,
        trust: Option<&QueryTrust>,
        checked: Checked,
    ) -> Result<Value, QueryRefusal> {
        match checked {
            Checked::Facts => Ok(self.facts()),
            Checked::AppliedRevision => match self.ask_executor(Frame::ReadLedger).await {
                Some(Frame::Ledger { ledger_head, bases }) => Ok(json!({
                    "ledger_head": ledger_head,
                    "bases": bases.iter().map(|Base { job, commit }| json!({"job": job, "commit": commit})).collect::<Vec<_>>(),
                })),
                _ => Err(QueryRefusal::Unavailable),
            },
            Checked::UnitState(unit) => self.unit_state(&unit).await,
            Checked::DiskHealth => Ok(self.disk_health().await),
            Checked::PathFree(path) => {
                let trust = trust.ok_or(QueryRefusal::NotAllowed)?;
                let real = resolve_allowed(trust, &path)?;
                statvfs(&real).ok_or(QueryRefusal::Unavailable)
            }
        }
    }

    /// Exposed for tests: `facts` from fixed host files and the runner's own build.
    pub fn facts(&self) -> Value {
        let uptime = read_bounded(&self.cfg.proc_uptime, 128).and_then(|t| {
            t.split_ascii_whitespace()
                .next()
                .and_then(|s| s.parse::<f64>().ok())
        });
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let uptime_s = uptime.map(|u| u as u64);
        json!({
            "hostname": uname_field(|u| &u.nodename[..]),
            "os": os_pretty_name(&self.cfg.os_release),
            "kernel": uname_field(|u| &u.release[..]),
            "arch": std::env::consts::ARCH,
            "uptime_s": uptime_s,
            "runner_version": env!("CARGO_PKG_VERSION"),
            "ansible_core": crate::venv::ansible_core_version(&self.cfg.venv),
        })
    }

    async fn unit_state(&self, unit: &str) -> Result<Value, QueryRefusal> {
        let mut cmd = tokio::process::Command::from(unit_state_command(&self.cfg.systemctl, unit));
        cmd.kill_on_drop(true);
        let child = cmd.spawn().map_err(|_| QueryRefusal::Unavailable)?;
        let out = tokio::time::timeout(STEP_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| QueryRefusal::Unavailable)?
            .map_err(|_| QueryRefusal::Unavailable)?;
        if !out.status.success() || out.stdout.len() as u64 > MAX_READ {
            return Err(QueryRefusal::Unavailable);
        }
        Ok(parse_unit_state(&String::from_utf8_lossy(&out.stdout)))
    }

    async fn disk_health(&self) -> Value {
        let mounts = read_bounded(&self.cfg.proc_mounts, MAX_READ)
            .map(|t| parse_mounts(&t, &self.cfg.sys_root))
            .unwrap_or_default();
        let (installed, disks) = match self.ask_executor(Frame::ReadSmart).await {
            Some(Frame::Smart { installed, disks }) => (Some(installed), disks),
            _ => (None, Vec::new()),
        };
        json!({
            "mounts": mounts,
            "smartctl": installed,
            "disks": disks.iter().map(|SmartDisk { name, smart_passed }| json!({"name": name, "smart_passed": smart_passed})).collect::<Vec<_>>(),
        })
    }

    /// One request on its own executor connection, as `ReadLedger` is asked for `Hello`.
    async fn ask_executor(&self, request: Frame) -> Option<Frame> {
        let fut = async {
            let mut sock = UnixStream::connect(&self.cfg.executor_socket).await.ok()?;
            write_frame(&mut sock, &request).await.ok()?;
            read_frame(&mut sock).await.ok().flatten()
        };
        // Inside Keel's 15 s query timeout; the executor bounds its smartctl calls below this.
        tokio::time::timeout(STEP_TIMEOUT, fut).await.ok().flatten()
    }
}
