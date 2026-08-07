//! Verifies what happens to a resource list's data when the user switches
//! Kubernetes context (cluster). See `tests/common/mod.rs` for the shared
//! harness and gotchas.

mod common;

use accesskit::Role;
use common::setup;
use egui_kittest::kittest::{By, Queryable};
use fathom::protocol::{
    BackendEvent, PodSummary, ResourceListEvent, Route, SystemEvent, section_keys,
};

/// Regression test for a bug found via this harness: `mailbox.rs`'s handler
/// for `SystemEvent::ContextChanged` used to be a no-op with the comment
/// "views handle their own state cleanup via on_unmount/on_mount lifecycle" -
/// which wasn't true. `on_mount`/`on_unmount` only fire on *section* changes
/// (`handle_view_switch` compares `previous_section` to `active_section`),
/// never on a context switch with the section held constant. Only
/// `ContextView` itself listened for `ContextChanged` at all, so every other
/// cached view (PodsView, the generic WorkloadView, etc.) kept rendering
/// whatever data it last received from the *previous* cluster, indefinitely,
/// with nothing in the UI to indicate it was stale.
///
/// Fixed in `mailbox.rs`: on `ContextChanged`, drop every cached view except
/// `ContextView` (which just processed the event) and reset
/// `previous_section = None`, so `handle_view_switch` treats the active
/// section as freshly mounted next frame - recreating it from `Default`
/// (clearing stale data) and re-sending its `Refresh*` command against the
/// new context.
///
/// This test drives the scenario end-to-end: load pod data for "cluster-a",
/// switch the context combo box to "cluster-b", simulate the backend echoing
/// back `ContextChanged { current: Some("cluster-b") }`, and check that the
/// stale "cluster-a" pod is gone.
#[test]
fn pods_are_cleared_after_context_switch() {
    let mut app = setup();

    // "Workloads" nav group is expanded by default; Pods is not nested under
    // Overview, so it gets the 2-space indent prefix.
    app.navigate("Workloads", "  Pods");

    app.push_event(
        Route::global(section_keys::workloads_pods()),
        BackendEvent::ResourceList(ResourceListEvent::PodsUpdated(vec![PodSummary {
            name: "pod-cluster-a".to_owned(),
            namespace: "default".to_owned(),
            phase: "Running".to_owned(),
            node: "node1".to_owned(),
            ready: 1,
            total: 1,
            restarts: 0,
        }])),
    );

    assert!(
        app.harness.query_by_label("pod-cluster-a").is_some(),
        "expected the pod to render before switching context"
    );

    // Populate the context selector - it starts empty in a fresh test app
    // (no ContextsUpdated event has fired yet).
    app.push_event(
        Route::global(section_keys::system()),
        BackendEvent::System(SystemEvent::ContextsUpdated {
            contexts: vec!["cluster-a".to_owned(), "cluster-b".to_owned()],
            current: Some("cluster-a".to_owned()),
        }),
    );

    // egui::ComboBox exposes its selected text as the node's *value*, not
    // its label - open it via that.
    app.harness
        .get(By::new().role(Role::ComboBox).value("cluster-a"))
        .click();
    app.harness.run_steps(2);

    // Rendered via ui.selectable_label(selected, &ctx_name), so a plain
    // label query works. This dispatches BackendCmd::SetContext -
    // it just queues unread in app.cmds_rx since no backend is attached.
    app.harness.get_by_label("cluster-b").click();
    app.harness.run_steps(2);

    // Simulate what a real backend would echo back after actually switching
    // context.
    app.push_event(
        Route::global(section_keys::system()),
        BackendEvent::System(SystemEvent::ContextChanged {
            current: Some("cluster-b".to_owned()),
        }),
    );

    assert!(
        app.harness.query_by_label("pod-cluster-a").is_none(),
        "stale pod data from the previous cluster context should not still be visible after switching context"
    );
}
