//! Test doubles for the runtime seams, for this crate's tests and its consumers' (`test-util`).
//!
//! Nothing here touches a daemon: a [`ScriptedProcess`] replays lines, a [`ChannelSink`] records
//! events, and the mock runners answer from a table.

use std::sync::mpsc::{Receiver, Sender, channel, sync_channel};
use std::sync::{Arc, Mutex};

use crate::sink::{StreamEvent, StreamSink};
use crate::stream::{Killer, RunningProcess, StreamLine};

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
}
