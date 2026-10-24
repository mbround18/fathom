//! Container containment: how far a process inside a container is from the host, and who may
//! open a shell in it.
//!
//! [`profile`] reads the fields of `docker inspect` that decide containment ([`Inspect`]) and the
//! daemon's mode ([`DaemonMode`], from `docker info`'s `SecurityOptions`) and returns a
//! [`Profile`]: a [`Level`] and the [`Reason`]s behind it, each with a code and one human
//! sentence.
//!
//! - [`Level::Escape`]: the container can reach the host (privileged, the runtime socket, a
//!   sensitive writable bind, a capability that breaks out, the host's pid/ipc/user namespace,
//!   devices, or no seccomp/AppArmor while running as root). A shell there is a shell on the
//!   host.
//! - [`Level::Root`]: uid 0 on a rootful daemon, and nothing worse.
//! - [`Level::Ok`]: neither. Root on a rootless or userns-remapped daemon lands here, with an
//!   informational reason: uid 0 inside maps to an unprivileged user outside.
//!
//! [`shell_rule`] turns a level and the [`ShellPolicy`] into who may open a shell. No policy lets
//! anyone but an admin into an escape container.
//!
//! Everything here is a pure function; the adapter runs `docker inspect` and `docker info`.

use std::fmt;

use serde::{Deserialize, Deserializer};

/// How contained a container is. Ordered: `Ok < Root < Escape`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Level {
    #[default]
    Ok,
    Root,
    Escape,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Root => "root",
            Self::Escape => "escape",
        }
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How much one reason weighs. `Info` never raises the level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Info,
    Root,
    Escape,
}

impl Severity {
    fn level(self) -> Level {
        match self {
            Self::Info => Level::Ok,
            Self::Root => Level::Root,
            Self::Escape => Level::Escape,
        }
    }
}

/// What was found. Machine-readable; the wire spells these in snake case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Code {
    /// Runs as uid 0.
    RootUser,
    /// Runs as uid 0, but the daemon maps it to an unprivileged host user.
    RootRemapped,
    Privileged,
    /// The docker or containerd socket is mounted.
    RuntimeSocket,
    /// The host's `/` is mounted.
    HostRootMount,
    /// A sensitive host path is mounted writable.
    SensitiveMount,
    /// A sensitive host path is mounted read-only.
    SensitiveMountReadOnly,
    /// A capability that breaks containment.
    DangerousCapability,
    /// An added capability that widens the container but does not break out on its own.
    AddedCapability,
    HostPid,
    HostIpc,
    HostUserns,
    HostNetwork,
    HostUts,
    /// seccomp, AppArmor or the masked system paths are off.
    Unconfined,
    Devices,
    /// A compose service with no `user:`: it runs as the image's user, often root.
    NoUser,
    /// A setting whose value comes from a variable, so it cannot be judged from the file.
    Interpolated,
}

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RootUser => "root_user",
            Self::RootRemapped => "root_remapped",
            Self::Privileged => "privileged",
            Self::RuntimeSocket => "runtime_socket",
            Self::HostRootMount => "host_root_mount",
            Self::SensitiveMount => "sensitive_mount",
            Self::SensitiveMountReadOnly => "sensitive_mount_read_only",
            Self::DangerousCapability => "dangerous_capability",
            Self::AddedCapability => "added_capability",
            Self::HostPid => "host_pid",
            Self::HostIpc => "host_ipc",
            Self::HostUserns => "host_userns",
            Self::HostNetwork => "host_network",
            Self::HostUts => "host_uts",
            Self::Unconfined => "unconfined",
            Self::Devices => "devices",
            Self::NoUser => "no_user",
            Self::Interpolated => "interpolated",
        }
    }
}

/// One finding: its code, how much it weighs, and one sentence for a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason {
    pub code: Code,
    pub severity: Severity,
    pub message: String,
}

impl Reason {
    fn new(code: Code, severity: Severity, message: impl Into<String>) -> Self {
        Self {
            code,
            severity,
            message: message.into(),
        }
    }
}

/// A container's containment: the worst severity found, and every reason, worst first.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Profile {
    pub level: Level,
    pub reasons: Vec<Reason>,
}

impl Profile {
    fn from_reasons(mut reasons: Vec<Reason>) -> Self {
        reasons.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.code.cmp(&b.code)));
        reasons.dedup();
        let level = reasons
            .iter()
            .map(|r| r.severity.level())
            .max()
            .unwrap_or(Level::Ok);
        Self { level, reasons }
    }

    /// The reasons that decide the level (the informational ones left out).
    pub fn deciding(&self) -> impl Iterator<Item = &Reason> {
        self.reasons
            .iter()
            .filter(move |r| r.severity.level() == self.level && self.level != Level::Ok)
    }
}

/// The worst of several levels; `Ok` for none.
pub fn rollup(levels: impl IntoIterator<Item = Level>) -> Level {
    levels.into_iter().max().unwrap_or(Level::Ok)
}

/// How the daemon runs, from `docker info`'s `SecurityOptions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DaemonMode {
    /// The daemon runs as an unprivileged user: uid 0 in a container is that user outside.
    Rootless,
    /// `userns-remap` is on: uid 0 in a container maps to a subordinate uid outside.
    Userns,
    /// The daemon runs as root and does not remap: uid 0 inside is uid 0 outside.
    /// Also what an unreadable or unknown daemon counts as.
    #[default]
    Rootful,
}

impl DaemonMode {
    /// Reads `SecurityOptions` (`["name=seccomp,profile=builtin", "name=rootless", ...]`).
    /// Anything else, or nothing, is [`DaemonMode::Rootful`]: the safe assumption.
    pub fn from_security_options<S: AsRef<str>>(options: &[S]) -> Self {
        let names: Vec<String> = options
            .iter()
            .filter_map(|o| {
                o.as_ref().split(',').find_map(|kv| {
                    let (k, v) = kv.split_once('=')?;
                    (k.trim().eq_ignore_ascii_case("name")).then(|| v.trim().to_ascii_lowercase())
                })
            })
            .collect();
        if names.iter().any(|n| n == "rootless") {
            Self::Rootless
        } else if names.iter().any(|n| n == "userns") {
            Self::Userns
        } else {
            Self::Rootful
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rootless => "rootless",
            Self::Userns => "userns",
            Self::Rootful => "rootful",
        }
    }
}

// ---------------------------------------------------------------------------------------------
// `docker inspect`

/// `null` and a missing field both read as the default: docker writes `"CapAdd": null`.
fn nullable<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// The fields of one `docker inspect` object that decide containment. Everything else is
/// ignored; anything missing or `null` is its default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Inspect {
    #[serde(default, deserialize_with = "nullable")]
    pub id: String,
    #[serde(default, deserialize_with = "nullable")]
    pub config: InspectConfig,
    #[serde(default, deserialize_with = "nullable")]
    pub host_config: HostConfig,
    #[serde(default, deserialize_with = "nullable")]
    pub mounts: Vec<Mount>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct InspectConfig {
    #[serde(default, deserialize_with = "nullable")]
    pub user: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct HostConfig {
    #[serde(default, deserialize_with = "nullable")]
    pub privileged: bool,
    #[serde(default, deserialize_with = "nullable")]
    pub cap_add: Vec<String>,
    #[serde(default, deserialize_with = "nullable")]
    pub cap_drop: Vec<String>,
    #[serde(default, deserialize_with = "nullable")]
    pub pid_mode: String,
    #[serde(default, deserialize_with = "nullable")]
    pub network_mode: String,
    #[serde(default, deserialize_with = "nullable")]
    pub ipc_mode: String,
    #[serde(default, rename = "UTSMode", deserialize_with = "nullable")]
    pub uts_mode: String,
    #[serde(default, deserialize_with = "nullable")]
    pub userns_mode: String,
    #[serde(default, deserialize_with = "nullable")]
    pub devices: Vec<Device>,
    #[serde(default, deserialize_with = "nullable")]
    pub security_opt: Vec<String>,
    #[serde(default, deserialize_with = "nullable")]
    pub readonly_rootfs: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Device {
    #[serde(default, deserialize_with = "nullable")]
    pub path_on_host: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Mount {
    /// `bind`, `volume`, `tmpfs`, ... Empty is read as a bind (the cautious reading).
    #[serde(default, deserialize_with = "nullable")]
    pub r#type: String,
    #[serde(default, deserialize_with = "nullable")]
    pub source: String,
    #[serde(default, deserialize_with = "nullable")]
    pub destination: String,
    #[serde(default, rename = "RW", deserialize_with = "nullable")]
    pub rw: bool,
}

// ---------------------------------------------------------------------------------------------
// Shared judgements (inspect and compose lint)

/// Capabilities that break containment on their own.
const ESCAPE_CAPS: &[&str] = &[
    "SYS_ADMIN",
    "SYS_MODULE",
    "SYS_RAWIO",
    "DAC_READ_SEARCH",
    "BPF",
];

/// `cap_sys_admin`, `CAP_SYS_ADMIN` and ` SYS_ADMIN ` are the same capability.
pub fn normalise_cap(cap: &str) -> String {
    let c = cap.trim().to_ascii_uppercase();
    c.strip_prefix("CAP_").map(str::to_owned).unwrap_or(c)
}

/// Whether a `user` value (`docker inspect`'s `Config.User`, compose's `user:`) is uid 0.
/// Empty is root: the image's default, which is root unless the image says otherwise, and
/// `docker inspect` shows the image's `USER` when it set one.
pub fn is_root_user(user: &str) -> bool {
    let u = user.trim();
    let name = u.split(':').next().unwrap_or("").trim();
    if name.is_empty() {
        return true;
    }
    if name.eq_ignore_ascii_case("root") {
        return true;
    }
    // `0`, `00`, `+0`: a number equal to zero. A name that is not a number is someone else.
    let digits = name.strip_prefix('+').unwrap_or(name);
    !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && digits.bytes().all(|b| b == b'0')
}

fn is_host(mode: &str) -> bool {
    mode.trim().eq_ignore_ascii_case("host")
}

/// One `security_opt` entry turned off a confinement (`seccomp=unconfined`,
/// `apparmor:unconfined`, `systempaths=unconfined`).
pub fn unconfined_opt(opt: &str) -> Option<String> {
    let (k, v) = opt.split_once(['=', ':'])?;
    let k = k.trim().to_ascii_lowercase();
    let v = v.trim().to_ascii_lowercase();
    (v == "unconfined" && matches!(k.as_str(), "seccomp" | "apparmor" | "systempaths")).then_some(k)
}

/// Lexically normalised absolute path: `//`, `.`, `..` and a trailing `/` resolved. `None` for a
/// relative path (a compose file's `./data`), which is under the stack, not the host.
pub fn normalise_host_path(p: &str) -> Option<String> {
    let p = p.trim();
    if !p.starts_with('/') {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    Some(format!("/{}", parts.join("/")))
}

/// A host path whose bind breaks containment when writable. `deep` paths are sensitive at any
/// depth (`/var/lib/docker/containers`); the others only themselves or an ancestor (`/etc`, but
/// not `/etc/localtime`).
const SENSITIVE: &[(&str, bool)] = &[
    ("/proc", true),
    ("/sys", true),
    ("/dev", true),
    ("/boot", true),
    ("/root", true),
    ("/var/lib/docker", true),
    ("/var/lib/containerd", true),
    ("/lib/modules", true),
    ("/usr/lib/modules", true),
    ("/etc", false),
    ("/home", false),
    ("/run", false),
    ("/var/run", false),
];

/// Where docker's and containerd's sockets live, rootless ones (`/run/user/<uid>`) included.
const SOCKET_DIRS: &[&str] = &[
    "/run/docker",
    "/var/run/docker",
    "/run/containerd",
    "/var/run/containerd",
    "/run/user",
    "/var/run/user",
];

/// What a host path is, for a bind of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostPath {
    /// `/` itself.
    Root,
    /// The docker or containerd socket, or a directory holding it.
    RuntimeSocket,
    /// One of [`SENSITIVE`], named.
    Sensitive(&'static str),
    Other,
}

fn under(path: &str, dir: &str) -> bool {
    path == dir || path.starts_with(&format!("{dir}/"))
}

/// Classifies a host path (already normalised).
pub fn classify_host_path(path: &str) -> HostPath {
    if path == "/" {
        return HostPath::Root;
    }
    let file = path.rsplit('/').next().unwrap_or("");
    if file.ends_with("docker.sock") || file.ends_with("containerd.sock") {
        return HostPath::RuntimeSocket;
    }
    // A directory that holds a socket, or any ancestor of one (`/run`, `/var`): a socket is
    // reachable through a read-only bind too, since connecting does not write the file.
    for dir in SOCKET_DIRS {
        if under(path, dir) || under(dir, path) {
            return HostPath::RuntimeSocket;
        }
    }
    for (dir, deep) in SENSITIVE {
        // An ancestor of a sensitive path (`/var`, `/usr`, `/lib`) exposes it.
        if dir.starts_with(&format!("{path}/")) || path == *dir || (*deep && under(path, dir)) {
            return HostPath::Sensitive(dir);
        }
    }
    HostPath::Other
}

/// The reason for a bind of `source` (writable or not), if it is one.
pub fn bind_reason(source: &str, writable: bool) -> Option<Reason> {
    let path = normalise_host_path(source)?;
    match classify_host_path(&path) {
        HostPath::Root => Some(Reason::new(
            Code::HostRootMount,
            Severity::Escape,
            format!(
                "mounts the host's whole filesystem ({}); it can read or change anything on the host",
                if writable { "writable" } else { "read-only" }
            ),
        )),
        HostPath::RuntimeSocket => Some(Reason::new(
            Code::RuntimeSocket,
            Severity::Escape,
            format!(
                "mounts the container runtime socket ({path}); it can start a privileged container on the host"
            ),
        )),
        HostPath::Sensitive(dir) if writable => Some(Reason::new(
            Code::SensitiveMount,
            Severity::Escape,
            format!(
                "mounts {path} from the host writable; changes to {dir} can take over the host"
            ),
        )),
        HostPath::Sensitive(_) => Some(Reason::new(
            Code::SensitiveMountReadOnly,
            Severity::Info,
            format!("mounts {path} from the host read-only; it can read host state"),
        )),
        HostPath::Other => None,
    }
}

/// Reasons for a list of added capabilities, given whether the pid namespace is the host's.
pub fn cap_reasons(caps: &[String], host_pid: bool) -> Vec<Reason> {
    let mut out = Vec::new();
    for raw in caps {
        let cap = normalise_cap(raw);
        if cap.is_empty() {
            continue;
        }
        if cap == "ALL" {
            out.push(Reason::new(
                Code::DangerousCapability,
                Severity::Escape,
                "adds every capability (cap_add ALL); that is as good as privileged",
            ));
        } else if ESCAPE_CAPS.contains(&cap.as_str()) || (cap == "SYS_PTRACE" && host_pid) {
            out.push(Reason::new(
                Code::DangerousCapability,
                Severity::Escape,
                if cap == "SYS_PTRACE" {
                    "adds CAP_SYS_PTRACE in the host's pid namespace; it can take over host processes"
                        .to_owned()
                } else {
                    format!("adds CAP_{cap}, which can break out of the container")
                },
            ));
        } else {
            out.push(Reason::new(
                Code::AddedCapability,
                Severity::Info,
                format!("adds CAP_{cap}"),
            ));
        }
    }
    out
}

fn root_reason(daemon: DaemonMode, userns_host: bool) -> Reason {
    match daemon {
        DaemonMode::Rootless => Reason::new(
            Code::RootRemapped,
            Severity::Info,
            "runs as root inside, which the rootless daemon maps to an unprivileged host user",
        ),
        DaemonMode::Userns if !userns_host => Reason::new(
            Code::RootRemapped,
            Severity::Info,
            "runs as root inside, which userns-remap maps to an unprivileged host user",
        ),
        _ => Reason::new(
            Code::RootUser,
            Severity::Root,
            "runs as root (uid 0), which is root on the host if it gets out",
        ),
    }
}

/// The containment of one container.
pub fn profile(inspect: &Inspect, daemon: DaemonMode) -> Profile {
    let hc = &inspect.host_config;
    let mut r = Vec::new();
    let root = is_root_user(&inspect.config.user);
    let host_pid = is_host(&hc.pid_mode);
    let userns_host = is_host(&hc.userns_mode);

    if root {
        r.push(root_reason(daemon, userns_host));
    }
    if hc.privileged {
        r.push(Reason::new(
            Code::Privileged,
            Severity::Escape,
            "runs privileged: every device and capability, no confinement; root in it is root on the host",
        ));
    }
    for m in &inspect.mounts {
        let ty = m.r#type.trim();
        if !(ty.is_empty() || ty.eq_ignore_ascii_case("bind")) {
            continue;
        }
        r.extend(bind_reason(&m.source, m.rw));
    }
    r.extend(cap_reasons(&hc.cap_add, host_pid));
    if host_pid {
        r.push(Reason::new(
            Code::HostPid,
            Severity::Escape,
            "shares the host's process namespace; it can see and signal every host process",
        ));
    }
    if is_host(&hc.ipc_mode) {
        r.push(Reason::new(
            Code::HostIpc,
            Severity::Escape,
            "shares the host's IPC namespace (shared memory of host processes)",
        ));
    }
    if userns_host && daemon != DaemonMode::Rootful {
        r.push(Reason::new(
            Code::HostUserns,
            Severity::Escape,
            "opts out of user-namespace remapping; its root is the host's root",
        ));
    } else if userns_host {
        r.push(Reason::new(
            Code::HostUserns,
            Severity::Escape,
            "uses the host's user namespace",
        ));
    }
    if is_host(&hc.network_mode) {
        r.push(Reason::new(
            Code::HostNetwork,
            Severity::Info,
            "uses the host's network; it can reach services bound to the host's loopback",
        ));
    }
    if is_host(&hc.uts_mode) {
        r.push(Reason::new(
            Code::HostUts,
            Severity::Info,
            "shares the host's hostname namespace",
        ));
    }
    for opt in &hc.security_opt {
        if let Some(which) = unconfined_opt(opt) {
            r.push(unconfined_reason(&which, root));
        }
    }
    let devices: Vec<&str> = hc
        .devices
        .iter()
        .map(|d| d.path_on_host.trim())
        .filter(|p| !p.is_empty())
        .collect();
    if !devices.is_empty() || hc.devices.len() > devices.len() {
        r.push(devices_reason(&devices));
    }
    Profile::from_reasons(r)
}

fn unconfined_reason(which: &str, root: bool) -> Reason {
    let what = match which {
        "seccomp" => "seccomp",
        "apparmor" => "AppArmor",
        _ => "the masked /proc and /sys paths",
    };
    if root {
        Reason::new(
            Code::Unconfined,
            Severity::Escape,
            format!("turns {what} off while running as root"),
        )
    } else {
        Reason::new(
            Code::Unconfined,
            Severity::Info,
            format!("turns {what} off"),
        )
    }
}

fn devices_reason(devices: &[&str]) -> Reason {
    Reason::new(
        Code::Devices,
        Severity::Escape,
        if devices.is_empty() {
            "has host devices passed in".to_owned()
        } else {
            format!("has host devices passed in ({})", devices.join(", "))
        },
    )
}

// ---------------------------------------------------------------------------------------------
// Shell policy

/// A shell path a session may run: absolute, at most 256 bytes, and only `[A-Za-z0-9/._-]`
/// with no `..` segment. No spaces, quotes or shell metacharacters, so it can never be read as
/// anything but one program path.
pub fn valid_shell_path(p: &str) -> bool {
    p.len() <= 256
        && p.starts_with('/')
        && p.len() > 1
        && p.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-'))
        && !p.split('/').any(|s| s == "..")
        && !p.ends_with('/')
}

/// A container id or name that is safe as one argv entry: never empty, never an option
/// (`-...`), only `[A-Za-z0-9_.-]`, at most 256 bytes.
pub fn valid_container_arg(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && !id.starts_with('-')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

/// `DEPTHS_SHELL_POLICY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShellPolicy {
    /// Escape: nobody. Root on a rootful daemon: admins. Otherwise per role.
    #[default]
    Strict,
    /// Escape: admins. Otherwise per role.
    Relaxed,
}

impl ShellPolicy {
    pub fn parse(v: &str) -> Option<Self> {
        match v.trim().to_ascii_lowercase().as_str() {
            "strict" => Some(Self::Strict),
            "relaxed" => Some(Self::Relaxed),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Relaxed => "relaxed",
        }
    }
}

/// Who may open a shell in a container, past the per-stack role check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellRule {
    /// Whoever the role table lets in.
    PerRole,
    AdminOnly,
    /// Nobody.
    Refused,
}

/// The policy matrix. A level of `None` (not known: the inspect failed) is refused: fail closed.
pub fn shell_rule(policy: ShellPolicy, level: Option<Level>) -> ShellRule {
    match (policy, level) {
        (_, None) => ShellRule::Refused,
        (ShellPolicy::Strict, Some(Level::Escape)) => ShellRule::Refused,
        (ShellPolicy::Strict, Some(Level::Root)) => ShellRule::AdminOnly,
        (ShellPolicy::Relaxed, Some(Level::Escape)) => ShellRule::AdminOnly,
        (_, Some(Level::Root | Level::Ok)) => ShellRule::PerRole,
    }
}

/// Whether someone who passed the role check may open the shell.
pub fn shell_permitted(rule: ShellRule, admin: bool) -> bool {
    match rule {
        ShellRule::PerRole => true,
        ShellRule::AdminOnly => admin,
        ShellRule::Refused => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inspect(json: &str) -> Inspect {
        serde_json::from_str(json).expect("fixture parses")
    }

    fn codes(p: &Profile) -> Vec<Code> {
        p.reasons.iter().map(|r| r.code).collect()
    }

    const PLAIN: &str = r#"{
        "Id": "abc",
        "Config": {"User": "1000:1000", "Image": "nginx"},
        "HostConfig": {"Privileged": false, "CapAdd": null, "CapDrop": ["ALL"], "PidMode": "",
            "NetworkMode": "bridge", "IpcMode": "private", "UTSMode": "", "UsernsMode": "",
            "Devices": [], "SecurityOpt": ["no-new-privileges:true"], "ReadonlyRootfs": true},
        "Mounts": [
            {"Type": "volume", "Source": "/var/lib/docker/volumes/data/_data", "Destination": "/data", "RW": true},
            {"Type": "bind", "Source": "/etc/localtime", "Destination": "/etc/localtime", "RW": false},
            {"Type": "bind", "Source": "/opt/stacks/web/conf", "Destination": "/conf", "RW": true}
        ]
    }"#;

    #[test]
    fn a_non_root_container_with_ordinary_mounts_is_ok() {
        let p = profile(&inspect(PLAIN), DaemonMode::Rootful);
        assert_eq!(p.level, Level::Ok);
        assert!(p.reasons.is_empty(), "{:?}", p.reasons);
    }

    #[test]
    fn root_users_are_spelled_many_ways() {
        for u in [
            "",
            " ",
            "0",
            "root",
            "ROOT",
            "0:0",
            "root:wheel",
            "0:1000",
            "00",
            "+0",
        ] {
            assert!(is_root_user(u), "{u:?} is root");
        }
        for u in [
            "1000", "1000:0", "nobody", "node", "10", "rooty", "-0", "0x0", "1",
        ] {
            assert!(!is_root_user(u), "{u:?} is not root");
        }
    }

    #[test]
    fn root_on_a_rootful_daemon_is_root_and_lowered_on_rootless_or_userns() {
        let i = inspect(r#"{"Id":"x","Config":{"User":""}}"#);
        let p = profile(&i, DaemonMode::Rootful);
        assert_eq!(p.level, Level::Root);
        assert_eq!(codes(&p), vec![Code::RootUser]);
        for d in [DaemonMode::Rootless, DaemonMode::Userns] {
            let p = profile(&i, d);
            assert_eq!(p.level, Level::Ok, "{d:?}");
            assert_eq!(codes(&p), vec![Code::RootRemapped]);
        }
    }

    #[test]
    fn opting_out_of_userns_remap_is_an_escape_and_root_stays_root() {
        let i = inspect(r#"{"Config":{"User":"0"},"HostConfig":{"UsernsMode":"host"}}"#);
        let p = profile(&i, DaemonMode::Userns);
        assert_eq!(p.level, Level::Escape);
        assert!(codes(&p).contains(&Code::RootUser));
        assert!(codes(&p).contains(&Code::HostUserns));
    }

    #[test]
    fn privileged_is_an_escape_even_as_a_non_root_user_or_rootless() {
        let i = inspect(r#"{"Config":{"User":"1000"},"HostConfig":{"Privileged":true}}"#);
        for d in [DaemonMode::Rootful, DaemonMode::Rootless] {
            let p = profile(&i, d);
            assert_eq!(p.level, Level::Escape);
            assert_eq!(codes(&p), vec![Code::Privileged]);
        }
    }

    #[test]
    fn runtime_sockets_are_escapes_read_only_or_not() {
        for src in [
            "/var/run/docker.sock",
            "/run/docker.sock",
            "/run/user/1000/docker.sock",
            "/run/containerd/containerd.sock",
            "/var/run/docker",
            "/run/containerd",
            "/run/user/1000",
            "/run",
            "/var/run",
            "/var",
            "//var//run/./docker.sock",
            "/tmp/../var/run/docker.sock",
        ] {
            let i = Inspect {
                config: InspectConfig {
                    user: "1000".into(),
                },
                mounts: vec![Mount {
                    r#type: "bind".into(),
                    source: src.into(),
                    destination: "/x".into(),
                    rw: false,
                }],
                ..Inspect::default()
            };
            let p = profile(&i, DaemonMode::Rootless);
            assert_eq!(p.level, Level::Escape, "{src}");
            assert_eq!(codes(&p), vec![Code::RuntimeSocket], "{src}");
        }
    }

    fn bind(src: &str, rw: bool) -> Profile {
        let i = Inspect {
            config: InspectConfig {
                user: "1000".into(),
            },
            mounts: vec![Mount {
                r#type: "bind".into(),
                source: src.into(),
                destination: "/mnt".into(),
                rw,
            }],
            ..Inspect::default()
        };
        profile(&i, DaemonMode::Rootful)
    }

    #[test]
    fn sensitive_writable_binds_are_escapes_read_only_ones_are_informational() {
        for src in [
            "/proc",
            "/sys",
            "/sys/fs/cgroup",
            "/dev",
            "/etc",
            "/etc/",
            "/root",
            "/root/.ssh",
            "/home",
            "/boot",
            "/var/lib/docker",
            "/var/lib/docker/containers",
            "/lib/modules",
            "/var/lib",
            "/usr",
            "/lib",
        ] {
            assert_eq!(bind(src, true).level, Level::Escape, "{src} writable");
            let ro = bind(src, false);
            assert_eq!(ro.level, Level::Ok, "{src} read-only");
            assert_eq!(codes(&ro), vec![Code::SensitiveMountReadOnly], "{src}");
        }
    }

    #[test]
    fn the_host_root_is_an_escape_even_read_only() {
        for src in ["/", "//", "/.", "/etc/.."] {
            let p = bind(src, false);
            assert_eq!(p.level, Level::Escape, "{src}");
            assert_eq!(codes(&p), vec![Code::HostRootMount]);
        }
    }

    #[test]
    fn ordinary_binds_are_not_flagged() {
        for src in [
            "/etc/localtime",
            "/etc/letsencrypt",
            "/home/alice/media",
            "/opt/stacks/web",
            "/srv/data",
            "/run/secrets-elsewhere",
            "relative/path",
        ] {
            assert!(bind(src, true).reasons.is_empty(), "{src}");
        }
    }

    #[test]
    fn volumes_and_tmpfs_are_not_host_binds_but_an_untyped_mount_is_read_cautiously() {
        let mut i = inspect(
            r#"{"Config":{"User":"1"},"Mounts":[
            {"Type":"volume","Source":"/var/lib/docker/volumes/x/_data","RW":true},
            {"Type":"tmpfs","Source":"","Destination":"/tmp","RW":true}]}"#,
        );
        assert_eq!(profile(&i, DaemonMode::Rootful).level, Level::Ok);
        i.mounts[0].r#type = String::new();
        assert_eq!(profile(&i, DaemonMode::Rootful).level, Level::Escape);
    }

    #[test]
    fn dangerous_capabilities_in_any_spelling() {
        for cap in [
            "SYS_ADMIN",
            "CAP_SYS_ADMIN",
            "cap_sys_module",
            " sys_rawio ",
            "DAC_READ_SEARCH",
            "bpf",
            "ALL",
            "all",
        ] {
            let i = Inspect {
                config: InspectConfig {
                    user: "1000".into(),
                },
                host_config: HostConfig {
                    cap_add: vec![cap.into()],
                    ..HostConfig::default()
                },
                ..Inspect::default()
            };
            let p = profile(&i, DaemonMode::Rootful);
            assert_eq!(p.level, Level::Escape, "{cap}");
            assert_eq!(codes(&p), vec![Code::DangerousCapability]);
        }
    }

    #[test]
    fn ptrace_is_an_escape_only_with_the_host_pid_namespace() {
        let mut i = Inspect {
            config: InspectConfig {
                user: "1000".into(),
            },
            host_config: HostConfig {
                cap_add: vec!["SYS_PTRACE".into(), "NET_ADMIN".into()],
                ..HostConfig::default()
            },
            ..Inspect::default()
        };
        let p = profile(&i, DaemonMode::Rootful);
        assert_eq!(p.level, Level::Ok);
        assert_eq!(
            codes(&p),
            vec![Code::AddedCapability, Code::AddedCapability]
        );
        i.host_config.pid_mode = "host".into();
        let p = profile(&i, DaemonMode::Rootful);
        assert_eq!(p.level, Level::Escape);
        assert!(codes(&p).contains(&Code::DangerousCapability));
        assert!(codes(&p).contains(&Code::HostPid));
    }

    #[test]
    fn host_namespaces() {
        let i = inspect(
            r#"{"Config":{"User":"1000"},"HostConfig":{"PidMode":"host","IpcMode":"HOST","NetworkMode":"host","UTSMode":"host"}}"#,
        );
        let p = profile(&i, DaemonMode::Rootful);
        assert_eq!(p.level, Level::Escape);
        assert_eq!(
            codes(&p),
            vec![
                Code::HostPid,
                Code::HostIpc,
                Code::HostNetwork,
                Code::HostUts
            ]
        );
        // `container:<id>` shares another container's namespace, not the host's.
        let i = inspect(
            r#"{"Config":{"User":"1000"},"HostConfig":{"PidMode":"container:abc","NetworkMode":"container:abc"}}"#,
        );
        assert!(profile(&i, DaemonMode::Rootful).reasons.is_empty());
    }

    #[test]
    fn unconfined_is_an_escape_only_with_root() {
        for opt in [
            "seccomp=unconfined",
            "seccomp:unconfined",
            "apparmor=unconfined",
            "systempaths=unconfined",
            "APPARMOR:Unconfined",
        ] {
            let mut i = Inspect {
                config: InspectConfig {
                    user: "1000".into(),
                },
                host_config: HostConfig {
                    security_opt: vec![opt.into()],
                    ..HostConfig::default()
                },
                ..Inspect::default()
            };
            let p = profile(&i, DaemonMode::Rootful);
            assert_eq!(p.level, Level::Ok, "{opt}");
            assert_eq!(codes(&p), vec![Code::Unconfined]);
            i.config.user = "root".into();
            assert_eq!(
                profile(&i, DaemonMode::Rootful).level,
                Level::Escape,
                "{opt}"
            );
        }
        for opt in [
            "no-new-privileges",
            "label=disable",
            "seccomp=/profile.json",
            "apparmor=docker-default",
        ] {
            assert!(unconfined_opt(opt).is_none(), "{opt}");
        }
    }

    #[test]
    fn devices_are_escapes() {
        let i = inspect(
            r#"{"Config":{"User":"1000"},"HostConfig":{"Devices":[{"PathOnHost":"/dev/dri","PathInContainer":"/dev/dri","CgroupPermissions":"rwm"}]}}"#,
        );
        let p = profile(&i, DaemonMode::Rootless);
        assert_eq!(p.level, Level::Escape);
        assert!(p.reasons[0].message.contains("/dev/dri"));
    }

    #[test]
    fn hostile_and_odd_inspect_json_parses_to_defaults_not_errors() {
        for json in [
            "{}",
            r#"{"Id":null,"Config":null,"HostConfig":null,"Mounts":null}"#,
            r#"{"HostConfig":{"CapAdd":null,"Devices":null,"SecurityOpt":null,"Privileged":null}}"#,
            r#"{"Unknown":{"deep":[1,2,3]},"Config":{"User":"1000","Extra":true}}"#,
            r#"{"Mounts":[{}]}"#,
        ] {
            let i: Inspect = serde_json::from_str(json).unwrap_or_else(|e| panic!("{json}: {e}"));
            let _ = profile(&i, DaemonMode::Rootful);
        }
        // An empty user is root; an untyped, sourceless mount is nothing.
        let p = profile(&inspect(r#"{"Mounts":[{}]}"#), DaemonMode::Rootful);
        assert_eq!(codes(&p), vec![Code::RootUser]);
        // Wrong types are an error the adapter reports, never a guess.
        assert!(serde_json::from_str::<Inspect>(r#"{"HostConfig":{"Privileged":"yes"}}"#).is_err());
    }

    #[test]
    fn reasons_are_worst_first_and_the_level_is_the_worst() {
        let i = inspect(
            r#"{"Config":{"User":""},"HostConfig":{"Privileged":true,"NetworkMode":"host"}}"#,
        );
        let p = profile(&i, DaemonMode::Rootful);
        assert_eq!(p.level, Level::Escape);
        assert_eq!(
            codes(&p),
            vec![Code::Privileged, Code::RootUser, Code::HostNetwork]
        );
        assert_eq!(
            p.deciding().map(|r| r.code).collect::<Vec<_>>(),
            vec![Code::Privileged]
        );
        assert_eq!(rollup([Level::Ok, Level::Root]), Level::Root);
        assert_eq!(rollup([]), Level::Ok);
    }

    #[test]
    fn the_daemon_mode_comes_from_security_options() {
        let rootless = [
            "name=seccomp,profile=builtin",
            "name=rootless",
            "name=cgroupns",
        ];
        assert_eq!(
            DaemonMode::from_security_options(&rootless),
            DaemonMode::Rootless
        );
        let userns = [
            "name=apparmor",
            "name=seccomp,profile=builtin",
            "name=userns",
        ];
        assert_eq!(
            DaemonMode::from_security_options(&userns),
            DaemonMode::Userns
        );
        let rootful = ["name=apparmor", "name=seccomp,profile=builtin"];
        assert_eq!(
            DaemonMode::from_security_options(&rootful),
            DaemonMode::Rootful
        );
        assert_eq!(
            DaemonMode::from_security_options::<&str>(&[]),
            DaemonMode::Rootful
        );
        // Only a `name=` entry counts; a profile path mentioning rootless does not.
        let sneaky = ["name=seccomp,profile=/etc/rootless.json", "rootless"];
        assert_eq!(
            DaemonMode::from_security_options(&sneaky),
            DaemonMode::Rootful
        );
    }

    #[test]
    fn shell_paths_and_container_args_are_strict() {
        for p in [
            "/bin/sh",
            "/bin/bash",
            "/usr/local/bin/fish-3.7",
            "/bin/busybox_sh",
        ] {
            assert!(valid_shell_path(p), "{p}");
        }
        for p in [
            "",
            "/",
            "sh",
            "bin/sh",
            "/bin/sh -c id",
            "/bin/sh;id",
            "/bin/$(id)",
            "/bin/`id`",
            "/bin/sh\n",
            "/../bin/sh",
            "/bin/../../sh",
            "/bin/",
            "/bin/sh|x",
            "/bin/s'h",
            "/bin/ünï",
        ] {
            assert!(!valid_shell_path(p), "{p:?}");
        }
        assert!(!valid_shell_path(&format!("/{}", "a".repeat(300))));
        for id in ["abc123", "web-1", "stack_web_1", "a.b"] {
            assert!(valid_container_arg(id), "{id}");
        }
        for id in [
            "",
            "-u",
            "--privileged",
            "--user=0",
            "a b",
            "a;b",
            "a/b",
            "a\0",
        ] {
            assert!(!valid_container_arg(id), "{id:?}");
        }
    }

    #[test]
    fn the_shell_policy_matrix() {
        use Level::*;
        use ShellPolicy::*;
        use ShellRule::*;
        let cases = [
            (Strict, Some(Escape), Refused),
            (Strict, Some(Root), AdminOnly),
            (Strict, Some(Ok), PerRole),
            (Strict, None, Refused),
            (Relaxed, Some(Escape), AdminOnly),
            (Relaxed, Some(Root), PerRole),
            (Relaxed, Some(Ok), PerRole),
            (Relaxed, None, Refused),
        ];
        for (policy, level, rule) in cases {
            assert_eq!(shell_rule(policy, level), rule, "{policy:?} {level:?}");
        }
        // No policy lets a non-admin into an escape container.
        for policy in [Strict, Relaxed] {
            assert!(!shell_permitted(shell_rule(policy, Some(Escape)), false));
        }
        assert_eq!(ShellPolicy::parse(" Strict "), Some(Strict));
        assert_eq!(ShellPolicy::parse("relaxed"), Some(Relaxed));
        assert_eq!(ShellPolicy::parse("off"), None);
        assert_eq!(ShellPolicy::default(), Strict);
    }
}
