//! One `keel-runner-v1` session and the per-run relays.
//!
//! `read_frame` is not cancel-safe, so every stream is read by its own task feeding a channel,
//! and the loops `select!` on channels only.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use keel_exec::client_config::open_subsystem;
use keel_exec::codec::{CodecError, read_frame, write_frame};
use keel_exec::frames::{Frame, Subsystem};
use russh::keys::Certificate;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep_until};

use super::{Runner, RunnerError, renew_at, unix_now};

/// How often the runner pings, and how long a silent server is tolerated.
const PING_EVERY: Duration = Duration::from_secs(15);
const SILENCE: Duration = Duration::from_secs(45);
/// After asking for a renewal, how long to wait before asking again.
const RENEW_RETRY: Duration = Duration::from_secs(60);
/// After relaying `Cancel`, how long the executor has to send `Finished` before the runner
/// closes the run's socket (the executor then kills the process group).
pub const CANCEL_GRACE: Duration = Duration::from_secs(30);

/// Reads frames from `r` into a channel until EOF or an error (which ends the channel).
pub(crate) fn spawn_reader<R>(mut r: R) -> (mpsc::Receiver<Frame>, JoinHandle<()>)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let (tx, rx) = mpsc::channel(64);
    let task = tokio::spawn(async move {
        loop {
            match read_frame(&mut r).await {
                Ok(Some(frame)) => {
                    if tx.send(frame).await.is_err() {
                        return;
                    }
                }
                Ok(None) | Err(_) => return,
            }
        }
    });
    (rx, task)
}

/// Writes frames from a channel to `w` until the channel closes or a write fails.
pub(crate) fn spawn_writer<W>(mut w: W) -> (mpsc::UnboundedSender<Frame>, JoinHandle<()>)
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (tx, mut rx) = mpsc::unbounded_channel::<Frame>();
    let task = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            let r: Result<(), CodecError> = write_frame(&mut w, &frame).await;
            if r.is_err() {
                return;
            }
        }
    });
    (tx, task)
}

/// Frames the server may send on `keel-runner-v1`.
fn from_server(frame: &Frame) -> bool {
    matches!(
        frame,
        Frame::RenewCert {}
            | Frame::Cert { .. }
            | Frame::Krl { .. }
            | Frame::Assign { .. }
            | Frame::Pack { .. }
            | Frame::Secrets { .. }
            | Frame::Approvals { .. }
            | Frame::Cancel { .. }
            | Frame::Ping
            | Frame::Pong
    )
}

/// Frames the executor may send back for a run (all carry the run id).
fn from_executor(frame: &Frame) -> bool {
    matches!(
        frame,
        Frame::Started { .. }
            | Frame::Refused { .. }
            | Frame::Event { .. }
            | Frame::Log { .. }
            | Frame::Finished { .. }
    )
}

struct RelayHandle {
    inbound: mpsc::UnboundedSender<Frame>,
    task: JoinHandle<()>,
}

/// Aborts every relay (and so closes every executor socket) when the session ends, however it
/// ends.
#[derive(Default)]
struct Relays(HashMap<String, RelayHandle>);

impl Drop for Relays {
    fn drop(&mut self) {
        for (_, r) in self.0.drain() {
            r.task.abort();
        }
    }
}

/// Aborts a task on drop.
struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Runner {
    /// Holds one session until it ends. Returns `Ok` when the server closed it.
    pub(crate) async fn session(&self, cert: Certificate) -> Result<(), RunnerError> {
        // The executor's ledger first: no Hello without it.
        let (ledger_head, bases) = self.read_ledger().await?;
        let host_id = self.host_id()?;

        let mut handle = self.connect(&self.host_cas).await?;
        let mut renew_deadline = deadline_for(renew_at(&cert));
        self.auth_cert(&mut handle, cert).await?;
        let stream = open_subsystem(&handle, Subsystem::Runner).await?;
        let (read_half, write_half) = tokio::io::split(stream);
        let (mut incoming, reader) = spawn_reader(read_half);
        let _reader = AbortOnDrop(reader);
        let (out, writer) = spawn_writer(write_half);
        let _writer = AbortOnDrop(writer);

        let hello = Frame::Hello {
            host_id,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            ansible_core: crate::venv::ansible_core_version(&self.cfg.venv),
            python: crate::venv::python_version(&self.cfg.venv),
            allowed_profiles: self.policy.allowed_profiles.clone(),
            require_signed: self.policy.require_signed,
            override_quorum: self.policy.override_quorum,
            ledger_head,
            bases,
        };
        send(&out, hello)?;

        let (done_tx, mut done_rx) = mpsc::unbounded_channel::<String>();
        let mut relays = Relays::default();
        let mut last_heard = Instant::now();
        let mut ping = tokio::time::interval_at(Instant::now() + PING_EVERY, PING_EVERY);

        let result = loop {
            tokio::select! {
                frame = incoming.recv() => {
                    let Some(frame) = frame else { break Ok(()) };
                    last_heard = Instant::now();
                    if !from_server(&frame) {
                        break Err(RunnerError::Protocol(format!(
                            "the server sent {}, which it may not send",
                            frame.name()
                        )));
                    }
                    match frame {
                        Frame::Ping => send(&out, Frame::Pong)?,
                        Frame::Pong => {}
                        Frame::RenewCert {} => {
                            send(&out, Frame::RenewCert {})?;
                            renew_deadline = Instant::now() + RENEW_RETRY;
                        }
                        Frame::Cert { cert } => {
                            let cert = self.accept_cert(&cert)?;
                            renew_deadline = deadline_for(renew_at(&cert));
                        }
                        Frame::Krl { bytes } => self.save_krl(&bytes)?,
                        Frame::Assign { ref run_id, .. } => {
                            let run_id = run_id.clone();
                            if relays.0.contains_key(&run_id) {
                                eprintln!("keel-runner: duplicate Assign for run {run_id}; dropped");
                                continue;
                            }
                            let (inbound, rx) = mpsc::unbounded_channel();
                            let task = tokio::spawn(relay(
                                self.cfg.executor_socket.clone(),
                                frame,
                                rx,
                                out.clone(),
                                done_tx.clone(),
                            ));
                            relays.0.insert(run_id, RelayHandle { inbound, task });
                        }
                        other => {
                            // Pack, Secrets, Approvals, Cancel: to the run's executor, unchanged.
                            let run_id = other.run_id().unwrap_or_default().to_owned();
                            match relays.0.get(&run_id) {
                                Some(r) => {
                                    let _ = r.inbound.send(other);
                                }
                                None => eprintln!(
                                    "keel-runner: {} for unknown run {run_id}; dropped",
                                    other.name()
                                ),
                            }
                        }
                    }
                }
                Some(run_id) = done_rx.recv() => {
                    relays.0.remove(&run_id);
                }
                _ = ping.tick() => {
                    if last_heard.elapsed() > SILENCE {
                        break Err(RunnerError::Protocol("the server went silent".to_owned()));
                    }
                    send(&out, Frame::Ping)?;
                }
                () = sleep_until(renew_deadline) => {
                    send(&out, Frame::RenewCert {})?;
                    renew_deadline = Instant::now() + RENEW_RETRY;
                }
            }
        };
        drop(relays);
        let _ = handle
            .disconnect(russh::Disconnect::ByApplication, "", "")
            .await;
        result
    }
}

fn send(out: &mpsc::UnboundedSender<Frame>, frame: Frame) -> Result<(), RunnerError> {
    out.send(frame)
        .map_err(|_| RunnerError::Protocol("the connection closed".to_owned()))
}

fn deadline_for(at_unix: u64) -> Instant {
    Instant::now() + Duration::from_secs(at_unix.saturating_sub(unix_now()))
}

/// Relays one run: its own executor connection, `Assign` and the later run frames in, the
/// executor's run frames out. Ends after `Refused` or `Finished`, when the executor closes, on
/// anything the executor may not send, or `CANCEL_GRACE` after a `Cancel`. Closing the socket
/// tells the executor to kill the run.
async fn relay(
    socket: PathBuf,
    assign: Frame,
    mut inbound: mpsc::UnboundedReceiver<Frame>,
    out: mpsc::UnboundedSender<Frame>,
    done: mpsc::UnboundedSender<String>,
) {
    let run_id = assign.run_id().unwrap_or_default().to_owned();
    let stream = match UnixStream::connect(&socket).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "keel-runner: run {run_id}: executor socket {}: {e}; dropped",
                socket.display()
            );
            let _ = done.send(run_id);
            return;
        }
    };
    let (read_half, write_half) = stream.into_split();
    let (mut from_exec, reader) = spawn_reader(read_half);
    let _reader = AbortOnDrop(reader);
    let (to_exec, writer) = spawn_writer(write_half);
    let _writer = AbortOnDrop(writer);
    let _ = to_exec.send(assign);

    let mut cancel_deadline: Option<Instant> = None;
    loop {
        let cancel = async {
            match cancel_deadline {
                Some(at) => sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            frame = inbound.recv() => {
                let Some(frame) = frame else { break };
                if matches!(frame, Frame::Cancel { .. }) && cancel_deadline.is_none() {
                    cancel_deadline = Some(Instant::now() + CANCEL_GRACE);
                }
                if to_exec.send(frame).is_err() {
                    break;
                }
            }
            frame = from_exec.recv() => {
                let Some(frame) = frame else {
                    eprintln!("keel-runner: run {run_id}: the executor closed without finishing");
                    break;
                };
                if !from_executor(&frame) || frame.run_id() != Some(run_id.as_str()) {
                    eprintln!(
                        "keel-runner: run {run_id}: the executor sent {}; closing the run",
                        frame.name()
                    );
                    break;
                }
                let terminal = matches!(frame, Frame::Refused { .. } | Frame::Finished { .. });
                if out.send(frame).is_err() || terminal {
                    break;
                }
            }
            () = cancel => {
                eprintln!("keel-runner: run {run_id}: no Finished {CANCEL_GRACE:?} after Cancel; closing the run");
                break;
            }
        }
    }
    let _ = done.send(run_id);
}
