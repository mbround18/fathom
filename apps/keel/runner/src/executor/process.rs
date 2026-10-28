//! Runs `ansible-playbook` in its own process group and streams what it produces.
//!
//! - `keel_events` lines arrive on fd 3 (`KEEL_EVENT_FD=3`); each line that parses is sent as an
//!   `Event`, verbatim; the last `stats` event gives `Finished.stats`.
//! - stdout and stderr are batched into `Log` frames every 100 ms or 64 KiB.
//! - `Cancel` or the deadline sends SIGINT to the group, and SIGKILL after the grace period. The
//!   socket closing (the network half gave up on the run) sends SIGKILL at once.
//! - When the leader exits, whatever is left in the group is killed.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::os::unix::process::ExitStatusExt as _;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use keel_exec::events::{HostStats, KeelEvent, parse_line};
use keel_exec::frames::{Frame, LogStream};
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, BufReader};
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep_until};

/// The fd `ansible-playbook` writes events to.
pub const EVENT_FD: i32 = 3;
const LOG_BATCH: usize = 64 * 1024;
const LOG_EVERY: Duration = Duration::from_millis(100);
/// How long output is drained after the process group is gone.
const DRAIN: Duration = Duration::from_secs(5);
/// Event lines longer than this are dropped.
const MAX_EVENT_LINE: usize = 4 * 1024 * 1024;

/// What to run.
#[derive(Debug)]
pub struct Invocation {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    /// The whole environment (it is cleared first). Values may be secret: never logged.
    pub env: Vec<(OsString, OsString)>,
}

/// How a run ended.
#[derive(Debug)]
pub struct Outcome {
    pub exit_code: i32,
    pub stats: BTreeMap<String, HostStats>,
}

enum Output {
    Log(LogStream, Vec<u8>),
    Event(String),
}

/// A pipe whose ends are both close-on-exec.
fn pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    // SAFETY: `fds` is a valid two-element array.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: pipe2 returned two fresh descriptors we now own.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

fn signal_group(pgid: i32, sig: i32) {
    if pgid > 0 {
        // SAFETY: plain syscall; a stale group only yields ESRCH.
        unsafe {
            libc::kill(-pgid, sig);
        }
    }
}

fn spawn_chunks<R>(r: R, stream: LogStream, tx: mpsc::Sender<Output>)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut r = r;
        let mut buf = vec![0u8; LOG_BATCH];
        loop {
            match r.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    if tx
                        .send(Output::Log(stream, buf[..n].to_vec()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
    });
}

/// Batches logs and numbers frames. `Event` and `Log` share one `seq`.
struct Emitter<'a> {
    run_id: &'a str,
    out: &'a mpsc::UnboundedSender<Frame>,
    seq: u64,
    pending: Vec<(LogStream, Vec<u8>)>,
    pending_len: usize,
    stats: BTreeMap<String, HostStats>,
}

impl Emitter<'_> {
    fn next_seq(&mut self) -> u64 {
        let s = self.seq;
        self.seq += 1;
        s
    }

    fn log(&mut self, stream: LogStream, bytes: Vec<u8>) {
        self.pending_len += bytes.len();
        match self.pending.last_mut() {
            Some((s, b)) if *s == stream => b.extend_from_slice(&bytes),
            _ => self.pending.push((stream, bytes)),
        }
        if self.pending_len >= LOG_BATCH {
            self.flush();
        }
    }

    fn flush(&mut self) {
        for (stream, bytes) in std::mem::take(&mut self.pending) {
            let seq = self.next_seq();
            let _ = self.out.send(Frame::Log {
                run_id: self.run_id.to_owned(),
                seq,
                stream,
                bytes,
            });
        }
        self.pending_len = 0;
    }

    fn event(&mut self, line: String) {
        match parse_line(&line) {
            Ok(event) => {
                if let KeelEvent::Stats { stats } = &event {
                    self.stats = stats.clone();
                }
                self.flush();
                let seq = self.next_seq();
                let _ = self.out.send(Frame::Event {
                    run_id: self.run_id.to_owned(),
                    seq,
                    json: line,
                });
            }
            Err(e) => eprintln!(
                "keel-executor: run {}: dropped an event line: {e}",
                self.run_id
            ),
        }
    }

    fn take(&mut self, o: Output) {
        match o {
            Output::Log(s, b) => self.log(s, b),
            Output::Event(l) => self.event(l),
        }
    }
}

/// Runs `inv` to completion. `incoming` carries the rest of the socket's frames (`Cancel`;
/// `None` once the socket closed).
pub async fn run(
    inv: Invocation,
    run_id: &str,
    out: &mpsc::UnboundedSender<Frame>,
    incoming: &mut mpsc::Receiver<Frame>,
    deadline: Instant,
    grace: Duration,
) -> std::io::Result<Outcome> {
    let (event_r, event_w) = pipe()?;
    let event_w_raw = event_w.as_raw_fd();

    let mut cmd = tokio::process::Command::new(&inv.program);
    cmd.args(&inv.args)
        .current_dir(&inv.cwd)
        .env_clear()
        .envs(inv.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // SAFETY: only async-signal-safe calls (setpgid, dup2, fcntl) between fork and exec.
    unsafe {
        cmd.pre_exec(move || {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if event_w_raw == EVENT_FD {
                let flags = libc::fcntl(EVENT_FD, libc::F_GETFD);
                if flags < 0 || libc::fcntl(EVENT_FD, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0
                {
                    return Err(std::io::Error::last_os_error());
                }
            } else if libc::dup2(event_w_raw, EVENT_FD) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;
    drop(event_w);
    let pgid = child.id().map_or(0, |p| p as i32);

    let (tx, mut rx) = mpsc::channel::<Output>(256);
    if let Some(stdout) = child.stdout.take() {
        spawn_chunks(stdout, LogStream::Stdout, tx.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_chunks(stderr, LogStream::Stderr, tx.clone());
    }
    let events = tokio::net::unix::pipe::Receiver::from_owned_fd(event_r)?;
    let etx = tx;
    tokio::spawn(async move {
        let mut r = BufReader::new(events);
        let mut line = Vec::new();
        loop {
            line.clear();
            match r.read_until(b'\n', &mut line).await {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    if line.len() > MAX_EVENT_LINE {
                        continue;
                    }
                    let Ok(text) = std::str::from_utf8(&line) else {
                        continue;
                    };
                    let text = text.trim_end_matches(['\n', '\r']);
                    if text.is_empty() {
                        continue;
                    }
                    if etx.send(Output::Event(text.to_owned())).await.is_err() {
                        return;
                    }
                }
            }
        }
    });

    let mut emit = Emitter {
        run_id,
        out,
        seq: 0,
        pending: Vec::new(),
        pending_len: 0,
        stats: BTreeMap::new(),
    };
    let mut flush = tokio::time::interval(LOG_EVERY);
    let mut kill_at: Option<Instant> = None;
    let mut interrupted = false;
    let mut socket_open = true;

    let status = loop {
        let kill = async {
            match kill_at {
                Some(at) => sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            status = child.wait() => break status?,
            Some(o) = rx.recv() => emit.take(o),
            _ = flush.tick() => emit.flush(),
            frame = incoming.recv(), if socket_open => match frame {
                Some(Frame::Cancel { .. }) if !interrupted => {
                    interrupted = true;
                    signal_group(pgid, libc::SIGINT);
                    kill_at = Some(Instant::now() + grace);
                }
                Some(_) => {}
                None => {
                    socket_open = false;
                    signal_group(pgid, libc::SIGKILL);
                }
            },
            () = sleep_until(deadline), if !interrupted => {
                interrupted = true;
                eprintln!("keel-executor: run {run_id}: deadline reached; interrupting");
                signal_group(pgid, libc::SIGINT);
                kill_at = Some(Instant::now() + grace);
            }
            () = kill => {
                signal_group(pgid, libc::SIGKILL);
                kill_at = None;
            }
        }
    };
    // Anything the playbook left behind in its group goes with it.
    signal_group(pgid, libc::SIGKILL);
    let drain_until = Instant::now() + DRAIN;
    loop {
        tokio::select! {
            o = rx.recv() => match o {
                Some(o) => emit.take(o),
                None => break,
            },
            () = sleep_until(drain_until) => break,
        }
    }
    emit.flush();
    let exit_code = status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0));
    Ok(Outcome {
        exit_code,
        stats: emit.stats,
    })
}
