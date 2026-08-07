//! Pods view: selection and log-stream flows. See `tests/common/mod.rs` for
//! the shared harness and gotchas.

mod common;

use common::setup;
use egui_kittest::kittest::Queryable;
use fathom::protocol::{
    BackendEvent, PodSummary, ResourceListEvent, Route, StreamEvent, StreamId, section_keys,
};

fn pod(name: &str, namespace: &str) -> PodSummary {
    PodSummary {
        name: name.to_owned(),
        namespace: namespace.to_owned(),
        phase: "Running".to_owned(),
        node: "node1".to_owned(),
        ready: 1,
        total: 1,
        restarts: 0,
    }
}

/// Regression test for a bug found by inspection: `PodsView::on_event` used
/// to append every `PodLogsChunk` to whatever `pod_logs_stream` was current,
/// completely ignoring the event's own `stream_id`. Backend log-tailing
/// tasks are stopped via `JoinHandle::abort()`, which cancels the task at
/// its next await point but doesn't retract chunks already queued for
/// delivery - so switching pods mid-stream could leave a handful of the
/// *previous* pod's log lines arriving after a fresh log view was created
/// for the new selection, and get silently appended into the wrong pod's
/// logs. Fixed by `LogStreamState::accepts_chunk`, which rejects chunks
/// tagged with an older backend stream id than the one already established
/// as current.
///
/// This test drives the actual race through the real view: select pod-a,
/// start its logs, switch to pod-b and start its logs too, then simulate a
/// straggler chunk from pod-a's now-superseded stream arriving late and
/// check it doesn't show up in pod-b's log view.
#[test]
fn stale_log_chunk_from_a_previous_pod_does_not_leak_into_the_new_selection() {
    let mut app = setup();
    app.navigate("Workloads", "  Pods");

    app.push_event(
        Route::global(section_keys::workloads_pods()),
        BackendEvent::ResourceList(ResourceListEvent::PodsUpdated(vec![
            pod("pod-a", "default"),
            pod("pod-b", "default"),
        ])),
    );

    // Select pod-a and switch to its Logs tab (this alone creates a fresh
    // LogStreamState, independent of clicking Start).
    app.harness.get_by_label("pod-a").click();
    app.harness.run_steps(2);
    app.harness.get_by_label("Logs").click();
    app.harness.run_steps(2);

    // pod-a's stream (backend stream id 1) delivers a line.
    app.push_event(
        Route::global(section_keys::workloads_pods()),
        BackendEvent::Stream(StreamEvent::PodLogsChunk {
            stream_id: StreamId(1),
            lines: vec!["from-pod-a".to_owned()],
        }),
    );
    assert!(app.harness.query_by_label("from-pod-a").is_some());

    // Switch to pod-b - this clears pod_logs_stream and, since the tab stays
    // on Logs, immediately recreates a fresh LogStreamState for pod-b.
    // Uses click_accesskit() rather than click() - see the note in
    // tests/common/mod.rs about coordinate clicks after a panel's content
    // has changed shape.
    app.harness.get_by_label("pod-b").click_accesskit();
    app.harness.run_steps(2);

    assert!(
        app.harness.query_by_label("from-pod-a").is_none(),
        "switching pods should drop the previous pod's log view entirely"
    );

    // pod-b's stream (backend stream id 2) delivers its own line first,
    // establishing 2 as the current stream id.
    app.push_event(
        Route::global(section_keys::workloads_pods()),
        BackendEvent::Stream(StreamEvent::PodLogsChunk {
            stream_id: StreamId(2),
            lines: vec!["from-pod-b".to_owned()],
        }),
    );
    assert!(app.harness.query_by_label("from-pod-b").is_some());

    // A straggler from pod-a's now-superseded stream (id 1 < current 2)
    // arrives late - it must be rejected, not appended into pod-b's log view.
    app.push_event(
        Route::global(section_keys::workloads_pods()),
        BackendEvent::Stream(StreamEvent::PodLogsChunk {
            stream_id: StreamId(1),
            lines: vec!["stale-from-pod-a".to_owned()],
        }),
    );
    assert!(
        app.harness.query_by_label("stale-from-pod-a").is_none(),
        "a straggler chunk from pod-a's superseded stream must not appear in pod-b's log view"
    );
    assert!(app.harness.query_by_label("from-pod-b").is_some());
}
