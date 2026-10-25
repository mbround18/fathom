//! Shells and stats against stand-in `docker` scripts (T066).
//!
//! One test in its own binary on purpose: writing an executable and then running it races with
//! any other thread forking meanwhile (ETXTBSY), so nothing else runs in this process.

use std::os::unix::fs::PermissionsExt as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use depths_core::ports::{ContainerRuntime, StreamEvent, StreamId, StreamSink};
use depths_runtime_cli::CliRuntime;

#[derive(Default)]
struct Term(Mutex<(String, bool)>);

impl StreamSink for Term {
    fn emit(&self, ev: StreamEvent) -> bool {
        let mut t = self.0.lock().unwrap();
        match ev {
            StreamEvent::ContainerSessionOutput { data, .. } => t.0.push_str(&data),
            StreamEvent::ContainerSessionClosed { .. } => t.1 = true,
            StreamEvent::ContainerLogBatch { .. } => panic!("a shell sends no log batches"),
        }
        true
    }
}

fn script(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn shells_and_stats_through_stand_in_scripts() {
    let dir = tempfile::tempdir().unwrap();
    // `docker exec --interactive --tty <id> <shell>`: record the argv, then run the shell.
    let argv_file = dir.path().join("argv");
    let exec = script(
        dir.path(),
        "docker-exec",
        &format!("echo \"$@\" > {}\nexec \"$5\"", argv_file.display()),
    );
    let stats = script(
        dir.path(),
        "docker-stats",
        r#"echo '{"ID":"abc123def456","CPUPerc":"1.50%","MemUsage":"1MiB / 2MiB"}'"#,
    );

    // A shell: it opens at the size asked for, input reaches it, output comes back, close ends it.
    let rt = CliRuntime::new(&exec);
    let term = Arc::new(Term::default());
    let mut shell = rt
        .open_shell(
            "abc123def456",
            "/bin/sh",
            (100, 30),
            StreamId(7),
            term.clone(),
        )
        .expect("a pty and a shell");
    shell.write(b"stty size; echo hi-$((40+2))\n").unwrap();
    wait_for("the shell's answer", || {
        term.0.lock().unwrap().0.contains("hi-42")
    });
    let seen = term.0.lock().unwrap().0.clone();
    assert!(
        seen.contains("30 100"),
        "the size reached the terminal: {seen:?}"
    );
    shell.resize(120, 40);
    shell.close();
    wait_for("the close", || term.0.lock().unwrap().1);
    assert_eq!(
        std::fs::read_to_string(&argv_file).unwrap().trim(),
        "exec --interactive --tty abc123def456 /bin/sh"
    );

    // Stats: matched by id prefix, stamped with the caller's time; no ids runs nothing.
    let rt = CliRuntime::new(&stats);
    let full = "abc123def456".repeat(6)[..64].to_owned();
    let samples = rt
        .stats(std::slice::from_ref(&full), "2026-10-23T00:00:00Z")
        .unwrap();
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].container_id, full);
    assert_eq!(samples[0].at, "2026-10-23T00:00:00Z");
    assert_eq!(samples[0].cpu_percent, Some(1.5));
    assert_eq!(samples[0].memory_bytes, Some(1024 * 1024));
    assert!(CliRuntime::new("false").stats(&[], "t").unwrap().is_empty());
}
