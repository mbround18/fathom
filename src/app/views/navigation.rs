use super::traits::{ViewAction, ViewComponent};
use crate::protocol::BackendEvent;
use crate::sections::{NavGroup, all_sections};

/// Navigation view - displays the left sidebar section tree
///
/// Stateless component that renders available sections organized by group.
/// Emits Navigate actions when user clicks a section.
pub struct NavigationView;

impl ViewComponent for NavigationView {
    fn id(&self) -> &'static str {
        "navigation"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

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
                let group_sections: Vec<_> = sections.iter().filter(|s| s.group == group).collect();

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
                    if ui.link(section.label).clicked() {
                        actions.push(ViewAction::Navigate(section.key.clone()));
                    }
                }

                ui.add_space(16.0);
            }
        });

        actions
    }

    fn on_event(&mut self, _event: &BackendEvent) {
        // Navigation view doesn't respond to backend events
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        // No state to initialize
        Vec::new()
    }

    fn on_unmount(&mut self) -> Vec<ViewAction> {
        // No state to clean up
        Vec::new()
    }
}
