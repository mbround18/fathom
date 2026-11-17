//! Joining with a join token (spec 055 T146, contracts/join.md, "Runner side").
//!
//! - `join-token` in the state directory, written by the base profile. An `enroll-token` beside
//!   it wins: the runner enrolls with that and deletes the join token afterwards.
//! - Before connecting: the token must not be expired, and the CA in its `keel.host_ca` must be
//!   in `host_ca.pub`. The connection then trusts only that CA. The runner cannot check the
//!   token's signature; it reads only connection settings from it.
//! - `JoinPending`: close, wait (30 s doubling to 10 min, with jitter), ask again with the same
//!   frame. Connection failures back off the same way.
//! - `Enrolled`: save the certificate and host id, delete `join-token`.
//! - `JoinRejected`: delete `join-token` and stop; the binary exits 78 (`EX_CONFIG`), which the
//!   unit does not restart.

use std::future::Future;
use std::io::ErrorKind;
use std::time::Duration;

use keel_exec::client_config::{open_subsystem, parse_host_ca};
use keel_exec::codec::{read_frame, write_frame};
use keel_exec::frames::{Frame, SecretString, Subsystem};
use keel_exec::join_token;

use super::{
    ANSWER_TIMEOUT, HOST_ID_FILE, Runner, RunnerError, TOKEN_FILE, check_issued, state_err,
    unix_now, valid_host_id,
};

/// The join token's file in the state directory.
pub const JOIN_TOKEN_FILE: &str = "join-token";
/// `EX_CONFIG`: the exit status for a configuration the runner cannot proceed with. The unit
/// sets `RestartPreventExitStatus=78`.
pub const EX_CONFIG: i32 = 78;

/// Back-off between join attempts: 30 s doubling to 10 min, each wait shortened by up to a
/// quarter at random so a fleet that joined together does not knock together.
#[derive(Debug, Clone)]
pub struct JoinBackoff {
    next: Duration,
}

impl JoinBackoff {
    pub const FIRST: Duration = Duration::from_secs(30);
    pub const MAX: Duration = Duration::from_secs(600);

    #[must_use]
    pub fn new() -> Self {
        Self { next: Self::FIRST }
    }

    /// The next wait. `jitter` is in `[0, 1)`; values outside are clamped.
    pub fn next_delay(&mut self, jitter: f64) -> Duration {
        let base = self.next;
        self.next = (base * 2).min(Self::MAX);
        base.mul_f64(1.0 - 0.25 * jitter.clamp(0.0, 1.0))
    }
}

impl Default for JoinBackoff {
    fn default() -> Self {
        Self::new()
    }
}

/// How the runner waits between attempts: tokio's clock, or a fake one in tests.
pub trait Sleeper {
    fn sleep(&self, d: Duration) -> impl Future<Output = ()> + Send;
}

/// [`Sleeper`] on tokio's clock.
pub struct TokioSleeper;

impl Sleeper for TokioSleeper {
    async fn sleep(&self, d: Duration) {
        tokio::time::sleep(d).await;
    }
}

/// One attempt's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinStep {
    /// Saved the certificate and host id.
    Enrolled,
    /// Waits for an operator, as this request.
    Pending(String),
}

/// What the host reports about itself, bounded as the server expects.
struct Facts {
    hostname: String,
    os: String,
    arch: String,
}

fn bounded(s: &str, max: usize) -> String {
    let mut out = String::new();
    for c in s.trim().chars().filter(|c| !c.is_control()) {
        if out.len() + c.len_utf8() > max {
            break;
        }
        out.push(c);
    }
    out
}

fn pretty_name(os_release: &str) -> Option<String> {
    os_release.lines().find_map(|l| {
        let v = l.strip_prefix("PRETTY_NAME=")?;
        Some(v.trim().trim_matches('"').to_owned())
    })
}

fn facts() -> Facts {
    let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default();
    let os = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|t| pretty_name(&t))
        .unwrap_or_default();
    Facts {
        hostname: bounded(&hostname, 253),
        os: bounded(&os, 128),
        arch: std::env::consts::ARCH.to_owned(),
    }
}

/// The checks made before connecting: the token reads as a join token, has not expired, and
/// its host CA is in `host_ca.pub`. Returns the CA keys to pin.
pub fn check_token(
    token: &str,
    host_cas: &[russh::keys::ssh_key::public::KeyData],
    now: u64,
) -> Result<Vec<russh::keys::ssh_key::public::KeyData>, RunnerError> {
    let (_, claims) =
        join_token::decode_unverified(token).map_err(|_| RunnerError::JoinTokenInvalid)?;
    if claims.aud != join_token::AUDIENCE {
        return Err(RunnerError::JoinTokenInvalid);
    }
    if claims.exp <= i64::try_from(now).unwrap_or(i64::MAX) {
        return Err(RunnerError::JoinTokenExpired);
    }
    let pinned = parse_host_ca(&claims.host_ca).map_err(|_| RunnerError::JoinCaMismatch)?;
    if pinned.is_empty() || !pinned.iter().all(|k| host_cas.contains(k)) {
        return Err(RunnerError::JoinCaMismatch);
    }
    Ok(pinned)
}

impl Runner {
    /// Asks once: connects trusting only `cas`, sends `JoinRequest`, reads the answer. On
    /// `Enrolled` saves the certificate and host id (the caller deletes the token).
    pub async fn join_once(
        &self,
        token: &str,
        cas: &[russh::keys::ssh_key::public::KeyData],
    ) -> Result<JoinStep, RunnerError> {
        let facts = facts();
        let mut handle = self.connect(cas).await?;
        self.auth_bare_key(&mut handle).await?;
        let mut stream = open_subsystem(&handle, Subsystem::Join).await?;
        let request = Frame::JoinRequest {
            token: SecretString::new(token.to_owned()),
            inventory_name: self.policy.inventory_name.clone(),
            allowed_profiles: self.policy.allowed_profiles.clone(),
            require_signed: self.policy.require_signed,
            hostname: facts.hostname,
            os: facts.os,
            arch: facts.arch,
            runner_version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        write_frame(&mut stream, &request)
            .await
            .map_err(|e| RunnerError::Protocol(e.to_string()))?;
        let answer = tokio::time::timeout(ANSWER_TIMEOUT, read_frame(&mut stream))
            .await
            .map_err(|_| RunnerError::Protocol("no answer to JoinRequest".to_owned()))?;
        let step = match answer {
            Ok(Some(Frame::JoinPending { request_id })) => JoinStep::Pending(request_id),
            Ok(Some(Frame::JoinRejected { reason })) => {
                return Err(RunnerError::JoinRejected(bounded(&reason, 64)));
            }
            Ok(Some(Frame::Enrolled { host_id, cert })) => {
                if !valid_host_id(&host_id) {
                    return Err(RunnerError::Protocol("invalid host id".to_owned()));
                }
                let cert = check_issued(&cert, &self.key, &self.policy.inventory_name, unix_now())?;
                let id_path = self.path(HOST_ID_FILE);
                crate::write_private_atomic(&id_path, format!("{host_id}\n").as_bytes())
                    .map_err(|e| state_err("write", &id_path, e))?;
                self.save_cert(&cert)?;
                JoinStep::Enrolled
            }
            Ok(Some(other)) => {
                return Err(RunnerError::Protocol(format!(
                    "expected JoinPending, Enrolled or JoinRejected, got {}",
                    other.name()
                )));
            }
            // A token that does not verify, a malformed frame, or the rate limit: closed.
            Ok(None) | Err(_) => {
                return Err(RunnerError::Protocol(
                    "the server closed the join request".to_owned(),
                ));
            }
        };
        let _ = handle
            .disconnect(russh::Disconnect::ByApplication, "", "")
            .await;
        Ok(step)
    }

    /// Joins when there is no certificate, no enrollment token and a join token, asking until
    /// an operator decides. Returns `Ok(false)` when there was nothing to do.
    ///
    /// # Errors
    ///
    /// The token is expired, unreadable or pins another CA ([`RunnerError::is_config`]), Keel
    /// rejected the request (the token is deleted first), or the state directory failed.
    /// Connection failures are retried, not returned.
    pub async fn join_if_needed_with(
        &self,
        sleeper: &impl Sleeper,
        mut jitter: impl FnMut() -> f64,
    ) -> Result<bool, RunnerError> {
        if self.certificate()?.is_some() || self.path(TOKEN_FILE).exists() {
            return Ok(false);
        }
        let path = self.path(JOIN_TOKEN_FILE);
        let token = match std::fs::read_to_string(&path) {
            Ok(t) => t.trim().to_owned(),
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(state_err("read", &path, e)),
        };
        let cas = check_token(&token, &self.host_cas, unix_now())?;
        eprintln!("keel-runner: asking to join");
        let mut backoff = JoinBackoff::new();
        loop {
            match self.join_once(&token, &cas).await {
                Ok(JoinStep::Enrolled) => {
                    self.forget_join_token()?;
                    eprintln!("keel-runner: join accepted, enrolled");
                    return Ok(true);
                }
                Ok(JoinStep::Pending(id)) => {
                    eprintln!("keel-runner: join request {id} waits for an operator");
                }
                Err(RunnerError::JoinRejected(reason)) => {
                    self.forget_join_token()?;
                    return Err(RunnerError::JoinRejected(reason));
                }
                Err(e @ (RunnerError::State(_) | RunnerError::BadCert(_))) => return Err(e),
                Err(e) => eprintln!("keel-runner: join: {e}"),
            }
            // Expired while waiting: stop rather than knock with a dead token.
            check_token(&token, &self.host_cas, unix_now())?;
            sleeper.sleep(backoff.next_delay(jitter())).await;
        }
    }

    /// [`Self::join_if_needed_with`] on tokio's clock.
    ///
    /// # Errors
    ///
    /// As [`Self::join_if_needed_with`].
    pub async fn join_if_needed(&self) -> Result<bool, RunnerError> {
        self.join_if_needed_with(&TokioSleeper, rand::random::<f64>)
            .await
    }

    /// Deletes `join-token` if present.
    pub(crate) fn forget_join_token(&self) -> Result<(), RunnerError> {
        let path = self.path(JOIN_TOKEN_FILE);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(state_err("delete", &path, e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_release_and_bounds() {
        let t = "NAME=\"Debian GNU/Linux\"\nPRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"\n";
        assert_eq!(
            pretty_name(t).as_deref(),
            Some("Debian GNU/Linux 13 (trixie)")
        );
        assert_eq!(bounded("ab\u{7}c\n", 253), "abc");
        assert_eq!(bounded(&"é".repeat(100), 5), "éé");
    }
}
