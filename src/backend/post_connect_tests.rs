//! `Initialize` must not hold the state lock through optional integration discovery.
//!
//! Drives the real `run_cmd` path against a fake API server whose metrics-API probe hangs until
//! the test releases it, then checks that `Initialize` finishes, a pane refresh
//! (`RefreshRoleBindings`) completes while that probe is still in flight, and the probe's result
//! is recorded afterwards (or dropped when the context changed meanwhile).

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use kube::Client;
use tokio::sync::{Mutex, Notify};

use super::access::ClusterAccess;
use super::audit_log::LocalFileSink;
use super::state::{BackendState, MetricsAvailability};
use super::{persistence, run_cmd};
use crate::protocol::{BackendCmd, BackendEvent, BackendEventEnvelope, CmdRequest};

/// Gates for the fake server's metrics-API probe.
#[derive(Default)]
struct MetricsGate {
    started: Notify,
    release: Notify,
}

fn json(status: u16, body: &str) -> http::Response<kube::client::Body> {
    http::Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(kube::client::Body::from(body.as_bytes().to_vec()))
        .expect("response")
}

/// A minimal API server: watches never answer, the metrics API hangs until released and is then
/// "not served" (404), discovery is empty, and every other GET is an empty list.
fn fake_client(gate: Arc<MetricsGate>) -> Client {
    let service = tower::service_fn(move |req: http::Request<kube::client::Body>| {
        let gate = gate.clone();
        async move {
            let path = req.uri().path().to_owned();
            let query = req.uri().query().unwrap_or_default().to_owned();
            if query.contains("watch=true") {
                std::future::pending::<()>().await;
            }
            let response = if path.contains("metrics.k8s.io") {
                gate.started.notify_one();
                gate.release.notified().await;
                json(
                    404,
                    r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"not found","reason":"NotFound","code":404}"#,
                )
            } else if path == "/apis" {
                json(
                    200,
                    r#"{"kind":"APIGroupList","apiVersion":"v1","groups":[]}"#,
                )
            } else if path == "/api" {
                json(
                    200,
                    r#"{"kind":"APIVersions","versions":["v1"],"serverAddressByClientCIDRs":[]}"#,
                )
            } else if path == "/version" {
                json(
                    200,
                    r#"{"major":"1","minor":"34","gitVersion":"v1.34.0","gitCommit":"","gitTreeState":"","buildDate":"","goVersion":"","compiler":"","platform":""}"#,
                )
            } else {
                json(
                    200,
                    r#"{"kind":"List","apiVersion":"v1","metadata":{"resourceVersion":"1"},"items":[]}"#,
                )
            };
            Ok::<_, Infallible>(response)
        }
    });
    Client::new(service, "default")
}

/// Hosted-style access (no kubeconfig reads) that hands out the fake client.
struct FakeAccess {
    client: Client,
}

#[async_trait]
impl ClusterAccess for FakeAccess {
    async fn build_client(&self, _context: Option<&str>) -> anyhow::Result<Client> {
        Ok(self.client.clone())
    }
    fn home_context(&self) -> Option<String> {
        Some("fake".into())
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
    gate: Arc<MetricsGate>,
    _root: tempfile::TempDir,
}

fn harness() -> Harness {
    let root = tempfile::tempdir().expect("state root");
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.worker_threads(2).enable_all();
    persistence::scope_runtime_to_state_root(&mut builder, root.path().to_owned());
    let rt = builder.build().expect("runtime");

    let gate = Arc::new(MetricsGate::default());
    let client = rt.block_on(async { fake_client(gate.clone()) });
    let mut state = BackendState::with_seams(
        Arc::new(FakeAccess { client }),
        Arc::new(LocalFileSink),
        None,
    );
    state.cluster.current_context = Some("fake".into());
    let (sender, events) = crossbeam_channel::unbounded();
    Harness {
        rt,
        state: Arc::new(Mutex::new(state)),
        sender,
        events,
        gate,
        _root: root,
    }
}

impl Harness {
    /// Run `cmd` through `run_cmd` on its own task (as the backend loop does).
    fn spawn(&self, cmd: BackendCmd) -> tokio::task::JoinHandle<()> {
        let state = self.state.clone();
        let sender = self.sender.clone();
        self.rt.spawn(async move {
            run_cmd(CmdRequest::from(cmd), &state, &sender).await;
        })
    }

    /// Block until the next `CommandFinished`, returning its `ok`.
    fn next_finished(&self, within: Duration) -> bool {
        let deadline = std::time::Instant::now() + within;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let env = self
                .events
                .recv_timeout(left)
                .expect("CommandFinished before the deadline");
            if let BackendEvent::CommandFinished { ok, error } = env.event {
                assert!(ok, "command failed: {error:?}");
                return ok;
            }
        }
    }

    fn wait_metrics_probe_started(&self) {
        self.rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(10), self.gate.started.notified())
                .await
                .expect("post-connect discovery probes the metrics API");
        });
    }

    fn availability(&self) -> MetricsAvailability {
        self.rt
            .block_on(async { self.state.lock().await.metrics.availability })
    }
}

#[test]
fn refresh_is_not_blocked_behind_slow_post_connect_discovery() {
    let h = harness();
    let init = h.spawn(BackendCmd::Initialize);

    // Initialize finishes its required work and reports, while discovery is still in flight.
    assert!(h.next_finished(Duration::from_secs(10)));
    h.wait_metrics_probe_started();
    assert!(
        !init.is_finished(),
        "discovery should still be waiting on the metrics API"
    );

    // A pane refresh gets the lock and completes although the metrics probe still hangs.
    let _refresh = h.spawn(BackendCmd::RefreshRoleBindings);
    assert!(h.next_finished(Duration::from_secs(5)));
    assert_eq!(h.availability(), MetricsAvailability::Unknown);

    // Once the probe answers, its result is written back.
    h.gate.release.notify_one();
    h.rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(15), init)
            .await
            .expect("post-connect discovery finishes")
            .expect("no panic");
    });
    assert_eq!(h.availability(), MetricsAvailability::Unavailable);
}

#[test]
fn discovery_results_for_a_replaced_context_are_dropped() {
    let h = harness();
    let init = h.spawn(BackendCmd::Initialize);
    assert!(h.next_finished(Duration::from_secs(10)));
    h.wait_metrics_probe_started();

    // The context changes while discovery runs (as `SetContext` would do under the lock).
    h.rt.block_on(async {
        h.state.lock().await.cluster.current_context = Some("other".into());
    });
    h.gate.release.notify_one();
    h.rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(15), init)
            .await
            .expect("post-connect discovery finishes")
            .expect("no panic");
    });
    assert_eq!(
        h.availability(),
        MetricsAvailability::Unknown,
        "a stale probe result must not be recorded for the new context"
    );
}
