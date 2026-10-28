//! `keel-executor`: runs Ansible as the keel-exec user, verifying what it is given (spec 055
//! T085, contracts/runner.md). Started per connection by `keel-executor.socket` (`Accept=yes`,
//! `StandardInput=socket`): fd 0 is the connection, and it carries frames only.

use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use keel_runner::executor::signed::verify_ledger_file;
use keel_runner::executor::{ExecutorConfig, LEDGER_FILE, serve};

/// Keel host runner: runs Ansible as the keel-exec user, verifying what it is given.
#[derive(Debug, Parser)]
#[command(name = "keel-executor", version, about)]
struct Cli {
    /// The ledger, the Git store and run directories.
    #[arg(
        long,
        env = "KEEL_EXECUTOR_STATE_DIR",
        default_value = "/var/lib/keel-executor"
    )]
    state_dir: PathBuf,
    /// `policy.toml` and the other trust files.
    #[arg(
        long,
        env = "KEEL_RUNNER_TRUST_DIR",
        default_value = "/etc/keel-runner/trust"
    )]
    trust_dir: PathBuf,
    /// The Ansible venv.
    #[arg(
        long,
        env = "KEEL_RUNNER_VENV",
        default_value = "/opt/keel-runner/venv"
    )]
    venv: PathBuf,
    /// `ansible-playbook` (default: `<venv>/bin/ansible-playbook`).
    #[arg(long)]
    ansible_playbook: Option<PathBuf>,
    /// The directory holding the `keel_events` callback (default: `<venv>/share/keel/callback`).
    #[arg(long)]
    callback_dir: Option<PathBuf>,
    /// Without a command, serve the connection on fd 0.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// The host ledger.
    Ledger {
        #[command(subcommand)]
        action: LedgerAction,
    },
}

#[derive(Debug, Subcommand)]
enum LedgerAction {
    /// Verify the whole hash chain. Exits 1 if it does not verify; never changes the file.
    Verify,
}

fn same_file(a: i32, b: i32) -> bool {
    // SAFETY: fstat on descriptors with a zeroed out-parameter.
    unsafe {
        let mut sa: libc::stat = std::mem::zeroed();
        let mut sb: libc::stat = std::mem::zeroed();
        libc::fstat(a, &mut sa) == 0
            && libc::fstat(b, &mut sb) == 0
            && sa.st_dev == sb.st_dev
            && sa.st_ino == sb.st_ino
    }
}

/// Takes the connection off fd 0 and makes sure nothing else can write to it: fds 0, 1 and 2
/// that still point at the socket (systemd's `inherit` default) are pointed at `/dev/null`.
fn take_connection() -> std::io::Result<std::os::unix::net::UnixStream> {
    // SAFETY: plain syscalls on descriptors this process owns.
    unsafe {
        let conn = libc::fcntl(0, libc::F_DUPFD_CLOEXEC, 3);
        if conn < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let conn = OwnedFd::from_raw_fd(conn);
        let null = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/null")?;
        for fd in [1, 2] {
            if same_file(fd, conn.as_raw_fd()) && libc::dup2(null.as_raw_fd(), fd) < 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        if libc::dup2(null.as_raw_fd(), 0) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let stream = std::os::unix::net::UnixStream::from(conn);
        stream.set_nonblocking(true)?;
        Ok(stream)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cli = Cli::parse();
    if let Some(Command::Ledger {
        action: LedgerAction::Verify,
    }) = cli.command
    {
        let path = cli.state_dir.join(LEDGER_FILE);
        match verify_ledger_file(&path) {
            Ok(msg) => println!("{}: {msg}", path.display()),
            Err(e) => {
                eprintln!("{}: does not verify: {e}", path.display());
                std::process::exit(1);
            }
        }
        return;
    }
    let stream = match take_connection() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("keel-executor: fd 0 is not a connection: {e}");
            std::process::exit(1);
        }
    };
    let stream = match tokio::net::UnixStream::from_std(stream) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("keel-executor: {e}");
            std::process::exit(1);
        }
    };
    let mut cfg = ExecutorConfig::with_defaults(cli.state_dir, cli.trust_dir, cli.venv);
    if let Some(p) = cli.ansible_playbook {
        cfg.ansible_playbook = p;
    }
    if let Some(d) = cli.callback_dir {
        cfg.callback_dir = d;
    }
    serve(stream, &cfg).await;
}
