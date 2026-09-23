//! Integrations panel (Prometheus scan) flows. See
//! `tests/common/mod.rs` for the shared harness and gotchas.

mod common;

use common::setup;
use egui_kittest::kittest::Queryable;
use fathom::protocol::{
    BackendCmd, BackendEvent, MetricsStackScanResult, MetricsStackTier, Route, SystemEvent,
    section_keys,
};

/// Regression test for a bug found by inspection: `RefreshIntegrations`'s
/// error/no-client branches (`src/backend.rs`) used to call
/// `events::notify_error` but never emit `SystemEvent::MetricsStackScanned` -
/// the only event `IntegrationsView::on_event` uses to clear its `scanning`
/// flag. A failed scan (no cluster connected, an API error, RBAC, ...) left
/// the "Scan Cluster for Metrics Stack" button permanently disabled and
/// "Scanning..." stuck on screen forever, with no way to retry short of
/// switching cluster context. Fixed by also emitting an empty
/// `MetricsStackScanned` result alongside the error notification.
#[test]
fn scan_button_recovers_after_a_failed_scan() {
    let mut app = setup();
    app.navigate("Integrations", "  Metrics");

    app.harness
        .get_by_label("Scan Cluster for Metrics Stack")
        .click();
    app.harness.run_steps(2);

    assert!(
        app.harness.query_by_label("Scanning...").is_some(),
        "expected the scanning indicator to show right after clicking Scan"
    );

    // Simulate the backend's failure path (e.g. scan_metrics_stack errored,
    // or there's no cluster client) - it now emits an empty result alongside
    // the error notification, instead of nothing at all.
    app.push_event(
        Route::global(section_keys::integrations_metrics()),
        BackendEvent::System(SystemEvent::MetricsStackScanned(MetricsStackScanResult {
            tier: MetricsStackTier::None,
            components: Vec::new(),
            suggested_endpoint: None,
        })),
    );

    assert!(
        app.harness.query_by_label("Scanning...").is_none(),
        "scanning indicator should clear once a result (even an empty one) arrives"
    );

    // The button itself should be clickable again - if it were still
    // add_enabled(false), a second click here would just be silently
    // swallowed rather than dispatching another RefreshIntegrations.
    app.drain_commands();
    app.harness
        .get_by_label("Scan Cluster for Metrics Stack")
        .click();
    app.harness.run_steps(2);

    match app.next_command() {
        BackendCmd::RefreshIntegrations => {}
        other => panic!("expected RefreshIntegrations, got {other:?}"),
    }
}
