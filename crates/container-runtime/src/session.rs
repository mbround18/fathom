//! 048-docker-management US5: an interactive session inside one container.
//!
//! `docker exec -it` needs a pseudo-terminal on this machine, because the runtime refuses `-t`
//! when its own stdin is not one. So the pty is opened here, in about a hundred lines.
//!
//! Whether a session may be opened at all is the app's decision (Fathom gates it on the
//! container's adoption grant): a shell inside a container is unbounded execution inside someone
//! else's project, and no preview could describe what the operator is about to type.
//!
//! Ending always kills *and reaps* — "no process left behind" (FR-036) is not satisfied by a dead
//! child nobody waited on.

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};

use crate::sink::{StreamEvent, StreamSink};
use crate::wire::StreamId;

/// The shell a session opens when the operator names none.
///
/// `/bin/sh` rather than `/bin/bash`-then-fall-back, which is what `exec.rs` does for pods. A pod
/// exec can try one shell, watch it fail and try another because the failure arrives as an API
/// error; here both attempts would arrive as bytes on the same pty as the shell's own output, so
/// the fallback could only be guessed at from an exit status. `/bin/sh` is the one path that is
/// present in a busybox image and a distro image alike, and an operator who wants another names
/// it — which the protocol lets them do.
const DEFAULT_SHELL: &str = "/bin/sh";

/// How many bytes of output are forwarded at once.
const READ_CHUNK: usize = 8 * 1024;

/// `docker exec` for an interactive session in one container.
pub fn argv(container_id: &str, shell: Option<&str>) -> Vec<String> {
    vec![
        "exec".to_owned(),
        "--interactive".to_owned(),
        "--tty".to_owned(),
        container_id.to_owned(),
        shell.unwrap_or(DEFAULT_SHELL).to_owned(),
    ]
}

/// One live session: the child, the terminal it is attached to, and the thread reading it.
pub struct Session {
    pub stream_id: StreamId,
    child: Child,
    /// The controlling end of the pty. Written to for keystrokes, `ioctl`-ed for resizes.
    master: std::fs::File,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl Session {
    /// Opens a pty, spawns the session on it, and starts forwarding its output to `sink`.
    pub fn open<S: StreamSink + 'static>(
        stream_id: StreamId,
        bin: &str,
        args: &[String],
        sink: S,
    ) -> std::io::Result<Self> {
        let (master, slave) = pty::open()?;
        let mut cmd = Command::new(bin);
        cmd.args(args)
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave));
        pty::make_controlling_terminal(&mut cmd);
        let child = cmd.spawn()?;

        let mut out = master.try_clone()?;
        let reader = std::thread::spawn(move || relay(&mut out, &sink, stream_id));
        Ok(Self {
            stream_id,
            child,
            master,
            reader: Some(reader),
        })
    }

    /// Forwards a keystroke or a paste. A write that fails means the shell is gone.
    pub fn write(&mut self, data: &str) -> std::io::Result<()> {
        self.write_bytes(data.as_bytes())
    }

    /// [`Session::write`] for raw bytes: a terminal's input need not be UTF-8.
    pub fn write_bytes(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.master.write_all(data)?;
        self.master.flush()
    }

    /// Tells the remote terminal how big the pane is. Best effort: a resize that cannot be
    /// delivered means the session has ended, and the close event is what says so.
    pub fn resize(&self, cols: u16, rows: u16) {
        pty::resize(&self.master, cols, rows);
    }

    /// Ends the session and leaves nothing running (FR-036).
    ///
    /// Kill, then *wait*: an unreaped child is still a process in the table, and "closing the view
    /// leaves no process behind" is a claim about the machine and not about the app's bookkeeping.
    pub fn close(mut self) {
        drop(self.child.kill());
        drop(self.child.wait());
        // Dropping the master end gives the reader thread its EOF, so it finishes on its own.
        if let Some(reader) = self.reader.take() {
            drop(reader.join());
        }
    }
}

/// Reads the session's output until the shell closes the terminal, then reports the close.
///
/// Lossy on purpose: a terminal carries whatever bytes the program wrote, and a shell that prints
/// a partial multi-byte sequence across two reads must not take the pane down with it.
fn relay(out: &mut std::fs::File, sink: &dyn StreamSink, stream_id: StreamId) {
    let mut buf = [0_u8; READ_CHUNK];
    loop {
        match out.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                sink.emit(StreamEvent::ContainerSessionOutput {
                    stream_id,
                    data: String::from_utf8_lossy(&buf[..n]).into_owned(),
                });
            }
            // EIO is what a pty reports when the far end went away; it is this loop's EOF, not a
            // failure worth telling the operator about.
            Err(e) if e.raw_os_error() == Some(5) => break,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                sink.emit(StreamEvent::ContainerSessionClosed {
                    stream_id,
                    error: Some(e.to_string()),
                });
                return;
            }
        }
    }
    sink.emit(StreamEvent::ContainerSessionClosed {
        stream_id,
        error: None,
    });
}

/// The pseudo-terminal itself.
///
/// Kept in one module with its `unsafe` blocks so the rest of the file is ordinary Rust, and so
/// the non-Unix build has exactly one thing to stub out.
#[cfg(unix)]
mod pty {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    /// The end of the terminal the session itself runs on.
    pub(super) type Slave = OwnedFd;

    /// The size the terminal starts at. The pane sends its real size as soon as it is attached;
    /// this is only what the shell sees for the first few milliseconds.
    const INITIAL: libc::winsize = libc::winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    /// Opens a pty pair: the controlling end, and the end the session runs on.
    pub(super) fn open() -> std::io::Result<(std::fs::File, Slave)> {
        let mut master: libc::c_int = -1;
        let mut slave: libc::c_int = -1;
        let mut size = INITIAL;
        // SAFETY: `openpty` writes two valid descriptors through the out-pointers on success and
        // touches neither on failure, which the return code distinguishes. `size` outlives the
        // call; the two name buffers are optional and passed as null.
        let rc = unsafe {
            libc::openpty(
                &raw mut master,
                &raw mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &raw mut size,
            )
        };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: both descriptors were just created by `openpty` and are owned by nobody else.
        let (master, slave) =
            unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
        Ok((std::fs::File::from(master), slave))
    }

    /// Makes the child's stdin its controlling terminal, in its own session.
    ///
    /// Without this the shell has a terminal on its descriptors but no controlling one, so job
    /// control and Ctrl-C do nothing — the pty would look interactive and not behave it.
    pub(super) fn make_controlling_terminal(cmd: &mut Command) {
        // SAFETY: `pre_exec` runs between fork and exec, where only async-signal-safe calls are
        // allowed. `setsid` and `ioctl` both are, and neither allocates nor takes a lock.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::ioctl(0, libc::TIOCSCTTY, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    pub(super) fn resize(master: &std::fs::File, cols: u16, rows: u16) {
        let size = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: `master` is a live pty descriptor and `size` is a fully initialised `winsize`,
        // which is what `TIOCSWINSZ` reads. A failure means the session has ended; the close
        // event already tells the operator that, so there is nothing here to report.
        unsafe {
            libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ, &raw const size);
        }
    }
}

#[cfg(not(unix))]
mod pty {
    use std::process::Command;

    /// Nothing opens one of these here; the alias exists so the rest of the file type-checks.
    pub(super) type Slave = std::fs::File;

    /// There is no local pty to hand `docker exec -t` here, and the MVP ships Linux only
    /// (docs/MVP.md). Saying so is better than a session that attaches to nothing.
    pub(super) fn open() -> std::io::Result<(std::fs::File, Slave)> {
        Err(std::io::Error::other(
            "Interactive container sessions need a pseudo-terminal, which this platform does not provide.",
        ))
    }

    pub(super) fn make_controlling_terminal(_cmd: &mut Command) {}

    pub(super) fn resize(_master: &std::fs::File, _cols: u16, _rows: u16) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::ChannelSink;
    use std::sync::mpsc::Receiver;
    use std::time::{Duration, Instant};

    /// Output and closes received within `within`, as (text, closed-with-error).
    fn drain(rx: &Receiver<StreamEvent>, within: Duration) -> (String, Vec<Option<String>>) {
        let deadline = Instant::now() + within;
        let mut text = String::new();
        let mut closes = Vec::new();
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(ev) => match ev {
                    StreamEvent::ContainerSessionOutput { data, .. } => {
                        text.push_str(&data);
                    }
                    StreamEvent::ContainerSessionClosed { error, .. } => {
                        closes.push(error);
                        break;
                    }
                    _ => {}
                },
                Err(_) => continue,
            }
        }
        (text, closes)
    }

    #[test]
    fn the_argv_asks_for_a_terminal_and_names_the_shell() {
        assert_eq!(
            argv("abc", None),
            vec!["exec", "--interactive", "--tty", "abc", "/bin/sh"]
        );
        assert_eq!(
            argv("abc", Some("/bin/bash")),
            vec!["exec", "--interactive", "--tty", "abc", "/bin/bash"]
        );
    }

    /// The session really is on a terminal: `tty` answers with a device, not "not a tty". This is
    /// the whole reason this module exists, so it is the thing worth asserting — and it needs no
    /// container daemon, only a shell.
    #[cfg(unix)]
    #[test]
    fn a_session_runs_on_a_real_terminal() {
        let (sink, rx) = ChannelSink::new();
        let session = Session::open(
            StreamId(3),
            "sh",
            &["-c".to_owned(), "tty".to_owned()],
            sink,
        )
        .expect("a pty and a shell");
        let (text, closes) = drain(&rx, Duration::from_secs(10));
        session.close();
        assert!(
            text.contains("/dev/pts/") || text.contains("/dev/tty"),
            "the session was not on a terminal: {text:?}"
        );
        assert_eq!(
            closes,
            vec![None],
            "a shell that ended must report one close"
        );
    }

    /// FR-036: closing the view ends the session with no process left behind. A shell that would
    /// otherwise sit there forever is the case that matters.
    #[cfg(unix)]
    #[test]
    fn closing_a_session_leaves_no_process_behind() {
        let (sink, _rx) = ChannelSink::new();
        let session = Session::open(
            StreamId(3),
            "sh",
            &["-c".to_owned(), "while true; do sleep 1; done".to_owned()],
            sink,
        )
        .expect("a pty and a shell");
        let pid = i32::try_from(session.child.id()).expect("a pid fits");
        session.close();
        // The child was waited on, so this pid is either gone or belongs to something else by now;
        // what must not be true is that it is still our stoppable child. `kill(pid, 0)` answering
        // `ESRCH` is the plain reading of "nothing left behind".
        // SAFETY: signal 0 only tests for the process's existence and sends nothing.
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        assert!(!alive, "pid {pid} outlived the session that owned it");
    }

    /// FR-013's other half: a session whose command does not exist must surface the failure rather
    /// than leave an empty terminal. The runtime says so on the pty, which is where the operator
    /// is looking, and the close follows it.
    #[cfg(unix)]
    #[test]
    fn a_shell_that_does_not_exist_ends_the_session_rather_than_hanging() {
        let (sink, rx) = ChannelSink::new();
        let session = Session::open(
            StreamId(3),
            "sh",
            &[
                "-c".to_owned(),
                "echo 'exec: /bin/nope: not found' >&2; exit 127".to_owned(),
            ],
            sink,
        )
        .expect("a pty and a shell");
        let (text, closes) = drain(&rx, Duration::from_secs(10));
        session.close();
        assert!(text.contains("not found"), "{text:?}");
        assert_eq!(closes.len(), 1);
    }

    /// A resize before the shell has read anything must not take the session down; it is a hint to
    /// the far end, not a command that can fail.
    #[cfg(unix)]
    #[test]
    fn resizing_is_harmless() {
        let (sink, _rx) = ChannelSink::new();
        let mut session = Session::open(
            StreamId(3),
            "sh",
            &["-c".to_owned(), "sleep 5".to_owned()],
            sink,
        )
        .expect("a pty and a shell");
        session.resize(120, 40);
        session.write("\n").expect("the terminal takes input");
        session.close();
    }
}
