use super::*;
use crate::sections::{NavGroup, all_sections};

impl FathomApp {
    pub(super) fn render_left_nav(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("left_nav")
            .resizable(false)
            .default_width(170.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    // Get all sections organized by group
                    let sections = all_sections();
                    let groups_to_show = vec![
                        NavGroup::Overview,
                        NavGroup::Workloads,
                        NavGroup::Network,
                        NavGroup::Storage,
                        NavGroup::Config,
                        NavGroup::AccessControl,
                        NavGroup::CustomResources,
                        NavGroup::Integrations,
                    ];

                    for group in groups_to_show {
                    // Filter sections by group
                    let group_sections: Vec<_> =
                        sections.iter().filter(|s| s.group == group).collect();

                    if group_sections.is_empty() {
                        continue;
                    }

                    // Render group heading
                    let group_label = match group {
                        NavGroup::Overview => "Overview",
                        NavGroup::Nodes => "Nodes",
                        NavGroup::Workloads => "Workloads",
                        NavGroup::Config => "Config",
                        NavGroup::Network => "Network",
                        NavGroup::Storage => "Storage",
                        NavGroup::Namespaces => "Namespaces",
                        NavGroup::Events => "Events",
                        NavGroup::AccessControl => "Access Control",
                        NavGroup::CustomResources => "Custom Resources",
                        NavGroup::Integrations => "Integrations",
                    };

                    ui.heading(group_label);
                    ui.separator();

                    // Render sections in this group
                    for section in group_sections {
                        let is_selected = self.active_section == section.key;
                        if ui.selectable_label(is_selected, section.label).clicked() {
                            self.active_section = section.key.clone();

                            // Clear all detail views when changing sections
                            self.selected_pod = None;
                            self.pod_details = None;
                            self.pod_metrics = None;
                            self.pod_logs.clear();
                            self.send(BackendCmd::StopPodLogs);

                            self.selected_deployment = None;
                            self.deployment_details = None;
                            self.selected_daemonset = None;
                            self.daemonset_details = None;
                            self.selected_statefulset = None;
                            self.statefulset_details = None;
                            self.selected_replicaset = None;
                            self.replicaset_details = None;
                            self.selected_replicationcontroller = None;
                            self.replicationcontroller_details = None;

                            self.send(BackendCmd::SetActiveSection {
                                section: self.active_section.clone(),
                            });
                        }
                    }

                    ui.add_space(12.0);
                }

                ui.separator();

                if ui.button("Simulate Slow Call").clicked() {
                    self.send(BackendCmd::SimulateSlowCall);
                }
                });
            });
    }
}
