//! 048-docker-management US5: following one container's log output.
//!
//! An adapter, not a second log pipeline. The registry, the batching cadence, the single
//! `StreamClosed` per stream and the "a restart under the same id is silent" rule all come from
//! [`crate::backend::log_streams`], which pod log follows already use; what this module adds is
//! the argv and the pump that turns [`StreamLine`]s into `ContainerLogBatch` events.
//!
//! Reading a container's output is allowed whether or not its project has been adopted (FR-035).
//! That is not an oversight in the gate: reading is how an operator decides whether to adopt, and
//! output a process already wrote is not a change to anyone's project. `adoption::requirement`
//! classifies `StartContainerLogStream` as `Free` for exactly that reason, while a session — which
//! is unbounded execution inside someone else's work — stays gated.

use std::sync::Arc;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use super::super::log_streams::{CloseGuard, StreamSink};
use super::stream::{Killer, RunningProcess, StreamLine, StreamRunner};
use crate::protocol::StreamEvent;

/// How long lines are allowed to accumulate before they are sent.
///
/// A line per event would make a chatty container a few thousand envelopes a second for no gain:
/// nobody reads faster than the screen repaints. A quiet container pays nothing for this, because
/// the wait is a `recv_timeout` on the line channel and not a poll.
const FLUSH_INTERVAL: Duration = Duration::from_millis(120);

/// The most lines one batch may carry. A burst past this is sent as several batches.
const BATCH_LINES: usize = 256;

/// The most lines one flush window may hold before the oldest are dropped.
///
/// Bounding is the point (Principle V). A container writing faster than the channel drains must
/// cost a marked gap in the operator's view, not an ever-growing queue — and the gap is marked,
/// which is the whole difference between bounding and quietly lying.
const WINDOW_CAP: usize = 4096;

/// `docker logs` for one container.
///
/// Timestamps always: a log pane whose lines cannot be placed in time is a log pane that cannot be
/// compared with anything else the operator is looking at. `--tail` is passed only when asked for,
/// because the runtime's own default is the whole retained log and that is the right default for
/// a container that has already exited (FR-035: its retained output is still readable).
pub fn argv(container_id: &str, follow: bool, tail_lines: Option<u32>) -> Vec<String> {
    let mut args = vec!["logs".to_owned(), "--timestamps".to_owned()];
    if let Some(n) = tail_lines {
        args.push("--tail".to_owned());
        args.push(n.to_string());
    }
    if follow {
        args.push("--follow".to_owned());
    }
    args.push(container_id.to_owned());
    args
}

/// Kills the process when the follow's future goes away.
///
/// `StopContainerLogStream`, a context switch and shutdown all end a follow by aborting its task,
/// which drops this. Without it a quiet container's `docker logs --follow` would outlive the pane
/// that asked for it: the pump is parked in a blocking `recv` and would never notice.
struct KillOnDrop(Killer);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        self.0.kill();
    }
}

/// Follows one container's output until it ends, fails, or the stream is stopped.
///
/// Spawned through `LogStreams::spawn`, so `guard` emits this stream's one `StreamClosed` however
/// the follow ends.
pub async fn follow(
    mut guard: CloseGuard,
    runner: Arc<dyn StreamRunner>,
    bin: String,
    args: Vec<String>,
) {
    let sink = guard.sink().clone();
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    let proc = match runner.spawn(&bin, &borrowed) {
        Ok(p) => p,
        // The runtime's own words, not a summary of them (FR-004).
        Err(e) => {
            guard.failed(match e {
                super::super::kind::RunError::NotFound => {
                    "The container runtime binary is no longer on PATH.".to_owned()
                }
                super::super::kind::RunError::Other(detail) => detail,
            });
            return;
        }
    };
    let stop = KillOnDrop(proc.killer());

    // The line channel is synchronous, so the pump owns the process and lives on a blocking
    // thread. Dropping this future cannot abort that thread — it ends because `stop` above kills
    // the process, the pipes reach EOF and `recv` disconnects.
    let pumped = tokio::task::spawn_blocking(move || pump(proc.as_ref(), &sink)).await;
    drop(stop);
    match pumped {
        Ok(Ok(())) => guard.ended(),
        Ok(Err(detail)) => guard.failed(detail),
        // The pump panicked. The guard's default outcome ("stopped") would claim the operator did
        // this, so say what happened instead.
        Err(e) => guard.failed(e.to_string()),
    }
}

/// Drains the process's lines into batched events. `Err` carries the runtime's own failure text.
fn pump(proc: &dyn RunningProcess, sink: &StreamSink) -> Result<(), String> {
    let mut window: Vec<String> = Vec::new();
    let mut dropped = false;
    // What the runtime said on stderr. Kept rather than emitted as output: a `docker logs` that
    // fails says why on stderr, and that sentence belongs in the close event where the operator is
    // told the follow ended, not in the middle of the container's own words.
    let mut complaint: Option<String> = None;

    loop {
        match proc.lines().recv_timeout(FLUSH_INTERVAL) {
            Ok(StreamLine::Stdout(line)) => {
                if window.len() >= WINDOW_CAP {
                    window.remove(0);
                    dropped = true;
                }
                window.push(line);
            }
            Ok(StreamLine::Stderr(line)) => complaint = Some(line),
            Ok(StreamLine::Exited { success }) => {
                flush(sink, &mut window, &mut dropped);
                return if success {
                    Ok(())
                } else {
                    Err(complaint.unwrap_or_else(|| {
                        "The container runtime stopped following this container's output.".to_owned()
                    }))
                };
            }
            Err(RecvTimeoutError::Timeout) => flush(sink, &mut window, &mut dropped),
            // The process is gone without an `Exited`: the pipes closed under us. Treating that as
            // a clean end is the honest reading — there is no failure to report and no more output
            // to wait for.
            Err(RecvTimeoutError::Disconnected) => {
                flush(sink, &mut window, &mut dropped);
                return Ok(());
            }
        }
    }
}

/// Sends everything that has accumulated, in batches no larger than `BATCH_LINES`.
///
/// Several batches rather than one: a container that wrote a burst must not turn into a single
/// envelope big enough to stall the channel it is sent on. Only the first batch of a flush carries
/// `truncated`, because that is the one the dropped lines came before — marking all of them would
/// make one gap read as several.
fn flush(sink: &StreamSink, window: &mut Vec<String>, dropped: &mut bool) {
    while !window.is_empty() {
        let take = window.len().min(BATCH_LINES);
        let lines: Vec<String> = window.drain(..take).collect();
        sink.emit(StreamEvent::ContainerLogBatch {
            stream_id: sink.stream_id,
            lines,
            truncated: std::mem::take(dropped),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{BackendEvent, BackendEventEnvelope, Route, StreamId, section_keys};
    use crossbeam_channel::{Receiver, unbounded};
    use std::sync::atomic::AtomicU64;

    use super::super::stream::ScriptedProcess;

    fn sink(tx: &crossbeam_channel::Sender<BackendEventEnvelope>) -> StreamSink {
        StreamSink {
            stream_id: StreamId(7),
            route: Route::global(section_keys::system()),
            seq: Arc::new(AtomicU64::new(0)),
            sender: tx.clone(),
        }
    }

    /// Every `ContainerLogBatch` the pump sent, as (lines, truncated).
    fn batches(rx: &Receiver<BackendEventEnvelope>) -> Vec<(Vec<String>, bool)> {
        let mut out = Vec::new();
        while let Ok(env) = rx.try_recv() {
            if let BackendEvent::Stream(StreamEvent::ContainerLogBatch {
                lines, truncated, ..
            }) = env.event
            {
                out.push((lines, truncated));
            }
        }
        out
    }

    #[test]
    fn a_tail_is_only_asked_for_when_one_was_asked_for() {
        assert_eq!(
            argv("abc", true, None),
            vec!["logs", "--timestamps", "--follow", "abc"]
        );
        assert_eq!(
            argv("abc", false, Some(50)),
            vec!["logs", "--timestamps", "--tail", "50", "abc"]
        );
    }

    /// FR-035: a container that has already exited still shows the output it retained. The follow
    /// ends immediately, which must be an end and not a failure — there is nothing wrong.
    #[test]
    fn a_stopped_container_yields_its_retained_output_and_then_ends() {
        let (tx, rx) = unbounded();
        let proc = ScriptedProcess::new(vec![
            StreamLine::Stdout("2026-10-02T12:00:00Z booted".into()),
            StreamLine::Stdout("2026-10-02T12:00:01Z exiting".into()),
            StreamLine::Exited { success: true },
        ]);
        assert_eq!(pump(&proc, &sink(&tx)), Ok(()));
        assert_eq!(
            batches(&rx),
            vec![(
                vec![
                    "2026-10-02T12:00:00Z booted".to_owned(),
                    "2026-10-02T12:00:01Z exiting".to_owned()
                ],
                false
            )]
        );
    }

    /// A container that wrote nothing is not an error and not a batch: an empty batch would make
    /// the pane say "no output yet" and then say it again, which is noise, not news.
    #[test]
    fn a_container_that_wrote_nothing_sends_no_batch() {
        let (tx, rx) = unbounded();
        let proc = ScriptedProcess::new(vec![StreamLine::Exited { success: true }]);
        assert_eq!(pump(&proc, &sink(&tx)), Ok(()));
        assert!(batches(&rx).is_empty());
    }

    /// FR-004: the runtime's own sentence, verbatim, rather than Fathom's guess at what it meant.
    #[test]
    fn a_failed_follow_reports_what_the_runtime_said() {
        let (tx, _rx) = unbounded();
        let proc = ScriptedProcess::new(vec![
            StreamLine::Stderr("Error: No such container: ghost".into()),
            StreamLine::Exited { success: false },
        ]);
        assert_eq!(
            pump(&proc, &sink(&tx)),
            Err("Error: No such container: ghost".to_owned())
        );
    }

    /// A failure the runtime did not explain still has to end as a failure rather than as a
    /// silently clean stream: "it stopped and we do not know why" is information.
    #[test]
    fn a_silent_failure_is_still_a_failure() {
        let (tx, _rx) = unbounded();
        let proc = ScriptedProcess::new(vec![StreamLine::Exited { success: false }]);
        assert!(pump(&proc, &sink(&tx)).is_err());
    }

    /// The bound holds and says so. The cap is what keeps a chatty container from growing the
    /// process; `truncated` is what keeps it from lying about having shown everything.
    #[test]
    fn output_past_the_window_cap_is_dropped_and_marked() {
        let (tx, rx) = unbounded();
        let mut script: Vec<StreamLine> = (0..WINDOW_CAP + 10)
            .map(|i| StreamLine::Stdout(format!("line {i}")))
            .collect();
        script.push(StreamLine::Exited { success: true });
        let proc = ScriptedProcess::new(script);
        assert_eq!(pump(&proc, &sink(&tx)), Ok(()));

        let sent = batches(&rx);
        assert!(
            sent.iter().any(|(_, truncated)| *truncated),
            "dropping lines to stay bounded must be marked, not hidden"
        );
        let total: usize = sent.iter().map(|(lines, _)| lines.len()).sum();
        assert!(
            total <= WINDOW_CAP,
            "the pump kept {total} lines, past its own cap of {WINDOW_CAP}"
        );
        // The newest line survived: a bounded log view that drops the end rather than the start is
        // a log view that cannot be used to watch something happen.
        assert_eq!(
            sent.last().and_then(|(lines, _)| lines.last()).cloned(),
            Some(format!("line {}", WINDOW_CAP + 9))
        );
    }

    /// Each batch stays within its cap, so a burst arrives as several batches rather than one
    /// envelope big enough to stall the channel it is sent on.
    #[test]
    fn a_burst_arrives_as_several_capped_batches() {
        let (tx, rx) = unbounded();
        let mut script: Vec<StreamLine> = (0..BATCH_LINES * 2 + 3)
            .map(|i| StreamLine::Stdout(format!("line {i}")))
            .collect();
        script.push(StreamLine::Exited { success: true });
        let proc = ScriptedProcess::new(script);
        assert_eq!(pump(&proc, &sink(&tx)), Ok(()));

        let sent = batches(&rx);
        assert!(sent.len() >= 3, "{} batches", sent.len());
        assert!(sent.iter().all(|(lines, _)| lines.len() <= BATCH_LINES));
    }

    /// Closing the pane must stop the process, not leave it following a container nobody is
    /// reading — the one way a log pane can cost CPU after it is gone (Principle V).
    #[test]
    fn dropping_the_follow_kills_the_process() {
        let proc = ScriptedProcess::new(vec![StreamLine::Exited { success: true }]);
        let killer = proc.killer();
        assert!(!proc.was_killed());
        drop(KillOnDrop(killer));
        assert!(proc.was_killed());
    }
}
