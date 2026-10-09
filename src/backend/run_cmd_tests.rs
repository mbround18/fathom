//! `run_cmd` must survive what its handlers do: a panicking handler reports
//! `CommandFinished { ok: false }` instead of killing the task (and, on the ordered lane, every
//! later terminal command with it), and a stalled handler is cut off at its timeout so the next
//! command gets the state lock.
//!
//! Both faults are injected through [`super::test_hooks`], keyed by request id, so the
//! protocol enum stays unchanged and parallel tests never see each other's hooks.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use kube::Client;
use tokio::sync::Mutex;

use super::access::ClusterAccess;
use super::audit_log::LocalFileSink;
use super::state::BackendState;
use super::{persistence, run_cmd, test_hooks};
use crate::protocol::{BackendCmd, BackendEvent, BackendEventEnvelope, CmdRequest, RequestId};

/// Access that never hands out a client: the commands under test touch no cluster.
struct NoAccess;

#[async_trait]
impl ClusterAccess for NoAccess {
    async fn build_client(&self, _context: Option<&str>) -> anyhow::Result<Client> {
        anyhow::bail!("no cluster in this test")
    }
    fn allows_local_features(&self) -> bool {
        false
    }
}

struct Harness {
    rt: tokio::runtime::Runtime,
    state: Arc<Mutex<BackendState>>,
    sender: crossbeam_channel::Sender<BackendEventEnvelope>,
    events: crossbeam_channel::Receiver<BackendEventEnvelope>,
    _root: tempfile::TempDir,
}

fn harness() -> Harness {
    let root = tempfile::tempdir().expect("state root");
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.worker_threads(2).enable_all();
    persistence::scope_runtime_to_state_root(&mut builder, root.path().to_owned());
    let rt = builder.build().expect("runtime");
    let state = BackendState::with_seams(Arc::new(NoAccess), Arc::new(LocalFileSink), None);
    let (sender, events) = crossbeam_channel::unbounded();
    Harness {
        rt,
        state: Arc::new(Mutex::new(state)),
        sender,
        events,
        _root: root,
    }
}

impl Harness {
    /// Run `requests` one after another on a single task, as the ordered lane does.
    fn run_in_order(&self, requests: Vec<CmdRequest>) -> tokio::task::JoinHandle<()> {
        let state = self.state.clone();
        let sender = self.sender.clone();
        self.rt.spawn(async move {
            for request in requests {
                run_cmd(request, &state, &sender).await;
            }
        })
    }

    /// Block until the next `CommandFinished`, returning its `(ok, error)` and correlation.
    fn next_finished(&self, within: Duration) -> (bool, Option<String>, Option<RequestId>) {
        let deadline = std::time::Instant::now() + within;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let env = self
                .events
                .recv_timeout(left)
                .expect("CommandFinished before the deadline");
            if let BackendEvent::CommandFinished { ok, error } = env.event {
                let id = match env.correlation {
                    crate::protocol::Correlation::Request(id) => Some(id),
                    _ => None,
                };
                return (ok, error, id);
            }
        }
    }
}

fn request(id: u64) -> CmdRequest {
    CmdRequest::new(RequestId(id), BackendCmd::SimulateSlowCall)
}

#[test]
fn a_panicking_handler_finishes_failed_and_the_lane_keeps_running() {
    let h = harness();
    test_hooks::panic_on(9001);

    let lane = h.run_in_order(vec![request(9001), request(9002)]);

    let (ok, error, id) = h.next_finished(Duration::from_secs(10));
    assert!(!ok, "a panic must report the command as failed");
    assert_eq!(id, Some(RequestId(9001)));
    let error = error.expect("an error message");
    assert!(
        error.starts_with("internal error: ") && error.contains("injected panic"),
        "unexpected error text: {error}"
    );

    // The next command on the same task still runs to completion (SimulateSlowCall ~2s).
    let (ok, error, id) = h.next_finished(Duration::from_secs(10));
    assert!(ok, "the lane must survive the panic: {error:?}");
    assert_eq!(id, Some(RequestId(9002)));
    h.rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), lane)
            .await
            .expect("lane task finishes")
            .expect("lane task did not panic");
    });
}
#[test]
fn a_stalled_handler_times_out_and_the_next_command_runs() {
    let h = harness();
    // SimulateSlowCall sleeps 2s; cut it off well before that.
    test_hooks::timeout_for(9101, Duration::from_millis(100));

    let started = std::time::Instant::now();
    let lane = h.run_in_order(vec![request(9101), request(9102)]);

    let (ok, error, id) = h.next_finished(Duration::from_secs(10));
    assert!(!ok, "a timed-out command must report as failed");
    assert_eq!(id, Some(RequestId(9101)));
    let error = error.expect("an error message");
    assert!(
        error.contains("timed out after"),
        "unexpected error: {error}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the timeout must fire before the handler would have finished"
    );

    let (ok, error, id) = h.next_finished(Duration::from_secs(10));
    assert!(ok, "the next command must run after a timeout: {error:?}");
    assert_eq!(id, Some(RequestId(9102)));
    h.rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), lane)
            .await
            .expect("lane task finishes")
            .expect("lane task did not panic");
    });
}
