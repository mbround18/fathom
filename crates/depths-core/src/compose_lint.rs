//! Compose lint: what a compose file asks for that weakens containment, before it runs.
//!
//! Warnings only: nothing here changes a file. Each [`Finding`] names its service, carries a
//! [`Reason`] with the same codes and severities as [`crate::security::profile`], and the line
//! of the setting when it can be found. The server shows them in the editor and the preview, and
//! with `DEPTHS_COMPOSE_POLICY=enforce` refuses an `up` or a save with an escape-level finding
//! unless an admin acknowledges it.
//!
//! The file is read as written: `${VAR}` is not interpolated, and `extends`/`include` are not
//! followed. A setting whose value is a variable is judged at its worst (a `privileged: ${X}` is
//! read as privileged), because the guard has to hold whatever the variable turns out to be.

use serde_yaml::{Mapping, Value};

use crate::security::{
    Code, Level, Reason, Severity, bind_reason, cap_reasons, is_root_user, unconfined_opt,
};

/// One warning about one service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub service: String,
    pub reason: Reason,
    /// 1-based line of the setting (or of the service) in the file, when found.
    pub line: Option<u32>,
}

impl Finding {
    pub fn level(&self) -> Level {
        match self.reason.severity {
            Severity::Info => Level::Ok,
            Severity::Root => Level::Root,
            Severity::Escape => Level::Escape,
        }
    }
}

/// The worst level among findings.
pub fn level_of(findings: &[Finding]) -> Level {
    findings
        .iter()
        .map(Finding::level)
        .max()
        .unwrap_or(Level::Ok)
}

/// Whether any finding is escape-level: what `DEPTHS_COMPOSE_POLICY=enforce` refuses.
pub fn has_escape(findings: &[Finding]) -> bool {
    level_of(findings) == Level::Escape
}

/// `DEPTHS_COMPOSE_POLICY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ComposePolicy {
    /// Show findings; block nothing.
    #[default]
    Warn,
    /// Refuse `up` and saves with an escape-level finding unless an admin acknowledges it.
    Enforce,
}

impl ComposePolicy {
    pub fn parse(v: &str) -> Option<Self> {
        match v.trim().to_ascii_lowercase().as_str() {
            "warn" => Some(Self::Warn),
            "enforce" => Some(Self::Enforce),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warn => "warn",
            Self::Enforce => "enforce",
        }
    }
}

/// Lints a compose file. A file that does not parse yields nothing: Compose's own validation
/// reports that.
pub fn lint(compose_text: &str) -> Vec<Finding> {
    let Ok(mut doc) = serde_yaml::from_str::<Value>(compose_text) else {
        return Vec::new();
    };
    if doc.apply_merge().is_err() {
        return Vec::new();
    }
    let Some(services) = doc.get("services").and_then(Value::as_mapping) else {
        return Vec::new();
    };
    let lines = Lines::new(compose_text);
    let mut out = Vec::new();
    for (name, body) in services {
        let Some(name) = scalar(name) else { continue };
        let Some(body) = body.as_mapping() else {
            continue;
        };
        lint_service(&name, body, &lines, &mut out);
    }
    out.sort_by(|a, b| {
        b.reason
            .severity
            .cmp(&a.reason.severity)
            .then(a.service.cmp(&b.service))
            .then(a.line.cmp(&b.line))
    });
    out
}

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn interpolated(s: &str) -> bool {
    s.contains('$')
}

/// `true`, `"true"`, `yes`, `1`, or a variable: on. Anything else: off.
fn truthy(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => Some(n.as_i64() != Some(0)),
        Value::String(s) if interpolated(s) => None,
        Value::String(s) => Some(matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "true" | "yes" | "on" | "1"
        )),
        _ => Some(false),
    }
}

fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Sequence(s)) => s.iter().filter_map(scalar).collect(),
        Some(v) => scalar(v).into_iter().collect(),
        None => Vec::new(),
    }
}

fn mode_is_host(body: &Mapping, key: &str) -> bool {
    body.get(key)
        .and_then(scalar)
        .is_some_and(|m| m.trim().eq_ignore_ascii_case("host"))
}

fn lint_service(name: &str, body: &Mapping, lines: &Lines, out: &mut Vec<Finding>) {
    let svc_line = lines.service(name);
    let at = |key: &str| lines.key(svc_line, key).or(svc_line);
    let mut push = |reason: Reason, line: Option<u32>| {
        out.push(Finding {
            service: name.to_owned(),
            reason,
            line,
        });
    };

    let user = body.get("user").and_then(scalar);
    let root = match &user {
        None => {
            push(
                Reason {
                    code: Code::NoUser,
                    severity: Severity::Root,
                    message: "has no user: and runs as the image's user, which is often root"
                        .into(),
                },
                svc_line,
            );
            true
        }
        Some(u) if interpolated(u) => true,
        Some(u) if is_root_user(u) => {
            push(
                Reason {
                    code: Code::RootUser,
                    severity: Severity::Root,
                    message: format!("runs as root (user: {u})"),
                },
                at("user"),
            );
            true
        }
        Some(_) => false,
    };

    if let Some(v) = body.get("privileged") {
        match truthy(v) {
            Some(false) => {}
            on => push(
                Reason {
                    code: if on.is_none() {
                        Code::Interpolated
                    } else {
                        Code::Privileged
                    },
                    severity: Severity::Escape,
                    message: if on.is_none() {
                        "sets privileged: from a variable; read as privileged".into()
                    } else {
                        "runs privileged: every device and capability, no confinement".into()
                    },
                },
                at("privileged"),
            ),
        }
    }

    let host_pid = mode_is_host(body, "pid");
    for r in cap_reasons(&strings(body.get("cap_add")), host_pid) {
        push(r, at("cap_add"));
    }
    for (key, code, sev, msg) in [
        (
            "pid",
            Code::HostPid,
            Severity::Escape,
            "shares the host's process namespace (pid: host)",
        ),
        (
            "ipc",
            Code::HostIpc,
            Severity::Escape,
            "shares the host's IPC namespace (ipc: host)",
        ),
        (
            "userns_mode",
            Code::HostUserns,
            Severity::Escape,
            "uses the host's user namespace (userns_mode: host)",
        ),
        (
            "network_mode",
            Code::HostNetwork,
            Severity::Info,
            "uses the host's network (network_mode: host)",
        ),
        (
            "uts",
            Code::HostUts,
            Severity::Info,
            "shares the host's hostname namespace (uts: host)",
        ),
    ] {
        if mode_is_host(body, key) {
            push(
                Reason {
                    code,
                    severity: sev,
                    message: msg.into(),
                },
                at(key),
            );
        }
    }

    let devices = strings(body.get("devices"));
    let device_maps = body
        .get("devices")
        .and_then(Value::as_sequence)
        .is_some_and(|s| s.iter().any(Value::is_mapping));
    if !devices.is_empty() || device_maps {
        let named: Vec<&str> = devices
            .iter()
            .map(|d| d.split(':').next().unwrap_or(d).trim())
            .collect();
        push(
            Reason {
                code: Code::Devices,
                severity: Severity::Escape,
                message: if named.is_empty() {
                    "passes host devices in".into()
                } else {
                    format!("passes host devices in ({})", named.join(", "))
                },
            },
            at("devices"),
        );
    }

    if let Some(Value::Sequence(vols)) = body.get("volumes") {
        for v in vols {
            if let Some((source, writable)) = bind_of(v)
                && let Some(r) = bind_reason(&source, writable)
            {
                push(r, at("volumes"));
            }
        }
    }

    for opt in strings(body.get("security_opt")) {
        if let Some(which) = unconfined_opt(&opt) {
            push(
                Reason {
                    code: Code::Unconfined,
                    severity: if root {
                        Severity::Escape
                    } else {
                        Severity::Info
                    },
                    message: if root {
                        format!("turns {which} off (security_opt: {opt}) while running as root")
                    } else {
                        format!("turns {which} off (security_opt: {opt})")
                    },
                },
                at("security_opt"),
            );
        }
    }
}

/// A volume entry's host source and whether it is writable, when it is a host bind.
/// Short syntax `src:dst[:mode]` (a named volume has no `/` and is skipped), or long syntax.
fn bind_of(v: &Value) -> Option<(String, bool)> {
    match v {
        Value::String(s) => {
            let mut parts = s.splitn(3, ':');
            let src = parts.next()?.trim();
            let _dst = parts.next()?;
            let mode = parts.next().unwrap_or("");
            if !src.starts_with('/') {
                return None;
            }
            let ro = mode.split(',').any(|m| m.trim() == "ro");
            Some((src.to_owned(), !ro))
        }
        Value::Mapping(m) => {
            let ty = m.get("type").and_then(scalar).unwrap_or_default();
            if !ty.is_empty() && ty != "bind" {
                return None;
            }
            let src = m.get("source").and_then(scalar)?;
            if !src.starts_with('/') {
                return None;
            }
            let ro = m.get("read_only").and_then(truthy).unwrap_or(false);
            Some((src, !ro))
        }
        _ => None,
    }
}

/// Finds lines by indentation: good enough to point at a setting, and `None` when unsure.
struct Lines<'a> {
    lines: Vec<&'a str>,
}

fn indent(l: &str) -> usize {
    l.len() - l.trim_start().len()
}

fn key_of(l: &str) -> Option<&str> {
    let t = l.trim_start();
    if t.starts_with('#') {
        return None;
    }
    let k = t.split_once(':')?.0.trim();
    Some(k.trim_matches(|c| c == '"' || c == '\''))
}

impl<'a> Lines<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            lines: text.lines().collect(),
        }
    }

    /// The line of `name:` directly under the top-level `services:`.
    fn service(&self, name: &str) -> Option<u32> {
        let start = self
            .lines
            .iter()
            .position(|l| indent(l) == 0 && key_of(l) == Some("services"))?;
        let mut child_indent = None;
        for (i, l) in self.lines.iter().enumerate().skip(start + 1) {
            if l.trim().is_empty() || l.trim_start().starts_with('#') {
                continue;
            }
            let ind = indent(l);
            if ind == 0 {
                break;
            }
            let ci = *child_indent.get_or_insert(ind);
            if ind == ci && key_of(l) == Some(name) {
                return u32::try_from(i + 1).ok();
            }
        }
        None
    }

    /// The line of `key:` directly in the service starting at `svc` (1-based).
    fn key(&self, svc: Option<u32>, key: &str) -> Option<u32> {
        let svc = usize::try_from(svc?).ok()? - 1;
        let base = indent(self.lines.get(svc)?);
        let mut child_indent = None;
        for (i, l) in self.lines.iter().enumerate().skip(svc + 1) {
            if l.trim().is_empty() || l.trim_start().starts_with('#') {
                continue;
            }
            let ind = indent(l);
            if ind <= base {
                break;
            }
            let ci = *child_indent.get_or_insert(ind);
            if ind == ci && key_of(l) == Some(key) {
                return u32::try_from(i + 1).ok();
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(f: &[Finding]) -> Vec<(&str, Code)> {
        f.iter()
            .map(|f| (f.service.as_str(), f.reason.code))
            .collect()
    }

    #[test]
    fn a_service_with_a_user_and_nothing_else_is_clean() {
        let f = lint(
            "services:\n  web:\n    image: nginx\n    user: \"1000:1000\"\n    volumes:\n      - ./html:/usr/share/nginx/html:ro\n      - data:/data\n      - /etc/localtime:/etc/localtime:ro\n",
        );
        assert!(f.is_empty(), "{f:?}");
        assert_eq!(level_of(&f), Level::Ok);
    }

    #[test]
    fn no_user_and_root_users_are_root_level_with_lines() {
        let text = "services:\n  a:\n    image: x\n  b:\n    image: y\n    user: root\n  c:\n    user: 0\n";
        let f = lint(text);
        assert_eq!(
            codes(&f),
            vec![
                ("a", Code::NoUser),
                ("b", Code::RootUser),
                ("c", Code::RootUser)
            ]
        );
        assert_eq!(f[0].line, Some(2));
        assert_eq!(f[1].line, Some(6));
        assert_eq!(f[2].line, Some(8));
        assert_eq!(level_of(&f), Level::Root);
        assert!(!has_escape(&f));
    }

    #[test]
    fn every_escape_setting_is_flagged() {
        let text = r#"
services:
  bad:
    image: x
    user: "1000"
    privileged: true
    cap_add: [SYS_ADMIN, NET_ADMIN]
    pid: host
    ipc: host
    userns_mode: host
    network_mode: host
    devices:
      - /dev/kvm:/dev/kvm
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
      - /:/host
      - type: bind
        source: /etc
        target: /hostetc
      - type: bind
        source: /proc
        target: /hostproc
        read_only: true
    security_opt:
      - seccomp:unconfined
"#;
        let f = lint(text);
        let c: Vec<Code> = f.iter().map(|f| f.reason.code).collect();
        for want in [
            Code::Privileged,
            Code::DangerousCapability,
            Code::AddedCapability,
            Code::HostPid,
            Code::HostIpc,
            Code::HostUserns,
            Code::HostNetwork,
            Code::Devices,
            Code::RuntimeSocket,
            Code::HostRootMount,
            Code::SensitiveMount,
            Code::SensitiveMountReadOnly,
            Code::Unconfined,
        ] {
            assert!(c.contains(&want), "{want:?} in {c:?}");
        }
        assert!(has_escape(&f));
        // Worst first; the privileged line is pointed at.
        assert_eq!(f[0].reason.severity, Severity::Escape);
        let p = f
            .iter()
            .find(|f| f.reason.code == Code::Privileged)
            .unwrap();
        assert_eq!(p.line, Some(6));
        // Not root, so unconfined is informational here.
        let u = f
            .iter()
            .find(|f| f.reason.code == Code::Unconfined)
            .unwrap();
        assert_eq!(u.reason.severity, Severity::Info);
    }

    #[test]
    fn unconfined_with_no_user_is_an_escape() {
        let f = lint("services:\n  a:\n    security_opt: [\"apparmor=unconfined\"]\n");
        assert!(has_escape(&f));
    }

    #[test]
    fn variables_are_read_at_their_worst() {
        let f = lint("services:\n  a:\n    user: \"${PUID}\"\n    privileged: ${PRIV:-false}\n");
        assert_eq!(codes(&f), vec![("a", Code::Interpolated)]);
        assert!(has_escape(&f));
        let f = lint(
            "services:\n  a:\n    user: \"${PUID}\"\n    security_opt: [seccomp=unconfined]\n",
        );
        assert!(has_escape(&f), "a variable user counts as root: {f:?}");
    }

    #[test]
    fn privileged_false_spelled_any_way_is_clean() {
        for v in ["false", "\"false\"", "no", "0"] {
            let f = lint(&format!(
                "services:\n  a:\n    user: \"1\"\n    privileged: {v}\n"
            ));
            assert!(f.is_empty(), "{v}: {f:?}");
        }
    }

    #[test]
    fn merge_keys_are_applied() {
        let text =
            "x-base: &base\n  privileged: true\nservices:\n  a:\n    <<: *base\n    user: \"1\"\n";
        let f = lint(text);
        assert_eq!(codes(&f), vec![("a", Code::Privileged)]);
    }

    #[test]
    fn hostile_and_odd_files_yield_nothing_or_findings_never_a_panic() {
        for text in [
            "",
            "not: [valid",
            "services: 3",
            "services:\n  - a\n  - b\n",
            "services:\n  a: null\n  b: [1, 2]\n  c: \"text\"\n",
            "services:\n  ? [complex, key]\n  : {privileged: true}\n",
            "services:\n  a:\n    volumes: [{}, 3, null, \"::\", \"/\"]\n    cap_add: {SYS_ADMIN: 1}\n    devices: [{source: /dev/x, target: /dev/x}]\n",
            "a: &a [*a]",
            "\u{feff}services:\n  a:\n    user: \"1\"\n",
        ] {
            let _ = lint(text);
        }
        // A device given in the long form still counts.
        let f = lint(
            "services:\n  a:\n    user: \"1\"\n    devices: [{source: /dev/x, target: /dev/x}]\n",
        );
        assert!(has_escape(&f));
    }

    #[test]
    fn named_volumes_and_relative_binds_are_not_host_binds() {
        let f = lint(
            "services:\n  a:\n    user: \"1\"\n    volumes: [\"data:/var/lib/docker\", \"./run:/run\", \"~/x:/x\", {type: volume, source: x, target: /etc}]\n",
        );
        assert!(f.is_empty(), "{f:?}");
    }

    #[test]
    fn the_policy_parses() {
        assert_eq!(ComposePolicy::parse("WARN"), Some(ComposePolicy::Warn));
        assert_eq!(
            ComposePolicy::parse(" enforce"),
            Some(ComposePolicy::Enforce)
        );
        assert_eq!(ComposePolicy::parse("block"), None);
        assert_eq!(ComposePolicy::default(), ComposePolicy::Warn);
    }

    #[test]
    fn read_only_host_root_and_etc_are_root_level_but_a_read_only_socket_is_an_escape() {
        // 051 FR-004: root-level findings never block (FR-018), a socket always does.
        let ro = "services:\n  a:\n    user: \"1\"\n    volumes:\n      - /:/host:ro\n      - /etc:/hostetc:ro\n";
        let f = lint(ro);
        assert_eq!(f.len(), 2, "{f:?}");
        assert!(f.iter().all(|f| f.reason.severity == Severity::Root));
        assert!(!has_escape(&f));
        let sock = "services:\n  a:\n    user: \"1\"\n    volumes:\n      - /var/run/docker.sock:/var/run/docker.sock:ro\n";
        assert!(has_escape(&lint(sock)));
    }
}
