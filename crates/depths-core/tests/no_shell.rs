//! T095: nothing in Depths runs a host shell. Every command is an argv handed to a runner, so a
//! name or a value can never be read as shell syntax.
//!
//! This scans the Depths sources for the shapes a shell invocation takes in Rust. A shell *inside*
//! a container (`docker exec -it <id> /bin/sh`) is an argument to docker, not a host shell, and is
//! not matched.

use std::fs;
use std::path::{Path, PathBuf};

/// What a host shell looks like in command construction.
const FORBIDDEN: [&str; 6] = [
    "\"-c\"",
    "Command::new(\"sh\")",
    "Command::new(\"bash\")",
    "Command::new(\"/bin/sh\")",
    "Command::new(\"/bin/bash\")",
    "Command::new(\"/usr/bin/env\")",
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_depths_source_builds_a_shell_command() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut dirs: Vec<PathBuf> = fs::read_dir(repo.join("crates"))
        .expect("crates/ is readable")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("depths-"))
        })
        .map(|p| p.join("src"))
        .collect();
    dirs.push(repo.join("apps/depths/server/src"));

    let mut files = Vec::new();
    for dir in &dirs {
        rust_files(dir, &mut files);
    }
    assert!(
        files.iter().any(|f| f.ends_with("depths-core/src/plan.rs")),
        "the scan must reach the planner"
    );

    let mut hits = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).expect("source is UTF-8");
        for (n, line) in text.lines().enumerate() {
            for pattern in FORBIDDEN {
                if line.contains(pattern) {
                    hits.push(format!("{}:{}: {}", file.display(), n + 1, line.trim()));
                }
            }
        }
    }
    assert!(hits.is_empty(), "shell invocations found:\n{}", hits.join("\n"));
}
