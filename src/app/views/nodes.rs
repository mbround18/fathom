use super::detail_sections::render_raw_json_collapsing;
/// Nodes section: displays node list with status, resource allocation, and pod capacity.
use egui::{RichText, ScrollArea, Ui};

use super::traits::{ViewAction, ViewComponent};
use crate::protocol::{BackendCmd, BackendEvent, NodeDetails, NodeSummary};

#[derive(Debug, Clone)]
pub struct NodesState {
    pub nodes: Vec<NodeSummary>,
    pub selected_node: Option<String>,
    pub node_details: Option<NodeDetails>,
}

impl Default for NodesState {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            selected_node: None,
            node_details: None,
        }
    }
}

pub struct NodesView {
    pub state: NodesState,
}

impl NodesView {
    pub fn new() -> Self {
        Self {
            state: NodesState::default(),
        }
    }
}

impl Default for NodesView {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewComponent for NodesView {
    fn id(&self) -> &'static str {
        "nodes"
    }

    fn render(&mut self, ui: &mut Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        ui.heading("Nodes");
        ui.separator();

        let mut actions = Vec::new();

        if self.state.nodes.is_empty() {
            ui.label(RichText::new("No nodes found").weak());
        } else {
            ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    egui::Grid::new("nodes_grid")
                        .striped(true)
                        .min_col_width(100.0)
                        .show(ui, |ui| {
                            // Headers
                            ui.label(RichText::new("Name").strong());
                            ui.label(RichText::new("Status").strong());
                            ui.label(RichText::new("Role").strong());
                            ui.label(RichText::new("Pods").strong());
                            ui.label(RichText::new("CPU (m)").strong());
                            ui.label(RichText::new("Memory (Mi)").strong());
                            ui.end_row();

                            // Rows
                            for node in &self.state.nodes {
                                let is_selected = self
                                    .state
                                    .selected_node
                                    .as_ref()
                                    .is_some_and(|s| s == &node.name);

                                if ui.selectable_label(is_selected, &node.name).clicked() {
                                    self.state.selected_node = Some(node.name.clone());
                                    self.state.node_details = None;
                                    actions.push(ViewAction::SendCommand(BackendCmd::SelectNode {
                                        name: node.name.clone(),
                                    }));
                                }
                                ui.label(&node.status);
                                ui.label(&node.role);
                                ui.label(node.pod_count.to_string());
                                ui.label(node.cpu_allocatable_millicores.to_string());
                                ui.label(format!(
                                    "{}",
                                    node.memory_allocatable_bytes / (1024 * 1024)
                                ));
                                ui.end_row();
                            }
                        });
                });
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        use crate::protocol::ResourceListEvent;
        if let BackendEvent::ResourceList(ResourceListEvent::NodesUpdated(summaries)) = event {
            tracing::debug!(
                "NodesView received NodesUpdated with {} nodes",
                summaries.len()
            );
            self.state.nodes = summaries.clone();
        }

        // Handle node details updates
        if let BackendEvent::ResourceRead(crate::protocol::ResourceReadEvent::NodeDetailsUpdated(
            details,
        )) = event
        {
            if Some(details.name.clone()) == self.state.selected_node {
                self.state.node_details = Some(details.clone());
            }
        }
    }

    fn render_details(&mut self, ui: &mut Ui, _ctx: &egui::Context) -> (bool, Vec<ViewAction>) {
        let Some(node_name) = &self.state.selected_node.clone() else {
            return (false, Vec::new());
        };

        let mut actions = Vec::new();

        ui.heading("Node");
        ui.separator();

        ui.horizontal(|ui| {
            ui.label(RichText::new(node_name.clone()).monospace().strong());
        });

        match &self.state.node_details {
            Some(details) => {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label("Status:");
                    let status_color = if details.status == "Ready" {
                        egui::Color32::GREEN
                    } else {
                        egui::Color32::RED
                    };
                    ui.colored_label(status_color, &details.status);
                });

                ui.add_space(8.0);
                ui.label(format!("Roles: {}", details.roles.join(", ")));
                ui.label(format!("Created: {}", details.creation_timestamp));

                ui.add_space(12.0);
                ui.group(|ui| {
                    ui.label(RichText::new("System Info").strong());
                    ui.separator();
                    ui.label(format!("OS: {} ({})", details.os, details.arch));
                    ui.label(format!("Kernel: {}", details.kernel_version));
                    ui.label(format!("Kubelet: {}", details.kubelet_version));
                    ui.label(format!("Container Runtime: {}", details.container_runtime));
                });

                ui.add_space(12.0);
                ui.group(|ui| {
                    ui.label(RichText::new("Capacity").strong());
                    ui.separator();
                    ui.label(format!("CPU: {}", details.capacity_cpu));
                    ui.label(format!("Memory: {}", details.capacity_memory));
                    ui.label(format!(
                        "Pods: {}/{}",
                        details.pod_count, details.pod_capacity
                    ));
                });

                ui.add_space(12.0);
                ui.group(|ui| {
                    ui.label(RichText::new("Allocatable").strong());
                    ui.separator();
                    ui.label(format!("CPU: {}", details.allocatable_cpu));
                    ui.label(format!("Memory: {}", details.allocatable_memory));
                });

                if !details.conditions.is_empty() {
                    ui.add_space(12.0);
                    ui.collapsing(
                        RichText::new(format!("Conditions ({})", details.conditions.len()))
                            .strong(),
                        |ui| {
                            for cond in &details.conditions {
                                let color = if cond.status == "True" {
                                    egui::Color32::GREEN
                                } else if cond.status == "Unknown" {
                                    egui::Color32::YELLOW
                                } else {
                                    egui::Color32::RED
                                };
                                ui.horizontal(|ui| {
                                    ui.colored_label(color, format!("●"));
                                    ui.monospace(format!(
                                        "{}: {}",
                                        cond.condition_type, cond.status
                                    ));
                                });
                                if !cond.reason.is_empty() {
                                    ui.monospace(format!("  Reason: {}", cond.reason));
                                }
                                if !cond.message.is_empty() {
                                    ui.monospace(format!("  Message: {}", cond.message));
                                }
                            }
                        },
                    );
                }

                if !details.labels.is_empty() {
                    ui.add_space(12.0);
                    ui.collapsing(
                        RichText::new(format!("Labels ({})", details.labels.len())).strong(),
                        |ui| {
                            for (k, v) in &details.labels {
                                ui.monospace(format!("{}={}", k, v));
                            }
                        },
                    );
                }

                if !details.annotations.is_empty() {
                    ui.add_space(12.0);
                    ui.collapsing(
                        RichText::new(format!("Annotations ({})", details.annotations.len()))
                            .strong(),
                        |ui| {
                            for (k, v) in &details.annotations {
                                ui.monospace(format!("{}={}", k, v));
                            }
                        },
                    );
                }

                ui.add_space(12.0);
                render_raw_json_collapsing(ui, &details.raw_json);

                (true, actions)
            }
            None => {
                ui.label("Loading node details...");
                (true, actions)
            }
        }
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshNodes)]
    }
}
