//! Log follows, interactive shells and stats ticks over the docker CLI (T066).
//!
//! Every buffer here is bounded, because whatever reads these streams sits on the far side of
//! the internet:
//! - a log line longer than [`LINE_CAP`] is split, so one newline-free container cannot grow a
//!   line without limit;
//! - at most [`BACKLOG`] lines wait between the pipe readers and the batcher, which back-pressures
//!   the CLI rather than queueing;
//! - a flush window holds at most [`WINDOW_CAP`] lines and [`WINDOW_BYTES`]; past that the oldest
//!   are dropped and the next batch says `truncated`;
//! - one batch carries at most [`BATCH_LINES`] lines and [`BATCH_BYTES`].
//!
//! So one follow holds at most about `BACKLOG * LINE_CAP + WINDOW_BYTES` (1.5 MiB).

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
pub const LINE_CAP: usize = 8 * 1024;
/// Lines waiting between the pipe readers and the batcher.
pub const BACKLOG: usize = 64;
/// How long lines gather before they are sent.
pub const FLUSH_INTERVAL: Duration = Duration::from_millis(120);
/// The most lines one batch carries.
pub const BATCH_LINES: usize = 256;
/// The most lines one flush window holds before the oldest are dropped.
pub const WINDOW_CAP: usize = 4096;
/// The most bytes of lines one flush window holds before the oldest are dropped.
pub const WINDOW_BYTES: usize = 1024 * 1024;
/// The most bytes of lines one batch carries (a batch always carries at least one line).
pub const BATCH_BYTES: usize = 256 * 1024;

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
    bytes: usize,
    dropped: bool,
}

impl Batcher {
    fn new(stream_id: StreamId) -> Self {
        Self {
            stream_id,
            window: VecDeque::new(),
            bytes: 0,
            dropped: false,
        }
    }

    fn push(&mut self, line: String) {
        while !self.window.is_empty()
            && (self.window.len() >= WINDOW_CAP || self.bytes + line.len() > WINDOW_BYTES)
        {
            if let Some(old) = self.window.pop_front() {
                self.bytes -= old.len();
            }
            self.dropped = true;
        }
        self.bytes += line.len();
        self.window.push_back(line);
    }

    /// Sends what gathered, in batches of at most [`BATCH_LINES`]. `false` when the sink has
    /// nobody behind it.
    fn flush(&mut self, sink: &dyn StreamSink) -> bool {
        while !self.window.is_empty() {
            let mut take = 0;
            let mut size = 0;
            for l in self.window.iter().take(BATCH_LINES) {
                if take > 0 && size + l.len() > BATCH_BYTES {
                    break;
                }
                take += 1;
                size += l.len();
            }
            self.bytes -= size;
            let lines: Vec<String> = self.window.drain(..take).collect();
            if !sink.emit(StreamEvent::ContainerLogBatch {
                stream_id: self.stream_id,
                lines,
                truncated: std::mem::take(&mut self.dropped),
            }) {
                self.window.clear();
                self.bytes = 0;
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

    fn stopper(&self) -> Arc<dyn Fn() + Send + Sync> {
        match &self.0 {
            Some(s) => {
                let stopper = s.stopper();
                Arc::new(move || stopper.stop())
            }
            None => Arc::new(|| {}),
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct Got(Mutex<Vec<(usize, usize, bool)>>);

    impl StreamSink for Got {
        fn emit(&self, ev: StreamEvent) -> bool {
            if let StreamEvent::ContainerLogBatch {
                lines, truncated, ..
            } = ev
            {
                let bytes = lines.iter().map(String::len).sum();
                self.0.lock().unwrap().push((lines.len(), bytes, truncated));
            }
            true
        }
    }

    #[test]
    fn the_window_drops_the_oldest_past_its_byte_cap() {
        let mut b = Batcher::new(StreamId(1));
        let line = "x".repeat(LINE_CAP);
        for _ in 0..(WINDOW_BYTES / LINE_CAP + 10) {
            b.push(line.clone());
        }
        assert_eq!(b.bytes, WINDOW_BYTES);
        assert_eq!(b.window.len(), WINDOW_BYTES / LINE_CAP);
        let got = Got::default();
        assert!(b.flush(&got));
        let got = got.0.into_inner().unwrap();
        assert!(got[0].2, "the first batch says lines were dropped");
        assert!(got[1..].iter().all(|g| !g.2));
        assert!(got.iter().all(|g| g.1 <= BATCH_BYTES));
        assert_eq!(got.iter().map(|g| g.1).sum::<usize>(), WINDOW_BYTES);
        assert_eq!(b.bytes, 0);
    }

    #[test]
    fn short_lines_batch_by_count() {
        let mut b = Batcher::new(StreamId(1));
        for _ in 0..(BATCH_LINES + 1) {
            b.push("a".into());
        }
        let got = Got::default();
        b.flush(&got);
        let got = got.0.into_inner().unwrap();
        assert_eq!(
            got.iter().map(|g| g.0).collect::<Vec<_>>(),
            [BATCH_LINES, 1]
        );
    }
}
