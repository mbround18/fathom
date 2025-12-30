// Integrations section: Prometheus, Grafana, ArgoCD, FluxCD
//!
//! This module manages external service integrations and their configurations.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsIntegration {
    pub enabled: bool,
    pub backend: MetricsBackend,
    pub namespace: String,
    pub service: String,
    pub port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetricsBackend {
    Prometheus,
    Grafana,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrafanaIntegration {
    pub enabled: bool,
    pub base_url: String,
    pub api_key: String, // Encrypted in storage
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArgoIntegration {
    pub enabled: bool,
    pub namespace: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FluxIntegration {
    pub enabled: bool,
    pub namespace: String,
}
