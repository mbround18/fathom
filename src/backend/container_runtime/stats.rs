//! Live CPU and memory, sampled in discrete ticks rather than by a sampler that outlives the view.
//!
//! `docker stats` without `--no-stream` is a long-lived process that emits on its own interval —
//! a background sampler that would keep running behind a hidden view, which is the one thing
//! FR-045 forbids. So each tick here is its own short invocation: startable and stoppable with the
//! view's visibility, and costing exactly nothing when nothing is on screen (Principle V).
//!
//! Two details are not cosmetic. The ids are always explicit: an unscoped `stats` samples every
//! container on the daemon, and on a machine whose daemon is shared that spends CPU measuring
//! other people's projects for a view nobody is looking at. And a figure the runtime did not
//! report stays `None` all the way to the protocol — a container with no usable measurement
//! reading as `0.00%` would say "idle" about something unmeasured, which is a worse answer than
//! "unavailable" (FR-038).
//!
//! The runtime hands its figures over pre-formatted for a terminal ("24.56MiB / 31.25GiB",
//! "0.00%"). Parsing them into numbers happens once, here, so the protocol carries numbers and
//! every reader of them — a row, a project aggregate, a sparkline — agrees about what they mean
//! (research D4).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Deserialize;

use crate::protocol::v1::UsageSample;

use super::super::kind::{RunError, Runner};
use super::{Cli, parse_size};

/// The floor on how often a tick may run.
///
/// Each tick is a process spawn and the daemon samples cgroups to answer it. Two seconds is the
/// cost acknowledged in research D4: fast enough that a container put under load is visibly
/// hotter within the latency of the cluster metrics views, slow enough that watching is not
/// itself the load.
pub const MIN_INTERVAL: Duration = Duration::from_secs(2);

/// How many ticks of history the usage view keeps, and therefore how many the operator can see.
///
/// Two minutes at the minimum interval. A bound, because a view left open for an afternoon must
/// not grow without end (FR-038) — and an announced bound, because a graph that silently forgets
/// its left-hand edge is a graph that lies about the period it covers (FR-034). `at_cap` on the
/// event is how the operator is told, and the figure is the same on both sides of the protocol.
pub const HISTORY_SAMPLES: usize = 60;

/// How often the sampler checks whether it has been told to stop.
///
/// Much shorter than a tick: "stop sampling when the view closes" (FR-045) is a claim about the
/// machine, and a thread that only noticed between ticks would keep a `docker stats` of its own
/// running for up to two seconds after the scene was gone.
const STOP_POLL: Duration = Duration::from_millis(100);

/// What the runtime answers with, per container, under `--format '{{json .}}'`.
///
/// Deserialized by the runtime's own field names. Every field is a string, including the numbers:
/// that is the format's doing, not ours.
#[derive(Debug, Deserialize)]
struct Row {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "CPUPerc", default)]
    cpu_perc: String,
    #[serde(rename = "MemUsage", default)]
    mem_usage: String,
}

/// The argv for one tick over exactly these containers.
pub fn argv(ids: &[String]) -> Vec<String> {
    let mut v = vec![
        "stats".to_owned(),
        "--no-stream".to_owned(),
        "--format".to_owned(),
        "{{json .}}".to_owned(),
    ];
    v.extend(ids.iter().cloned());
    v
}

/// `"12.34%"` → `12.34`. `"--"`, `""` and anything else → `None`.
///
/// The runtime writes `--` for a container it has no measurement for, which is a different fact
/// from zero and must not be flattened into one.
pub fn parse_percent(raw: &str) -> Option<f32> {
    let s = raw.trim().strip_suffix('%')?;
    s.trim().parse::<f32>().ok().filter(|v| v.is_finite())
}

/// `"24.56MiB / 31.25GiB"` → `(used, limit)`. Either side may be unavailable on its own.
pub fn parse_mem_usage(raw: &str) -> (Option<u64>, Option<u64>) {
    let mut parts = raw.split('/');
    let used = parts.next().and_then(parse_size);
    let limit = parts.next().and_then(parse_size);
    (used, limit)
}

/// One tick: the usage of exactly these containers, as of `at`.
///
/// `ids` empty is not an empty tick, it is no tick at all — the invocation is skipped entirely,
/// because `stats` with no ids means "every container on this daemon".
///
/// A container the runtime left out of its answer still gets a sample, with every figure
/// unavailable. Leaving it out instead would make its row keep showing the last figure it had,
/// which is the lie this module exists to avoid.
pub fn sample_with(
    cli: Cli,
    runner: &dyn Runner,
    ids: &[String],
    at: &str,
) -> Result<Vec<UsageSample>, RunError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let args = argv(ids);
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = cli.run(runner, &argv)?;
    if !out.success {
        let said = out.stderr.trim();
        return Err(RunError::Other(if said.is_empty() {
            "the container runtime failed without a message".to_owned()
        } else {
            said.to_owned()
        }));
    }

    let mut samples = Vec::with_capacity(ids.len());
    for id in ids {
        // Matched by prefix: the ids the operator's rows carry are full 64-character ones, and
        // `stats` answers with the short form the runtime displays.
        let mut row = out.stdout.lines().filter_map(|line| {
            let row: Row = serde_json::from_str(line.trim()).ok()?;
            (id.starts_with(row.id.as_str()) || row.id.starts_with(id.as_str())).then_some(row)
        });
        let found = row.next();
        let (memory_bytes, memory_limit_bytes) = found
            .as_ref()
            .map_or((None, None), |r| parse_mem_usage(&r.mem_usage));
        samples.push(UsageSample {
            container_id: id.clone(),
            at: at.to_owned(),
            cpu_percent: found.as_ref().and_then(|r| parse_percent(&r.cpu_perc)),
            memory_bytes,
            memory_limit_bytes,
        });
    }
    Ok(samples)
}

// ---------------------------------------------------------------------------
// The sampler
// ---------------------------------------------------------------------------

/// A running tick loop over a fixed set of containers.
///
/// Owned by the backend's runtime state, one at a time: a second start replaces the first, which
/// ends it. The replacement matters more than it looks — the visible rows change as the operator
/// scrolls or selects, and a sampler per change would leave a growing pile of loops all ticking at
/// the same daemon.
pub struct Sampler {
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Sampler {
    /// Starts ticking, first tick immediately.
    ///
    /// Immediately, because the alternative is a usage view that shows nothing for two seconds and
    /// cannot be told apart from one whose figures are unavailable.
    pub fn start(
        cli: Cli,
        runner: Arc<dyn Runner>,
        ids: Vec<String>,
        interval: Duration,
        sink: Box<dyn Fn(Vec<UsageSample>, bool) + Send + 'static>,
    ) -> Self {
        let interval = interval.max(MIN_INTERVAL);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let worker = std::thread::spawn(move || {
            let mut ticks = 0_usize;
            while !flag.load(Ordering::Relaxed) {
                let at = chrono::Utc::now().to_rfc3339();
                // A tick that failed is not a tick of zeroes, and it is not the end of the loop
                // either: a daemon that was restarting answers the next one, and until it does the
                // figures stay as unavailable as they already were.
                if let Ok(samples) = sample_with(cli, runner.as_ref(), &ids, &at) {
                    ticks = ticks.saturating_add(1);
                    sink(samples, ticks > HISTORY_SAMPLES);
                }
                let mut waited = Duration::ZERO;
                while waited < interval && !flag.load(Ordering::Relaxed) {
                    std::thread::sleep(STOP_POLL);
                    waited += STOP_POLL;
                }
            }
        });
        Self {
            stop,
            worker: Some(worker),
        }
    }

    /// Stops ticking and waits for the loop to actually be gone.
    pub fn stop(mut self) {
        self.halt();
    }

    fn halt(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            drop(worker.join());
        }
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.halt();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::kind::CmdOutput;
    use std::sync::Mutex;

    struct Scripted {
        answer: Result<CmdOutput, RunError>,
        seen: Mutex<Vec<Vec<String>>>,
    }

    impl Scripted {
        fn ok(stdout: &str) -> Self {
            Self {
                answer: Ok(CmdOutput {
                    success: true,
                    stdout: stdout.to_owned(),
                    stderr: String::new(),
                }),
                seen: Mutex::new(Vec::new()),
            }
        }
    }

    impl Runner for Scripted {
        fn run(&self, _bin: &str, args: &[&str]) -> Result<CmdOutput, RunError> {
            self.seen
                .lock()
                .expect("lock")
                .push(args.iter().map(|a| (*a).to_owned()).collect());
            self.answer.clone()
        }
    }

    const ID_A: &str = "aaaaaaaaaaaa000000000000000000000000000000000000000000000000000a";
    const ID_B: &str = "bbbbbbbbbbbb000000000000000000000000000000000000000000000000000b";

    #[test]
    fn the_tick_is_discrete_and_names_every_container_it_samples() {
        let args = argv(&[ID_A.to_owned(), ID_B.to_owned()]);
        assert_eq!(args[0], "stats");
        assert!(
            args.contains(&"--no-stream".to_owned()),
            "a streaming stats is a sampler that outlives the view: {args:?}"
        );
        assert_eq!(args.last(), Some(&ID_B.to_owned()));
    }

    /// An unscoped `stats` samples every container on a daemon this machine shares.
    #[test]
    fn no_visible_containers_means_no_invocation_at_all() {
        let runner = Scripted::ok("");
        let samples = sample_with(Cli::docker(), &runner, &[], "2026-10-03T00:00:00Z")
            .expect("an empty tick cannot fail");
        assert!(samples.is_empty());
        assert!(
            runner.seen.lock().expect("lock").is_empty(),
            "an empty id list must not reach the runtime"
        );
    }

    #[test]
    fn the_runtimes_formatted_figures_become_numbers() {
        assert_eq!(parse_percent("0.00%"), Some(0.0));
        assert_eq!(parse_percent(" 12.34% "), Some(12.34));
        assert_eq!(
            parse_mem_usage("24.56MiB / 31.25GiB"),
            (Some(25_753_026), Some(33_554_432_000))
        );
    }

    /// Zero is a measurement. `--` is the absence of one, and the two must not read alike.
    #[test]
    fn an_unreported_figure_is_unavailable_and_not_zero() {
        assert_eq!(parse_percent("--"), None);
        assert_eq!(parse_percent(""), None);
        assert_eq!(parse_percent("12.34"), None);
        assert_eq!(parse_mem_usage("-- / --"), (None, None));
        assert_eq!(parse_mem_usage("24.56MiB / --"), (Some(25_753_026), None));
    }

    #[test]
    fn a_tick_reads_one_sample_per_container_asked_for() {
        let stdout = format!(
            "{}\n{}\n",
            r#"{"ID":"aaaaaaaaaaaa","Name":"shop-web","CPUPerc":"3.21%","MemUsage":"24.56MiB / 31.25GiB","MemPerc":"0.08%"}"#,
            r#"{"ID":"bbbbbbbbbbbb","Name":"shop-db","CPUPerc":"--","MemUsage":"-- / --","MemPerc":"--"}"#,
        );
        let runner = Scripted::ok(&stdout);
        let samples = sample_with(
            Cli::docker(),
            &runner,
            &[ID_A.to_owned(), ID_B.to_owned()],
            "2026-10-03T00:00:00Z",
        )
        .expect("the tick answered");

        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].container_id, ID_A);
        assert_eq!(samples[0].cpu_percent, Some(3.21));
        assert_eq!(samples[0].memory_bytes, Some(25_753_026));
        // Answered for, but with no figure in it: unavailable, and still its own row.
        assert_eq!(samples[1].container_id, ID_B);
        assert_eq!(samples[1].cpu_percent, None);
        assert_eq!(samples[1].memory_bytes, None);
    }

    /// A container the runtime did not answer about keeps a row, with nothing in it. A missing row
    /// would leave the last figure it had on screen, which would then be a figure about a moment
    /// that has passed.
    #[test]
    fn a_container_left_out_of_the_answer_still_gets_an_unavailable_sample() {
        let runner =
            Scripted::ok(r#"{"ID":"aaaaaaaaaaaa","CPUPerc":"1.00%","MemUsage":"1MiB / 1GiB"}"#);
        let samples = sample_with(
            Cli::docker(),
            &runner,
            &[ID_A.to_owned(), ID_B.to_owned()],
            "2026-10-03T00:00:00Z",
        )
        .expect("the tick answered");
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[1].container_id, ID_B);
        assert_eq!(samples[1].cpu_percent, None);
        assert_eq!(samples[1].memory_limit_bytes, None);
    }

    /// FR-045: the loop is gone when the view is, not a tick or two later.
    #[test]
    fn stopping_the_sampler_leaves_nothing_ticking() {
        use std::sync::atomic::AtomicUsize;

        struct Counting(AtomicUsize);
        impl Runner for Counting {
            fn run(&self, _bin: &str, _args: &[&str]) -> Result<CmdOutput, RunError> {
                self.0.fetch_add(1, Ordering::Relaxed);
                Ok(CmdOutput {
                    success: true,
                    stdout: r#"{"ID":"aaaaaaaaaaaa","CPUPerc":"1.00%","MemUsage":"1MiB / 1GiB"}"#
                        .to_owned(),
                    stderr: String::new(),
                })
            }
        }

        let runner = Arc::new(Counting(AtomicUsize::new(0)));
        let seen = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&seen);
        let sampler = Sampler::start(
            Cli::docker(),
            Arc::clone(&runner) as Arc<dyn Runner>,
            vec![ID_A.to_owned()],
            MIN_INTERVAL,
            Box::new(move |samples, _| {
                assert_eq!(samples.len(), 1);
                counted.fetch_add(1, Ordering::Relaxed);
            }),
        );
        // The first tick is immediate, so a view that has just opened is not blank.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while seen.load(Ordering::Relaxed) == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(seen.load(Ordering::Relaxed), 1, "one tick, and only one");
        sampler.stop();

        let after = runner.0.load(Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            runner.0.load(Ordering::Relaxed),
            after,
            "the runtime was invoked again after the sampler was stopped"
        );
    }

    #[test]
    fn a_refusing_runtime_fails_rather_than_reporting_zeroes() {
        let runner = Scripted {
            answer: Ok(CmdOutput {
                success: false,
                stdout: String::new(),
                stderr: "Cannot connect to the Docker daemon at unix:///var/run/docker.sock.\n"
                    .to_owned(),
            }),
            seen: Mutex::new(Vec::new()),
        };
        let err = sample_with(Cli::docker(), &runner, &[ID_A.to_owned()], "t")
            .expect_err("a refused tick is not an empty one");
        assert_eq!(
            err,
            RunError::Other(
                "Cannot connect to the Docker daemon at unix:///var/run/docker.sock.".to_owned()
            )
        );
    }
}
