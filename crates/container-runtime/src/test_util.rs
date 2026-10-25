//! Test doubles for the runtime seams, for this crate's tests and its consumers' (`test-util`).
//!
//! Nothing here touches a daemon: a [`ScriptedProcess`] replays lines, a [`ChannelSink`] records
//! events, and the mock runners answer from a table.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel, sync_channel};
use std::sync::{Arc, Mutex};

use crate::runner::{CmdOutput, RunError, Runner};
use crate::sink::{StreamEvent, StreamSink};
use crate::stream::{Killer, RunningProcess, StreamLine, StreamRunner};

/// A successful run that printed `stdout`.
pub fn ok(stdout: &str) -> Result<CmdOutput, RunError> {
    Ok(CmdOutput {
        success: true,
        stdout: stdout.to_owned(),
        stderr: String::new(),
    })
}

/// A run that exited non-zero, saying `stderr`.
pub fn failed(stderr: &str) -> Result<CmdOutput, RunError> {
    Ok(CmdOutput {
        success: false,
        stdout: String::new(),
        stderr: stderr.to_owned(),
    })
}

/// Answers by binary name from a table. A binary not in the table is not installed.
pub struct MockRunner(HashMap<String, Result<CmdOutput, RunError>>);

impl MockRunner {
    pub fn new<'a>(
        entries: impl IntoIterator<Item = (&'a str, Result<CmdOutput, RunError>)>,
    ) -> Self {
        Self(
            entries
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect(),
        )
    }
}

impl Runner for MockRunner {
    fn run(&self, bin: &str, _args: &[&str]) -> Result<CmdOutput, RunError> {
        self.0.get(bin).cloned().unwrap_or(Err(RunError::NotFound))
    }
}

/// Records every argv it is asked to run (program first) and answers each with the same reply.
pub struct RecordingRunner {
    reply: Result<CmdOutput, RunError>,
    seen: Mutex<Vec<Vec<String>>>,
}

impl RecordingRunner {
    /// Answers every call with this exit status and output.
    pub fn new(success: bool, stdout: &str, stderr: &str) -> Self {
        Self::answering(Ok(CmdOutput {
            success,
            stdout: stdout.to_owned(),
            stderr: stderr.to_owned(),
        }))
    }

    /// Answers every call successfully with `stdout`.
    pub fn ok(stdout: &str) -> Self {
        Self::new(true, stdout, "")
    }

    pub fn answering(reply: Result<CmdOutput, RunError>) -> Self {
        Self {
            reply,
            seen: Mutex::new(Vec::new()),
        }
    }

    /// Every argv run so far, in order.
    pub fn calls(&self) -> Vec<Vec<String>> {
        self.seen.lock().expect("lock").clone()
    }

    /// The first argv run, or empty if nothing ran.
    pub fn argv(&self) -> Vec<String> {
        self.calls().into_iter().next().unwrap_or_default()
    }

    pub fn call_count(&self) -> usize {
        self.seen.lock().expect("lock").len()
    }
}

impl Runner for RecordingRunner {
    fn run(&self, bin: &str, args: &[&str]) -> Result<CmdOutput, RunError> {
        let mut argv = vec![bin.to_owned()];
        argv.extend(args.iter().map(|a| (*a).to_owned()));
        self.seen.lock().expect("lock").push(argv);
        self.reply.clone()
    }
}

/// A stream runner whose first spawn replays `lines` as a [`ScriptedProcess`]; later spawns
/// replay nothing. Records every argv it spawned (program first).
pub struct ScriptedStreamRunner {
    script: Mutex<Option<Vec<StreamLine>>>,
    seen: Mutex<Vec<Vec<String>>>,
}

impl ScriptedStreamRunner {
    pub fn new(lines: Vec<StreamLine>) -> Self {
        Self {
            script: Mutex::new(Some(lines)),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub fn calls(&self) -> Vec<Vec<String>> {
        self.seen.lock().expect("lock").clone()
    }
}

impl StreamRunner for ScriptedStreamRunner {
    fn spawn(&self, bin: &str, args: &[&str]) -> Result<Box<dyn RunningProcess>, RunError> {
        let mut argv = vec![bin.to_owned()];
        argv.extend(args.iter().map(|a| (*a).to_owned()));
        self.seen.lock().expect("lock").push(argv);
        let script = self.script.lock().expect("lock").take().unwrap_or_default();
        Ok(Box::new(ScriptedProcess::new(script)))
    }
}

/// A scripted process for tests: replays canned lines, then exits.
pub struct ScriptedProcess {
    rx: Receiver<StreamLine>,
    killed: Arc<std::sync::atomic::AtomicBool>,
}

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

impl RunningProcess for ScriptedProcess {
    fn lines(&self) -> &Receiver<StreamLine> {
        &self.rx
    }
    fn kill(&self) {
        self.killed
            .store(true, std::sync::atomic::Ordering::Release);
    }

    fn killer(&self) -> Killer {
        let killed = Arc::clone(&self.killed);
        Killer::new(move || {
            killed.store(true, std::sync::atomic::Ordering::Release);
        })
    }
}

/// A sink that forwards every event to a channel, for asserting on what a stream produced.
pub struct ChannelSink(Mutex<Sender<StreamEvent>>);

impl ChannelSink {
    pub fn new() -> (Self, Receiver<StreamEvent>) {
        let (tx, rx) = channel();
        (Self(Mutex::new(tx)), rx)
    }
}

impl StreamSink for ChannelSink {
    fn emit(&self, ev: StreamEvent) -> bool {
        self.0.lock().is_ok_and(|tx| tx.send(ev).is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn the_mock_runners_answer_from_their_scripts() {
        let mock = MockRunner::new([("docker", ok("29.8.1"))]);
        assert_eq!(mock.run("docker", &["version"]).unwrap().stdout, "29.8.1");
        assert!(matches!(mock.run("podman", &[]), Err(RunError::NotFound)));

        let rec = RecordingRunner::new(false, "", "refused");
        assert!(!rec.run("docker", &["ps", "-a"]).unwrap().success);
        assert_eq!(rec.argv(), vec!["docker", "ps", "-a"]);
        assert_eq!(rec.call_count(), 1);

        let streams = ScriptedStreamRunner::new(vec![StreamLine::Exited { success: true }]);
        let first: Vec<StreamLine> = streams
            .spawn("docker", &["logs"])
            .unwrap()
            .lines()
            .iter()
            .collect();
        assert_eq!(first.len(), 1);
        let second: Vec<StreamLine> = streams
            .spawn("docker", &["logs"])
            .unwrap()
            .lines()
            .iter()
            .collect();
        assert!(second.is_empty(), "the script plays once");
        assert_eq!(streams.calls().len(), 2);
    }
}
