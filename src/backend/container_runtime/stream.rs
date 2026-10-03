//! A long-running invocation seam, alongside the one-shot `Runner`.
//!
//! `Runner::run` waits for the process to exit and hands back its whole output, which is right for
//! `docker ps` and wrong for `docker logs --follow` and `docker build`: those produce output for
//! minutes and must be readable while they run, and killable without waiting for them to finish.
//! `Runner` stays exactly as it is — its tests must keep passing untouched — and this trait sits
//! next to it for the streaming cases.
//!
//! The seam is a trait for the same reason `Runner` is: nearly every test in this feature should
//! run without a container daemon.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};

use super::super::kind::RunError;

/// One line of a running process's output, tagged with which stream it came from.
///
/// stderr is kept separate rather than interleaved: a build writes its progress to stderr, and a
/// log follow writes the container's own stderr there. Merging them would make it impossible to
/// tell Fathom's tooling noise from the container's words, and FR-004's "the runtime's own
/// message, verbatim" needs the distinction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamLine {
    Stdout(String),
    Stderr(String),
    /// The process exited. Always the last item, exactly once.
    Exited {
        success: bool,
    },
}

/// A running invocation. Dropping it kills the process.
pub trait RunningProcess: Send {
    /// Lines as they arrive. The channel closes after [`StreamLine::Exited`].
    fn lines(&self) -> &Receiver<StreamLine>;
    /// Asks the process to stop. Idempotent — stopping a process that already exited is not an
    /// error, because a follow whose container died races with the operator closing the pane.
    fn kill(&self);
}

/// Spawns a long-running invocation. Argument arrays only; never a shell string.
pub trait StreamRunner: Send + Sync {
    fn spawn(&self, bin: &str, args: &[&str]) -> Result<Box<dyn RunningProcess>, RunError>;
}

/// How many lines may sit unread before the producer blocks.
///
/// Bounded on purpose. A container writing a megabyte a second into a pane nobody is reading must
/// slow down rather than grow a queue until the process is out of memory (Principle V). The reader
/// marks what it dropped; it does not pretend to have everything.
const BACKLOG: usize = 1024;

pub struct SystemStreamRunner;

impl StreamRunner for SystemStreamRunner {
    fn spawn(&self, bin: &str, args: &[&str]) -> Result<Box<dyn RunningProcess>, RunError> {
        let mut child = match Command::new(bin)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(RunError::NotFound),
            Err(e) => return Err(RunError::Other(e.to_string())),
        };

        let (tx, rx) = sync_channel(BACKLOG);
        if let Some(out) = child.stdout.take() {
            pump(out, tx.clone(), StreamLine::Stdout);
        }
        if let Some(err) = child.stderr.take() {
            pump(err, tx.clone(), StreamLine::Stderr);
        }

        let handle = Arc::new(Mutex::new(child));
        let waiter = Arc::clone(&handle);
        std::thread::spawn(move || {
            let success = waiter
                .lock()
                .is_ok_and(|mut c| c.wait().is_ok_and(|s| s.success()));
            // Best-effort: the receiver may already be gone because the operator closed the pane.
            drop(tx.send(StreamLine::Exited { success }));
        });

        Ok(Box::new(SpawnedProcess { rx, handle }))
    }
}

fn pump<R: std::io::Read + Send + 'static>(
    reader: R,
    tx: SyncSender<StreamLine>,
    wrap: fn(String) -> StreamLine,
) {
    std::thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            let Ok(line) = line else { break };
            if tx.send(wrap(line)).is_err() {
                break;
            }
        }
    });
}

struct SpawnedProcess {
    rx: Receiver<StreamLine>,
    handle: Arc<Mutex<Child>>,
}

impl RunningProcess for SpawnedProcess {
    fn lines(&self) -> &Receiver<StreamLine> {
        &self.rx
    }

    fn kill(&self) {
        if let Ok(mut child) = self.handle.lock() {
            drop(child.kill());
        }
    }
}

impl Drop for SpawnedProcess {
    fn drop(&mut self) {
        self.kill();
    }
}

/// A scripted process for tests: replays canned lines, then exits.
#[cfg(test)]
pub struct ScriptedProcess {
    rx: Receiver<StreamLine>,
    killed: Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(test)]
impl ScriptedProcess {
    pub fn new(lines: Vec<StreamLine>) -> Self {
        let (tx, rx) = sync_channel(lines.len().max(1));
        for line in lines {
            drop(tx.send(line));
        }
        Self {
            rx,
            killed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub fn was_killed(&self) -> bool {
        self.killed.load(std::sync::atomic::Ordering::Acquire)
    }
}

#[cfg(test)]
impl RunningProcess for ScriptedProcess {
    fn lines(&self) -> &Receiver<StreamLine> {
        &self.rx
    }
    fn kill(&self) {
        self.killed
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `true` is on every system that can run this crate's tests, and needs no daemon.
    #[test]
    fn a_process_that_exits_cleanly_reports_success_last() {
        let proc = SystemStreamRunner
            .spawn("sh", &["-c", "echo one; echo two"])
            .expect("sh exists");
        let collected: Vec<StreamLine> = proc.lines().iter().collect();
        assert_eq!(
            collected.last(),
            Some(&StreamLine::Exited { success: true }),
            "{collected:?}"
        );
        assert!(collected.contains(&StreamLine::Stdout("one".into())));
        assert!(collected.contains(&StreamLine::Stdout("two".into())));
    }

    /// stderr stays its own stream, so the runtime's own words are never mistaken for output.
    #[test]
    fn stderr_is_not_interleaved_into_stdout() {
        let proc = SystemStreamRunner
            .spawn("sh", &["-c", "echo out; echo problem >&2"])
            .expect("sh exists");
        let collected: Vec<StreamLine> = proc.lines().iter().collect();
        assert!(collected.contains(&StreamLine::Stdout("out".into())));
        assert!(collected.contains(&StreamLine::Stderr("problem".into())));
    }

    #[test]
    fn a_failing_process_says_so_rather_than_going_quiet() {
        let proc = SystemStreamRunner
            .spawn("sh", &["-c", "exit 3"])
            .expect("sh exists");
        let collected: Vec<StreamLine> = proc.lines().iter().collect();
        assert_eq!(
            collected.last(),
            Some(&StreamLine::Exited { success: false })
        );
    }

    #[test]
    fn a_missing_binary_is_not_found_not_a_generic_failure() {
        let Err(err) = SystemStreamRunner.spawn("fathom-no-such-binary-048", &[]) else {
            panic!("a binary that does not exist must not spawn");
        };
        assert_eq!(err, RunError::NotFound);
    }

    /// A follow the operator closed must stop the process, not leave it running for the life of
    /// the app (Principle V: under 2% of one core when idle).
    #[test]
    fn killing_a_follow_ends_it() {
        let proc = SystemStreamRunner
            .spawn("sh", &["-c", "while true; do echo tick; sleep 0.05; done"])
            .expect("sh exists");
        assert_eq!(
            proc.lines().recv().ok(),
            Some(StreamLine::Stdout("tick".into()))
        );
        proc.kill();
        // Drains whatever was already buffered, then ends. Without the kill this loops forever.
        let tail: Vec<StreamLine> = proc.lines().iter().collect();
        assert!(
            tail.iter().any(|l| matches!(l, StreamLine::Exited { .. })),
            "the stream closed without an exit marker: {tail:?}"
        );
    }

    #[test]
    fn a_scripted_process_replays_its_lines_and_records_the_kill() {
        let proc = ScriptedProcess::new(vec![
            StreamLine::Stdout("hello from busybox".into()),
            StreamLine::Exited { success: true },
        ]);
        let collected: Vec<StreamLine> = proc.lines().iter().collect();
        assert_eq!(collected.len(), 2);
        assert!(!proc.was_killed());
        proc.kill();
        assert!(proc.was_killed());
    }
}
