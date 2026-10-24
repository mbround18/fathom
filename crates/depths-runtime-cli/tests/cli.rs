//! The adapter against stand-in binaries: `echo`, `cat`, `sleep`, `ls` play `docker`, so these
//! tests need no daemon and run anywhere.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use depths_core::plan::{RunStep, StepStdin};
use depths_core::ports::{
    CancelToken, ContainerRuntime, OutputChunk, OutputSink, OutputStream, RuntimeError,
    StepOutcome, Validation,
};
use depths_runtime_cli::CliRuntime;

#[derive(Default)]
struct Collect(Mutex<Vec<OutputChunk>>);

impl OutputSink for Collect {
    fn emit(&self, chunk: OutputChunk) -> bool {
        self.0.lock().unwrap().push(chunk);
        true
    }
}

impl Collect {
    fn text(&self, stream: OutputStream) -> String {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.stream == stream)
            .map(|c| c.text.as_str())
            .collect()
    }
}

fn step(args: &[&str]) -> RunStep {
    let mut argv = vec!["docker".to_owned()];
    argv.extend(args.iter().map(|a| (*a).to_owned()));
    RunStep {
        argv,
        cwd: std::env::temp_dir(),
        stdin: StepStdin::None,
    }
}

#[test]
fn output_streams_to_the_sink_and_success_is_reported() {
    let rt = CliRuntime::new("echo");
    let sink = Collect::default();
    let out = rt
        .run_step(
            &step(&["compose", "up", "-d"]),
            None,
            &sink,
            &CancelToken::new(),
        )
        .unwrap();
    assert_eq!(out, StepOutcome::Succeeded);
    assert_eq!(sink.text(OutputStream::Stdout), "compose up -d\n");
}

#[test]
fn the_step_runs_in_its_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("compose.yaml"), "services: {}\n").unwrap();
    let rt = CliRuntime::new("ls");
    let sink = Collect::default();
    let mut s = step(&[]);
    s.cwd = dir.path().to_path_buf();
    rt.run_step(&s, None, &sink, &CancelToken::new()).unwrap();
    assert_eq!(sink.text(OutputStream::Stdout), "compose.yaml\n");
}

#[test]
fn stdin_reaches_the_command() {
    let rt = CliRuntime::new("cat");
    let sink = Collect::default();
    let out = rt
        .run_step(
            &step(&[]),
            Some(b"services:\n  web: {}\n"),
            &sink,
            &CancelToken::new(),
        )
        .unwrap();
    assert_eq!(out, StepOutcome::Succeeded);
    assert_eq!(sink.text(OutputStream::Stdout), "services:\n  web: {}\n");
}

#[test]
fn a_failure_carries_stderr_and_the_exit_code() {
    let rt = CliRuntime::new("ls");
    let sink = Collect::default();
    let out = rt
        .run_step(
            &step(&["/nonexistent-depths-path"]),
            None,
            &sink,
            &CancelToken::new(),
        )
        .unwrap();
    match out {
        StepOutcome::Failed {
            exit_code,
            stderr_tail,
        } => {
            assert!(matches!(exit_code, Some(c) if c != 0));
            assert!(
                stderr_tail.contains("nonexistent-depths-path"),
                "{stderr_tail}"
            );
        }
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn cancel_interrupts_a_running_command() {
    let rt = CliRuntime::new("sleep").with_grace(Duration::from_secs(5));
    let cancel = CancelToken::new();
    let c = cancel.clone();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        c.cancel();
    });
    let started = Instant::now();
    let out = rt
        .run_step(&step(&["30"]), None, &Collect::default(), &cancel)
        .unwrap();
    t.join().unwrap();
    assert_eq!(out, StepOutcome::Cancelled);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "SIGINT, not the grace kill"
    );
}

#[test]
fn only_docker_is_ever_run() {
    let rt = CliRuntime::new("echo");
    let mut s = step(&["hi"]);
    s.argv[0] = "rm".into();
    let err = rt
        .run_step(&s, None, &Collect::default(), &CancelToken::new())
        .unwrap_err();
    assert!(matches!(err, RuntimeError::Failed(_)), "{err:?}");
    s.argv.clear();
    assert!(
        rt.run_step(&s, None, &Collect::default(), &CancelToken::new())
            .is_err()
    );
}

#[test]
fn a_missing_cli_is_unavailable() {
    let rt = CliRuntime::new("/nonexistent-depths-docker");
    let err = rt
        .run_step(
            &step(&["ps"]),
            None,
            &Collect::default(),
            &CancelToken::new(),
        )
        .unwrap_err();
    assert!(matches!(err, RuntimeError::Unavailable(_)), "{err:?}");
    assert!(matches!(
        rt.containers_by_project(),
        Err(RuntimeError::Unavailable(_))
    ));
    assert!(matches!(rt.ping(), Err(RuntimeError::Unavailable(_))));
}

#[test]
fn validate_reads_the_exit_status_and_compose_errors() {
    assert_eq!(
        CliRuntime::new("cat")
            .validate(&step(&[]), b"services: {}\n")
            .unwrap(),
        Validation::Valid
    );
    match CliRuntime::new("ls")
        .validate(&step(&["/nonexistent-depths-path"]), b"")
        .unwrap()
    {
        Validation::Invalid(issues) => {
            assert!(!issues.is_empty());
            assert!(issues[0].message.contains("nonexistent-depths-path"));
        }
        Validation::Valid => panic!("a failing validate is invalid"),
    }
}

#[test]
fn ping_follows_the_exit_status() {
    assert!(CliRuntime::new("true").ping().is_ok());
    assert!(matches!(
        CliRuntime::new("false").ping(),
        Err(RuntimeError::Unavailable(_))
    ));
}

#[test]
fn events_call_back_once_per_line_and_an_ended_stream_is_unavailable() {
    // `echo events --format {{json .}}` prints one line and exits: one event, then the stream
    // ends, which the watcher treats as the runtime going away.
    let seen = AtomicUsize::new(0);
    let res = CliRuntime::new("echo").follow_events(
        &|| {
            seen.fetch_add(1, Ordering::SeqCst);
        },
        &CancelToken::new(),
    );
    assert_eq!(seen.load(Ordering::SeqCst), 1);
    assert!(matches!(res, Err(RuntimeError::Unavailable(_))));
}

#[test]
fn a_cancelled_event_stream_ends_cleanly() {
    let cancel = CancelToken::new();
    cancel.cancel();
    // `sleep events ...` fails at once, but a cancel already asked for makes that a clean stop.
    let res = CliRuntime::new("sleep").follow_events(&|| {}, &cancel);
    assert!(res.is_ok(), "{res:?}");
}

// ---------------------------------------------------------------------------
// Log follows (T066): `sh -c` plays `docker logs --follow`.
// ---------------------------------------------------------------------------

mod follow {
    use super::*;
    use depths_core::ports::{StreamEvent, StreamId, StreamSink};
    use depths_runtime_cli::LINE_CAP;

    #[derive(Default)]
    struct Batches {
        got: Mutex<Vec<(Vec<String>, bool)>>,
        /// Answer `false` after this many batches.
        stop_after: Option<usize>,
    }

    impl StreamSink for Batches {
        fn emit(&self, ev: StreamEvent) -> bool {
            let StreamEvent::ContainerLogBatch {
                lines, truncated, ..
            } = ev
            else {
                panic!("a follow sends only log batches: {ev:?}");
            };
            let mut got = self.got.lock().unwrap();
            got.push((lines, truncated));
            self.stop_after.is_none_or(|n| got.len() < n)
        }
    }

    impl Batches {
        fn lines(&self) -> Vec<String> {
            self.got
                .lock()
                .unwrap()
                .iter()
                .flat_map(|(l, _)| l.clone())
                .collect()
        }
    }

    fn sh(script: &str) -> RunStep {
        step(&["-c", script])
    }

    #[test]
    fn lines_arrive_in_order_and_a_failure_carries_the_last_stderr_line() {
        let rt = CliRuntime::new("sh");
        let sink = Batches::default();
        let out = rt.follow_logs(
            &sh("echo one; echo two; echo 'no such container' >&2; exit 1"),
            StreamId(1),
            &sink,
            &CancelToken::new(),
        );
        assert_eq!(out, Err(RuntimeError::Failed("no such container".into())));
        let lines = sink.lines();
        assert_eq!(&lines[..2], ["one", "two"]);
        assert!(lines.contains(&"no such container".to_owned()));
    }

    #[test]
    fn a_clean_end_is_ok() {
        let rt = CliRuntime::new("sh");
        let sink = Batches::default();
        let out = rt.follow_logs(&sh("echo a"), StreamId(1), &sink, &CancelToken::new());
        assert_eq!(out, Ok(()));
        assert_eq!(sink.lines(), ["a"]);
    }

    #[test]
    fn cancel_stops_a_follow_that_never_ends() {
        let rt = CliRuntime::new("sh");
        let sink = Batches::default();
        let cancel = CancelToken::new();
        let stopper = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            stopper.cancel();
        });
        let started = Instant::now();
        let out = rt.follow_logs(
            &sh("while true; do echo tick; sleep 0.05; done"),
            StreamId(1),
            &sink,
            &cancel,
        );
        assert_eq!(out, Ok(()));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(sink.lines().iter().all(|l| l == "tick"));
    }

    #[test]
    fn a_sink_with_nobody_behind_it_stops_the_follow() {
        let rt = CliRuntime::new("sh");
        let sink = Batches {
            stop_after: Some(1),
            ..Batches::default()
        };
        let started = Instant::now();
        let out = rt.follow_logs(
            &sh("while true; do echo tick; sleep 0.01; done"),
            StreamId(1),
            &sink,
            &CancelToken::new(),
        );
        assert_eq!(out, Ok(()));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(sink.got.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_line_with_no_newline_is_split_rather_than_grown() {
        let rt = CliRuntime::new("sh");
        let sink = Batches::default();
        let n = LINE_CAP * 2 + 10;
        rt.follow_logs(
            &sh(&format!("head -c {n} /dev/zero | tr '\\0' x")),
            StreamId(1),
            &sink,
            &CancelToken::new(),
        )
        .unwrap();
        let lens: Vec<usize> = sink.lines().iter().map(String::len).collect();
        assert_eq!(lens, [LINE_CAP, LINE_CAP, 10]);
    }

    #[test]
    fn a_follow_runs_only_docker() {
        let rt = CliRuntime::new("sh");
        let mut s = sh("echo a");
        s.argv[0] = "sh".into();
        let out = rt.follow_logs(&s, StreamId(1), &Batches::default(), &CancelToken::new());
        assert!(matches!(out, Err(RuntimeError::Failed(_))));
    }
}
