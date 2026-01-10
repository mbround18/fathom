use super::traits::{ViewAction, ViewComponent};
use crate::backend::persistence;
use crate::protocol::BackendEvent;
use crate::sections::{NavGroup, all_sections};
use std::collections::HashSet;

/// Navigation view - displays the left sidebar section tree
///
/// Collapsible groups with expanded/collapsed state persisted to config.
/// Emits Navigate actions when user clicks a section.
pub struct NavigationView {
    expanded_groups: HashSet<String>,
}

impl NavigationView {
    pub fn new() -> Self {
        let expanded_groups = Self::load_expanded_groups();
        Self { expanded_groups }
    }

    fn load_expanded_groups() -> HashSet<String> {
        match persistence::load_config() {
            Ok(config) => {
                if let Some(nav) = config.get("navigation") {
                    if let Some(expanded) = nav.get("expanded_groups").and_then(|v| v.as_array()) {
                        expanded
                            .iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    } else {
                        // Default: expand Overview and Workloads
                        vec!["Overview".to_string(), "Workloads".to_string()]
                            .into_iter()
                            .collect()
                    }
                } else {
                    vec!["Overview".to_string(), "Workloads".to_string()]
                        .into_iter()
                        .collect()
                }
            }
            Err(_) => {
                // Default: expand Overview and Workloads
                vec!["Overview".to_string(), "Workloads".to_string()]
                    .into_iter()
                    .collect()
            }
        }
    }

    fn save_expanded_groups(&self) {
        let mut config = persistence::load_config().unwrap_or(serde_json::json!({}));
        let expanded_vec: Vec<String> = self.expanded_groups.iter().cloned().collect();

        if config.get("navigation").is_none() {
            config["navigation"] = serde_json::json!({});
        }
        config["navigation"]["expanded_groups"] =
            serde_json::to_value(&expanded_vec).unwrap_or(serde_json::Value::Array(vec![]));

        let _ = persistence::save_config(&config);
    }

    fn toggle_group(&mut self, group_label: &str) {
        if self.expanded_groups.contains(group_label) {
            self.expanded_groups.remove(group_label);
        } else {
            self.expanded_groups.insert(group_label.to_string());
        }
        self.save_expanded_groups();
    }
}

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
                    NavGroup::Workloads => "Workloads",
                    NavGroup::Config => "Config",
                    NavGroup::Network => "Network",
                    NavGroup::Storage => "Storage",
                    NavGroup::AccessControl => "Access Control",
                    NavGroup::CustomResources => "Custom Resources",
                    NavGroup::Integrations => "Integrations",
                    _ => continue,
                };

                let is_expanded = self.expanded_groups.contains(group_label);
                let arrow = if is_expanded { "▼" } else { "▶" };

                // Clickable group header with expand/collapse
                let header_text = format!("{} {}", arrow, group_label);
                if ui.button(&header_text).clicked() {
                    self.toggle_group(group_label);
                }

                ui.separator();

                // Render sections in this group if expanded
                if is_expanded {
                    for section in &group_sections {
                        // Add indentation for subsections under Overview (Nodes, Events, Namespaces)
                        let is_subsection =
                            group == NavGroup::Overview && section.key.0 != "overview";
                        let item_text = if is_subsection {
                            format!("    {}", section.label)
                        } else {
                            format!("  {}", section.label)
                        };

                        if ui.link(item_text).clicked() {
                            actions.push(ViewAction::Navigate(section.key.clone()));
                        }
                    }

                    ui.add_space(16.0);
                }
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
