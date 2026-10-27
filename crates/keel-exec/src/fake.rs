//! An in-process runner test double, behind the `fake` feature (T067).
//!
//! [`FakeRunner`] speaks the `keel-runner-v1` frames over any byte stream: it sends `Hello`, then
//! answers each `Assign` with a scripted run (`Started`, `Event`/`Log`, `Finished`), a `Refused`,
//! or a run that hangs until `Cancel`. It answers `Ping` with `Pong` unless told not to, and
//! records every frame it receives so a test can assert on what the server sent.
//!
//! A run starts once the pack has arrived: immediately for `pack_size == 0`, otherwise on the
//! `Pack` with `last: true`. Nothing is checked out or executed.

use std::collections::{BTreeMap, HashMap, HashSet};

use tokio::io::{AsyncRead, AsyncWrite, DuplexStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::codec::{self, CodecError};
use crate::events::HostStats;
use crate::frames::{Frame, LogStream, Refusal};

/// The exit code a cancelled hanging run finishes with (as after SIGINT).
pub const CANCELLED_EXIT_CODE: i32 = 130;

/// One step of a scripted run, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// A `keel_events` line, sent verbatim as `Event.json`.
    Event(String),
    Log(LogStream, Vec<u8>),
}

/// How a scripted run ends.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// `Started`, the steps, then `Finished`.
    Finish,
    /// `Refused` instead of `Started`; no steps are sent.
    Refuse { reason: Refusal, detail: String },
    /// `Started`, the steps, then nothing until `Cancel`, which yields `Finished` with
    /// [`CANCELLED_EXIT_CODE`].
    Hang,
}

/// What the fake does with one `Assign`.
#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    pub steps: Vec<Step>,
    pub outcome: Outcome,
    pub exit_code: i32,
    pub stats: BTreeMap<String, HostStats>,
    pub ledger_line: String,
}

impl Script {
    /// A run that finishes with `exit_code`.
    pub fn finish(exit_code: i32) -> Self {
        Script {
            steps: Vec::new(),
            outcome: Outcome::Finish,
            exit_code,
            stats: BTreeMap::new(),
            ledger_line: String::new(),
        }
    }

    /// A run that is refused before it starts.
    pub fn refuse(reason: Refusal, detail: impl Into<String>) -> Self {
        Script { outcome: Outcome::Refuse { reason, detail: detail.into() }, ..Script::finish(0) }
    }

    /// A run that starts and never finishes on its own.
    pub fn hang() -> Self {
        Script { outcome: Outcome::Hang, ..Script::finish(0) }
    }

    pub fn event(mut self, line: impl Into<String>) -> Self {
        self.steps.push(Step::Event(line.into()));
        self
    }

    pub fn log(mut self, stream: LogStream, bytes: impl Into<Vec<u8>>) -> Self {
        self.steps.push(Step::Log(stream, bytes.into()));
        self
    }

    pub fn stats(mut self, host: impl Into<String>, stats: HostStats) -> Self {
        self.stats.insert(host.into(), stats);
        self
    }

    pub fn ledger_line(mut self, line: impl Into<String>) -> Self {
        self.ledger_line = line.into();
        self
    }
}

/// A scripted runner session.
#[derive(Debug, Clone)]
pub struct FakeRunner {
    hello: Frame,
    scripts: HashMap<String, Script>,
    default_script: Script,
    answer_pings: bool,
}

/// The test's side of a spawned fake.
#[derive(Debug)]
pub struct FakeHandle {
    /// Every frame the fake received, in order.
    pub received: mpsc::UnboundedReceiver<Frame>,
    /// Resolves when the session ends (stream closed or a codec error).
    pub task: JoinHandle<Result<(), CodecError>>,
}

struct Pending {
    run_id: String,
    script: Script,
}

impl FakeRunner {
    /// A runner for `host_id` with an empty ledger, allowing the `base` profile. Unscripted jobs
    /// finish with exit code 0.
    pub fn new(host_id: impl Into<String>) -> Self {
        FakeRunner {
            hello: Frame::Hello {
                host_id: host_id.into(),
                version: env!("CARGO_PKG_VERSION").into(),
                ansible_core: "2.19.0".into(),
                python: "3.12.3".into(),
                allowed_profiles: vec!["base".into()],
                require_signed: false,
                override_quorum: 1,
                ledger_head: None,
                bases: Vec::new(),
            },
            scripts: HashMap::new(),
            default_script: Script::finish(0),
            answer_pings: true,
        }
    }

    /// Replace the `Hello` sent on connect (any frame, so tests can send a bad one).
    pub fn with_hello(mut self, hello: Frame) -> Self {
        self.hello = hello;
        self
    }

    /// The script for `Assign`s of `job`.
    pub fn script(mut self, job: impl Into<String>, script: Script) -> Self {
        self.scripts.insert(job.into(), script);
        self
    }

    /// The script for jobs without one.
    pub fn default_script(mut self, script: Script) -> Self {
        self.default_script = script;
        self
    }

    /// Whether `Ping` gets a `Pong` (turn off to simulate a lost host).
    pub fn answer_pings(mut self, yes: bool) -> Self {
        self.answer_pings = yes;
        self
    }

    /// Spawn the session on one end of an in-memory duplex; the returned stream is the server's
    /// end.
    pub fn spawn(self) -> (DuplexStream, FakeHandle) {
        let (server_end, runner_end) = tokio::io::duplex(1 << 20);
        let (tx, received) = mpsc::unbounded_channel();
        let task = tokio::spawn(self.run(runner_end, tx));
        (server_end, FakeHandle { received, task })
    }

    /// Run the session on `stream` until it closes, reporting received frames on `seen`.
    pub async fn run<S>(self, mut stream: S, seen: mpsc::UnboundedSender<Frame>) -> Result<(), CodecError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        codec::write_frame(&mut stream, &self.hello).await?;
        let mut awaiting_pack: Option<Pending> = None;
        let mut hanging: HashSet<String> = HashSet::new();

        while let Some(frame) = codec::read_frame(&mut stream).await? {
            // The receiver may be gone when the test does not care; that is fine.
            let _ = seen.send(frame.clone());
            match frame {
                Frame::Ping if self.answer_pings => codec::write_frame(&mut stream, &Frame::Pong).await?,
                Frame::Assign { run_id, job, pack_size, .. } => {
                    let script = self.scripts.get(&job).unwrap_or(&self.default_script).clone();
                    let pending = Pending { run_id, script };
                    if pack_size == 0 {
                        play(&mut stream, pending, &mut hanging).await?;
                    } else {
                        awaiting_pack = Some(pending);
                    }
                }
                Frame::Pack { run_id, last: true, .. } => {
                    if let Some(p) = awaiting_pack.take_if(|p| p.run_id == run_id) {
                        play(&mut stream, p, &mut hanging).await?;
                    }
                }
                Frame::Cancel { run_id } => {
                    awaiting_pack.take_if(|p| p.run_id == run_id);
                    if hanging.remove(&run_id) {
                        let done = Frame::Finished {
                            run_id,
                            exit_code: CANCELLED_EXIT_CODE,
                            stats: BTreeMap::new(),
                            ledger_line: String::new(),
                        };
                        codec::write_frame(&mut stream, &done).await?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

async fn play<S>(stream: &mut S, p: Pending, hanging: &mut HashSet<String>) -> Result<(), CodecError>
where
    S: AsyncWrite + Unpin,
{
    let Pending { run_id, script } = p;
    if let Outcome::Refuse { reason, detail } = script.outcome {
        return codec::write_frame(stream, &Frame::Refused { run_id, reason, detail }).await;
    }
    codec::write_frame(stream, &Frame::Started { run_id: run_id.clone() }).await?;
    for (seq, step) in (1u64..).zip(script.steps) {
        let frame = match step {
            Step::Event(json) => Frame::Event { run_id: run_id.clone(), seq, json },
            Step::Log(stream, bytes) => Frame::Log { run_id: run_id.clone(), seq, stream, bytes },
        };
        codec::write_frame(stream, &frame).await?;
    }
    match script.outcome {
        Outcome::Hang => {
            hanging.insert(run_id);
            Ok(())
        }
        _ => {
            let done = Frame::Finished {
                run_id,
                exit_code: script.exit_code,
                stats: script.stats,
                ledger_line: script.ledger_line,
            };
            codec::write_frame(stream, &done).await
        }
    }
}
