//! Holds (research R4, FR-010): what stops an update between the pull and the tag.
//!
//! Two checks, both before anything on the host changes but one more image:
//!
//! - **Root escalation.** A service whose compose sets `user:` runs as that user whatever the
//!   image says, so it is never held. Otherwise the effective user it runs as today is compared
//!   with the new image's `User`: non-root to root holds on a rootful daemon (a
//!   [`Level::Root`](crate::security::Level) finding), and on a rootless or userns daemon it is
//!   shown as informational (`RootRemapped`), not held.
//! - **Escape findings.** [`compose_lint::lint`] is re-run on the current compose text, so an
//!   Auto stack whose file was edited since its policy was set is judged on what it says now.
//!   Under `DEPTHS_COMPOSE_POLICY=enforce` an escape finding refuses the update; under `warn`
//!   a person's confirm must acknowledge it, and a scheduled run holds.
//!
//! 054's guard is not in the tree yet: until `depths-guard-core` exists, a guard refusal can
//! only surface from `up`, and the runner records it as a hold when the old container is still
//! running.

use crate::compose_lint::{self, ComposePolicy, Finding};
use crate::security::{Code, DaemonMode, Reason, Severity, is_root_user};

/// What the root-escalation check found for one service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserCheck {
    /// Nothing changed that matters: `user:` is set, or the image does not newly run as root.
    Clear,
    /// Non-root to root on a rootful daemon: hold.
    Hold(Reason),
    /// Non-root to root, remapped by the daemon: shown, not held.
    Shown(Reason),
}

impl UserCheck {
    pub fn holds(&self) -> bool {
        matches!(self, Self::Hold(_))
    }
}

/// The root-escalation check (research R4 step 1).
///
/// `compose_user` is the service's `user:`, if set; `old_user` the `Config.User` of the running
/// container (or, without one, of the image it would run), `""` meaning the image default;
/// `new_user` the pulled image's `Config.User`.
pub fn user_check(
    compose_user: Option<&str>,
    old_user: &str,
    new_user: Option<&str>,
    daemon: DaemonMode,
) -> UserCheck {
    if compose_user.is_some_and(|u| !u.trim().is_empty()) {
        return UserCheck::Clear;
    }
    let new_user = new_user.unwrap_or("");
    if is_root_user(old_user) || !is_root_user(new_user) {
        return UserCheck::Clear;
    }
    let was = if old_user.trim().is_empty() {
        "the image default".to_owned()
    } else {
        format!("user {}", old_user.trim())
    };
    match daemon {
        DaemonMode::Rootful => UserCheck::Hold(Reason {
            code: Code::RootUser,
            severity: Severity::Root,
            message: format!(
                "the new image runs as root (it ran as {was}); on this daemon that is root on \
                 the host"
            ),
        }),
        DaemonMode::Rootless | DaemonMode::Userns => UserCheck::Shown(Reason {
            code: Code::RootRemapped,
            severity: Severity::Info,
            message: format!(
                "the new image runs as root (it ran as {was}); the daemon maps it to an \
                 unprivileged host user"
            ),
        }),
    }
}

/// Who asked for the update: a person confirms findings, a schedule cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// A person's confirm; `acknowledged` is its `acknowledge_findings`.
    Manual { acknowledged: bool },
    Scheduled,
}

/// What the compose lint decides for an update (research R4 step 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LintDecision {
    /// No escape finding, or a person acknowledged it under `warn`.
    Proceed,
    /// `enforce` and an escape finding: refused, as for any deploy.
    Refuse,
    /// `warn`, an escape finding, and no acknowledgement: a person must acknowledge it.
    NeedsAck,
    /// `warn`, an escape finding, and a scheduled run: held for a person.
    Hold,
}

/// The current compose text's lint, and what it decides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintCheck {
    /// Every finding, informational ones included, for the preview.
    pub findings: Vec<Finding>,
    pub decision: LintDecision,
}

impl LintCheck {
    /// The escape-level findings, as `service:code`, for a hold or refusal reason.
    pub fn escape_codes(&self) -> Vec<String> {
        self.findings
            .iter()
            .filter(|f| f.reason.severity == Severity::Escape)
            .map(|f| format!("{}:{}", f.service, f.reason.code.as_str()))
            .collect()
    }
}

/// Re-runs the lint on `compose_text` and decides.
pub fn lint_check(compose_text: &str, policy: ComposePolicy, trigger: Trigger) -> LintCheck {
    let findings = compose_lint::lint(compose_text);
    let decision = if !compose_lint::has_escape(&findings) {
        LintDecision::Proceed
    } else {
        match (policy, trigger) {
            (ComposePolicy::Enforce, _) => LintDecision::Refuse,
            (ComposePolicy::Warn, Trigger::Scheduled) => LintDecision::Hold,
            (ComposePolicy::Warn, Trigger::Manual { acknowledged: true }) => LintDecision::Proceed,
            (ComposePolicy::Warn, Trigger::Manual { acknowledged: false }) => {
                LintDecision::NeedsAck
            }
        }
    };
    LintCheck { findings, decision }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: &str = "services:\n  app:\n    image: nginx\n    user: \"1000\"\n";
    const ESCAPE: &str = "services:\n  app:\n    image: nginx\n    privileged: true\n";

    #[test]
    fn a_service_with_user_set_is_never_held() {
        for daemon in [DaemonMode::Rootful, DaemonMode::Rootless] {
            assert_eq!(
                user_check(Some("1000:1000"), "1000", Some("root"), daemon),
                UserCheck::Clear
            );
        }
        // An empty `user:` is not a user.
        assert!(user_check(Some(" "), "1000", Some("root"), DaemonMode::Rootful).holds());
    }

    #[test]
    fn non_root_to_root_on_a_rootful_daemon_holds() {
        let c = user_check(None, "1000", Some(""), DaemonMode::Rootful);
        let UserCheck::Hold(r) = &c else {
            panic!("{c:?}")
        };
        assert_eq!(r.code, Code::RootUser);
        assert_eq!(r.severity, Severity::Root);
        assert!(r.message.contains("user 1000"), "{}", r.message);
        assert!(user_check(None, "app", Some("0:0"), DaemonMode::Rootful).holds());
        assert!(user_check(None, "nobody", None, DaemonMode::Rootful).holds());
    }

    #[test]
    fn on_rootless_or_userns_it_is_shown_not_held() {
        for daemon in [DaemonMode::Rootless, DaemonMode::Userns] {
            let c = user_check(None, "1000", Some("root"), daemon);
            let UserCheck::Shown(r) = &c else {
                panic!("{c:?}")
            };
            assert_eq!(r.code, Code::RootRemapped);
            assert_eq!(r.severity, Severity::Info);
            assert!(!c.holds());
        }
    }

    #[test]
    fn root_to_root_and_non_root_to_non_root_are_clear() {
        assert_eq!(
            user_check(None, "", Some(""), DaemonMode::Rootful),
            UserCheck::Clear
        );
        assert_eq!(
            user_check(None, "root", Some("0"), DaemonMode::Rootful),
            UserCheck::Clear
        );
        assert_eq!(
            user_check(None, "1000", Some("2000"), DaemonMode::Rootful),
            UserCheck::Clear
        );
        assert_eq!(
            user_check(None, "", Some("1000"), DaemonMode::Rootful),
            UserCheck::Clear,
            "dropping root is never held"
        );
    }

    #[test]
    fn the_lint_is_re_run_on_the_current_text() {
        let clean = lint_check(PLAIN, ComposePolicy::Enforce, Trigger::Scheduled);
        assert_eq!(clean.decision, LintDecision::Proceed);
        let warned = lint_check(
            ESCAPE,
            ComposePolicy::Warn,
            Trigger::Manual {
                acknowledged: false,
            },
        );
        assert_eq!(warned.decision, LintDecision::NeedsAck);
        assert!(!warned.findings.is_empty(), "listed under warn");
        assert_eq!(warned.escape_codes(), vec!["app:privileged".to_owned()]);
    }

    #[test]
    fn enforce_refuses_and_warn_lists_and_needs_an_acknowledgement() {
        let manual = |acknowledged| Trigger::Manual { acknowledged };
        assert_eq!(
            lint_check(ESCAPE, ComposePolicy::Enforce, manual(true)).decision,
            LintDecision::Refuse
        );
        assert_eq!(
            lint_check(ESCAPE, ComposePolicy::Enforce, Trigger::Scheduled).decision,
            LintDecision::Refuse
        );
        assert_eq!(
            lint_check(ESCAPE, ComposePolicy::Warn, manual(true)).decision,
            LintDecision::Proceed
        );
        assert_eq!(
            lint_check(ESCAPE, ComposePolicy::Warn, Trigger::Scheduled).decision,
            LintDecision::Hold,
            "Auto holds an escape finding nobody acknowledged"
        );
    }
}
