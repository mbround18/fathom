/// Overview section: displays cluster summary, context info, and key metrics.
use egui::{RichText, Ui};

use super::traits::{ViewAction, ViewComponent};
use crate::protocol::section_keys;
use crate::protocol::{BackendCmd, BackendEvent, ResourceListEvent};

#[derive(Debug, Clone)]
pub struct OverviewState {
    pub context_name: Option<String>,
    pub server_url: Option<String>,
    pub node_count: u32,
    pub pod_count: u32,
    pub namespace_count: u32,
    pub last_updated: Option<String>,
}

impl Default for OverviewState {
    fn default() -> Self {
        Self {
            context_name: None,
            server_url: None,
            node_count: 0,
            pod_count: 0,
            namespace_count: 0,
            last_updated: None,
        }
    }
}

pub struct OverviewView {
    pub state: OverviewState,
}

impl OverviewView {
    pub fn new() -> Self {
        Self {
            state: OverviewState::default(),
        }
    }
}

impl Default for OverviewView {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewComponent for OverviewView {
    fn id(&self) -> &'static str {
        "overview"
    }

    fn render(&mut self, ui: &mut Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        ui.heading("Cluster Overview");
        ui.separator();

        // Context Info
        ui.label(RichText::new("Context").strong());
        if let Some(ctx) = &self.state.context_name {
            ui.label(format!("  Name: {}", ctx));
        }
        if let Some(url) = &self.state.server_url {
            ui.label(format!("  Server: {}", url));
        }
        ui.add_space(12.0);

        // Summary Grid with Navigation
        ui.label(RichText::new("Summary").strong());
        ui.horizontal(|ui| {
            if summary_card_button(ui, "Nodes", self.state.node_count.to_string()) {
                actions.push(ViewAction::Navigate(section_keys::nodes()));
            }
            ui.add_space(16.0);
            if summary_card_button(ui, "Pods", self.state.pod_count.to_string()) {
                actions.push(ViewAction::Navigate(section_keys::workloads_pods()));
            }
            ui.add_space(16.0);
            if summary_card_button(ui, "Namespaces", self.state.namespace_count.to_string()) {
                actions.push(ViewAction::Navigate(section_keys::namespaces()));
            }
        });

        ui.add_space(20.0);
        ui.separator();

        // Quick Links
        ui.label(RichText::new("Quick Links").strong());
        ui.horizontal(|ui| {
            if ui.link("📋 Events").clicked() {
                actions.push(ViewAction::Navigate(section_keys::events()));
            }
            if ui.link("📦 Nodes").clicked() {
                actions.push(ViewAction::Navigate(section_keys::nodes()));
            }
            if ui.link("📑 Namespaces").clicked() {
                actions.push(ViewAction::Navigate(section_keys::namespaces()));
            }
        });

        if let Some(updated) = &self.state.last_updated {
            ui.add_space(8.0);
            ui.label(RichText::new(format!("Last updated: {}", updated)).weak());
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        if let BackendEvent::ResourceList(list_event) = event {
            match list_event {
                ResourceListEvent::NodesUpdated(nodes) => {
                    self.state.node_count = nodes.len() as u32;
                    self.state.last_updated =
                        Some(chrono::Local::now().format("%H:%M:%S").to_string());
                }
                ResourceListEvent::NamespaceSummariesUpdated(namespaces) => {
                    self.state.namespace_count = namespaces.len() as u32;
                    self.state.last_updated =
                        Some(chrono::Local::now().format("%H:%M:%S").to_string());
                }
                _ => {}
            }
        }
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![
            ViewAction::SendCommand(BackendCmd::RefreshNodes),
            ViewAction::SendCommand(BackendCmd::RefreshPods),
            ViewAction::SendCommand(BackendCmd::RefreshNamespaces),
        ]
    }
}

fn summary_card(ui: &mut Ui, title: &str, value: String) {
    ui.vertical(|ui| {
        ui.label(RichText::new(title).small());
        ui.label(RichText::new(value).strong().size(18.0));
    });
}

fn summary_card_button(ui: &mut Ui, title: &str, value: String) -> bool {
    let clicked = ui
        .button(
            RichText::new(format!("{}\n{}", title, value))
                .size(14.0)
                .strong(),
        )
        .clicked();
    clicked
}
