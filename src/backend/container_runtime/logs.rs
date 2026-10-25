//! 048-docker-management US5: following one container's log output.
//!
//! An adapter, not a second log pipeline. The argv, the batching cadence and the pump live in the
//! shared crate (spec 050 T008); the registry, the single `StreamClosed` per stream and the "a
//! restart under the same id is silent" rule all come from [`crate::backend::log_streams`], which
//! pod log follows already use. What this module adds is the bridge: the crate reports how the
//! follow ended, and the [`CloseGuard`] turns that into this stream's close event.
//!
//! Reading a container's output is allowed whether or not its project has been adopted (FR-035).
//! `adoption::requirement` classifies `StartContainerLogStream` as `Free` for exactly that reason,
//! while a session — which is unbounded execution inside someone else's work — stays gated.

use std::sync::Arc;

use super::super::log_streams::CloseGuard;
use super::stream::StreamRunner;

pub use ::container_runtime::logs::argv;

/// Follows one container's output until it ends, fails, or the stream is stopped.
///
/// Spawned through `LogStreams::spawn`, so `guard` emits this stream's one `StreamClosed` however
/// the follow ends. Aborting the task drops the crate's follow, which kills the process.
pub async fn follow(
    mut guard: CloseGuard,
    runner: Arc<dyn StreamRunner>,
    bin: String,
    args: Vec<String>,
) {
    let sink = guard.sink().clone();
    let stream_id = sink.stream_id;
    match ::container_runtime::logs::follow(sink, stream_id, runner, bin, args).await {
        Ok(()) => guard.ended(),
        Err(detail) => guard.failed(detail),
    }
}
