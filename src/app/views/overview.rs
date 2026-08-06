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
    pub total_pod_capacity: u32,
    pub total_memory_bytes: f64,      // sum of all node capacity memory
    pub total_cpu_millicores: u64,    // sum of all node capacity CPU
    pub workload_memory_bytes: f64,   // sum of all pod/workload memory usage
    pub workload_cpu_millicores: u64, // sum of all pod/workload CPU usage
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
            total_pod_capacity: 0,
            total_memory_bytes: 0.0,
            total_cpu_millicores: 0,
            workload_memory_bytes: 0.0,
            workload_cpu_millicores: 0,
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
        ui.label(RichText::new("🔗 Context").strong());
        if let Some(ctx) = &self.state.context_name {
            ui.label(format!("  Name: {}", ctx));
        }
        if let Some(url) = &self.state.server_url {
            ui.label(format!("  Server: {}", url));
        }
        ui.add_space(16.0);

        // Cluster Resource Summary - Grid of 3 cards
        ui.label(RichText::new("📊 Cluster Resources").strong());
        ui.horizontal(|ui| {
            // Nodes Card
            if summary_card_button(ui, "Nodes", self.state.node_count.to_string()) {
                actions.push(ViewAction::Navigate(section_keys::nodes()));
            }
            ui.add_space(12.0);

            // Pods Card (with capacity)
            let pods_label = format!(
                "Pods\n{} / {}",
                self.state.pod_count, self.state.total_pod_capacity
            );
            if summary_card_button(ui, "", pods_label) {
                actions.push(ViewAction::Navigate(section_keys::workloads_pods()));
            }
            ui.add_space(12.0);

            // Namespaces Card
            if summary_card_button(ui, "Namespaces", self.state.namespace_count.to_string()) {
                actions.push(ViewAction::Navigate(section_keys::namespaces()));
            }
        });

        ui.add_space(16.0);

        // Capacity vs Usage Overview
        ui.label(RichText::new("💾 Cluster Capacity").strong());
        ui.group(|ui| {
            ui.vertical(|ui| {
                // Memory - only show if metrics available
                if self.state.total_memory_bytes > 0.0 {
                    let mem_total_gb = self.state.total_memory_bytes / 1_073_741_824.0;
                    let mem_used_gb = self.state.workload_memory_bytes / 1_073_741_824.0;
                    let mem_pct = if self.state.total_memory_bytes > 0.0 {
                        (self.state.workload_memory_bytes / self.state.total_memory_bytes * 100.0)
                            .min(100.0)
                    } else {
                        0.0
                    };

                    render_capacity_bar(
                        ui,
                        "Memory",
                        format!("{:.1} GB / {:.1} GB", mem_used_gb, mem_total_gb),
                        mem_pct,
                    );
                    ui.add_space(10.0);
                } else {
                    ui.horizontal(|ui| {
                        ui.label("💾 Memory");
                        ui.label(RichText::new("(Requires metrics-server)").weak().size(10.0));
                    });
                    ui.add_space(10.0);
                }

                // CPU - only show if metrics available
                if self.state.total_cpu_millicores > 0 {
                    let cpu_total_cores = self.state.total_cpu_millicores as f64 / 1000.0;
                    let cpu_used_cores = self.state.workload_cpu_millicores as f64 / 1000.0;
                    let cpu_pct = if self.state.total_cpu_millicores > 0 {
                        (self.state.workload_cpu_millicores as f64
                            / self.state.total_cpu_millicores as f64
                            * 100.0)
                            .min(100.0)
                    } else {
                        0.0
                    };

                    render_capacity_bar(
                        ui,
                        "CPU",
                        format!("{:.2} cores / {:.2} cores", cpu_used_cores, cpu_total_cores),
                        cpu_pct,
                    );
                    ui.add_space(10.0);
                } else {
                    ui.horizontal(|ui| {
                        ui.label("⚙️ CPU");
                        ui.label(
                            RichText::new("(Allocatable capacity available)")
                                .weak()
                                .size(10.0),
                        );
                    });
                    ui.add_space(10.0);
                }

                // Pod Density - always show (this works with node capacity)
                let pod_pct = if self.state.total_pod_capacity > 0 {
                    (self.state.pod_count as f64 / self.state.total_pod_capacity as f64 * 100.0)
                        .min(100.0)
                } else {
                    0.0
                };

                render_capacity_bar(
                    ui,
                    "Pod Density",
                    format!(
                        "{} / {} pods",
                        self.state.pod_count, self.state.total_pod_capacity
                    ),
                    pod_pct,
                );
            });
        });

        ui.add_space(16.0);
        ui.separator();

        // Quick Links
        ui.label(RichText::new("🔗 Quick Links").strong());
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
            ui.add_space(12.0);
            ui.label(
                RichText::new(format!("Last updated: {}", updated))
                    .weak()
                    .size(10.0),
            );
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        if let BackendEvent::ResourceList(list_event) = event {
            match list_event {
                ResourceListEvent::NodesUpdated(nodes) => {
                    self.state.node_count = nodes.len() as u32;

                    // Calculate total capacity across all nodes (use allocatable metrics)
                    let mut total_memory = 0i64;
                    let mut total_cpu = 0i64;
                    let mut total_pod_capacity = 0u32;

                    for node in nodes {
                        total_memory += node.memory_allocatable_bytes;
                        total_cpu += node.cpu_allocatable_millicores;
                        total_pod_capacity += node.pod_capacity;
                    }

                    self.state.total_memory_bytes = total_memory as f64;
                    self.state.total_cpu_millicores = total_cpu as u64;
                    self.state.total_pod_capacity = total_pod_capacity;
                    self.state.last_updated =
                        Some(chrono::Local::now().format("%H:%M:%S").to_string());
                }
                ResourceListEvent::PodsUpdated(pods) => {
                    self.state.pod_count = pods.len() as u32;
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

/// Render a capacity bar showing usage vs. total with percentage
fn render_capacity_bar(ui: &mut Ui, label: &str, value_text: String, percentage: f64) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).strong().size(12.0));
        ui.label(value_text);

        // Progress bar visualization
        let bar_width = ui.available_width() - 60.0;
        let bar_height = 8.0;

        let color = if percentage > 80.0 {
            egui::Color32::RED
        } else if percentage > 50.0 {
            egui::Color32::YELLOW
        } else {
            egui::Color32::GREEN
        };

        let (_, rect) = ui.allocate_space(egui::Vec2 {
            x: bar_width,
            y: bar_height,
        });
        let painter = ui.painter();

        // Background bar (light gray)
        painter.rect_filled(rect, 2.0, egui::Color32::DARK_GRAY);

        // Filled portion
        let filled_width = (bar_width as f64 * percentage / 100.0).max(0.0) as f32;
        let filled_rect = egui::Rect::from_min_size(
            rect.min,
            egui::Vec2 {
                x: filled_width,
                y: bar_height,
            },
        );
        painter.rect_filled(filled_rect, 2.0, color);

        // Percentage text
        ui.label(RichText::new(format!("{:.0}%", percentage)).small());
    });
}

/// Parse memory capacity from Kubernetes format (e.g., "123456Ki", "1Gi", "123456789")
fn parse_memory_capacity(capacity_str: &str) -> Result<f64, String> {
    let trimmed = capacity_str.trim();

    if trimmed.is_empty() {
        return Ok(0.0);
    }

    // Try to parse with suffix
    if let Some(stripped) = trimmed.strip_suffix("Ki") {
        if let Ok(val) = stripped.parse::<f64>() {
            return Ok(val * 1024.0); // Ki to bytes
        }
    } else if let Some(stripped) = trimmed.strip_suffix("Mi") {
        if let Ok(val) = stripped.parse::<f64>() {
            return Ok(val * 1_048_576.0); // Mi to bytes
        }
    } else if let Some(stripped) = trimmed.strip_suffix("Gi") {
        if let Ok(val) = stripped.parse::<f64>() {
            return Ok(val * 1_073_741_824.0); // Gi to bytes
        }
    } else if let Some(stripped) = trimmed.strip_suffix("Ti") {
        if let Ok(val) = stripped.parse::<f64>() {
            return Ok(val * 1_099_511_627_776.0); // Ti to bytes
        }
    } else if let Ok(val) = trimmed.parse::<f64>() {
        // No suffix - assume bytes
        return Ok(val);
    }

    Err(format!("Could not parse memory capacity: {}", capacity_str))
}

/// Parse CPU capacity from Kubernetes format (e.g., "2", "2000m", "0.5")
fn parse_cpu_capacity(capacity_str: &str) -> Result<u64, String> {
    let trimmed = capacity_str.trim();

    if trimmed.is_empty() {
        return Ok(0);
    }

    if let Some(stripped) = trimmed.strip_suffix('m') {
        // Millicores
        if let Ok(val) = stripped.parse::<u64>() {
            return Ok(val);
        }
    }

    if let Ok(val) = trimmed.parse::<f64>() {
        // Cores - convert to millicores
        return Ok((val * 1000.0) as u64);
    }

    Err(format!("Could not parse CPU capacity: {}", capacity_str))
}
