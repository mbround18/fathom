//! The command-runner seam: every short-lived runtime call goes through [`Runner`], so tests can
//! script the runtime's answers and the real thing is an argv, never a shell line.

use std::io::Write as _;
use std::path::Path;
use std::process::Stdio;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmdOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// Binary not found on `PATH`.
    NotFound,
    /// This runner cannot do what was asked (a scripted runner asked to feed stdin, say).
    Unsupported,
    Other(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("binary not found on PATH"),
            Self::Unsupported => f.write_str("operation not supported by this runner"),
            Self::Other(detail) => f.write_str(detail),
        }
    }
}

/// Runs a short-lived probe command. Implementations must use argument arrays only.
pub trait Runner: Send + Sync {
    fn run(&self, bin: &str, args: &[&str]) -> Result<CmdOutput, RunError>;

    /// Runs `bin` in `cwd` (the current directory when `None`) with `stdin` as its whole input.
    ///
    /// Compose reads a definition from stdin (`-f -`), which is how a caller validates a file it
    /// has not written yet. Runners that cannot do this say so rather than run without the input.
    fn run_with_stdin(
        &self,
        _bin: &str,
        _args: &[&str],
        _cwd: Option<&Path>,
        _stdin: &[u8],
    ) -> Result<CmdOutput, RunError> {
        Err(RunError::Unsupported)
    }
}

/// Real runner: `std::process::Command` with an argument array, no shell.
pub struct SystemRunner;

fn spawn_error(e: &std::io::Error) -> RunError {
    if e.kind() == std::io::ErrorKind::NotFound {
        RunError::NotFound
    } else {
        RunError::Other(e.to_string())
    }
}

fn collect(out: &std::process::Output) -> CmdOutput {
    CmdOutput {
        success: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

impl Runner for SystemRunner {
    fn run(&self, bin: &str, args: &[&str]) -> Result<CmdOutput, RunError> {
        std::process::Command::new(bin)
            .args(args)
            .output()
            .map(|out| collect(&out))
            .map_err(|e| spawn_error(&e))
    }

    fn run_with_stdin(
        &self,
        bin: &str,
        args: &[&str],
        cwd: Option<&Path>,
        stdin: &[u8],
    ) -> Result<CmdOutput, RunError> {
        let mut cmd = std::process::Command::new(bin);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        let mut child = cmd.spawn().map_err(|e| spawn_error(&e))?;
        // Written from its own thread: a child that fills its stdout pipe before reading all of
        // stdin would otherwise deadlock against us waiting to finish the write.
        let mut pipe = child
            .stdin
            .take()
            .ok_or_else(|| RunError::Other("child stdin was not piped".to_owned()))?;
        let input = stdin.to_vec();
        let writer = std::thread::spawn(move || {
            // A child that exits without reading everything closes the pipe; that is its answer,
            // and its exit status says more than the broken pipe would.
            drop(pipe.write_all(&input));
        });
        let out = child
            .wait_with_output()
            .map_err(|e| RunError::Other(e.to_string()))?;
        drop(writer.join());
        Ok(collect(&out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Plain;
    impl Runner for Plain {
        fn run(&self, _bin: &str, _args: &[&str]) -> Result<CmdOutput, RunError> {
            Ok(CmdOutput {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn a_runner_without_stdin_support_says_so() {
        assert_eq!(
            Plain.run_with_stdin("cat", &[], None, b"x"),
            Err(RunError::Unsupported)
        );
    }

    #[cfg(unix)]
    #[test]
    fn stdin_reaches_the_child_whole() {
        let input = "services:\n  app:\n    image: busybox\n".repeat(4096);
        let out = SystemRunner
            .run_with_stdin("cat", &[], None, input.as_bytes())
            .expect("cat runs");
        assert!(out.success);
        assert_eq!(out.stdout, input);
    }

    #[cfg(unix)]
    #[test]
    fn the_child_runs_in_the_directory_asked_for() {
        let dir = std::env::temp_dir().canonicalize().expect("temp dir");
        let out = SystemRunner
            .run_with_stdin("pwd", &[], Some(&dir), b"")
            .expect("pwd runs");
        assert_eq!(out.stdout.trim(), dir.to_string_lossy());
    }

    #[test]
    fn a_missing_binary_is_not_found() {
        assert_eq!(
            SystemRunner.run("depths-test-no-such-binary", &[]),
            Err(RunError::NotFound)
        );
        assert_eq!(
            SystemRunner.run_with_stdin("depths-test-no-such-binary", &[], None, b""),
            Err(RunError::NotFound)
        );
    }
}
