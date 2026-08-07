//! Flux dashboard, Namespaces, and delete/scale/bulk-delete modal flows.
//! See `tests/common/mod.rs` for the shared harness and gotchas.

mod common;

use accesskit::Role;
use common::setup;
use egui_kittest::kittest::{By, Queryable};
use fathom::protocol::{
    BackendCmd, BackendEvent, ConfigMapSummary, FluxOverview, FluxReadyState, FluxResourceSummary,
    NamespaceSummary, ResourceListEvent, Route, SystemEvent, WorkloadSummary, section_keys,
};

fn flux_resource(
    kind: &str,
    name: &str,
    ready: FluxReadyState,
    message: &str,
) -> FluxResourceSummary {
    FluxResourceSummary {
        kind: kind.to_owned(),
        name: name.to_owned(),
        namespace: "flux-system".to_owned(),
        ready,
        message: message.to_owned(),
        revision: "main@sha1:abc123".to_owned(),
        source_ref: None,
        suspended: false,
        interval: "5m".to_owned(),
        last_transition: "2026-01-01T00:00:00Z".to_owned(),
    }
}

#[test]
fn overview_is_the_default_landing_section() {
    let app = setup();
    assert!(app.harness.query_by_label("Cluster Overview").is_some());
}

#[test]
fn navigates_to_flux_dashboard_and_renders_empty_state() {
    let mut app = setup();
    app.navigate("Integrations", "  Flux");

    // No backend attached, so FluxView is still at its default (installed: false)
    // state - it should render the documented empty state, not panic.
    assert!(
        app.harness.query_by_label("Flux not detected").is_some(),
        "expected Flux dashboard empty state after navigating with no backend connected"
    );
}

#[test]
fn flux_dashboard_renders_sources_kustomizations_and_helmreleases() {
    let mut app = setup();
    app.navigate("Integrations", "  Flux");

    let overview = FluxOverview {
        installed: true,
        sources: vec![flux_resource(
            "GitRepository",
            "webapp-repo",
            FluxReadyState::Ready,
            "stored artifact for revision 'main@sha1:abc123'",
        )],
        kustomizations: vec![flux_resource(
            "Kustomization",
            "webapp-ks",
            FluxReadyState::Failed,
            "health check failed: Deployment/webapp not ready",
        )],
        helmreleases: vec![flux_resource(
            "HelmRelease",
            "webapp-release",
            FluxReadyState::Ready,
            "Helm install succeeded",
        )],
    };
    app.push_event(
        Route::global(section_keys::integrations_flux()),
        BackendEvent::System(SystemEvent::FluxOverviewUpdated(overview)),
    );

    // "Flux not detected" empty state should be gone now that data arrived.
    assert!(app.harness.query_by_label("Flux not detected").is_none());

    // Sources is the default tab - the Git source should already be visible.
    assert!(app.harness.query_by_label("webapp-repo").is_some());
    assert!(app.harness.query_by_label("webapp-ks").is_none());

    // "Kustomizations" also appears as a summary-card label, so disambiguate
    // by role - the tab strip renders its entries as selectable buttons.
    app.harness
        .get_by_role_and_label(Role::Button, "Kustomizations")
        .click();
    app.harness.run_steps(2);
    assert!(app.harness.query_by_label("webapp-ks").is_some());

    app.harness
        .get_by_role_and_label(Role::Button, "HelmReleases")
        .click();
    app.harness.run_steps(2);
    assert!(app.harness.query_by_label("webapp-release").is_some());
}

/// Regression test for a bug where the Namespaces list page stayed empty
/// forever: the backend only ever emitted `NamespacesUpdated(Vec<String>)`
/// (for the filter dropdown), never the `NamespaceSummariesUpdated` event
/// this page actually listens for. Fixed in `backend/sync.rs::refresh_namespaces`.
#[test]
fn namespaces_list_renders_after_backend_refresh() {
    let mut app = setup();
    // "Namespaces" is a subsection under "Overview", which is expanded by default.
    app.navigate("Overview", "    Namespaces");

    assert!(app.harness.query_by_label("No namespaces found").is_some());

    app.push_event(
        Route::global(section_keys::system()),
        BackendEvent::ResourceList(ResourceListEvent::NamespaceSummariesUpdated(vec![
            NamespaceSummary {
                name: "prod".to_owned(),
                status: "Active".to_owned(),
                pod_count: 12,
                deployment_count: 4,
                creation_timestamp: "2026-01-01T00:00:00Z".to_owned(),
            },
            NamespaceSummary {
                name: "staging".to_owned(),
                status: "Active".to_owned(),
                pod_count: 3,
                deployment_count: 1,
                creation_timestamp: "2026-01-01T00:00:00Z".to_owned(),
            },
        ])),
    );

    assert!(app.harness.query_by_label("No namespaces found").is_none());
    assert!(app.harness.query_by_label("prod").is_some());
    assert!(app.harness.query_by_label("staging").is_some());
}

/// Regression test for a bug found via this harness: `render_right_panel`
/// (`src/app/ui_right.rs`) used to only re-attempt showing the details panel
/// on the app's very first frame, or when `show_details_panel` was already
/// `true` - a flag only ever armed by `render_center` for a hardcoded
/// whitelist of `BackendCmd::Select*` variants. FluxView (and ~14 other views
/// with a `render_details()` impl) never dispatched one of those commands, so
/// their details panels could never appear after the first frame. Fixed by
/// adding `ViewComponent::has_details(&self) -> bool`, which every view with
/// a details panel now overrides and `render_right_panel` checks fresh every
/// frame instead of relying on a stale, command-specific flag.
#[test]
fn flux_details_panel_shows_selected_resource() {
    let mut app = setup();
    app.navigate("Integrations", "  Flux");

    let overview = FluxOverview {
        installed: true,
        sources: vec![flux_resource(
            "GitRepository",
            "webapp-repo",
            FluxReadyState::Ready,
            "stored artifact for revision 'main@sha1:abc123'",
        )],
        kustomizations: vec![],
        helmreleases: vec![],
    };
    app.push_event(
        Route::global(section_keys::integrations_flux()),
        BackendEvent::System(SystemEvent::FluxOverviewUpdated(overview)),
    );

    app.harness.get_by_label("webapp-repo").click();
    app.harness.run_steps(3);

    // `render_details()`'s heading is the resource's kind - only appears in
    // the details panel, never the list rows, so this is an unambiguous probe.
    assert!(
        app.harness.query_by_label("GitRepository").is_some(),
        "expected the details panel to show after selecting a Flux resource"
    );
}

/// Selecting a namespace in the top-bar filter should dispatch the namespace
/// list the backend needs to actually filter - not, say, an empty list or
/// stale data from a previous selection.
#[test]
fn selecting_a_namespace_sends_the_correct_filter_command() {
    let mut app = setup();

    app.push_event(
        Route::global(section_keys::system()),
        BackendEvent::ResourceList(ResourceListEvent::NamespacesUpdated(vec![
            "prod".to_owned(),
            "staging".to_owned(),
        ])),
    );
    app.drain_commands();

    // The namespace combo box shows "All" until a specific namespace is
    // selected - egui::ComboBox exposes its selected text as the node's
    // *value*, not its label, so query on that. Open it (its checkboxes are
    // only rendered once open).
    app.harness
        .get(By::new().role(Role::ComboBox).value("All"))
        .click();
    app.harness.run_steps(2);

    app.harness.get_by_label("prod").click();
    app.harness.run_steps(2);

    match app.next_command() {
        BackendCmd::SetNamespaceFilter { namespaces } => {
            assert_eq!(namespaces, vec!["prod".to_owned()]);
        }
        other => panic!("expected SetNamespaceFilter, got {other:?}"),
    }
}

/// End-to-end delete flow for ConfigMaps: select a row, open the details
/// panel's Delete button, confirm in the modal, and check the exact
/// namespace/name that reaches the backend - catching both "wrong resource
/// selected" and "modal confirmed but nothing was actually sent" bugs.
#[test]
fn delete_configmap_confirmation_sends_correct_command() {
    let mut app = setup();
    app.navigate("Config", "  ConfigMaps");

    app.push_event(
        Route::global(section_keys::config_configmaps()),
        BackendEvent::ResourceList(ResourceListEvent::ConfigMapsUpdated(vec![
            ConfigMapSummary {
                name: "app-config".to_owned(),
                namespace: "prod".to_owned(),
                keys_count: 3,
            },
        ])),
    );

    app.harness.get_by_label("app-config").click();
    app.harness.run_steps(2);

    // has_details() gates this button - if it never appears, selection or
    // the details-panel gating broke.
    app.harness.get_by_label("Delete").click();
    app.harness.run_steps(2);

    assert!(
        app.harness.query_by_label("Delete Config Map").is_some(),
        "expected the confirmation modal to open"
    );

    app.drain_commands();
    assert!(
        app.cmds_rx.try_recv().is_err(),
        "delete command must not fire before the modal is confirmed"
    );

    // Two "Delete" buttons exist now: the (obscured) details-panel one and
    // the modal's confirm button, rendered after it this frame - the last
    // match in tree order is the modal's.
    app.harness
        .get_all_by_label("Delete")
        .last()
        .expect("modal confirm button")
        .click();
    app.harness.run_steps(2);

    match app.next_command() {
        BackendCmd::DeleteConfigMap { namespace, name } => {
            assert_eq!(namespace, "prod");
            assert_eq!(name, "app-config");
        }
        other => panic!("expected DeleteConfigMap, got {other:?}"),
    }
}

/// Bulk-select two Deployment rows via their checkboxes, confirm the bulk
/// delete modal, and check the exact set of (namespace, name) pairs sent -
/// catching both "wrong items selected" and the same
/// modal-never-created class of bug fixed for single-resource deletes.
#[test]
fn bulk_delete_selected_deployments_sends_correct_command() {
    let mut app = setup();
    app.navigate("Workloads", "  Deployments");

    app.push_event(
        Route::global(section_keys::workloads_deployments()),
        BackendEvent::ResourceList(ResourceListEvent::DeploymentsUpdated(vec![
            WorkloadSummary {
                name: "dep-a".to_owned(),
                namespace: "team-a".to_owned(),
                desired: 2,
                ready: 2,
                updated: 2,
                available: 2,
            },
            WorkloadSummary {
                name: "dep-b".to_owned(),
                namespace: "team-b".to_owned(),
                desired: 1,
                ready: 1,
                updated: 1,
                available: 1,
            },
        ])),
    );

    // Every row checkbox (and the header "select all" checkbox) renders the
    // same "☐"/"☑" glyph as its label, so before anything is selected the
    // header and both rows are indistinguishable by label - skip the header
    // (tree order: header, then rows) and take the first row's checkbox.
    app.harness
        .get_all_by_label("☐")
        .nth(1)
        .expect("first row checkbox")
        .click();
    app.harness.run_steps(2);

    // Once anything is selected, the header flips to the partial-selection
    // glyph "☒", leaving exactly one "☐" - the second row - unambiguous now.
    app.harness.get_by_label("☐").click();
    app.harness.run_steps(2);

    assert!(app.harness.query_by_label("2 selected").is_some());

    app.harness.get_by_label("🗑 Delete 2").click();
    app.harness.run_steps(2);

    assert!(
        app.harness.query_by_label("Delete All").is_some(),
        "expected the bulk delete confirmation modal to open"
    );

    app.drain_commands();
    app.harness.get_by_label("Delete All").click();
    app.harness.run_steps(2);

    match app.next_command() {
        BackendCmd::BulkDelete { mut items } => {
            items.sort();
            assert_eq!(
                items,
                vec![
                    ("team-a".to_owned(), "dep-a".to_owned()),
                    ("team-b".to_owned(), "dep-b".to_owned()),
                ]
            );
        }
        other => panic!("expected BulkDelete, got {other:?}"),
    }
}

/// End-to-end scale flow: select a Deployment, open the Scale modal, apply
/// it, and check the exact command sent - this exercises the same
/// modal-creation path as delete/bulk-delete but through a different
/// `ViewAction` (`OpenScaleModal` instead of `OpenModal`).
#[test]
fn scale_deployment_confirmation_sends_correct_command() {
    let mut app = setup();
    app.navigate("Workloads", "  Deployments");

    app.push_event(
        Route::global(section_keys::workloads_deployments()),
        BackendEvent::ResourceList(ResourceListEvent::DeploymentsUpdated(vec![
            WorkloadSummary {
                name: "web".to_owned(),
                namespace: "prod".to_owned(),
                desired: 3,
                ready: 3,
                updated: 3,
                available: 3,
            },
        ])),
    );

    app.harness.get_by_label("web").click();
    app.harness.run_steps(2);

    app.harness.get_by_label("Scale").click();
    app.harness.run_steps(2);

    assert!(
        app.harness.query_by_label("Scale Workload").is_some(),
        "expected the scale modal to open"
    );

    app.drain_commands();
    app.harness.get_by_label("Apply").click();
    app.harness.run_steps(2);

    match app.next_command() {
        BackendCmd::ScaleDeployment {
            namespace,
            name,
            replicas,
        } => {
            assert_eq!(namespace, "prod");
            assert_eq!(name, "web");
            assert_eq!(replicas, 3);
        }
        other => panic!("expected ScaleDeployment, got {other:?}"),
    }
}
