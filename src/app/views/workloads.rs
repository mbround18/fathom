use super::super::components::Table;
use super::traits::{ViewAction, ViewComponent};
use crate::app::{ConfirmAction, WorkloadKind};
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

/// Shared helper for rendering workload grids using the Table component
fn render_workload_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<DeploymentSummary>,
) -> Option<(String, String)> {
    ui.heading(title);
    ui.separator();

    let columns = [
        super::super::components::table::Column {
            header: "Namespace".to_string(),
            width: 100.0,
            sort_value: |w: &DeploymentSummary| w.namespace.clone(),
            render: |w: &DeploymentSummary| w.namespace.clone(),
        },
        super::super::components::table::Column {
            header: "Name".to_string(),
            width: 150.0,
            sort_value: |w: &DeploymentSummary| w.name.clone(),
            render: |w: &DeploymentSummary| w.name.clone(),
        },
        super::super::components::table::Column {
            header: "Ready".to_string(),
            width: 60.0,
            sort_value: |w: &DeploymentSummary| w.ready.to_string(),
            render: |w: &DeploymentSummary| w.ready.to_string(),
        },
        super::super::components::table::Column {
            header: "Updated".to_string(),
            width: 60.0,
            sort_value: |w: &DeploymentSummary| w.updated.to_string(),
            render: |w: &DeploymentSummary| w.updated.to_string(),
        },
        super::super::components::table::Column {
            header: "Available".to_string(),
            width: 60.0,
            sort_value: |w: &DeploymentSummary| w.available.to_string(),
            render: |w: &DeploymentSummary| w.available.to_string(),
        },
        super::super::components::table::Column {
            header: "Desired".to_string(),
            width: 60.0,
            sort_value: |w: &DeploymentSummary| w.desired.to_string(),
            render: |w: &DeploymentSummary| w.desired.to_string(),
        },
    ];

    let search_fn = |w: &DeploymentSummary, query: &str| {
        w.namespace.to_lowercase().contains(query) || w.name.to_lowercase().contains(query)
    };

    // Track selected row to emit selection action
    let mut newly_selected: Option<(String, String)> = None;
    let on_row_clicked = |original_idx: usize| {
        // We'll capture this in the render_table call
        // This is a limitation of the current design - we'll handle this differently
    };

    table.render_table(ui, &columns, search_fn, None);

    // Extract selected from table and convert back to (namespace, name)
    if let Some(selected_idx) = table.selected {
        if let Some(item) = table.items.get(selected_idx) {
            newly_selected = Some((item.namespace.clone(), item.name.clone()));
        }
    }

    newly_selected
}

/// Generic state shared by all workload views
pub struct WorkloadViewState {
    pub summaries: Vec<DeploymentSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<DeploymentDetails>,
    pub workload_type: &'static str,
    pub supports_scaling: bool,
    pub table: Table<DeploymentSummary>,
}

impl WorkloadViewState {
    pub fn new(workload_type: &'static str, supports_scaling: bool) -> Self {
        Self {
            summaries: Vec::new(),
            selected: None,
            details: None,
            workload_type,
            supports_scaling,
            table: Table::new(Vec::new()),
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

        // Update table data
        self.0.table.set_items(self.0.summaries.clone());

        let newly_selected = render_workload_grid(ui, "Deployments", &mut self.0.table);

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
                    actions.push(ViewAction::OpenModal(
                        crate::app::ConfirmAction::DeleteDeployment(ns.clone(), name.clone()),
                    ));
                }
                if ui.button("Restart").clicked() {
                    actions.push(ViewAction::OpenModal(
                        crate::app::ConfirmAction::RestartDeployment(ns.clone(), name.clone()),
                    ));
                }
                if ui.button("Scale").clicked() {
                    // Get current replica count from the summary
                    let current = self
                        .0
                        .summaries
                        .iter()
                        .find(|s| s.namespace == ns && s.name == name)
                        .map(|s| s.desired)
                        .unwrap_or(0);

                    actions.push(ViewAction::OpenScaleModal(
                        crate::app::WorkloadKind::Deployment,
                        ns.clone(),
                        name.clone(),
                        current,
                    ));
                }
                if ui.button("Close").clicked() {
                    self.0.selected = None;
                    self.0.details = None;
                }
            });
        });

        ui.horizontal(|ui| {
            ui.label(&ns);
            ui.label(format!("/{}", &name));
        });

        ui.separator();
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(details) = &self.0.details {
                // Find the summary for replica counts
                let summary = self
                    .0
                    .summaries
                    .iter()
                    .find(|s| s.namespace == ns && s.name == name);

                // 📊 Replicas Info
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("📊 Replicas");
                        ui.separator();
                        if let Some(summary) = summary {
                            ui.horizontal(|ui| {
                                ui.label(format!("Desired:    {}", summary.desired));
                                ui.label(format!("Ready:      {}", summary.ready));
                                ui.label(format!("Updated:    {}", summary.updated));
                                ui.label(format!("Available:  {}", summary.available));
                            });
                        } else {
                            ui.label("(loading replica info...)");
                        }
                    });
                });
                ui.add_space(8.0);

                // ℹ️ Metadata
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("ℹ️ Metadata");
                        ui.separator();
                        ui.monospace(format!("Created: {}", details.creation_timestamp));
                        ui.label(format!("Containers: {}", details.containers.len()));
                        for container in &details.containers {
                            ui.monospace(format!("  - {}", container));
                        }
                    });
                });
                ui.add_space(8.0);

                // 🔍 Selector
                if !details.pod_selector.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label("🔍 Pod Selector");
                            ui.separator();
                            ui.monospace(&details.pod_selector);
                        });
                    });
                    ui.add_space(8.0);
                }

                // 🏷️ Labels & Annotations
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.collapsing("🏷️ Labels", |ui| {
                            if details.labels.is_empty() {
                                ui.label("(none)");
                            } else {
                                for (k, v) in &details.labels {
                                    ui.monospace(format!("{}={}", k, v));
                                }
                            }
                        });
                        ui.collapsing("📝 Annotations", |ui| {
                            if details.annotations.is_empty() {
                                ui.label("(none)");
                            } else {
                                for (k, v) in &details.annotations {
                                    ui.monospace(format!("{}={}", k, v));
                                }
                            }
                        });
                    });
                });
                ui.add_space(8.0);

                // 📦 Related Pods
                if !details.pods.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.collapsing(format!("📦 Pods ({})", details.pods.len()), |ui| {
                                for pod in &details.pods {
                                    ui.monospace(format!(
                                        "- {}: {} ({}/{})",
                                        pod.name, pod.phase, pod.ready, pod.total
                                    ));
                                }
                            });
                        });
                    });
                    ui.add_space(8.0);
                }

                // 📄 Raw JSON
                ui.collapsing("📄 Raw JSON", |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(200.0)
                        .show(ui, |ui| {
                            ui.monospace(&details.raw_json);
                        });
                });
            } else {
                ui.label("Loading deployment details...");
            }
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

        // Update table data
        self.0.table.set_items(self.0.summaries.clone());

        let newly_selected = render_workload_grid(ui, "DaemonSets", &mut self.0.table);

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
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteDaemonSet(ns.clone(), name.clone())));
                }
                if ui.button("Restart").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::RestartDaemonSet(ns.clone(), name.clone())));
                }
                if ui.button("Close").clicked() {
                    self.0.selected = None;
                    self.0.details = None;
                }
            });
        });

        ui.horizontal(|ui| {
            ui.label(&ns);
            ui.label(format!("/{}", name));
        });

        ui.separator();
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(details) = &self.0.details {
                // Find the summary for replica counts
                let summary = self
                    .0
                    .summaries
                    .iter()
                    .find(|s| s.namespace == ns && s.name == name);

                // 📊 Status
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("📊 Status");
                        ui.separator();
                        if let Some(summary) = summary {
                            ui.monospace(format!("Desired: {}", summary.desired));
                            ui.monospace(format!("Ready: {}", summary.ready));
                            ui.monospace(format!("Updated: {}", summary.updated));
                            ui.monospace(format!("Available: {}", summary.available));
                        } else {
                            ui.label("(loading status...)");
                        }
                    });
                });
                ui.add_space(8.0);

                // ℹ️ Metadata
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("ℹ️ Metadata");
                        ui.separator();
                        ui.monospace(format!("Created: {}", details.creation_timestamp));
                        ui.label(format!("Containers: {}", details.containers.len()));
                        for container in &details.containers {
                            ui.monospace(format!("  - {}", container));
                        }
                    });
                });
                ui.add_space(8.0);

                // 🔍 Selector
                if !details.pod_selector.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label("🔍 Pod Selector");
                            ui.separator();
                            ui.monospace(&details.pod_selector);
                        });
                    });
                    ui.add_space(8.0);
                }

                // 🏷️ Labels & Annotations
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.collapsing("🏷️ Labels", |ui| {
                            if details.labels.is_empty() {
                                ui.label("(none)");
                            } else {
                                for (k, v) in &details.labels {
                                    ui.monospace(format!("{}={}", k, v));
                                }
                            }
                        });
                        ui.collapsing("📝 Annotations", |ui| {
                            if details.annotations.is_empty() {
                                ui.label("(none)");
                            } else {
                                for (k, v) in &details.annotations {
                                    ui.monospace(format!("{}={}", k, v));
                                }
                            }
                        });
                    });
                });
                ui.add_space(8.0);

                // 📦 Related Pods
                if !details.pods.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.collapsing(format!("📦 Pods ({})", details.pods.len()), |ui| {
                                for pod in &details.pods {
                                    ui.monospace(format!(
                                        "- {}: {} ({}/{})",
                                        pod.name, pod.phase, pod.ready, pod.total
                                    ));
                                }
                            });
                        });
                    });
                    ui.add_space(8.0);
                }

                // 📄 Raw JSON
                ui.collapsing("📄 Raw JSON", |ui| {
                    if let Ok(json_str) = serde_json::to_string_pretty(&details) {
                        ui.code(json_str);
                    }
                });
            } else {
                ui.label("Loading DaemonSet details...");
            }
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

        // Update table data
        self.0.table.set_items(self.0.summaries.clone());

        let newly_selected = render_workload_grid(ui, "StatefulSets", &mut self.0.table);

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
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteStatefulSet(ns.clone(), name.clone())));
                }
                if ui.button("Restart").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::RestartStatefulSet(ns.clone(), name.clone())));
                }
                if ui.button("Scale").clicked() {
                    // Get current replica count from the summary
                    let current = self
                        .0
                        .summaries
                        .iter()
                        .find(|s| s.namespace == ns && s.name == name)
                        .map(|s| s.desired)
                        .unwrap_or(0);

                    actions.push(ViewAction::OpenScaleModal(
                        WorkloadKind::StatefulSet,
                        ns.clone(),
                        name.clone(),
                        current,
                    ));
                }
                if ui.button("Close").clicked() {
                    self.0.selected = None;
                    self.0.details = None;
                }
            });
        });

        ui.horizontal(|ui| {
            ui.label(&ns);
            ui.label(format!("/{}", name));
        });

        ui.separator();
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(details) = &self.0.details {
                // Find the summary for replica counts
                let summary = self
                    .0
                    .summaries
                    .iter()
                    .find(|s| s.namespace == ns && s.name == name);

                // 📊 Status
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("📊 Status");
                        ui.separator();
                        if let Some(summary) = summary {
                            ui.monospace(format!("Desired: {}", summary.desired));
                            ui.monospace(format!("Ready: {}", summary.ready));
                            ui.monospace(format!("Updated: {}", summary.updated));
                            ui.monospace(format!("Available: {}", summary.available));
                        } else {
                            ui.label("(loading status...)");
                        }
                    });
                });
                ui.add_space(8.0);

                // ℹ️ Metadata
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("ℹ️ Metadata");
                        ui.separator();
                        ui.monospace(format!("Created: {}", details.creation_timestamp));
                        ui.label(format!("Containers: {}", details.containers.len()));
                        for container in &details.containers {
                            ui.monospace(format!("  - {}", container));
                        }
                    });
                });
                ui.add_space(8.0);

                // 🔍 Selector
                if !details.pod_selector.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label("🔍 Pod Selector");
                            ui.separator();
                            ui.monospace(&details.pod_selector);
                        });
                    });
                    ui.add_space(8.0);
                }

                // 🏷️ Labels & Annotations
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.collapsing("🏷️ Labels", |ui| {
                            if details.labels.is_empty() {
                                ui.label("(none)");
                            } else {
                                for (k, v) in &details.labels {
                                    ui.monospace(format!("{}={}", k, v));
                                }
                            }
                        });
                        ui.collapsing("📝 Annotations", |ui| {
                            if details.annotations.is_empty() {
                                ui.label("(none)");
                            } else {
                                for (k, v) in &details.annotations {
                                    ui.monospace(format!("{}={}", k, v));
                                }
                            }
                        });
                    });
                });
                ui.add_space(8.0);

                // 📦 Related Pods
                if !details.pods.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.collapsing(format!("📦 Pods ({})", details.pods.len()), |ui| {
                                for pod in &details.pods {
                                    ui.monospace(format!(
                                        "- {}: {} ({}/{})",
                                        pod.name, pod.phase, pod.ready, pod.total
                                    ));
                                }
                            });
                        });
                    });
                    ui.add_space(8.0);
                }

                // 📄 Raw JSON
                ui.collapsing("📄 Raw JSON", |ui| {
                    if let Ok(json_str) = serde_json::to_string_pretty(&details) {
                        ui.code(json_str);
                    }
                });
            } else {
                ui.label("Loading StatefulSet details...");
            }
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

        // Update table data
        self.0.table.set_items(self.0.summaries.clone());

        let newly_selected = render_workload_grid(ui, "ReplicaSets", &mut self.0.table);

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
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteReplicaSet(ns.clone(), name.clone())));
                }
                if ui.button("Restart").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::RestartReplicaSet(ns.clone(), name.clone())));
                }
                if ui.button("Scale").clicked() {
                    // Get current replica count from the summary
                    let current = self
                        .0
                        .summaries
                        .iter()
                        .find(|s| s.namespace == ns && s.name == name)
                        .map(|s| s.desired)
                        .unwrap_or(0);

                    actions.push(ViewAction::OpenScaleModal(
                        WorkloadKind::ReplicaSet,
                        ns.clone(),
                        name.clone(),
                        current,
                    ));
                }
                if ui.button("Close").clicked() {
                    self.0.selected = None;
                    self.0.details = None;
                }
            });
        });

        ui.horizontal(|ui| {
            ui.label(&ns);
            ui.label(format!("/{}", name));
        });

        ui.separator();
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(details) = &self.0.details {
                // Find the summary for replica counts
                let summary = self
                    .0
                    .summaries
                    .iter()
                    .find(|s| s.namespace == ns && s.name == name);

                // 📊 Status
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("📊 Status");
                        ui.separator();
                        if let Some(summary) = summary {
                            ui.monospace(format!("Desired: {}", summary.desired));
                            ui.monospace(format!("Ready: {}", summary.ready));
                            ui.monospace(format!("Updated: {}", summary.updated));
                            ui.monospace(format!("Available: {}", summary.available));
                        } else {
                            ui.label("(loading status...)");
                        }
                    });
                });
                ui.add_space(8.0);

                // ℹ️ Metadata
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("ℹ️ Metadata");
                        ui.separator();
                        ui.monospace(format!("Created: {}", details.creation_timestamp));
                        ui.label(format!("Containers: {}", details.containers.len()));
                        for container in &details.containers {
                            ui.monospace(format!("  - {}", container));
                        }
                    });
                });
                ui.add_space(8.0);

                // 🔍 Selector
                if !details.pod_selector.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label("🔍 Pod Selector");
                            ui.separator();
                            ui.monospace(&details.pod_selector);
                        });
                    });
                    ui.add_space(8.0);
                }

                // 🏷️ Labels & Annotations
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.collapsing("🏷️ Labels", |ui| {
                            if details.labels.is_empty() {
                                ui.label("(none)");
                            } else {
                                for (k, v) in &details.labels {
                                    ui.monospace(format!("{}={}", k, v));
                                }
                            }
                        });
                        ui.collapsing("📝 Annotations", |ui| {
                            if details.annotations.is_empty() {
                                ui.label("(none)");
                            } else {
                                for (k, v) in &details.annotations {
                                    ui.monospace(format!("{}={}", k, v));
                                }
                            }
                        });
                    });
                });
                ui.add_space(8.0);

                // 📦 Related Pods
                if !details.pods.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.collapsing(format!("📦 Pods ({})", details.pods.len()), |ui| {
                                for pod in &details.pods {
                                    ui.monospace(format!(
                                        "- {}: {} ({}/{})",
                                        pod.name, pod.phase, pod.ready, pod.total
                                    ));
                                }
                            });
                        });
                    });
                    ui.add_space(8.0);
                }

                // 📄 Raw JSON
                ui.collapsing("📄 Raw JSON", |ui| {
                    if let Ok(json_str) = serde_json::to_string_pretty(&details) {
                        ui.code(json_str);
                    }
                });
            } else {
                ui.label("Loading ReplicaSet details...");
            }
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

        // Update table data
        self.0.table.set_items(self.0.summaries.clone());

        let newly_selected = render_workload_grid(ui, "ReplicationControllers", &mut self.0.table);

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
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteReplicationController(ns.clone(), name.clone())));
                }
                if ui.button("Restart").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::RestartReplicationController(ns.clone(), name.clone())));
                }
                if ui.button("Scale").clicked() {
                    // Get current replica count from the summary
                    let current = self
                        .0
                        .summaries
                        .iter()
                        .find(|s| s.namespace == ns && s.name == name)
                        .map(|s| s.desired)
                        .unwrap_or(0);

                    actions.push(ViewAction::OpenScaleModal(
                        WorkloadKind::ReplicationController,
                        ns.clone(),
                        name.clone(),
                        current,
                    ));
                }
                if ui.button("Close").clicked() {
                    self.0.selected = None;
                    self.0.details = None;
                }
            });
        });

        ui.horizontal(|ui| {
            ui.label(&ns);
            ui.label(format!("/{}", name));
        });

        ui.separator();
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(details) = &self.0.details {
                // Find the summary for replica counts
                let summary = self
                    .0
                    .summaries
                    .iter()
                    .find(|s| s.namespace == ns && s.name == name);

                // 📊 Status
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("📊 Status");
                        ui.separator();
                        if let Some(summary) = summary {
                            ui.monospace(format!("Desired: {}", summary.desired));
                            ui.monospace(format!("Ready: {}", summary.ready));
                            ui.monospace(format!("Updated: {}", summary.updated));
                            ui.monospace(format!("Available: {}", summary.available));
                        } else {
                            ui.label("(loading status...)");
                        }
                    });
                });
                ui.add_space(8.0);

                // ℹ️ Metadata
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("ℹ️ Metadata");
                        ui.separator();
                        ui.monospace(format!("Created: {}", details.creation_timestamp));
                        ui.label(format!("Containers: {}", details.containers.len()));
                        for container in &details.containers {
                            ui.monospace(format!("  - {}", container));
                        }
                    });
                });
                ui.add_space(8.0);

                // 🔍 Selector
                if !details.pod_selector.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label("🔍 Pod Selector");
                            ui.separator();
                            ui.monospace(&details.pod_selector);
                        });
                    });
                    ui.add_space(8.0);
                }

                // 🏷️ Labels & Annotations
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.collapsing("🏷️ Labels", |ui| {
                            if details.labels.is_empty() {
                                ui.label("(none)");
                            } else {
                                for (k, v) in &details.labels {
                                    ui.monospace(format!("{}={}", k, v));
                                }
                            }
                        });
                        ui.collapsing("📝 Annotations", |ui| {
                            if details.annotations.is_empty() {
                                ui.label("(none)");
                            } else {
                                for (k, v) in &details.annotations {
                                    ui.monospace(format!("{}={}", k, v));
                                }
                            }
                        });
                    });
                });
                ui.add_space(8.0);

                // 📦 Related Pods
                if !details.pods.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.collapsing(format!("📦 Pods ({})", details.pods.len()), |ui| {
                                for pod in &details.pods {
                                    ui.monospace(format!(
                                        "- {}: {} ({}/{})",
                                        pod.name, pod.phase, pod.ready, pod.total
                                    ));
                                }
                            });
                        });
                    });
                    ui.add_space(8.0);
                }

                // 📄 Raw JSON
                ui.collapsing("📄 Raw JSON", |ui| {
                    if let Ok(json_str) = serde_json::to_string_pretty(&details) {
                        ui.code(json_str);
                    }
                });
            } else {
                ui.label("Loading ReplicationController details...");
            }
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


