// Example: Integrating Prometheus metrics into Pod details panel
// This shows how to:
// 1. Detect if Prometheus is available
// 2. Query metrics when pod is selected
// 3. Display metrics in the UI

use crate::backend::metrics_provider::{MetricsProviderTrait, PrometheusClient, query_pod_metrics};
use crate::protocol::{PrometheusMetrics, BackendCmd};

/// Example backend handler for fetching pod metrics
/// This would go in src/backend/pods.rs or similar
pub async fn handle_fetch_pod_metrics(
    namespace: &str,
    pod_name: &str,
    prometheus_url: Option<&str>,
) -> anyhow::Result<Option<PrometheusMetrics>> {
    // If no Prometheus URL configured, skip metrics
    let url = match prometheus_url {
        Some(url) => url.to_string(),
        None => return Ok(None),
    };

    try {
        let client = PrometheusClient::new(url);
        
        // First, test the connection
        let _health = client.query_instant("up", None).await?;
        
        // Query pod metrics
        let metrics = query_pod_metrics(&client, namespace, pod_name).await?;
        
        Ok(Some(metrics))
    } catch (e) {
        tracing::warn!(
            namespace = namespace,
            pod = pod_name,
            error = %e,
            "Failed to fetch pod metrics from Prometheus"
        );
        Ok(None)
    }
}

/// Example UI rendering for pod metrics
/// This would go in src/app/views/pods.rs render_details()
pub fn render_pod_metrics_panel(
    ui: &mut egui::Ui,
    metrics: &Option<PrometheusMetrics>,
) {
    if let Some(m) = metrics {
        ui.group(|ui| {
            ui.vertical(|ui| {
                ui.label("📊 Prometheus Metrics");
                ui.separator();
                
                // CPU Metrics
                if let Some(cpu_pct) = m.cpu.usage_percent {
                    let color = match cpu_pct {
                        p if p > 80.0 => egui::Color32::RED,
                        p if p > 50.0 => egui::Color32::YELLOW,
                        _ => egui::Color32::GREEN,
                    };
                    ui.horizontal(|ui| {
                        ui.colored_label(color, format!("🔥 CPU: {:.1}%", cpu_pct));
                    });
                }
                
                // Memory Metrics
                if let Some(mem_bytes) = m.memory.usage_bytes {
                    let mem_mb = mem_bytes as f64 / 1_048_576.0;
                    let mem_pct = m.memory.usage_percent.unwrap_or(0.0);
                    let color = match mem_pct {
                        p if p > 80.0 => egui::Color32::RED,
                        p if p > 50.0 => egui::Color32::YELLOW,
                        _ => egui::Color32::GREEN,
                    };
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            color,
                            format!("💾 Memory: {:.1}MB ({:.1}%)", mem_mb, mem_pct),
                        );
                    });
                }
                
                // Network Metrics
                if let Some(rx_bytes) = m.network.bytes_in {
                    let rx_mb = rx_bytes as f64 / 1_048_576.0;
                    ui.horizontal(|ui| {
                        ui.label(format!("📥 RX: {:.1}MB", rx_mb));
                    });
                }
                
                if let Some(tx_bytes) = m.network.bytes_out {
                    let tx_mb = tx_bytes as f64 / 1_048_576.0;
                    ui.horizontal(|ui| {
                        ui.label(format!("📤 TX: {:.1}MB", tx_mb));
                    });
                }
            });
        });
        ui.add_space(8.0);
    } else {
        ui.group(|ui| {
            ui.label("📊 Prometheus Metrics: Unavailable");
        });
        ui.add_space(8.0);
    }
}

/// Example: Discovering Prometheus service
/// This would go in src/backend/integrations.rs
pub async fn discover_prometheus_service(
    client: &kube::Client,
    namespace: &str,
    service_name: &str,
    port: u16,
) -> anyhow::Result<String> {
    use kube::api::Api;
    use k8s_openapi::api::core::v1::Service;
    
    let services: Api<Service> = Api::namespaced(client.clone(), namespace);
    
    match services.get(service_name).await {
        Ok(svc) => {
            // Service found, we'll port-forward to it
            tracing::info!(
                namespace = namespace,
                service = service_name,
                port = port,
                "Found Prometheus service"
            );
            Ok(format!("http://localhost:{}", port))
        }
        Err(e) => {
            tracing::warn!(
                namespace = namespace,
                service = service_name,
                error = %e,
                "Prometheus service not found"
            );
            Err(e.into())
        }
    }
}

/// Example: Port-forward to Prometheus
/// This would use the existing portforward module
pub async fn port_forward_to_prometheus(
    namespace: &str,
    pod_name: &str,
    local_port: u16,
    remote_port: u16,
) -> BackendCmd {
    BackendCmd::StartPortForward {
        namespace: namespace.to_string(),
        pod: pod_name.to_string(),
        local_port,
        remote_port,
        container: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_metrics_parsing() {
        // Example metric value parsing
        let metric_string = "42.5";
        let parsed: f64 = metric_string.parse().unwrap();
        assert!((parsed - 42.5).abs() < 0.01);
    }
    
    #[test]
    fn test_cpu_threshold_colors() {
        let high_cpu = 95.0;
        let medium_cpu = 60.0;
        let low_cpu = 20.0;
        
        assert!(high_cpu > 80.0);
        assert!(medium_cpu > 50.0 && medium_cpu <= 80.0);
        assert!(low_cpu <= 50.0);
    }
}
