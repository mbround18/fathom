//! External integrations: Prometheus, Grafana, ArgoCD, FluxCD
//!
//! This module manages configuration and connections to external services.

use super::persistence;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrometheusConfig {
    pub enabled: bool,
    pub namespace: String,
    pub service: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrafanaConfig {
    pub enabled: bool,
    pub base_url: String,
    pub api_key: String, // Stored encrypted
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArgoConfig {
    pub enabled: bool,
    pub namespace: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FluxConfig {
    pub enabled: bool,
    pub namespace: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationConfig {
    pub context: String,
    pub prometheus: Option<PrometheusConfig>,
    pub grafana: Option<GrafanaConfig>,
    pub argo: Option<ArgoConfig>,
    pub flux: Option<FluxConfig>,
}

impl IntegrationConfig {
    /// Load integrations config from ~/.fathom/integrations.json
    pub fn load(context: &str) -> Result<Self> {
        match persistence::load_integrations() {
            Ok(json) => {
                if let Ok(config) = serde_json::from_value::<IntegrationConfig>(json) {
                    Ok(config)
                } else {
                    // If parsing fails, return new default config
                    Ok(IntegrationConfig {
                        context: context.to_string(),
                        prometheus: None,
                        grafana: None,
                        argo: None,
                        flux: None,
                    })
                }
            }
            Err(_) => {
                // File doesn't exist, return new default config
                Ok(IntegrationConfig {
                    context: context.to_string(),
                    prometheus: None,
                    grafana: None,
                    argo: None,
                    flux: None,
                })
            }
        }
    }

    /// Save integrations config to ~/.fathom/integrations.json
    pub fn save(&self) -> Result<()> {
        let json = serde_json::to_value(self)?;
        persistence::save_integrations(&json)?;
        Ok(())
    }
}

/// Query Prometheus for time-series metrics
/// Returns JSON metric data
pub async fn query_prometheus(
    _base_url: &str,
    _query: &str,
    _step: &str,
) -> Result<serde_json::Value> {
    // TODO: Implement prometheus HTTP client
    // Example query: http://localhost:9090/api/v1/query_range?query=...&start=...&end=...&step=...
    Ok(serde_json::json!({}))
}

/// Get Grafana dashboard URL with variable substitution
pub fn grafana_dashboard_url(
    base_url: &str,
    dashboard_uid: &str,
    namespace: &str,
    pod: &str,
) -> String {
    // TODO: Generate URL with variable interpolation
    format!(
        "{}/d/{}?var-namespace={}&var-pod={}",
        base_url, dashboard_uid, namespace, pod
    )
}
