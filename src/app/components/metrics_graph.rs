use crate::protocol::ContainerMetricsHistory;
use eframe::egui;
use egui_plot::{HLine, Line, LineStyle, Plot, PlotPoints};

/// Renders a single container's metrics graphs (CPU and Memory).
/// Shows usage trends with request/limit reference lines and warnings for missing specs.
pub fn render_container_metrics(ui: &mut egui::Ui, container: &ContainerMetricsHistory) {
    if container.samples.is_empty() {
        ui.label("(no samples yet)");
        return;
    }

    // Build graph data for CPU and Memory
    let cpu_line: Vec<[f64; 2]> = container
        .samples
        .iter()
        .enumerate()
        .map(|(i, sample)| [i as f64, sample.cpu_millicores])
        .collect();

    let mem_line: Vec<[f64; 2]> = container
        .samples
        .iter()
        .enumerate()
        .map(|(i, sample)| [i as f64, sample.memory_bytes / 1024.0 / 1024.0])
        .collect();

    // CPU graph
    if !cpu_line.is_empty() {
        ui.label("CPU (millicores):");
        Plot::new(&format!("cpu_{}", container.name))
            .height(100.0)
            .include_x(0.0)
            .show(ui, |plot_ui| {
                // Actual usage
                plot_ui.line(
                    Line::new("Usage", PlotPoints::new(cpu_line.clone()))
                        .color(egui::Color32::CYAN)
                        .width(2.0),
                );

                // Request line (if available) - dashed green
                if let Some(req) = container.cpu_request_millicores {
                    plot_ui.hline(
                        HLine::new("Request", req)
                            .color(egui::Color32::GREEN)
                            .width(1.5)
                            .style(LineStyle::Dashed { length: 5.0 }),
                    );
                }

                // Limit line (if available) - solid red (ceiling)
                if let Some(limit) = container.cpu_limit_millicores {
                    plot_ui.hline(
                        HLine::new("Limit", limit)
                            .color(egui::Color32::RED)
                            .width(1.5),
                    );
                }
            });

        // Warnings for missing specs
        if container.cpu_request_millicores.is_none() || container.cpu_limit_millicores.is_none() {
            let mut msg = String::new();
            if container.cpu_request_millicores.is_none() {
                msg.push_str("⚠ No request");
            }
            if container.cpu_limit_millicores.is_none() {
                if !msg.is_empty() {
                    msg.push_str(" / ");
                }
                msg.push_str("No limit");
            }
            ui.colored_label(egui::Color32::YELLOW, msg);
        }
    }

    // Memory graph
    if !mem_line.is_empty() {
        ui.label("Memory (MB):");
        Plot::new(&format!("mem_{}", container.name))
            .height(100.0)
            .include_x(0.0)
            .show(ui, |plot_ui| {
                // Actual usage
                plot_ui.line(
                    Line::new("Usage", PlotPoints::new(mem_line.clone()))
                        .color(egui::Color32::LIGHT_GREEN)
                        .width(2.0),
                );

                // Request line (if available) - dashed green
                if let Some(req) = container.memory_request_bytes {
                    let req_mb = req / 1024.0 / 1024.0;
                    plot_ui.hline(
                        HLine::new("Request", req_mb)
                            .color(egui::Color32::GREEN)
                            .width(1.5)
                            .style(LineStyle::Dashed { length: 5.0 }),
                    );
                }

                // Limit line (if available) - solid red (ceiling)
                if let Some(limit) = container.memory_limit_bytes {
                    let limit_mb = limit / 1024.0 / 1024.0;
                    plot_ui.hline(
                        HLine::new("Limit", limit_mb)
                            .color(egui::Color32::RED)
                            .width(1.5),
                    );
                }
            });

        // Warnings for missing specs
        if container.memory_request_bytes.is_none() || container.memory_limit_bytes.is_none() {
            let mut msg = String::new();
            if container.memory_request_bytes.is_none() {
                msg.push_str("⚠ No request");
            }
            if container.memory_limit_bytes.is_none() {
                if !msg.is_empty() {
                    msg.push_str(" / ");
                }
                msg.push_str("No limit");
            }
            ui.colored_label(egui::Color32::YELLOW, msg);
        }
    }
}

/// Renders metrics graphs for all containers in a pod/workload.
/// Shows a header, then renders each container's metrics.
/// Skips "unknown" containers.
pub fn render_metrics_history(
    ui: &mut egui::Ui,
    containers: &[ContainerMetricsHistory],
    title: &str,
) {
    ui.group(|ui| {
        ui.label(egui::RichText::new(title).strong());
        ui.separator();

        for container in containers {
            // Skip unknown containers
            if container.name.to_lowercase() == "unknown" {
                continue;
            }

            ui.label(egui::RichText::new(&container.name).strong());
            render_container_metrics(ui, container);
            ui.add_space(4.0);
        }
    });
    ui.add_space(8.0);
}
