use crate::protocol::ContainerMetricsHistory;
use eframe::egui;
use egui_plot::{HLine, Line, LineStyle, Plot, PlotPoints};

/// Find the closest sample index to a given x position.
fn find_closest_sample(x: f64, num_samples: usize) -> Option<usize> {
    if num_samples == 0 {
        return None;
    }
    let idx = x.round() as usize;
    if idx < num_samples { Some(idx) } else { None }
}

/// Parse ISO8601 timestamp and format as local time HH:MM:SS.
/// Returns the original string if parsing fails.
fn format_timestamp_local(iso_timestamp: &str) -> String {
    // Try to parse ISO8601 format (e.g., "2025-01-02T15:30:45Z")
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(iso_timestamp) {
        let local = dt.with_timezone(&chrono::Local);
        return local.format("%H:%M:%S").to_string();
    }
    // Fallback: try parsing without timezone
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(iso_timestamp, "%Y-%m-%dT%H:%M:%S") {
        return dt.format("%H:%M:%S").to_string();
    }
    iso_timestamp.to_string()
}

/// Renders a single container's metrics graphs (CPU and Memory).
/// Shows usage trends with request/limit reference lines and warnings for missing specs.
/// Hover over the graph to see current usage, requests, and limits at that moment in time.
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

    // CPU graph with hover details
    if !cpu_line.is_empty() {
        ui.label("CPU (millicores):");
        let mut cpu_hover_info: Option<String> = None;
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

                // Capture hover information for display below plot
                if let Some(cursor) = plot_ui.pointer_coordinate() {
                    if let Some(sample_idx) = find_closest_sample(cursor.x, container.samples.len())
                    {
                        let sample = &container.samples[sample_idx];
                        let time_label = format_timestamp_local(&sample.timestamp);
                        let mut info =
                            format!("[{}] Usage: {:.1} m", time_label, sample.cpu_millicores);

                        if let Some(req) = container.cpu_request_millicores {
                            info.push_str(&format!(" | Request: {:.1} m", req));
                        }
                        if let Some(limit) = container.cpu_limit_millicores {
                            info.push_str(&format!(" | Limit: {:.1} m", limit));
                        }

                        cpu_hover_info = Some(info);
                    }
                }
            });

        // Display hover info or warnings
        if let Some(info) = cpu_hover_info {
            ui.label(
                egui::RichText::new(info)
                    .small()
                    .color(egui::Color32::LIGHT_GRAY),
            );
        } else if container.cpu_request_millicores.is_none()
            || container.cpu_limit_millicores.is_none()
        {
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

    // Memory graph with hover details
    if !mem_line.is_empty() {
        ui.label("Memory (MB):");
        let mut mem_hover_info: Option<String> = None;
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

                // Capture hover information for display below plot
                if let Some(cursor) = plot_ui.pointer_coordinate() {
                    if let Some(sample_idx) = find_closest_sample(cursor.x, container.samples.len())
                    {
                        let sample = &container.samples[sample_idx];
                        let time_label = format_timestamp_local(&sample.timestamp);
                        let usage_mb = sample.memory_bytes / 1024.0 / 1024.0;
                        let mut info = format!("[{}] Usage: {:.1} MB", time_label, usage_mb);

                        if let Some(req) = container.memory_request_bytes {
                            let req_mb = req / 1024.0 / 1024.0;
                            info.push_str(&format!(" | Request: {:.1} MB", req_mb));
                        }
                        if let Some(limit) = container.memory_limit_bytes {
                            let limit_mb = limit / 1024.0 / 1024.0;
                            info.push_str(&format!(" | Limit: {:.1} MB", limit_mb));
                        }

                        mem_hover_info = Some(info);
                    }
                }
            });

        // Display hover info or warnings
        if let Some(info) = mem_hover_info {
            ui.label(
                egui::RichText::new(info)
                    .small()
                    .color(egui::Color32::LIGHT_GRAY),
            );
        } else if container.memory_request_bytes.is_none() || container.memory_limit_bytes.is_none()
        {
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
