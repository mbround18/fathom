use super::traits::{ViewAction, ViewComponent};
use crate::protocol::{
    BackendCmd, BackendEvent, DeploymentDetails, DeploymentSummary, ResourceListEvent,
    ResourceReadEvent,
};

/// Shared helper for rendering workload details panel
fn render_workload_details(
    ui: &mut egui::Ui,
    workload_type: &str,
    details: &Option<DeploymentDetails>,
    supports_scaling: bool,
    actions: &mut Vec<ViewAction>,
) {
    let Some(details) = details else {
        ui.label(format!(
            "Loading {} details...",
            workload_type.to_lowercase()
        ));
        return;
    };

    ui.monospace(format!("Created: {}", details.creation_timestamp));
    ui.add_space(6.0);

    ui.label(format!("Namespace: {}", details.namespace));
    ui.label(format!("Containers: {}", details.containers.join(", ")));

    if supports_scaling {
        ui.add_space(8.0);
        if ui.button(format!("Scale {}", workload_type)).clicked() {
            // TODO: Open scale modal in Phase 7
            actions.push(ViewAction::NotifyStatus(format!(
                "Scale {} to replicas",
                workload_type.to_lowercase()
            )));
        }
    }

    ui.add_space(10.0);
    ui.collapsing("Labels", |ui| {
        for (k, v) in &details.labels {
            ui.monospace(format!("{}={}", k, v));
        }
    });

    ui.collapsing("Annotations", |ui| {
        for (k, v) in &details.annotations {
            ui.monospace(format!("{}={}", k, v));
        }
    });

    ui.add_space(10.0);
    ui.collapsing("Raw JSON", |ui| {
        egui::ScrollArea::vertical()
            .max_height(250.0)
            .show(ui, |ui| {
                ui.monospace(&details.raw_json);
            });
    });

    // Show related pods if available
    if !details.pods.is_empty() {
        ui.add_space(10.0);
        ui.collapsing(format!("Pods ({})", details.pods.len()), |ui| {
            for pod in &details.pods {
                ui.monospace(format!(
                    "- {}: {} ({}/{})",
                    pod.name, pod.phase, pod.ready, pod.total
                ));
            }
        });
    }
}

/// Shared helper for rendering workload grids
fn render_workload_grid(
    ui: &mut egui::Ui,
    title: &str,
    grid_id: &str,
    workloads: &[DeploymentSummary],
    selected: &Option<(String, String)>,
) -> Option<(String, String)> {
    ui.heading(title);
    ui.separator();

    let mut newly_selected: Option<(String, String)> = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new(grid_id)
            .striped(true)
            .min_col_width(120.0)
            .show(ui, |ui| {
                ui.label("Namespace");
                ui.label("Name");
                ui.label("Ready");
                ui.label("Updated");
                ui.label("Available");
                ui.label("Desired");
                ui.end_row();

                for w in workloads {
                    let is_selected = selected
                        .as_ref()
                        .is_some_and(|(ns, name)| ns == &w.namespace && name == &w.name);

                    let ns_clicked = ui.selectable_label(is_selected, &w.namespace).clicked();
                    let name_clicked = ui.selectable_label(is_selected, &w.name).clicked();
                    let row_clicked = ns_clicked || name_clicked;

                    ui.label(w.ready.to_string());
                    ui.label(w.updated.to_string());
                    ui.label(w.available.to_string());
                    ui.label(w.desired.to_string());
                    ui.end_row();

                    if row_clicked {
                        newly_selected = Some((w.namespace.clone(), w.name.clone()));
                    }
                }
            });
    });

    newly_selected
}

/// Generic state shared by all workload views
pub struct WorkloadViewState {
    pub summaries: Vec<DeploymentSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<DeploymentDetails>,
    pub workload_type: &'static str,
    pub supports_scaling: bool,
}

impl WorkloadViewState {
    pub fn new(workload_type: &'static str, supports_scaling: bool) -> Self {
        Self {
            summaries: Vec::new(),
            selected: None,
            details: None,
            workload_type,
            supports_scaling,
        }
    }
}

/// Deployment view
pub struct DeploymentView(pub WorkloadViewState);

impl DeploymentView {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for DeploymentView {
    fn default() -> Self {
        Self(WorkloadViewState::new("Deployment", true))
    }
}

impl ViewComponent for DeploymentView {
    fn id(&self) -> &'static str {
        "workloads_deployments"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        let newly_selected = render_workload_grid(
            ui,
            "Deployments",
            "deployments_grid",
            &self.0.summaries,
            &self.0.selected,
        );

        if let Some((ns, name)) = newly_selected {
            self.0.selected = Some((ns.clone(), name.clone()));
            self.0.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectDeployment {
                namespace: ns,
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        if let BackendEvent::ResourceList(ResourceListEvent::DeploymentsUpdated(deployments)) =
            event
        {
            self.0.summaries = deployments.clone();
        }

        if let BackendEvent::ResourceRead(ResourceReadEvent::DeploymentDetailsUpdated(details)) =
            event
        {
            if Some((details.namespace.clone(), details.name.clone())) == self.0.selected {
                self.0.details = Some(details.clone());
            }
        }
    }

    fn render_details(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
    ) -> (bool, Vec<ViewAction>) {
        if self.0.selected.is_none() {
            return (false, Vec::new());
        }

        let mut actions = Vec::new();
        let (ns, name) = self.0.selected.as_ref().unwrap().clone();

        ui.horizontal(|ui| {
            ui.heading("Deployment");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::NotifyStatus(format!(
                        "Delete deployment {}/{}",
                        ns, name
                    )));
                }
                if ui.button("Close").clicked() {
                    self.0.selected = None;
                    self.0.details = None;
                }
            });
        });

        ui.horizontal(|ui| {
            ui.label(ns);
            ui.label(format!("/{}", name));
        });

        ui.separator();
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            render_workload_details(
                ui,
                "Deployment",
                &self.0.details,
                self.0.supports_scaling,
                &mut actions,
            );
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshDeployments)]
    }

    fn on_unmount(&mut self) -> Vec<ViewAction> {
        self.0.selected = None;
        self.0.details = None;
        Vec::new()
    }
}

/// DaemonSet view
pub struct DaemonSetView(pub WorkloadViewState);

impl DaemonSetView {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for DaemonSetView {
    fn default() -> Self {
        Self(WorkloadViewState::new("DaemonSet", false))
    }
}

impl ViewComponent for DaemonSetView {
    fn id(&self) -> &'static str {
        "workloads_daemonsets"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        let newly_selected = render_workload_grid(
            ui,
            "DaemonSets",
            "daemonsets_grid",
            &self.0.summaries,
            &self.0.selected,
        );

        if let Some((ns, name)) = newly_selected {
            self.0.selected = Some((ns.clone(), name.clone()));
            self.0.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectDaemonSet {
                namespace: ns,
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        if let BackendEvent::ResourceList(ResourceListEvent::DaemonSetsUpdated(daemonsets)) = event
        {
            self.0.summaries = daemonsets.clone();
        }

        if let BackendEvent::ResourceRead(ResourceReadEvent::DaemonSetDetailsUpdated(details)) =
            event
        {
            if Some((details.namespace.clone(), details.name.clone())) == self.0.selected {
                self.0.details = Some(details.clone());
            }
        }
    }

    fn render_details(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
    ) -> (bool, Vec<ViewAction>) {
        if self.0.selected.is_none() {
            return (false, Vec::new());
        }

        let mut actions = Vec::new();
        let (ns, name) = self.0.selected.as_ref().unwrap().clone();

        ui.horizontal(|ui| {
            ui.heading("DaemonSet");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::NotifyStatus(format!(
                        "Delete daemonset {}/{}",
                        ns, name
                    )));
                }
                if ui.button("Close").clicked() {
                    self.0.selected = None;
                    self.0.details = None;
                }
            });
        });

        ui.horizontal(|ui| {
            ui.label(ns);
            ui.label(format!("/{}", name));
        });

        ui.separator();
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            render_workload_details(
                ui,
                "DaemonSet",
                &self.0.details,
                self.0.supports_scaling,
                &mut actions,
            );
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshDaemonSets)]
    }

    fn on_unmount(&mut self) -> Vec<ViewAction> {
        self.0.selected = None;
        self.0.details = None;
        Vec::new()
    }
}

/// StatefulSet view
pub struct StatefulSetView(pub WorkloadViewState);

impl StatefulSetView {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for StatefulSetView {
    fn default() -> Self {
        Self(WorkloadViewState::new("StatefulSet", true))
    }
}

impl ViewComponent for StatefulSetView {
    fn id(&self) -> &'static str {
        "workloads_statefulsets"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        let newly_selected = render_workload_grid(
            ui,
            "StatefulSets",
            "statefulsets_grid",
            &self.0.summaries,
            &self.0.selected,
        );

        if let Some((ns, name)) = newly_selected {
            self.0.selected = Some((ns.clone(), name.clone()));
            self.0.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectStatefulSet {
                namespace: ns,
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        if let BackendEvent::ResourceList(ResourceListEvent::StatefulSetsUpdated(statefulsets)) =
            event
        {
            self.0.summaries = statefulsets.clone();
        }

        if let BackendEvent::ResourceRead(ResourceReadEvent::StatefulSetDetailsUpdated(details)) =
            event
        {
            if Some((details.namespace.clone(), details.name.clone())) == self.0.selected {
                self.0.details = Some(details.clone());
            }
        }
    }

    fn render_details(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
    ) -> (bool, Vec<ViewAction>) {
        if self.0.selected.is_none() {
            return (false, Vec::new());
        }

        let mut actions = Vec::new();
        let (ns, name) = self.0.selected.as_ref().unwrap().clone();

        ui.horizontal(|ui| {
            ui.heading("StatefulSet");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::NotifyStatus(format!(
                        "Delete statefulset {}/{}",
                        ns, name
                    )));
                }
                if ui.button("Close").clicked() {
                    self.0.selected = None;
                    self.0.details = None;
                }
            });
        });

        ui.horizontal(|ui| {
            ui.label(ns);
            ui.label(format!("/{}", name));
        });

        ui.separator();
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            render_workload_details(
                ui,
                "StatefulSet",
                &self.0.details,
                self.0.supports_scaling,
                &mut actions,
            );
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshStatefulSets)]
    }

    fn on_unmount(&mut self) -> Vec<ViewAction> {
        self.0.selected = None;
        self.0.details = None;
        Vec::new()
    }
}

/// ReplicaSet view
pub struct ReplicaSetView(pub WorkloadViewState);

impl ReplicaSetView {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for ReplicaSetView {
    fn default() -> Self {
        Self(WorkloadViewState::new("ReplicaSet", true))
    }
}

impl ViewComponent for ReplicaSetView {
    fn id(&self) -> &'static str {
        "workloads_replicasets"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        let newly_selected = render_workload_grid(
            ui,
            "ReplicaSets",
            "replicasets_grid",
            &self.0.summaries,
            &self.0.selected,
        );

        if let Some((ns, name)) = newly_selected {
            self.0.selected = Some((ns.clone(), name.clone()));
            self.0.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectReplicaSet {
                namespace: ns,
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        if let BackendEvent::ResourceList(ResourceListEvent::ReplicaSetsUpdated(replicasets)) =
            event
        {
            self.0.summaries = replicasets.clone();
        }

        if let BackendEvent::ResourceRead(ResourceReadEvent::ReplicaSetDetailsUpdated(details)) =
            event
        {
            if Some((details.namespace.clone(), details.name.clone())) == self.0.selected {
                self.0.details = Some(details.clone());
            }
        }
    }

    fn render_details(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
    ) -> (bool, Vec<ViewAction>) {
        if self.0.selected.is_none() {
            return (false, Vec::new());
        }

        let mut actions = Vec::new();
        let (ns, name) = self.0.selected.as_ref().unwrap().clone();

        ui.horizontal(|ui| {
            ui.heading("ReplicaSet");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::NotifyStatus(format!(
                        "Delete replicaset {}/{}",
                        ns, name
                    )));
                }
                if ui.button("Close").clicked() {
                    self.0.selected = None;
                    self.0.details = None;
                }
            });
        });

        ui.horizontal(|ui| {
            ui.label(ns);
            ui.label(format!("/{}", name));
        });

        ui.separator();
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            render_workload_details(
                ui,
                "ReplicaSet",
                &self.0.details,
                self.0.supports_scaling,
                &mut actions,
            );
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshReplicaSets)]
    }

    fn on_unmount(&mut self) -> Vec<ViewAction> {
        self.0.selected = None;
        self.0.details = None;
        Vec::new()
    }
}

/// ReplicationController view
pub struct ReplicationControllerView(pub WorkloadViewState);

impl ReplicationControllerView {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for ReplicationControllerView {
    fn default() -> Self {
        Self(WorkloadViewState::new("ReplicationController", true))
    }
}

impl ViewComponent for ReplicationControllerView {
    fn id(&self) -> &'static str {
        "workloads_replicationcontrollers"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        let newly_selected = render_workload_grid(
            ui,
            "ReplicationControllers",
            "replicationcontrollers_grid",
            &self.0.summaries,
            &self.0.selected,
        );

        if let Some((ns, name)) = newly_selected {
            self.0.selected = Some((ns.clone(), name.clone()));
            self.0.details = None;
            actions.push(ViewAction::SendCommand(
                BackendCmd::SelectReplicationController {
                    namespace: ns,
                    name,
                },
            ));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        if let BackendEvent::ResourceList(ResourceListEvent::ReplicationControllersUpdated(rcs)) =
            event
        {
            self.0.summaries = rcs.clone();
        }

        if let BackendEvent::ResourceRead(ResourceReadEvent::ReplicationControllerDetailsUpdated(
            details,
        )) = event
        {
            if Some((details.namespace.clone(), details.name.clone())) == self.0.selected {
                self.0.details = Some(details.clone());
            }
        }
    }

    fn render_details(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
    ) -> (bool, Vec<ViewAction>) {
        if self.0.selected.is_none() {
            return (false, Vec::new());
        }

        let mut actions = Vec::new();
        let (ns, name) = self.0.selected.as_ref().unwrap().clone();

        ui.horizontal(|ui| {
            ui.heading("ReplicationController");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::NotifyStatus(format!(
                        "Delete replicationcontroller {}/{}",
                        ns, name
                    )));
                }
                if ui.button("Close").clicked() {
                    self.0.selected = None;
                    self.0.details = None;
                }
            });
        });

        ui.horizontal(|ui| {
            ui.label(ns);
            ui.label(format!("/{}", name));
        });

        ui.separator();
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            render_workload_details(
                ui,
                "ReplicationController",
                &self.0.details,
                self.0.supports_scaling,
                &mut actions,
            );
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(
            BackendCmd::RefreshReplicationControllers,
        )]
    }

    fn on_unmount(&mut self) -> Vec<ViewAction> {
        self.0.selected = None;
        self.0.details = None;
        Vec::new()
    }
}
