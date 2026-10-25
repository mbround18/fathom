//! Where stream output goes: the one seam between the runtime and an app's event channel.
//!
//! Log follows and interactive sessions produce events while they run. The crate does not know
//! how an app delivers them (Fathom wraps them in its `BackendEventEnvelope`; Depths sends them
//! over a WebSocket), so it writes to a [`StreamSink`] and each app implements one.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::wire::StreamId;

/// What a running follow or session reports.
///
/// Only the events this crate itself produces. The variant names and the serde shape (`type` /
/// `data`) match the same variants of Fathom's own `StreamEvent`, so an app can forward them
/// unchanged; Fathom maps them 1:1 into its wider enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum StreamEvent {
    /// Bounded batch of container log lines.
    ContainerLogBatch {
        stream_id: StreamId,
        lines: Vec<String>,
        truncated: bool,
    },
    /// Output from an interactive container session.
    ContainerSessionOutput { stream_id: StreamId, data: String },
    /// The session ended, with the runtime's reason when it had one.
    ContainerSessionClosed {
        stream_id: StreamId,
        error: Option<String>,
    },
}

/// Receives a stream's events.
pub trait StreamSink: Send + Sync {
    /// Delivers one event. `false` means nobody is listening any more; a producer may stop early,
    /// but is never required to.
    fn emit(&self, ev: StreamEvent) -> bool;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::logs;
    use crate::runner::RunError;
    use crate::stream::{RunningProcess, StreamLine, StreamRunner};
    use crate::test_util::{ChannelSink, ScriptedProcess};

    /// Hands out one scripted process, whatever it is asked to run.
    struct Scripted(std::sync::Mutex<Option<Vec<StreamLine>>>);

    impl StreamRunner for Scripted {
        fn spawn(&self, _bin: &str, _args: &[&str]) -> Result<Box<dyn RunningProcess>, RunError> {
            let script = self.0.lock().unwrap().take().unwrap_or_default();
            Ok(Box::new(ScriptedProcess::new(script)))
        }
    }

    /// The seam itself: a follow over a scripted process delivers its output to whatever sink the
    /// app handed in, tagged with the app's stream id, and nothing else.
    #[tokio::test]
    async fn a_sink_receives_what_a_log_follow_produces() {
        let (sink, rx) = ChannelSink::new();
        let runner = Arc::new(Scripted(std::sync::Mutex::new(Some(vec![
            StreamLine::Stdout("2026-10-02T12:00:00Z booted".into()),
            StreamLine::Exited { success: true },
        ]))));
        let args = logs::argv("abc", false, None);
        let outcome = logs::follow(sink, StreamId(9), runner, "docker".into(), args).await;
        assert_eq!(outcome, Ok(()));
        let got: Vec<StreamEvent> = rx.try_iter().collect();
        assert_eq!(
            got,
            vec![StreamEvent::ContainerLogBatch {
                stream_id: StreamId(9),
                lines: vec!["2026-10-02T12:00:00Z booted".into()],
                truncated: false,
            }]
        );
    }

    /// The wire shape an app forwards: the same `type`/`data` envelope Fathom's enum uses.
    #[test]
    fn events_serialise_in_the_tagged_shape() {
        let json = serde_json::to_string(&StreamEvent::ContainerSessionClosed {
            stream_id: StreamId(3),
            error: None,
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"type":"ContainerSessionClosed","data":{"stream_id":3,"error":null}}"#
        );
    }
}
