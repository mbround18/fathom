//! Port forward creation flow. See `tests/common/mod.rs` for the shared
//! harness and gotchas.

mod common;

use common::setup;
use egui_kittest::kittest::{By, Queryable};
use fathom::protocol::{
    BackendCmd, BackendEvent, ContainerPort, PodDetails, PodSummary, ResourceListEvent,
    ResourceReadEvent, Route, section_keys,
};
use std::collections::BTreeMap;

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

fn pod_details() -> PodDetails {
    PodDetails {
        namespace: "prefill-ns".to_owned(),
        name: "prefill-pod".to_owned(),
        creation_timestamp: "2026-01-01T00:00:00Z".to_owned(),
        labels: BTreeMap::new(),
        annotations: BTreeMap::new(),
        containers: vec!["app".to_owned()],
        ports: vec![ContainerPort {
            container: "app".to_owned(),
            name: Some("http".to_owned()),
            port: 8080,
            protocol: "TCP".to_owned(),
        }],
        resource_specs: Vec::new(),
        owner: None,
        resolved_owner: None,
        persistent_volume_claims: Vec::new(),
        raw_json: "{}".to_owned(),
        containers_lifecycle: Vec::new(),
    }
}

/// Regression test for a bug found by inspection: the "Create Port Forward"
/// modal (`src/app/views/modal.rs`) called
/// `render_autocomplete_field(ui, "Namespace", &mut namespace.clone(), ...)` -
/// passing a mutable reference to a *throwaway clone* of `namespace`, not
/// to `namespace` itself. `ui.text_edit_singleline` inside that helper
/// dutifully edited the clone every frame, which was then dropped at the end
/// of the call - so the Namespace and Pod fields were rendered, looked
/// interactive, accepted keystrokes, but silently discarded every one of
/// them. Clicking "Create" always used whatever namespace/pod the modal was
/// originally opened with, no matter what was typed. Same bug, same fix, for
/// the Pod field.
///
/// This test opens the modal pre-filled from a pod's port ("prefill-ns" /
/// "prefill-pod"), clears and retypes both fields to different values, and
/// checks the dispatched `StartPortForward` command reflects what was
/// actually typed - not the original pre-fill.
#[test]
fn typed_namespace_and_pod_are_used_when_creating_a_port_forward() {
    let mut app = setup();
    app.navigate("Network", "  Port Forwards");
    app.drain_commands();

    // Populate the Pods view (a different section) with the pod whose
    // details we're about to inject, then select it and open the Forward
    // button - mirrors how a real user reaches this modal.
    app.navigate("Workloads", "  Pods");
    app.push_event(
        Route::global(section_keys::workloads_pods()),
        BackendEvent::ResourceList(ResourceListEvent::PodsUpdated(vec![pod(
            "prefill-pod",
            "prefill-ns",
        )])),
    );
    app.harness.get_by_label("prefill-pod").click();
    app.harness.run_steps(2);

    app.push_event(
        Route::global(section_keys::workloads_pods()),
        BackendEvent::ResourceRead(ResourceReadEvent::PodDetailsUpdated(pod_details())),
    );

    app.harness.get_by_label("⚡ Forward").click();
    app.harness.run_steps(2);

    assert!(
        app.harness.query_by_label("Create Port Forward").is_some(),
        "expected the Create Port Forward modal to open, pre-filled from the pod"
    );

    // Namespace field: select-all and retype. The field is a plain
    // TextEdit with no distinguishing label/value while non-empty except
    // its current text - it's pre-filled with "prefill-ns", click it,
    // select all (Harness-level key combo - focus lives with whatever was
    // last clicked), and type over the selection.
    let ns_field = By::new()
        .role(accesskit::Role::TextInput)
        .value("prefill-ns");
    app.harness.get(ns_field.clone()).click();
    app.harness.run_steps(1);
    app.harness
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    app.harness.get(ns_field).type_text("typed-ns");
    app.harness.run_steps(2);

    let pod_field = By::new()
        .role(accesskit::Role::TextInput)
        .value("prefill-pod");
    app.harness.get(pod_field.clone()).click();
    app.harness.run_steps(1);
    app.harness
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    app.harness.get(pod_field).type_text("typed-pod");
    app.harness.run_steps(2);

    app.drain_commands();
    app.harness.get_by_label("Create").click();
    app.harness.run_steps(2);

    match app.next_command() {
        BackendCmd::StartPortForward { namespace, pod, .. } => {
            assert_eq!(
                namespace, "typed-ns",
                "expected the typed namespace, not the original pre-fill"
            );
            assert_eq!(
                pod, "typed-pod",
                "expected the typed pod name, not the original pre-fill"
            );
        }
        other => panic!("expected StartPortForward, got {other:?}"),
    }
}
