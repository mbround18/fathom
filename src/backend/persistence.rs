//! File persistence for ports and integrations configuration
//!
//! Manages loading/saving configuration to ~/.fathom/ directory.

use crate::sections::network::PortForward;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json;
use std::fs;
use std::path::{Path, PathBuf};

/// Auto-start configuration for various Fathom services.
/// Allows extensible gating of features that should start automatically on app initialization.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutoStartConfig {
    /// Whether to auto-start the MCP (Model Context Protocol) server
    pub mcp_http: bool,
}

impl AutoStartConfig {
    /// Load auto-start configuration from disk
    pub fn load() -> Result<Self> {
        let config = load_config()?;
        let auto_start_obj = config
            .get("auto_start")
            .and_then(|v| v.as_object())
            .cloned();

        if let Some(obj) = auto_start_obj {
            let json_value = serde_json::Value::Object(obj);
            serde_json::from_value(json_value)
                .map_err(|e| anyhow!("Failed to deserialize auto-start config: {}", e))
        } else {
            Ok(Self::default())
        }
    }

    /// Save auto-start configuration to disk
    pub fn save(&self) -> Result<()> {
        let mut config = load_config()?;
        config["auto_start"] = serde_json::to_value(self)
            .map_err(|e| anyhow!("Failed to serialize auto-start config: {}", e))?;
        save_config(&config)?;
        tracing::info!("Saved auto-start configuration");
        Ok(())
    }
}

/// Get the Fathom config directory (~/.fathom/)
pub fn fathom_config_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("Could not determine home directory"))?;
    Ok(home.join(".fathom"))
}

/// Ensure the Fathom config directory exists
pub fn ensure_config_dir() -> Result<()> {
    let dir = fathom_config_dir()?;
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
        tracing::info!("Created Fathom config directory: {}", dir.display());
    }
    Ok(())
}

/// Get the path to ports.json
pub fn ports_config_path() -> Result<PathBuf> {
    Ok(fathom_config_dir()?.join("ports.json"))
}

/// Get the path to metrics.json
pub fn metrics_history_path() -> Result<PathBuf> {
    Ok(fathom_config_dir()?.join("metrics.json"))
}

/// Save metrics history to disk
pub fn save_metrics_history(metrics: &serde_json::Value) -> Result<()> {
    let path = metrics_history_path()?;
    let json_str = serde_json::to_string_pretty(metrics)
        .map_err(|e| anyhow!("Failed to serialize metrics: {}", e))?;
    fs::write(&path, json_str)?;
    Ok(())
}

/// Load metrics history from disk
pub fn load_metrics_history() -> Result<serde_json::Value> {
    let path = metrics_history_path()?;
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let json_str = fs::read_to_string(&path)?;
    serde_json::from_str(&json_str).map_err(|e| anyhow!("Failed to deserialize metrics: {}", e))
}

/// Clear metrics history file on startup
pub fn clear_metrics_history() -> Result<()> {
    let path = metrics_history_path()?;
    if path.exists() {
        fs::remove_file(&path)?;
        tracing::info!("Cleared metrics history file");
    }
    Ok(())
}

/// Get the path to integrations.json
pub fn integrations_config_path() -> Result<PathBuf> {
    Ok(fathom_config_dir()?.join("integrations.json"))
}

/// Get the path to credentials.json (encrypted)
pub fn credentials_config_path() -> Result<PathBuf> {
    Ok(fathom_config_dir()?.join("credentials.json"))
}

/// Get the path to config.json (main application config)
pub fn main_config_path() -> Result<PathBuf> {
    Ok(fathom_config_dir()?.join("config.json"))
}

/// Load JSON from file, return empty object if file doesn't exist
pub fn load_json_file(path: &Path) -> Result<serde_json::Value> {
    ensure_config_dir()?;
    if path.exists() {
        let content = fs::read_to_string(path)?;
        let value = serde_json::from_str(&content)
            .map_err(|e| anyhow!("Failed to parse {}: {}", path.display(), e))?;
        Ok(value)
    } else {
        Ok(serde_json::json!({}))
    }
}

/// Save JSON to file
pub fn save_json_file(path: &Path, value: &serde_json::Value) -> Result<()> {
    ensure_config_dir()?;
    let content = serde_json::to_string_pretty(value)?;
    fs::write(path, content)?;
    tracing::debug!("Saved configuration to {}", path.display());
    Ok(())
}

/// Load ports configuration
pub fn load_ports() -> Result<serde_json::Value> {
    let path = ports_config_path()?;
    load_json_file(&path)
}

/// Save ports configuration
pub fn save_ports(value: &serde_json::Value) -> Result<()> {
    let path = ports_config_path()?;
    save_json_file(&path, value)
}

/// Load integrations configuration
pub fn load_integrations() -> Result<serde_json::Value> {
    let path = integrations_config_path()?;
    load_json_file(&path)
}

/// Save integrations configuration
pub fn save_integrations(value: &serde_json::Value) -> Result<()> {
    let path = integrations_config_path()?;
    save_json_file(&path, value)
}

/// Load credentials configuration (encrypted)
pub fn load_credentials() -> Result<serde_json::Value> {
    let path = credentials_config_path()?;
    load_json_file(&path)
}

/// Save credentials configuration (encrypted)
pub fn save_credentials(value: &serde_json::Value) -> Result<()> {
    let path = credentials_config_path()?;
    save_json_file(&path, value)
}

/// Load main configuration
pub fn load_config() -> Result<serde_json::Value> {
    let path = main_config_path()?;
    load_json_file(&path)
}

/// Save main configuration
pub fn save_config(value: &serde_json::Value) -> Result<()> {
    let path = main_config_path()?;
    save_json_file(&path, value)
}

/// Load MCP auto-start preference (legacy function for backward compatibility)
pub fn load_mcp_auto_start() -> Result<bool> {
    let auto_start = AutoStartConfig::load()?;
    Ok(auto_start.mcp_http)
}

/// Save MCP auto-start preference (legacy function for backward compatibility)
pub fn save_mcp_auto_start(enabled: bool) -> Result<()> {
    let mut config = AutoStartConfig::load()?;
    config.mcp_http = enabled;
    config.save()?;
    tracing::info!("Saved MCP auto-start preference: {}", enabled);
    Ok(())
}

/// Load saved port forwards from ports.json
pub fn load_all_ports() -> Result<Vec<PortForward>> {
    let path = ports_config_path()?;
    ensure_config_dir()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path)?;
    let ports: Vec<PortForward> =
        serde_json::from_str(&content).map_err(|e| anyhow!("Failed to parse ports.json: {}", e))?;
    Ok(ports)
}

/// Save port forwards to ports.json
pub fn save_all_ports(ports: &[PortForward]) -> Result<()> {
    ensure_config_dir()?;
    let path = ports_config_path()?;
    let content = serde_json::to_string_pretty(ports)?;
    fs::write(&path, content)?;
    tracing::info!("Saved {} port forwards to {}", ports.len(), path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fathom_config_dir() {
        let result = fathom_config_dir();
        assert!(result.is_ok());
    }

    #[test]
    fn test_ensure_config_dir() {
        let result = ensure_config_dir();
        assert!(result.is_ok());
    }
}
