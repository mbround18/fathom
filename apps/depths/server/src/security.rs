//! 051: the wire shapes of depths-core's container security profile and compose lint.
//!
//! Everything here is a plain mapping; the decisions live in `depths_core::security` and
//! `depths_core::compose_lint`.

use depths_core::compose_lint::{ComposePolicy, Finding};
use depths_core::security::{DaemonMode, Level, Profile, Reason, Severity, ShellPolicy, ShellRule};
use depths_protocol as wire;

pub fn level(l: Level) -> wire::SecurityLevel {
    match l {
        Level::Ok => wire::SecurityLevel::Ok,
        Level::Root => wire::SecurityLevel::Root,
        Level::Escape => wire::SecurityLevel::Escape,
    }
}

pub fn severity(s: Severity) -> wire::SecuritySeverity {
    match s {
        Severity::Info => wire::SecuritySeverity::Info,
        Severity::Root => wire::SecuritySeverity::Root,
        Severity::Escape => wire::SecuritySeverity::Escape,
    }
}

pub fn reason(r: &Reason) -> wire::SecurityReason {
    wire::SecurityReason {
        code: r.code.as_str().to_owned(),
        severity: severity(r.severity),
        message: r.message.clone(),
    }
}

pub fn shell_rule(r: ShellRule) -> wire::ShellRule {
    match r {
        ShellRule::PerRole => wire::ShellRule::PerRole,
        ShellRule::AdminOnly => wire::ShellRule::AdminOnly,
        ShellRule::Refused => wire::ShellRule::Refused,
    }
}

/// A container's rating as the inventory carries it: its level, its reasons and the
/// server-wide rule for its shell (051 FR-005).
pub fn container(p: &Profile, policy: ShellPolicy) -> wire::ContainerSecurity {
    wire::ContainerSecurity {
        level: level(p.level),
        reasons: p.reasons.iter().map(reason).collect(),
        shell: shell_rule(depths_core::security::shell_rule(policy, Some(p.level))),
    }
}

pub fn warning(f: &Finding) -> wire::ComposeWarning {
    wire::ComposeWarning {
        service: f.service.clone(),
        code: f.reason.code.as_str().to_owned(),
        severity: severity(f.reason.severity),
        message: f.reason.message.clone(),
        line: f.line,
    }
}

/// The compose lint of a file as the wire carries it: informational findings included.
pub fn warnings(compose_text: &str) -> Vec<wire::ComposeWarning> {
    depths_core::compose_lint::lint(compose_text)
        .iter()
        .map(warning)
        .collect()
}

pub fn shell_policy(p: ShellPolicy) -> wire::ShellPolicy {
    match p {
        ShellPolicy::Strict => wire::ShellPolicy::Strict,
        ShellPolicy::Relaxed => wire::ShellPolicy::Relaxed,
    }
}

pub fn compose_policy(p: ComposePolicy) -> wire::ComposePolicy {
    match p {
        ComposePolicy::Warn => wire::ComposePolicy::Warn,
        ComposePolicy::Enforce => wire::ComposePolicy::Enforce,
    }
}

pub fn daemon(d: DaemonMode) -> wire::DaemonMode {
    match d {
        DaemonMode::Rootless => wire::DaemonMode::Rootless,
        DaemonMode::Userns => wire::DaemonMode::Userns,
        DaemonMode::Rootful => wire::DaemonMode::Rootful,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_privileged_service_is_an_escape_warning_on_the_wire() {
        let w = warnings("services:\n  app:\n    image: busybox\n    privileged: true\n");
        assert!(
            w.iter()
                .any(|w| w.service == "app" && w.severity == wire::SecuritySeverity::Escape),
            "{w:?}"
        );
        assert!(warnings("not: [valid").is_empty());
    }
}
