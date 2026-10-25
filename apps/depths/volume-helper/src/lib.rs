//! The Depths volume helper (spec 053): the only code that opens volume paths. The server starts
//! it with `docker run -i` and the fixed flags in contracts/helper.md; it sees exactly one mount
//! at `/volume` and speaks frames on stdin/stdout.
//!
//! [`serve`] is the whole protocol loop. It touches no globals, environment or stdio of its own,
//! so the server's tests can run it on a thread over pipes against a temp root.

pub mod manage;
pub mod ops;
pub mod resolve;
pub mod save;
pub mod transfer;

use std::io::{Read, Write};
use std::os::fd::OwnedFd;

use depths_volumes::frame::{FrameError, FrameKind, read_frame, write_json};
use depths_volumes::op::{Hello, HelperError, Op, PROTO};

use crate::resolve::Root;

/// Exit code after stdin closes.
pub const EXIT_OK: i32 = 0;
/// Exit code after a protocol error (`EX_SOFTWARE`); the server kills and removes the helper.
pub const EXIT_PROTOCOL: i32 = 70;

/// Sends `Hello`, then answers one op at a time until `input` ends. Returns the exit code:
/// [`EXIT_OK`] on EOF, [`EXIT_PROTOCOL`] when the frames do not follow the protocol (after a
/// best-effort `protocol` Error frame) or `output` fails.
pub fn serve<R: Read, W: Write>(root: OwnedFd, mut input: R, mut output: W) -> i32 {
    let root = Root::from_fd(root);
    let hello = Hello {
        proto: PROTO,
        openat2: root.probe_openat2(),
        uid: rustix::process::getuid().as_raw(),
        gid: rustix::process::getgid().as_raw(),
    };
    if write_json(&mut output, FrameKind::Hello, &hello).is_err() || output.flush().is_err() {
        return EXIT_PROTOCOL;
    }
    loop {
        match next_op(&mut input) {
            Ok(None) => return EXIT_OK,
            Ok(Some(op)) => {
                if ops::handle(&root, &op, &mut input, &mut output).is_err()
                    || output.flush().is_err()
                {
                    return EXIT_PROTOCOL;
                }
            }
            Err(e) => {
                let error = HelperError::Protocol {
                    message: e.to_string(),
                };
                let _ = write_json(&mut output, FrameKind::Error, &error);
                let _ = output.flush();
                return EXIT_PROTOCOL;
            }
        }
    }
}

/// The next op, `None` on a clean EOF. The Data… End that follow `Write`, `Put` and `Untar` are
/// read by the op itself.
fn next_op<R: Read>(input: &mut R) -> Result<Option<Op>, FrameError> {
    let Some(frame) = read_frame(input)? else {
        return Ok(None);
    };
    Ok(Some(frame.parse(FrameKind::Op)?))
}
