//! Log follows, interactive shells and stats ticks over the docker CLI (T066).
//!
//! Every buffer here is bounded, because whatever reads these streams sits on the far side of
//! the internet:
//! - a log line longer than [`LINE_CAP`] is split, so one newline-free container cannot grow a
//!   line without limit;
//! - at most [`BACKLOG`] lines wait between the pipe readers and the batcher, which back-pressures
//!   the CLI rather than queueing;
//! - a flush window holds at most [`WINDOW_CAP`] lines; past that the oldest are dropped and the
//!   next batch says `truncated`.

use std::collections::VecDeque;
use std::io::{BufRead as _, BufReader, Read};
use std::os::unix::process::CommandExt as _;
use std::process::{Child, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::time::{Duration, Instant};

use container_runtime::cli::Cli;
use container_runtime::session::{self, Session};
use depths_core::plan::RunStep;
use depths_core::ports::{
    CancelToken, RuntimeError, ShellSession, StreamEvent, StreamId, StreamSink, UsageSample,
};

use crate::{CliRuntime, Mapped};

/// The longest log line passed on whole; longer ones arrive as several.
pub const LINE_CAP: usize = 16 * 1024;
/// Lines waiting between the pipe readers and the batcher.
pub const BACKLOG: usize = 1024;
/// How long lines gather before they are sent.
pub const FLUSH_INTERVAL: Duration = Duration::from_millis(120);
/// The most lines one batch carries.
pub const BATCH_LINES: usize = 256;
/// The most lines one flush window holds before the oldest are dropped.
pub const WINDOW_CAP: usize = 4096;

const POLL: Duration = Duration::from_millis(50);

impl CliRuntime {
    pub(crate) fn follow(
        &self,
        step: &RunStep,
        stream_id: StreamId,
        sink: &dyn StreamSink,
        cancel: &CancelToken,
    ) -> Result<(), RuntimeError> {
        let args = self.args_of(&step.argv)?;
        let mut cmd = self.command(args, Some(&step.cwd));
        // Its own process group, so a stop reaches `docker compose`'s plugin process too.
        cmd.process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| self.spawn_error(&e))?;

        let (tx, rx) = mpsc::sync_channel::<Line>(BACKLOG);
        let mut readers = Vec::new();
        if let Some(out) = child.stdout.take() {
            readers.push(read_lines(out, false, tx.clone()));
        }
        if let Some(err) = child.stderr.take() {
            readers.push(read_lines(err, true, tx.clone()));
        }
        drop(tx);

        let mut batcher = Batcher::new(stream_id);
        let mut complaint: Option<String> = None;
        let mut stopped = false;
        let mut last_flush = Instant::now();
        loop {
            if cancel.is_cancelled() {
                stopped = true;
                break;
            }
            match rx.recv_timeout(POLL) {
                Ok(line) => {
                    if line.stderr {
                        complaint = Some(line.text.clone());
                    }
                    batcher.push(line.text);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if last_flush.elapsed() >= FLUSH_INTERVAL {
                last_flush = Instant::now();
                if !batcher.flush(sink) {
                    // Nobody is listening any more.
                    stopped = true;
                    break;
                }
            }
        }
        if stopped {
            kill_group(&child);
            drop(rx);
        } else {
            batcher.flush(sink);
        }
        for r in readers {
            let _ = r.join();
        }
        let status = child
            .wait()
            .map_err(|e| RuntimeError::Failed(format!("waiting for the log follow: {e}")))?;
        if stopped || status.success() {
            Ok(())
        } else {
            Err(RuntimeError::Failed(complaint.unwrap_or_else(|| {
                format!("the log follow ended: {status}")
            })))
        }
    }

    pub(crate) fn shell(
        &self,
        container_id: &str,
        shell: &str,
        (cols, rows): (u16, u16),
        stream_id: StreamId,
        sink: Arc<dyn StreamSink>,
    ) -> Result<Box<dyn ShellSession>, RuntimeError> {
        let bin = self.docker_bin.to_str().ok_or_else(|| {
            RuntimeError::Unavailable("the docker binary path is not UTF-8".into())
        })?;
        let args = session::argv(container_id, Some(shell));
        let session = Session::open(stream_id, bin, &args, Forward(sink))
            .map_err(|e| self.spawn_error(&e))?;
        session.resize(cols, rows);
        Ok(Box::new(CliShell(Some(session))))
    }

    pub(crate) fn sample(
        &self,
        ids: &[String],
        at: &str,
    ) -> Result<Vec<UsageSample>, RuntimeError> {
        container_runtime::stats::sample_with(Cli::docker(), &Mapped(self), ids, at)
            .map_err(|e| self.unavailable(e))
    }
}

fn kill_group(child: &Child) {
    let pid = i32::try_from(child.id())
        .ok()
        .and_then(rustix::process::Pid::from_raw);
    if let Some(pid) = pid {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
}

struct Line {
    stderr: bool,
    text: String,
}

/// Reads `from` in lines of at most [`LINE_CAP`] bytes, lossily decoded, into `tx`.
fn read_lines(
    from: impl Read + Send + 'static,
    stderr: bool,
    tx: SyncSender<Line>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(from);
        let mut buf = Vec::with_capacity(1024);
        loop {
            buf.clear();
            match (&mut reader)
                .take(LINE_CAP as u64)
                .read_until(b'\n', &mut buf)
            {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    if buf.last() == Some(&b'\n') {
                        buf.pop();
                        if buf.last() == Some(&b'\r') {
                            buf.pop();
                        }
                    }
                    let text = String::from_utf8_lossy(&buf).into_owned();
                    if tx.send(Line { stderr, text }).is_err() {
                        return;
                    }
                }
            }
        }
    })
}

/// The flush window: bounded, oldest dropped first, the gap marked.
struct Batcher {
    stream_id: StreamId,
    window: VecDeque<String>,
    dropped: bool,
}

impl Batcher {
    fn new(stream_id: StreamId) -> Self {
        Self {
            stream_id,
            window: VecDeque::new(),
            dropped: false,
        }
    }

    fn push(&mut self, line: String) {
        if self.window.len() >= WINDOW_CAP {
            self.window.pop_front();
            self.dropped = true;
        }
        self.window.push_back(line);
    }

    /// Sends what gathered, in batches of at most [`BATCH_LINES`]. `false` when the sink has
    /// nobody behind it.
    fn flush(&mut self, sink: &dyn StreamSink) -> bool {
        while !self.window.is_empty() {
            let take = self.window.len().min(BATCH_LINES);
            let lines: Vec<String> = self.window.drain(..take).collect();
            if !sink.emit(StreamEvent::ContainerLogBatch {
                stream_id: self.stream_id,
                lines,
                truncated: std::mem::take(&mut self.dropped),
            }) {
                self.window.clear();
                return false;
            }
        }
        true
    }
}

/// `Session::open` wants an owned sink.
struct Forward(Arc<dyn StreamSink>);

impl StreamSink for Forward {
    fn emit(&self, ev: StreamEvent) -> bool {
        self.0.emit(ev)
    }
}

/// A [`Session`] behind the port. Dropping it closes the session.
struct CliShell(Option<Session>);

impl ShellSession for CliShell {
    fn write(&mut self, data: &[u8]) -> Result<(), RuntimeError> {
        match &mut self.0 {
            Some(s) => s
                .write_bytes(data)
                .map_err(|e| RuntimeError::Failed(format!("the shell is gone: {e}"))),
            None => Err(RuntimeError::Failed("the shell is closed".into())),
        }
    }

    fn resize(&self, cols: u16, rows: u16) {
        if let Some(s) = &self.0 {
            s.resize(cols, rows);
        }
    }

    fn close(mut self: Box<Self>) {
        if let Some(s) = self.0.take() {
            s.close();
        }
    }
}

impl Drop for CliShell {
    fn drop(&mut self) {
        if let Some(s) = self.0.take() {
            s.close();
        }
    }
}
