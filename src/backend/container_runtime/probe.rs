//! Is there a usable container runtime, and if not, which of the three ways is it unusable.
//!
//! `kind::detect_prerequisites_with` already probes `docker info` and `podman info`, but it
//! reduces the answer to three booleans and discards the daemon's own message. FR-004 needs that
//! message: "permission denied while trying to connect to the Docker daemon socket" is the whole
//! remediation, and a boolean cannot carry it. So this probes the same two binaries with the same
//! `info` invocation and the same timeout discipline, keeping stderr and the endpoint.

use crate::protocol::v1::{KindRuntimeKind, RuntimePresence, RuntimePresenceState};

use super::super::kind::{RunError, Runner, SystemRunner};

/// Matches the kind probe's budget: a wedged socket must not stall the scene.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Classifies the local runtime from one `info` call per candidate binary.
///
/// Docker is tried first, then Podman, and a runtime that answers wins over one that is merely
/// installed — the same preference order as the kind probe, so the two never disagree about which
/// runtime this machine has.
pub fn detect_presence_with(runner: &dyn Runner) -> RuntimePresence {
    let mut best: Option<RuntimePresence> = None;

    for (bin, kind) in [
        ("docker", KindRuntimeKind::Docker),
        ("podman", KindRuntimeKind::Podman),
    ] {
        let candidate = match runner.run(bin, &["info", "--format", "{{.Name}}|{{.ServerVersion}}"])
        {
            // Not installed: this binary contributes nothing.
            Err(RunError::NotFound) => continue,
            Err(RunError::Other(e)) => RuntimePresence {
                state: RuntimePresenceState::Unreachable,
                runtime: Some(kind),
                endpoint: Some(default_endpoint(bin)),
                detail: Some(e),
            },
            Ok(out) if out.success => RuntimePresence {
                state: RuntimePresenceState::Ready,
                runtime: Some(kind),
                endpoint: Some(default_endpoint(bin)),
                detail: None,
            },
            Ok(out) => {
                let message = first_meaningful_line(&out.stderr)
                    .or_else(|| first_meaningful_line(&out.stdout));
                // A daemon that is simply not running says so; anything else is a reachability
                // problem the operator has to see verbatim to act on.
                let state = match message.as_deref() {
                    Some(m) if looks_like_daemon_down(m) => RuntimePresenceState::Stopped,
                    Some(_) => RuntimePresenceState::Unreachable,
                    None => RuntimePresenceState::Stopped,
                };
                RuntimePresence {
                    state,
                    runtime: Some(kind),
                    endpoint: Some(default_endpoint(bin)),
                    detail: message,
                }
            }
        };

        let ready = candidate.state == RuntimePresenceState::Ready;
        if best.is_none() || ready {
            best = Some(candidate);
        }
        if ready {
            break;
        }
    }

    best.unwrap_or_else(RuntimePresence::absent)
}

/// Same spawn-and-timeout discipline as `kind::detect_prerequisites`: a blocking probe on its own
/// thread, bounded, falling back to `Absent` rather than hanging the scene.
pub async fn detect_presence() -> RuntimePresence {
    let probe = tokio::task::spawn_blocking(|| detect_presence_with(&SystemRunner));
    match tokio::time::timeout(PROBE_TIMEOUT, probe).await {
        Ok(Ok(p)) => p,
        _ => RuntimePresence::absent(),
    }
}

fn default_endpoint(bin: &str) -> String {
    match bin {
        "podman" => "unix:///run/podman/podman.sock".to_owned(),
        _ => "unix:///var/run/docker.sock".to_owned(),
    }
}

/// The runtime's own first line of complaint, with its trailing hint lines dropped.
fn first_meaningful_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(ToOwned::to_owned)
}

/// Distinguishes "the daemon is not running" from every other failure.
///
/// Deliberately narrow: anything not recognised here becomes `Unreachable` and carries the
/// runtime's text, which is the honest answer. Guessing the other way would turn a permission
/// denial into "installed and stopped" and send the operator to restart a service that is fine.
fn looks_like_daemon_down(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    (m.contains("is the docker daemon running")
        || m.contains("cannot connect to the docker daemon"))
        && !m.contains("permission denied")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::kind::CmdOutput;
    use std::collections::HashMap;

    struct Mock(HashMap<String, Result<CmdOutput, RunError>>);

    impl Mock {
        fn new(entries: Vec<(&str, Result<CmdOutput, RunError>)>) -> Self {
            Self(
                entries
                    .into_iter()
                    .map(|(k, v)| (k.to_owned(), v))
                    .collect(),
            )
        }
    }

    impl Runner for Mock {
        fn run(&self, bin: &str, _args: &[&str]) -> Result<CmdOutput, RunError> {
            self.0.get(bin).cloned().unwrap_or(Err(RunError::NotFound))
        }
    }

    fn ok(stdout: &str) -> Result<CmdOutput, RunError> {
        Ok(CmdOutput {
            success: true,
            stdout: stdout.to_owned(),
            stderr: String::new(),
        })
    }

    fn failed(stderr: &str) -> Result<CmdOutput, RunError> {
        Ok(CmdOutput {
            success: false,
            stdout: String::new(),
            stderr: stderr.to_owned(),
        })
    }

    #[test]
    fn nothing_installed_is_absent() {
        let p = detect_presence_with(&Mock::new(vec![]));
        assert_eq!(p.state, RuntimePresenceState::Absent);
        assert_eq!(p.runtime, None);
        assert_eq!(p.endpoint, None);
    }

    #[test]
    fn running_daemon_is_ready() {
        let p = detect_presence_with(&Mock::new(vec![("docker", ok("tux|29.8.1"))]));
        assert_eq!(p.state, RuntimePresenceState::Ready);
        assert_eq!(p.runtime, Some(KindRuntimeKind::Docker));
        assert!(p.detail.is_none());
    }

    /// FR-003: "installed and stopped" is its own sentence, not "no containers".
    #[test]
    fn installed_but_stopped_says_so() {
        let p = detect_presence_with(&Mock::new(vec![(
            "docker",
            failed(
                "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. \
                 Is the docker daemon running?",
            ),
        )]));
        assert_eq!(p.state, RuntimePresenceState::Stopped);
        assert_eq!(p.runtime, Some(KindRuntimeKind::Docker));
        assert!(p.detail.is_some());
    }

    /// FR-004: a permission denial is not a stopped daemon, and the remediation is the runtime's
    /// own words. Collapsing this into `Stopped` sends the operator to restart a healthy service.
    #[test]
    fn permission_denial_is_unreachable_and_carries_the_runtimes_words() {
        let stderr = "permission denied while trying to connect to the Docker daemon socket at \
                      unix:///var/run/docker.sock";
        let p = detect_presence_with(&Mock::new(vec![("docker", failed(stderr))]));
        assert_eq!(p.state, RuntimePresenceState::Unreachable);
        assert_eq!(p.detail.as_deref(), Some(stderr));
        assert_eq!(
            p.endpoint.as_deref(),
            Some("unix:///var/run/docker.sock"),
            "the endpoint has to be named for the message to be actionable"
        );
    }

    #[test]
    fn spawn_failure_is_unreachable_not_absent() {
        let p = detect_presence_with(&Mock::new(vec![(
            "docker",
            Err(RunError::Other("no such file or directory".into())),
        )]));
        assert_eq!(p.state, RuntimePresenceState::Unreachable);
    }

    /// Matches `kind::detect_prerequisites_with`: a runtime that answers beats one that is only
    /// installed, so the two probes never disagree about this machine's runtime.
    #[test]
    fn a_running_podman_wins_over_a_stopped_docker() {
        let p = detect_presence_with(&Mock::new(vec![
            (
                "docker",
                failed("Cannot connect to the Docker daemon. Is the docker daemon running?"),
            ),
            ("podman", ok("tux|5.0.0")),
        ]));
        assert_eq!(p.state, RuntimePresenceState::Ready);
        assert_eq!(p.runtime, Some(KindRuntimeKind::Podman));
    }

    /// None of the four states may read as "there are no containers here".
    #[test]
    fn no_state_implies_an_empty_inventory() {
        for p in [
            detect_presence_with(&Mock::new(vec![])),
            detect_presence_with(&Mock::new(vec![("docker", ok("tux|29.8.1"))])),
            detect_presence_with(&Mock::new(vec![(
                "docker",
                failed("Cannot connect to the Docker daemon. Is the docker daemon running?"),
            )])),
            detect_presence_with(&Mock::new(vec![("docker", failed("permission denied"))])),
        ] {
            let detail = p.detail.unwrap_or_default().to_ascii_lowercase();
            assert!(
                !detail.contains("no containers"),
                "presence must never claim an empty inventory: {detail}"
            );
        }
    }
}
