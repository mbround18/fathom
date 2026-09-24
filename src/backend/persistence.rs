//! File persistence for ports and integrations configuration
//!
//! Manages loading/saving configuration to ~/.fathom/ directory.

use crate::protocol::{
    AdoptedConnectionProfile, ClusterSettings, NodeShellSettings, OidcConfig, UiConfig,
};
use crate::sections::network::PortForward;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Auto-start configuration for various Fathom services.
/// Allows extensible gating of features that should start automatically on app initialization.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutoStartConfig {
    /// Whether the stdio MCP (Model Context Protocol) server is enabled. Off by default
    /// (FR-002). Renamed from `mcp_http` when the HTTP MCP prototype was replaced with the
    /// stdio server; `alias` keeps older `config.json` files loading their prior value.
    #[serde(alias = "mcp_http")]
    pub mcp_stdio: bool,
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

/// Load the persisted MCP server enabled/disabled preference.
pub fn load_mcp_auto_start() -> Result<bool> {
    let auto_start = AutoStartConfig::load()?;
    Ok(auto_start.mcp_stdio)
}

/// Persist the MCP server enabled/disabled preference.
pub fn save_mcp_auto_start(enabled: bool) -> Result<()> {
    let mut config = AutoStartConfig::load()?;
    config.mcp_stdio = enabled;
    config.save()?;
    tracing::info!("Saved MCP server enabled preference: {}", enabled);
    Ok(())
}

/// Navigation groups expanded when `navigation.expanded_groups` is absent.
pub const DEFAULT_EXPANDED_GROUPS: [&str; 2] = ["Overview", "Workloads"];

/// Everything `SystemEvent::UiConfigLoaded` reports, read from `config.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedUiConfig {
    /// `config.json → ui`, or `None` when absent or invalid.
    pub ui: Option<UiConfig>,
    /// `config.json → navigation.expanded_groups`.
    pub expanded_groups: Vec<String>,
    /// `config.json → auto_start.mcp_stdio`.
    pub mcp_stdio_enabled: bool,
}

impl Default for LoadedUiConfig {
    fn default() -> Self {
        Self {
            ui: None,
            expanded_groups: DEFAULT_EXPANDED_GROUPS.map(str::to_owned).to_vec(),
            mcp_stdio_enabled: false,
        }
    }
}

/// Load the UI preferences from `config.json`.
///
/// An unreadable `config.json` yields defaults and is left untouched on disk.
/// An invalid `ui` blob (bad shape or `schema != 1`) is moved to `ui_invalid`
/// (replacing any older one) and `ui: None` is returned; every other key is preserved.
pub fn load_ui_config() -> LoadedUiConfig {
    let mut config = match load_config() {
        Ok(config) => config,
        Err(e) => {
            tracing::warn!("Failed to read config.json; using UI defaults: {}", e);
            return LoadedUiConfig::default();
        }
    };

    let mut loaded = LoadedUiConfig {
        mcp_stdio_enabled: config
            .get("auto_start")
            .and_then(|a| a.get("mcp_stdio").or_else(|| a.get("mcp_http")))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        ..LoadedUiConfig::default()
    };
    if let Some(groups) = expanded_groups_from(&config) {
        loaded.expanded_groups = groups;
    }
    loaded.ui = take_valid_ui(&mut config);
    loaded
}

/// `config.json → navigation.expanded_groups`, if present.
fn expanded_groups_from(config: &serde_json::Value) -> Option<Vec<String>> {
    let groups = config
        .get("navigation")?
        .get("expanded_groups")?
        .as_array()?;
    Some(
        groups
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
    )
}

/// Parse `config → ui`. If it is invalid, move it to `ui_invalid` and persist that.
fn take_valid_ui(config: &mut serde_json::Value) -> Option<UiConfig> {
    let raw = config.get("ui")?.clone();
    match UiConfig::from_value(raw.clone()) {
        Ok(ui) => Some(ui),
        Err(e) => {
            tracing::warn!("Invalid ui config; moving it to ui_invalid: {}", e);
            if let Some(obj) = config.as_object_mut() {
                obj.remove("ui");
                obj.insert("ui_invalid".to_owned(), raw);
                if let Err(e) = save_config(config) {
                    tracing::warn!("Failed to move invalid ui config aside: {}", e);
                }
            }
            None
        }
    }
}

/// Persist `ui` and `navigation.expanded_groups` to `config.json`, leaving every other
/// key untouched. Refuses to write (and returns an error) if `ui` is invalid or the
/// existing `config.json` can't be parsed, so the user's file is never clobbered.
pub fn save_ui_config(ui: &UiConfig, expanded_groups: &[String]) -> Result<()> {
    ui.validate()
        .map_err(|e| anyhow!("Refusing to save invalid ui config: {}", e))?;

    let mut config = load_config()?;
    let obj = config
        .as_object_mut()
        .ok_or_else(|| anyhow!("config.json is not a JSON object"))?;

    obj.insert(
        "ui".to_owned(),
        serde_json::to_value(ui).map_err(|e| anyhow!("Failed to serialize ui config: {}", e))?,
    );

    let nav = obj
        .entry("navigation")
        .or_insert_with(|| serde_json::json!({}));
    if nav.is_null() {
        *nav = serde_json::json!({});
    }
    let nav = nav
        .as_object_mut()
        .ok_or_else(|| anyhow!("config.json navigation is not a JSON object"))?;
    nav.insert(
        "expanded_groups".to_owned(),
        serde_json::to_value(expanded_groups)?,
    );

    save_config(&config)?;
    tracing::debug!("Saved ui config");
    Ok(())
}

/// Load `config.json -> oidc` (033). Absent, unreadable or invalid yields defaults; an invalid
/// blob is moved to `oidc_invalid` (replacing any older one) so the original is preserved and
/// every other key is untouched. A `config.json` that cannot be parsed at all is never rewritten.
pub fn load_oidc_config() -> OidcConfig {
    let mut config = match load_config() {
        Ok(config) => config,
        Err(e) => {
            tracing::warn!("Failed to read config.json; using OIDC defaults: {}", e);
            return OidcConfig::default();
        }
    };
    let Some(raw) = config.get("oidc").cloned() else {
        return OidcConfig::default();
    };
    match OidcConfig::from_value(raw.clone()) {
        Ok(oidc) => oidc,
        Err(e) => {
            tracing::warn!("Invalid oidc config; moving it to oidc_invalid: {}", e);
            if let Some(obj) = config.as_object_mut() {
                obj.remove("oidc");
                obj.insert("oidc_invalid".to_owned(), raw);
                if let Err(e) = save_config(&config) {
                    tracing::warn!("Failed to move invalid oidc config aside: {}", e);
                }
            }
            OidcConfig::default()
        }
    }
}

/// Persist `config.json -> oidc`, leaving every other key untouched. Refuses to write an
/// invalid config or over an unparseable `config.json`.
pub fn save_oidc_config(oidc: &OidcConfig) -> Result<()> {
    oidc.validate()
        .map_err(|e| anyhow!("Refusing to save invalid oidc config: {}", e))?;
    let mut config = load_config()?;
    let obj = config
        .as_object_mut()
        .ok_or_else(|| anyhow!("config.json is not a JSON object"))?;
    obj.insert(
        "oidc".to_owned(),
        serde_json::to_value(oidc)
            .map_err(|e| anyhow!("Failed to serialize oidc config: {}", e))?,
    );
    save_config(&config)?;
    tracing::debug!("Saved oidc config");
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

// ---------------------------------------------------------------------------------------
// 014-teams-organizations: local Cluster Settings (§10) and team (shared-store) state.
// ---------------------------------------------------------------------------------------

/// Get the path to `cluster-settings.json` (local, per-operator, keyed by context name).
pub fn cluster_settings_path() -> Result<PathBuf> {
    Ok(fathom_config_dir()?.join("cluster-settings.json"))
}

/// Get the path to `team.json` (shared-store pointer, adopted profiles, org-sync state).
pub fn team_config_path() -> Result<PathBuf> {
    Ok(fathom_config_dir()?.join("team.json"))
}

/// Load the local Cluster Settings for `context`. An unreadable or invalid file yields
/// this context's defaults and leaves the file untouched (FR-009, Constitution Principle VI).
pub fn load_cluster_settings(context: &str) -> ClusterSettings {
    let path = match cluster_settings_path() {
        Ok(p) => p,
        Err(_) => return ClusterSettings::default(),
    };
    let Ok(content) = fs::read_to_string(&path) else {
        return ClusterSettings::default();
    };
    let Ok(all) = serde_json::from_str::<BTreeMap<String, serde_json::Value>>(&content) else {
        tracing::warn!("cluster-settings.json is not valid JSON; using defaults for {context}");
        return ClusterSettings::default();
    };
    match all.get(context) {
        Some(raw) => match serde_json::from_value::<ClusterSettings>(raw.clone()) {
            Ok(settings) if settings.validate().is_ok() => settings,
            Ok(_) | Err(_) => {
                tracing::warn!(
                    "cluster-settings.json entry for {context} is invalid; using defaults"
                );
                ClusterSettings::default()
            }
        },
        None => ClusterSettings::default(),
    }
}

/// Save the local Cluster Settings for `context`, leaving every other context's entry
/// untouched. Refuses to write invalid settings.
pub fn save_cluster_settings(context: &str, settings: &ClusterSettings) -> Result<()> {
    settings
        .validate()
        .map_err(|e| anyhow!("Refusing to save invalid cluster settings: {}", e))?;
    let path = cluster_settings_path()?;
    ensure_config_dir()?;
    let mut all: BTreeMap<String, serde_json::Value> = if path.exists() {
        let content = fs::read_to_string(&path)?;
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        BTreeMap::new()
    };
    all.insert(context.to_owned(), serde_json::to_value(settings)?);
    let content = serde_json::to_string_pretty(&all)?;
    fs::write(&path, content)?;
    Ok(())
}

/// 037-node-shell: per-context Node Shell settings (`node-shell.json`).
pub fn node_shell_config_path() -> Result<PathBuf> {
    Ok(fathom_config_dir()?.join("node-shell.json"))
}

/// Load `context`'s Node Shell settings from `path`. Missing, unreadable or invalid input yields
/// defaults and leaves the file untouched; each field defaults independently (FR-015).
pub fn load_node_shell_settings_at(path: &Path, context: &str) -> NodeShellSettings {
    let Ok(content) = fs::read_to_string(path) else {
        return NodeShellSettings::default();
    };
    let Ok(all) = serde_json::from_str::<BTreeMap<String, serde_json::Value>>(&content) else {
        tracing::warn!("node-shell.json is not valid JSON; using defaults for {context}");
        return NodeShellSettings::default();
    };
    all.get(context)
        .and_then(|raw| serde_json::from_value::<NodeShellSettings>(raw.clone()).ok())
        .and_then(|s| s.normalized().ok())
        .unwrap_or_default()
}

/// Save `context`'s settings into `path`, leaving other contexts' entries untouched. Refuses to
/// overwrite a file that exists but is not valid JSON.
pub fn save_node_shell_settings_at(
    path: &Path,
    context: &str,
    settings: &NodeShellSettings,
) -> Result<()> {
    let settings = settings.clone().normalized().map_err(|e| anyhow!(e))?;
    let mut all: BTreeMap<String, serde_json::Value> = if path.exists() {
        serde_json::from_str(&fs::read_to_string(path)?)
            .map_err(|e| anyhow!("node-shell.json is not valid JSON; not overwriting it: {e}"))?
    } else {
        BTreeMap::new()
    };
    all.insert(context.to_owned(), serde_json::to_value(&settings)?);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, serde_json::to_string_pretty(&all)?)?;
    Ok(())
}

pub fn load_node_shell_settings(context: &str) -> NodeShellSettings {
    match node_shell_config_path() {
        Ok(p) => load_node_shell_settings_at(&p, context),
        Err(_) => NodeShellSettings::default(),
    }
}

pub fn save_node_shell_settings(context: &str, settings: &NodeShellSettings) -> Result<()> {
    save_node_shell_settings_at(&node_shell_config_path()?, context, settings)
}

/// `team.json`'s on-disk shape.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TeamConfig {
    #[serde(default)]
    pub store_path: Option<String>,
    /// Adopted connection profiles, keyed by profile id. Never deleted just because the
    /// shared store becomes unreachable or the upstream profile disappears — see
    /// `AdoptedConnectionProfile::still_shared` (Constitution Principle VI).
    #[serde(default)]
    pub adopted: BTreeMap<String, AdoptedConnectionProfile>,
    /// Highest org-settings `rev` per context this operator has already been shown
    /// (accepted or dismissed), so a dismissed publish doesn't re-prompt forever.
    #[serde(default)]
    pub org_settings_rev_seen: BTreeMap<String, u64>,
}

/// Load `team.json`. Unreadable/invalid yields defaults and leaves the file untouched.
pub fn load_team_config() -> TeamConfig {
    let path = match team_config_path() {
        Ok(p) => p,
        Err(_) => return TeamConfig::default(),
    };
    let Ok(content) = fs::read_to_string(&path) else {
        return TeamConfig::default();
    };
    serde_json::from_str(&content).unwrap_or_else(|e| {
        tracing::warn!("team.json is invalid; using defaults: {e}");
        TeamConfig::default()
    })
}

/// Save `team.json` in full (it's small and entirely owned by this feature).
pub fn save_team_config(config: &TeamConfig) -> Result<()> {
    let path = team_config_path()?;
    ensure_config_dir()?;
    let content = serde_json::to_string_pretty(config)?;
    fs::write(&path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_shell_settings_additive_defaults_and_invalid_preserved() {
        let dir = std::env::temp_dir().join(format!("fathom-ns-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("node-shell.json");
        let _ = fs::remove_file(&path);
        // Missing file -> defaults.
        assert_eq!(
            load_node_shell_settings_at(&path, "a"),
            NodeShellSettings::default()
        );
        // Missing keys default individually (additive).
        fs::write(&path, r#"{"a":{"image":"alpine:3"}}"#).unwrap();
        let s = load_node_shell_settings_at(&path, "a");
        assert_eq!(s.image, "alpine:3");
        assert_eq!(s.namespace, NodeShellSettings::default().namespace);
        // Round trip keeps other contexts.
        let b = NodeShellSettings {
            namespace: "kube-system".to_owned(),
            ..Default::default()
        };
        save_node_shell_settings_at(&path, "b", &b).unwrap();
        assert_eq!(load_node_shell_settings_at(&path, "a").image, "alpine:3");
        assert_eq!(
            load_node_shell_settings_at(&path, "b").namespace,
            "kube-system"
        );
        // Invalid file: defaults on load, untouched on save.
        fs::write(&path, "{ not json").unwrap();
        assert_eq!(
            load_node_shell_settings_at(&path, "a"),
            NodeShellSettings::default()
        );
        assert!(save_node_shell_settings_at(&path, "a", &b).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ not json");
        let _ = fs::remove_dir_all(&dir);
    }

    /// 015-mcp-agent-exposure (T039), the half of the default that needs no filesystem: an
    /// `AutoStartConfig` that nothing has ever written is *off*. `AutoStartConfig::load()`
    /// returns exactly this value when `~/.fathom/config.json` has no `auto_start` object, so
    /// "off by default" (FR-002) holds for a fresh install. The on-disk half of T039 —
    /// `load()` with no file at all, and a `save(true)` round-trip — lives in
    /// `tests/ui_config_persistence.rs`, which owns the `$HOME` temp-dir harness (`$HOME` is
    /// process-global, so it cannot be moved from inside the lib test binary).
    #[test]
    fn auto_start_defaults_to_the_mcp_server_off() {
        assert!(
            !AutoStartConfig::default().mcp_stdio,
            "the stdio MCP server must be off until an operator turns it on (FR-002)"
        );
    }

    /// The pre-015 `mcp_http` key is still read (the field was renamed in place, not
    /// re-introduced), but everything written from now on uses `mcp_stdio`.
    #[test]
    fn auto_start_reads_the_pre_015_key_and_writes_the_new_one() {
        let legacy: AutoStartConfig =
            serde_json::from_value(serde_json::json!({ "mcp_http": true })).expect("legacy shape");
        assert!(legacy.mcp_stdio);

        let written = serde_json::to_value(AutoStartConfig { mcp_stdio: true }).expect("serialize");
        assert_eq!(written, serde_json::json!({ "mcp_stdio": true }));
    }

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

// ---------------------------------------------------------------------------------------
// 038-sdk-login-ollama: non-secret AI-assist settings in `config.json` under `ai_assist`.
// Secrets never live here (they are in the OS credential store). Older files without the
// key load the defaults; unknown/other keys in the file are always left untouched.
// ---------------------------------------------------------------------------------------

use crate::protocol::v1::{AiAssistSettings, Provider};

/// Atomic (tmp + rename) JSON write so a crash never leaves a half-written file.
pub fn save_json_file_atomic(path: &Path, value: &serde_json::Value) -> Result<()> {
    ensure_config_dir()?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(value)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn ai_settings_from_config(config: &serde_json::Value) -> AiAssistSettings {
    config
        .get("ai_assist")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

/// Load the AI-assist settings from `path` (a `config.json`). A missing or unparsable file
/// yields defaults and is left untouched.
pub fn load_ai_settings_at(path: &Path) -> AiAssistSettings {
    match load_json_file(path) {
        Ok(config) => ai_settings_from_config(&config),
        Err(e) => {
            tracing::warn!(
                "Failed to read config.json; using AI-assist defaults: {}",
                e
            );
            AiAssistSettings::default()
        }
    }
}

/// Persist `settings` into `path`, preserving every other key. Refuses to write when the
/// existing file cannot be parsed.
pub fn save_ai_settings_at(path: &Path, settings: &AiAssistSettings) -> Result<()> {
    let mut config = load_json_file(path)?;
    let obj = config
        .as_object_mut()
        .ok_or_else(|| anyhow!("config.json is not a JSON object"))?;
    obj.insert("ai_assist".to_owned(), serde_json::to_value(settings)?);
    save_json_file_atomic(path, &config)
}

pub fn load_ai_settings() -> AiAssistSettings {
    main_config_path()
        .map(|p| load_ai_settings_at(&p))
        .unwrap_or_default()
}

pub fn save_ai_settings(settings: &AiAssistSettings) -> Result<()> {
    save_ai_settings_at(&main_config_path()?, settings)
}

/// Non-secret account labels (e.g. the GitHub login) so a connected provider still shows
/// its account after a restart. `config.json -> ai_assist_accounts`.
pub fn load_ai_account_labels_at(path: &Path) -> BTreeMap<String, String> {
    load_json_file(path)
        .ok()
        .and_then(|c| c.get("ai_assist_accounts").cloned())
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

pub fn save_ai_account_label_at(
    path: &Path,
    provider: Provider,
    label: Option<&str>,
) -> Result<()> {
    let mut config = load_json_file(path)?;
    let obj = config
        .as_object_mut()
        .ok_or_else(|| anyhow!("config.json is not a JSON object"))?;
    let entry = obj
        .entry("ai_assist_accounts")
        .or_insert_with(|| serde_json::json!({}));
    if !entry.is_object() {
        *entry = serde_json::json!({});
    }
    if let Some(map) = entry.as_object_mut() {
        match label {
            Some(l) => {
                map.insert(provider.as_str().to_owned(), serde_json::json!(l));
            }
            None => {
                map.remove(provider.as_str());
            }
        }
    }
    save_json_file_atomic(path, &config)
}

#[cfg(test)]
mod ai_settings_tests {
    use super::*;

    #[test]
    fn missing_or_old_config_loads_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        assert_eq!(load_ai_settings_at(&path), AiAssistSettings::default());
        fs::write(&path, r#"{"ui":{"theme":"x"}}"#).unwrap();
        assert_eq!(load_ai_settings_at(&path), AiAssistSettings::default());
    }

    #[test]
    fn save_preserves_other_keys_and_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, r#"{"auto_start":{"mcp_stdio":true},"ui":{"a":1}}"#).unwrap();
        let s = AiAssistSettings {
            active_provider: Some(Provider::Ollama),
            ollama_model: Some("llama3.2".into()),
            offline_mode: true,
            ..AiAssistSettings::default()
        };
        save_ai_settings_at(&path, &s).unwrap();
        assert_eq!(load_ai_settings_at(&path), s);
        let raw: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw["auto_start"]["mcp_stdio"], true);
        assert_eq!(raw["ui"]["a"], 1);
    }

    #[test]
    fn corrupt_config_is_not_clobbered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, "{ not json").unwrap();
        assert!(save_ai_settings_at(&path, &AiAssistSettings::default()).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ not json");
        assert_eq!(load_ai_settings_at(&path), AiAssistSettings::default());
    }

    #[test]
    fn account_labels_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        save_ai_account_label_at(&path, Provider::Github, Some("octocat")).unwrap();
        assert_eq!(
            load_ai_account_labels_at(&path)
                .get("github")
                .map(String::as_str),
            Some("octocat")
        );
        save_ai_account_label_at(&path, Provider::Github, None).unwrap();
        assert!(load_ai_account_labels_at(&path).is_empty());
    }
}
