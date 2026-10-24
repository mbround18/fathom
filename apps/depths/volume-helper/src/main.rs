//! The Depths volume helper (spec 053). The server starts it with `docker run -i` and the fixed
//! flags in contracts/helper.md; it sees exactly one mount at `/volume`.
//!
//! Debug builds take the root from `DEPTHS_HELPER_ROOT` instead, for the tests that spawn this
//! binary against a temp tree; release builds ignore it.

use std::path::PathBuf;

use depths_volume_helper::resolve::{Root, errno_message, errno_name};
use depths_volume_helper::{EXIT_PROTOCOL, serve};
use depths_volumes::frame::{FrameKind, write_json};
use depths_volumes::op::HelperError;

fn root_path() -> PathBuf {
    #[cfg(debug_assertions)]
    if let Some(root) = std::env::var_os("DEPTHS_HELPER_ROOT") {
        return root.into();
    }
    PathBuf::from("/volume")
}

fn main() {
    let code = match Root::open(root_path()) {
        Ok(root) => serve(
            root.into_fd(),
            std::io::stdin().lock(),
            std::io::stdout().lock(),
        ),
        // No Hello without a root: the server sees an Error frame first and kills the helper.
        Err(e) => {
            let error = HelperError::OsError {
                errno: errno_name(e),
                message: errno_message(e),
            };
            let mut out = std::io::stdout().lock();
            let _ = write_json(&mut out, FrameKind::Error, &error);
            let _ = std::io::Write::flush(&mut out);
            EXIT_PROTOCOL
        }
    };
    std::process::exit(code);
}
